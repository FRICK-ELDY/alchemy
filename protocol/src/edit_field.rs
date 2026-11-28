//! EditSetField value encodings + BLAKE3 optimistic-lock hashes (x00b3 / x00f2).

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

/// Encode POS axis (`f64` LE) — Inspector world position (x00f2).
pub fn encode_f64(v: f64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Decode POS axis; `None` if length ≠ 8 or non-finite.
pub fn decode_f64(value: &[u8]) -> Option<f64> {
    let b: [u8; 8] = value.try_into().ok()?;
    let v = f64::from_le_bytes(b);
    v.is_finite().then_some(v)
}

/// General-purpose BLAKE3-256 of arbitrary bytes.
///
/// Prefer this over [`field_value_hash`] when the caller is **not** SetField
/// optimistic locking (e.g. op_chain digests). Same algorithm; clearer ownership.
pub fn blake3_hash_32(value: &[u8]) -> [u8; 32] {
    *blake3::hash(value).as_bytes()
}

/// BLAKE3-256 of a SetField value blob (optimistic lock).
///
/// Thin alias of [`blake3_hash_32`] — SetField callers keep this name.
pub fn field_value_hash(value: &[u8]) -> [u8; 32] {
    blake3_hash_32(value)
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

/// Hash for a POS axis (`f64` LE blob).
pub fn hash_f64(v: f64) -> [u8; 32] {
    field_value_hash(&encode_f64(v))
}

/// True when `field_id` is a known SetField id (x00b3 + x00f2 POS).
pub fn known_field_id(field_id: u32) -> bool {
    matches!(
        field_id,
        edit_field_id::FLAGS
            | edit_field_id::COLOR_RGBA
            | edit_field_id::BLAKE3
            | edit_field_id::POS_X
            | edit_field_id::POS_Y
            | edit_field_id::POS_Z
    )
}

/// True when `field_id` is an Inspector world-position axis (x00f2).
#[inline]
pub fn is_pos_field_id(field_id: u32) -> bool {
    matches!(
        field_id,
        edit_field_id::POS_X | edit_field_id::POS_Y | edit_field_id::POS_Z
    )
}

/// Map POS field_id → axis index 0..2.
#[inline]
pub fn pos_field_axis(field_id: u32) -> Option<usize> {
    match field_id {
        edit_field_id::POS_X => Some(0),
        edit_field_id::POS_Y => Some(1),
        edit_field_id::POS_Z => Some(2),
        _ => None,
    }
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

    #[test]
    fn f64_pos_roundtrip_and_rejects_nan() {
        let enc = encode_f64(1.25);
        assert_eq!(decode_f64(&enc), Some(1.25));
        assert_eq!(hash_f64(1.25), field_value_hash(&enc));
        assert!(decode_f64(&f64::NAN.to_le_bytes()).is_none());
        assert!(known_field_id(edit_field_id::POS_X));
        assert_eq!(pos_field_axis(edit_field_id::POS_Y), Some(1));
    }
}
