//! Spawn reference draft (x0055).
//!
//! # Guarantees
//! - [`ItemRef`] is a shared CAS spawn pointer: blake3 hex only in this Slice.
//! - [`ITEM_REF_SCHEMA_VERSION`] and serde `kind` tag leave room for `repo@rev`
//!   (roadmap x0063) without renaming the blake3 field.
//! - Valid values always store **lowercase** blake3 hex (`PartialEq` / encode が digest 同一性と一致).
//! - Safe construction: [`ItemRef::blake3`], [`ItemRef::decode_json`], and
//!   [`Deserialize`] (all normalize + validate). Prefer these over struct literals.
//!
//! # Non-guarantees
//! - `repo_id@rev` resolution (x0063).
//! - Carriage on Manifest / Frame / Intent (wire load is a later Slice).
//! - Spawn execution / BlobGrant checks.
//! - Struct-literal / field mutation staying valid (fields are `pub` for draft
//!   pattern matching only — call [`ItemRef::normalize`] afterward if needed).
//!
//! # PROTOCOL_PIN
//! Unchanged. This module is **not** a protobuf wire schema. Bump
//! [`crate::PROTOCOL_PIN`] only when a later Slice loads `ItemRef` onto a wire
//! message (or otherwise breaks peer compatibility).

use serde::{Deserialize, Serialize};

use crate::decode_limits::MAX_BLAKE3_HEX_BYTES;

/// Current draft schema for [`ItemRef`]. Bump when the JSON shape changes
/// incompatibly *before* wire carriage (still without PROTOCOL_PIN unless loaded).
pub const ITEM_REF_SCHEMA_VERSION: u32 = 1;

/// BLAKE3 digest as lowercase hex length (32 bytes → 64 nibbles).
///
/// Same ceiling as [`MAX_BLAKE3_HEX_BYTES`] (Manifest `SceneEntry.blake3`).
pub const BLAKE3_HEX_LEN: usize = MAX_BLAKE3_HEX_BYTES;

/// Why [`ItemRef`] construction / serde rejected a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemRefError {
    /// `schema_version` is not [`ITEM_REF_SCHEMA_VERSION`].
    UnsupportedSchemaVersion { got: u32 },
    /// blake3 is not exactly [`BLAKE3_HEX_LEN`] ASCII hex digits.
    InvalidBlake3Hex,
    /// Serde JSON encode or decode failure (unknown `kind`, truncated, etc.).
    Serde(String),
}

impl std::fmt::Display for ItemRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { got } => {
                write!(
                    f,
                    "unsupported ItemRef schema_version={got} (want {ITEM_REF_SCHEMA_VERSION})"
                )
            }
            Self::InvalidBlake3Hex => {
                write!(
                    f,
                    "ItemRef blake3 must be {BLAKE3_HEX_LEN} ASCII hex digits"
                )
            }
            Self::Serde(msg) => write!(f, "ItemRef serde: {msg}"),
        }
    }
}

impl std::error::Error for ItemRefError {}

/// Shared spawn reference (draft · not on protobuf wire yet).
///
/// Tagged by `kind` so x0063 can add a `repo_rev` arm without renaming
/// [`ItemRef::Blake3`]'s `blake3` field.
///
/// **Canonical API:** [`ItemRef::blake3`], [`ItemRef::decode_json`], serde
/// [`Deserialize`]. Variant fields are `pub` for matching in a draft, but a
/// hand-built value may be invalid until [`ItemRef::normalize`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemRef {
    /// CAS content-addressed blob (`GET /api/v1/blobs/{blake3}`).
    Blake3 {
        schema_version: u32,
        /// Lowercase hex of the 32-byte BLAKE3 digest (invariant after normalize).
        blake3: String,
    },
}

/// Wire shape before case / schema normalization (Deserialize only).
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ItemRefWire {
    Blake3 { schema_version: u32, blake3: String },
}

impl<'de> Deserialize<'de> for ItemRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ItemRefWire::deserialize(deserializer)?;
        ItemRef::from_wire(wire).map_err(serde::de::Error::custom)
    }
}

impl ItemRef {
    /// Build a blake3 ref; normalizes hex to lowercase.
    pub fn blake3(hex: impl AsRef<str>) -> Result<Self, ItemRefError> {
        Ok(Self::Blake3 {
            schema_version: ITEM_REF_SCHEMA_VERSION,
            blake3: normalize_blake3_hex(hex.as_ref())?,
        })
    }

    /// Draft schema version carried on this value.
    pub fn schema_version(&self) -> u32 {
        match self {
            Self::Blake3 { schema_version, .. } => *schema_version,
        }
    }

    /// CAS blake3 hex when this is a [`ItemRef::Blake3`].
    pub fn blake3_hex(&self) -> Option<&str> {
        match self {
            Self::Blake3 { blake3, .. } => Some(blake3.as_str()),
        }
    }

    /// Fail-closed check: schema + **already-lowercase** blake3 hex.
    ///
    /// Does not mutate. For mixed-case input use [`ItemRef::normalize`] or the
    /// constructors / [`Deserialize`].
    pub fn validate(&self) -> Result<(), ItemRefError> {
        match self {
            Self::Blake3 {
                schema_version,
                blake3,
            } => {
                if *schema_version != ITEM_REF_SCHEMA_VERSION {
                    return Err(ItemRefError::UnsupportedSchemaVersion {
                        got: *schema_version,
                    });
                }
                if !is_lowercase_blake3_hex(blake3) {
                    return Err(ItemRefError::InvalidBlake3Hex);
                }
                Ok(())
            }
        }
    }

