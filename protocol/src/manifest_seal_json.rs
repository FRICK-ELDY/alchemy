//! JSON → canonical [`SceneManifest`] for `manifest-seal-hash` (x0039 / x0075 / x0079).
//!
//! Shared by the CLI binary and DemoScene vector tests so seal input schema
//! stays in one place.

use serde::Deserialize;

use crate::wire_crypto::manifest_seal_blake3_hex;
use crate::{
    normalize_scene_entry_trs, LocomotionConfig, LocomotionMode, SceneEntry, SceneManifest,
    PROTOCOL_PIN,
};

/// Seal-hash CLI / Port JSON body.
#[derive(Debug, Deserialize)]
pub struct ManifestSealJson {
    pub room_id: String,
    pub revision: u64,
    /// World-assigned host generation (x003a). Required (>0) for canonical seal.
    pub host_generation: u64,
    #[serde(default)]
    pub protocol_pin: Option<String>,
    /// 64-char hex of 32-byte Ed25519 host_pub.
    pub host_pub_hex: String,
    pub host_attestation: String,
    #[serde(default)]
    pub locomotion: Option<LocomotionJson>,
    #[serde(default)]
    pub entries: Vec<ManifestSealEntryJson>,
}

/// Locomotion object inside seal JSON.
#[derive(Debug, Deserialize)]
pub struct LocomotionJson {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub move_speed: f32,
}

/// One scene entry inside seal JSON.
#[derive(Debug, Deserialize)]
pub struct ManifestSealEntryJson {
    pub entity_id: u32,
    #[serde(default)]
    pub blake3: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    #[serde(default)]
    pub half_extent: f32,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub r: f32,
    #[serde(default)]
    pub g: f32,
    #[serde(default)]
    pub b: f32,
    #[serde(default)]
    pub a: f32,
    #[serde(default)]
    pub qx: f64,
    #[serde(default)]
    pub qy: f64,
    #[serde(default)]
    pub qz: f64,
    #[serde(default)]
    pub qw: f64,
    #[serde(default)]
    pub sx: f64,
    #[serde(default)]
    pub sy: f64,
    #[serde(default)]
    pub sz: f64,
}

fn loco_mode(mode: &str, quiet: bool) -> LocomotionMode {
    match mode.trim().to_ascii_lowercase().as_str() {
        "fly" => LocomotionMode::Fly,
        "walk" => LocomotionMode::Walk,
        "noclip" | "" => LocomotionMode::Noclip,
        other => {
            if !quiet {
                eprintln!("manifest-seal-hash: unknown locomotion mode {other:?}; using noclip");
            }
            LocomotionMode::Noclip
        }
    }
}

fn decode_host_pub_hex(hex_str: &str) -> Result<Vec<u8>, String> {
    let down = hex_str.trim().to_ascii_lowercase();
    if down.len() != 64 || !down.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "host_pub_hex must be 64 hex chars (got len={})",
            down.len()
        ));
    }
    hex::decode(&down).map_err(|e| format!("host_pub_hex decode: {e}"))
}

fn entry_from_json(e: ManifestSealEntryJson) -> SceneEntry {
    let mut entry = SceneEntry {
        entity_id: e.entity_id,
        blake3: e.blake3,
        x: e.x,
        y: e.y,
        z: e.z,
        half_extent: e.half_extent,
        flags: e.flags,
        r: e.r,
        g: e.g,
        b: e.b,
        a: e.a,
        qx: e.qx,
        qy: e.qy,
        qz: e.qz,
        qw: e.qw,
        sx: e.sx,
        sy: e.sy,
        sz: e.sz,
    };
    normalize_scene_entry_trs(&mut entry);
    entry
}

/// Parse seal JSON into a [`SceneManifest`] (same path as `manifest-seal-hash`).
pub fn scene_manifest_from_json_str(raw: &str, quiet: bool) -> Result<SceneManifest, String> {
    let input: ManifestSealJson =
        serde_json::from_str(raw).map_err(|err| format!("invalid JSON: {err}"))?;
    let host_pub = decode_host_pub_hex(&input.host_pub_hex)?;
    if input.host_generation == 0 {
        return Err("host_generation must be > 0".into());
    }
    let pin = input
        .protocol_pin
        .unwrap_or_else(|| PROTOCOL_PIN.to_string());
    let loco = input.locomotion.map(|l| LocomotionConfig {
        mode: loco_mode(&l.mode, quiet).into(),
        move_speed: l.move_speed,
    });
    let entries: Vec<SceneEntry> = input.entries.into_iter().map(entry_from_json).collect();
    Ok(SceneManifest {
        room_id: input.room_id,
        revision: input.revision,
        entries,
        locomotion: loco,
        protocol_pin: pin,
        host_pub,
        host_attestation: input.host_attestation,
        world_seal: String::new(),
        host_generation: input.host_generation,
    })
}

/// BLAKE3 hex of seal JSON via the canonical protobuf encode path.
pub fn blake3_hex_from_json_str(raw: &str, quiet: bool) -> Result<String, String> {
    let manifest = scene_manifest_from_json_str(raw, quiet)?;
    Ok(manifest_seal_blake3_hex(&manifest))
}

/// Parse raw bytes as UTF-8 seal JSON and hash.
pub fn blake3_hex_from_json_bytes(raw: &[u8], quiet: bool) -> Result<String, String> {
    let text = std::str::from_utf8(raw).map_err(|e| format!("utf8: {e}"))?;
    blake3_hex_from_json_str(text, quiet)
}
