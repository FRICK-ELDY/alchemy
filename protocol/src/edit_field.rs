//! EditSetField value encodings + BLAKE3 optimistic-lock hashes (x00b3).

use crate::edit_field_id;

/// Encode FLAGS field (`u32` LE).
pub fn encode_flags(flags: u32) -> [u8; 4] {
    flags.to_le_bytes()
}

/// Decode FLAGS; `None` if length ≠ 4.
pub fn decode_flags(value: &[u8]) -> Option<u32> {
    let b: [u8; 4] = value.try_into().ok()?;
    Some(u32::from_le_bytes(b))
}

/// Encode COLOR_RGBA (`4×f32` LE).
pub fn encode_color_rgba(rgba: [f32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, c) in rgba.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
    }
    out
}

/// Decode COLOR_RGBA; `None` if length ≠ 16 or any component non-finite.
pub fn decode_color_rgba(value: &[u8]) -> Option<[f32; 4]> {
    if value.len() != 16 {
        return None;
    }
    let mut rgba = [0f32; 4];
    for (i, slot) in rgba.iter_mut().enumerate() {
        let b: [u8; 4] = value[i * 4..i * 4 + 4].try_into().ok()?;
        let v = f32::from_le_bytes(b);
        if !v.is_finite() {
            return None;
        }
        *slot = v;
    }
    Some(rgba)
}

/// BLAKE3-256 of a SetField value blob (optimistic lock).
pub fn field_value_hash(value: &[u8]) -> [u8; 32] {
    *blake3::hash(value).as_bytes()
}

/// Hash for the current FLAGS value.
pub fn hash_flags(flags: u32) -> [u8; 32] {
    field_value_hash(&encode_flags(flags))
}

/// Hash for the current COLOR_RGBA value.
pub fn hash_color_rgba(rgba: [f32; 4]) -> [u8; 32] {
    field_value_hash(&encode_color_rgba(rgba))
}

/// Hash for the current BLAKE3 hex string (UTF-8 bytes as stored on the wire value).
pub fn hash_blake3_hex(hex: &str) -> [u8; 32] {
    field_value_hash(hex.as_bytes())
}

/// True when `field_id` is a known x00b3 SetField id.
pub fn known_field_id(field_id: u32) -> bool {
    matches!(
        field_id,
        edit_field_id::FLAGS | edit_field_id::COLOR_RGBA | edit_field_id::BLAKE3
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_roundtrip_and_hash_stable() {
        let enc = encode_flags(0x11);
        assert_eq!(decode_flags(&enc), Some(0x11));
        assert_eq!(hash_flags(0x11), field_value_hash(&enc));
    }

    #[test]
    fn color_rejects_non_finite() {
        assert!(decode_color_rgba(&encode_color_rgba([1.0, 0.0, 0.0, 1.0])).is_some());
        let mut bad = encode_color_rgba([1.0, 0.0, 0.0, 1.0]);
        bad[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode_color_rgba(&bad).is_none());
    }
}
