//! GpuUiPrim / GpuPanelHeader layout contract (x00ba).
//!
//! Guarantees: 64-byte prims, Pod cast to storage buffers, DirectNumber fixed-point pack.
//! Non-guarantees: MSDF atlas contents; CRDT panel sync; scene entities per widget.

#![forbid(unsafe_code)]

use bytemuck::{Pod, Zeroable};

/// UI primitive kind packed into [`GpuUiPrim::panel_and_kind`] low byte of kind field.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiPrimKind {
    RoundedBox = 0,
    MsdfGlyph = 1,
    DirectNumber = 2,
    CubicBezierWire = 3,
}

/// 64-byte GPU UI primitive (one cache line).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuUiPrim {
    /// Panel-local rect `[x, y, w, h]` (Bezier: endpoints).
    pub rect: [f32; 4],
    /// Clip rect `[min_x, min_y, max_x, max_y]`.
    pub clip_rect: [f32; 4],
    /// Kind-specific payload (see architecture §6.1).
    pub payload: [u32; 4],
    /// `panel_index(u16) | kind(u8) << 16 | flags(u8) << 24`.
    pub panel_and_kind: u32,
    pub color_rgba: u32,
    pub border_rgba: u32,
    /// Hit reverse-lookup id (row / field).
    pub binding_id: u32,
}

const _: () = assert!(core::mem::size_of::<GpuUiPrim>() == 64);

/// One floating panel = one OBB board in world space.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuPanelHeader {
    pub world_matrix: [[f32; 4]; 4],
    /// `[width_px, height_px, width_m, height_m]`.
    pub dimensions: [f32; 4],
    pub prim_start_index: u32,
    pub prim_count: u32,
    pub lod_level: u32,
    pub _pad: u32,
}

const _: () = assert!(core::mem::size_of::<GpuPanelHeader>() == 96);

/// Pack panel index + kind + flags.
#[inline]
pub fn pack_panel_kind(panel: u16, kind: UiPrimKind, flags: u8) -> u32 {
    u32::from(panel) | ((kind as u32) << 16) | (u32::from(flags) << 24)
}

#[inline]
pub fn unpack_kind(panel_and_kind: u32) -> UiPrimKind {
    match (panel_and_kind >> 16) & 0xff {
        1 => UiPrimKind::MsdfGlyph,
        2 => UiPrimKind::DirectNumber,
        3 => UiPrimKind::CubicBezierWire,
        _ => UiPrimKind::RoundedBox,
    }
}

/// RGBA8 → packed u32 (R in low byte).
#[inline]
pub fn pack_rgba8(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (u32::from(a) << 24)
}

/// Pack `value` as DirectNumber payload without heap string alloc.
///
/// `payload = [int_lo, int_hi, frac_x10000, precision_and_sign]`
/// where `precision_and_sign` low 8 bits = decimal places (0..=4), bit 8 = negative.
#[inline]
pub fn pack_direct_number(value: f64, decimals: u8) -> [u32; 4] {
    let decimals = decimals.min(4);
    let neg = value < 0.0;
    let abs = value.abs();
    let scale = 10f64.powi(i32::from(decimals));
    let scaled = (abs * scale).round() as i64;
    let int_part = scaled / (scale as i64).max(1);
    let frac = if decimals == 0 {
        0
    } else {
        (scaled % (scale as i64).max(1)) as u32
    };
    // Scale frac to x10000 slot when decimals < 4.
    let frac_x10000 = match decimals {
        0 => 0,
        1 => frac * 1000,
        2 => frac * 100,
        3 => frac * 10,
        _ => frac,
    };
    let int_bits = int_part as u64;
    let mut flags = u32::from(decimals);
    if neg {
        flags |= 1 << 8;
    }
    [
        (int_bits & 0xffff_ffff) as u32,
        (int_bits >> 32) as u32,
        frac_x10000,
        flags,
    ]
}

/// Decode packed DirectNumber back to f64 (CPU tests / scrub preview).
#[inline]
#[cfg_attr(not(test), allow(dead_code))]
pub fn unpack_direct_number(payload: [u32; 4]) -> f64 {
    let int_part = (u64::from(payload[1]) << 32) | u64::from(payload[0]);
    let decimals = (payload[3] & 0xff) as i32;
    let neg = (payload[3] & (1 << 8)) != 0;
    let frac = payload[2] as f64 / 10_000.0;
    let v = int_part as f64 + frac;
    let v = if decimals > 0 {
        // frac_x10000 already absolute; int is truncated absolute.
        let scale = 10f64.powi(decimals);
        let scaled = (int_part as f64) * scale + (payload[2] as f64) * scale / 10_000.0;
        scaled.round() / scale
    } else {
        v
    };
    if neg {
        -v
    } else {
        v
    }
}

#[inline]
pub fn rounded_box_payload(corner_radius: f32, border_thickness: f32, glow: f32) -> [u32; 4] {
    [
        corner_radius.to_bits(),
        border_thickness.to_bits(),
        glow.to_bits(),
        0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prim_size_is_64() {
        assert_eq!(std::mem::size_of::<GpuUiPrim>(), 64);
        assert_eq!(std::mem::align_of::<GpuUiPrim>(), 16);
    }

    #[test]
    fn panel_header_size() {
        assert_eq!(std::mem::size_of::<GpuPanelHeader>(), 96);
    }

    #[test]
    fn direct_number_roundtrip_positive() {
        let p = pack_direct_number(12.3456, 3);
        let v = unpack_direct_number(p);
        assert!((v - 12.346).abs() < 0.002, "got {v}");
    }

    #[test]
    fn direct_number_roundtrip_negative() {
        let p = pack_direct_number(-1.5, 2);
        let v = unpack_direct_number(p);
        assert!((v + 1.5).abs() < 0.01, "got {v}");
    }

    #[test]
    fn pack_panel_kind_bits() {
        let v = pack_panel_kind(3, UiPrimKind::DirectNumber, 0x0a);
        assert_eq!(v & 0xffff, 3);
        assert_eq!(unpack_kind(v), UiPrimKind::DirectNumber);
        assert_eq!((v >> 24) & 0xff, 0x0a);
    }
}