    /// Normalize blake3 to lowercase and enforce schema_version.
    ///
    /// Use after a struct literal or field edit. Constructors / Deserialize
    /// already return a normalized value.
    pub fn normalize(self) -> Result<Self, ItemRefError> {
        match self {
            Self::Blake3 {
                schema_version,
                blake3,
            } => {
                if schema_version != ITEM_REF_SCHEMA_VERSION {
                    return Err(ItemRefError::UnsupportedSchemaVersion {
                        got: schema_version,
                    });
                }
                Ok(Self::Blake3 {
                    schema_version,
                    blake3: normalize_blake3_hex(&blake3)?,
                })
            }
        }
    }

    /// Compact JSON (draft interchange · not PROTOCOL_PIN wire).
    pub fn encode_json(&self) -> Result<String, ItemRefError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|e| ItemRefError::Serde(e.to_string()))
    }

    /// Parse JSON (normalizes + validates). Preserves typed [`ItemRefError`]
    /// (unlike bare `serde_json::from_str`, which folds failures into serde's error).
    pub fn decode_json(s: &str) -> Result<Self, ItemRefError> {
        let wire: ItemRefWire =
            serde_json::from_str(s).map_err(|e| ItemRefError::Serde(e.to_string()))?;
        Self::from_wire(wire)
    }

    fn from_wire(wire: ItemRefWire) -> Result<Self, ItemRefError> {
        match wire {
            ItemRefWire::Blake3 {
                schema_version,
                blake3,
            } => Self::Blake3 {
                schema_version,
                blake3,
            }
            .normalize(),
        }
    }
}

fn is_blake3_hex(value: &str) -> bool {
    value.len() == BLAKE3_HEX_LEN && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_lowercase_blake3_hex(value: &str) -> bool {
    is_blake3_hex(value) && value.bytes().all(|b| !b.is_ascii_uppercase())
}

fn normalize_blake3_hex(value: &str) -> Result<String, ItemRefError> {
    if !is_blake3_hex(value) {
        return Err(ItemRefError::InvalidBlake3Hex);
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn blake3_hex_len_matches_decode_limits() {
        assert_eq!(BLAKE3_HEX_LEN, MAX_BLAKE3_HEX_BYTES);
        assert_eq!(BLAKE3_HEX_LEN, 64);
    }

    #[test]
    fn blake3_ctor_normalizes_case() {
        let upper = SAMPLE.to_ascii_uppercase();
        let r = ItemRef::blake3(&upper).expect("valid");
        assert_eq!(r.schema_version(), ITEM_REF_SCHEMA_VERSION);
        assert_eq!(r.blake3_hex(), Some(SAMPLE));
        r.validate().expect("normalized is valid");
    }

    #[test]
    fn blake3_rejects_bad_length_and_charset() {
        assert_eq!(ItemRef::blake3("abc"), Err(ItemRefError::InvalidBlake3Hex));
        assert_eq!(
            ItemRef::blake3("g".repeat(64)),
            Err(ItemRefError::InvalidBlake3Hex)
        );
    }

    #[test]
    fn json_roundtrip() {
        let r = ItemRef::blake3(SAMPLE).unwrap();
        let json = r.encode_json().unwrap();
        assert!(json.contains("\"kind\":\"blake3\""));
        assert!(json.contains("\"schema_version\":1"));
        let back = ItemRef::decode_json(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn decode_and_deserialize_normalize_uppercase_blake3() {
        let json = format!(
            r#"{{"kind":"blake3","schema_version":1,"blake3":"{}"}}"#,
            SAMPLE.to_ascii_uppercase()
        );
        let via_api = ItemRef::decode_json(&json).unwrap();
        let via_serde: ItemRef = serde_json::from_str(&json).unwrap();
        assert_eq!(via_api.blake3_hex(), Some(SAMPLE));
        assert_eq!(via_serde, via_api);
        let encoded = via_api.encode_json().unwrap();
        assert!(encoded.contains(SAMPLE));
        assert!(!encoded.contains(&SAMPLE.to_ascii_uppercase()));
    }

    #[test]
    fn validate_rejects_uppercase_normalize_fixes() {
        let raw = ItemRef::Blake3 {
            schema_version: ITEM_REF_SCHEMA_VERSION,
            blake3: SAMPLE.to_ascii_uppercase(),
        };
        assert_eq!(raw.validate(), Err(ItemRefError::InvalidBlake3Hex));
        let fixed = raw.normalize().unwrap();
        assert_eq!(fixed.blake3_hex(), Some(SAMPLE));
        fixed.validate().unwrap();
    }

    #[test]
    fn decode_rejects_unknown_kind_and_bad_schema() {
        let unknown = r#"{"kind":"repo_rev","schema_version":1,"repo_id":"x","rev":"main"}"#;
        assert!(matches!(
            ItemRef::decode_json(unknown),
            Err(ItemRefError::Serde(_))
        ));

        let bad_ver = format!(r#"{{"kind":"blake3","schema_version":99,"blake3":"{SAMPLE}"}}"#);
        assert_eq!(
            ItemRef::decode_json(&bad_ver),
            Err(ItemRefError::UnsupportedSchemaVersion { got: 99 })
        );
    }

    #[test]
    fn blake3_field_name_stable_in_json() {
        // x0063 must extend via new `kind`, not rename this field.
        let json = ItemRef::blake3(SAMPLE).unwrap().encode_json().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("blake3").is_some());
        assert!(v.get("blake3_hex").is_none());
    }
}
