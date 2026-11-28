//! Tier-1 Static MSDF atlas (x00ed / feel T1).
//!
//! Guarantees: 512² RGBA8 resident atlas; ASCII printable + basic icons; UV lookup;
//! MsdfGlyph payload pack; bake without new crates (pseudo-MSDF R=G=B from 5×7 bits).
//! Atlas cell: `v0` is glyph **top** (image row 0). Board local UV is bottom-left / y-up —
//! `uber_sdf::sample_msdf_rect` flips V; bake / UV pack stay image-space.
//! Non-guarantees: true msdfgen multi-channel quality; CJK / emoji / LRU dynamic atlas;
//! message catalog / locale.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use super::prim::{pack_panel_kind, GpuUiPrim, UiPrimKind};

pub const ATLAS_SIZE: u32 = 512;
pub const CELL: u32 = 32;
pub const GRID: u32 = ATLAS_SIZE / CELL; // 16

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

/// Pad inside each cell so SDF edges stay away from neighbor cells.
pub const CELL_PAD: u32 = 4;

/// First atlas slot reserved for ASCII `0x20` (space).
pub const ASCII_BASE: u8 = 0x20;
/// Printable ASCII count `0x20..=0x7E`.
const ASCII_COUNT: u32 = 95;
/// Icon slots start after printable ASCII.
const ICON_BASE_SLOT: u32 = ASCII_COUNT;

/// Constant AA half-width in SDF space (mid=0.5). Shared with `uber_sdf.wgsl`.
#[cfg_attr(not(test), allow(dead_code))]
pub const MSDF_AA_HALF_WIDTH: f32 = 0.06;
/// Exact WGSL decimal for [`MSDF_AA_HALF_WIDTH`] (SSoT for sync; avoid Display rounding).
#[cfg_attr(not(test), allow(dead_code))]
pub const MSDF_AA_HALF_WIDTH_WGSL: &str = "0.06";

static ATLAS_CACHE: OnceLock<Vec<u8>> = OnceLock::new();

/// WGSL `const MSDF_ATLAS` line — must appear verbatim in `uber_sdf.wgsl`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_size_wgsl_const() -> String {
    format!("const MSDF_ATLAS: f32 = {ATLAS_SIZE}.0;")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_cell_wgsl_const() -> String {
    format!("const MSDF_CELL: f32 = {CELL}.0;")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_pad_wgsl_const() -> String {
    format!("const MSDF_PAD: f32 = {CELL_PAD}.0;")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_grid_wgsl_const() -> String {
    format!("const MSDF_GRID: u32 = {GRID}u;")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_ascii_base_wgsl_const() -> String {
    format!("const MSDF_ASCII_BASE: u32 = 0x{ASCII_BASE:02x}u;")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn atlas_aa_half_wgsl_const() -> String {
    format!("const MSDF_AA_HALF: f32 = {MSDF_AA_HALF_WIDTH_WGSL};")
}

/// Basic UI icons (hand-drawn 5×7), independent of system_ui.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconId {
    Close = 0,
    Check = 1,
    ChevronRight = 2,
    ChevronDown = 3,
}

impl IconId {
    #[inline]
    fn slot(self) -> u32 {
        ICON_BASE_SLOT + u32::from(self as u8)
    }
}

/// Pack MsdfGlyph UV rect as payload `[u0, v0, u1, v1]` (f32 bits).
#[inline]
pub fn msdf_glyph_payload(uv: [f32; 4]) -> [u32; 4] {
    [
        uv[0].to_bits(),
        uv[1].to_bits(),
        uv[2].to_bits(),
        uv[3].to_bits(),
    ]
}

/// Atlas UV for printable ASCII, or `None` if outside Tier-1 set / missing bits.
#[inline]
pub fn uv_for_ascii(c: u8) -> Option<[f32; 4]> {
    if !(ASCII_BASE..=0x7E).contains(&c) {
        return None;
    }
    if c != b' ' && glyph_bits(c).is_none() {
        return None;
    }
    Some(uv_for_slot(u32::from(c - ASCII_BASE)))
}

#[inline]
#[cfg_attr(not(test), allow(dead_code))]
pub fn uv_for_icon(id: IconId) -> [f32; 4] {
    uv_for_slot(id.slot())
}

#[inline]
fn uv_for_slot(slot: u32) -> [f32; 4] {
    let col = slot % GRID;
    let row = slot / GRID;
    let s = ATLAS_SIZE as f32;
    let cell = CELL as f32;
    let pad = CELL_PAD as f32;
    let u0 = (col as f32 * cell + pad) / s;
    let v0 = (row as f32 * cell + pad) / s;
    let u1 = (col as f32 * cell + cell - pad) / s;
    let v1 = (row as f32 * cell + cell - pad) / s;
    [u0, v0, u1, v1]
}

/// Resident Static MSDF atlas (RGBA8). Baked once process-wide (`OnceLock`).
///
/// Channels are identical (degenerate MSDF) so the WGSL median path stays valid
/// when a true multi-channel bake is swapped in later.
pub fn bake_static_atlas() -> &'static [u8] {
    ATLAS_CACHE
        .get_or_init(bake_static_atlas_uncached)
        .as_slice()
}

fn bake_static_atlas_uncached() -> Vec<u8> {
    let n = (ATLAS_SIZE * ATLAS_SIZE * 4) as usize;
    let mut rgba = vec![0u8; n];
    for c in ASCII_BASE..=0x7E {
        let slot = u32::from(c - ASCII_BASE);
        let bits = if c == b' ' {
            [0u8; GLYPH_H]
        } else {
            match glyph_bits(c) {
                Some(b) => b,
                None => continue,
            }
        };
        bake_slot(&mut rgba, slot, &bits);
    }
    for id in [
        IconId::Close,
        IconId::Check,
        IconId::ChevronRight,
        IconId::ChevronDown,
    ] {
        bake_slot(&mut rgba, id.slot(), &icon_bits(id));
    }
    rgba
}

fn bake_slot(rgba: &mut [u8], slot: u32, bits: &[u8; GLYPH_H]) {
    let col0 = ((slot % GRID) * CELL) as i32;
    let row0 = ((slot / GRID) * CELL) as i32;
    let cell = CELL as i32;
    let pad = CELL_PAD as i32;
    let inner = cell - 2 * pad;
    // Binary mask at cell resolution (1 = inside glyph).
    let mut mask = [false; (CELL * CELL) as usize];
    for y in 0..cell {
        for x in 0..cell {
            let lx = x - pad;
            let ly = y - pad;
            if lx < 0 || ly < 0 || lx >= inner || ly >= inner {
                continue;
            }
            let gx = (lx * GLYPH_W as i32) / inner;
            let gy = (ly * GLYPH_H as i32) / inner;
            if !(0..GLYPH_W as i32).contains(&gx) || !(0..GLYPH_H as i32).contains(&gy) {
                continue;
            }
            let row_bits = bits[gy as usize];
            let on = row_bits & (1 << (GLYPH_W - 1 - gx as usize)) != 0;
            mask[(y * cell + x) as usize] = on;
        }
    }
    // Brute-force SDF within the cell (small; init-only).
    let max_r = (inner as f32) * 0.5;
    for y in 0..cell {
        for x in 0..cell {
            let inside = mask[(y * cell + x) as usize];
            let mut best = f32::MAX;
            let search = 8i32;
            for dy in -search..=search {
                for dx in -search..=search {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx < 0 || ny < 0 || nx >= cell || ny >= cell {
                        continue;
                    }
                    let other = mask[(ny * cell + nx) as usize];
                    if other != inside {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        if d < best {
                            best = d;
                        }
                    }
                }
            }
            if best == f32::MAX {
                best = max_r;
            }
            let signed = if inside { best } else { -best };
            // Map to 0..1 with mid at 0.5 (MSDF-style).
            let t = (signed / max_r).clamp(-1.0, 1.0);
            let v = ((t * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            let ax = (col0 + x) as u32;
            let ay = (row0 + y) as u32;
            let i = ((ay * ATLAS_SIZE + ax) * 4) as usize;
            rgba[i] = v;
            rgba[i + 1] = v;
            rgba[i + 2] = v;
            rgba[i + 3] = 255;
        }
    }
}

/// Args for [`push_ascii_glyphs`] (keeps the call site under clippy's arity limit).
pub struct AsciiGlyphBlit<'a> {
    pub panel: u16,
    pub origin_x: f32,
    pub origin_y: f32,
    pub glyph_w: f32,
    pub glyph_h: f32,
    pub clip: [f32; 4],
    pub color_rgba: u32,
    pub binding_id: u32,
    pub text: &'a str,
}

/// Emit one MsdfGlyph prim per **ASCII** character in `text` (Tier-1 labels only).
///
/// Non-ASCII codepoints are skipped without advancing the cursor (no UTF-8 byte gaps).
/// Debug builds assert the whole string is ASCII. Returns number of prims pushed.
pub fn push_ascii_glyphs(out: &mut Vec<GpuUiPrim>, args: AsciiGlyphBlit<'_>) -> usize {
    debug_assert!(
        args.text.is_ascii(),
        "Tier-1 MsdfGlyph labels must be ASCII; got {:?}",
        args.text
    );
    let mut x = args.origin_x;
    let mut n = 0usize;
    for ch in args.text.chars() {
        if !ch.is_ascii() {
            continue;
        }
        let b = ch as u8;
        if let Some(uv) = uv_for_ascii(b) {
            out.push(GpuUiPrim {
                rect: [x, args.origin_y, args.glyph_w, args.glyph_h],
                clip_rect: args.clip,
                payload: msdf_glyph_payload(uv),
                panel_and_kind: pack_panel_kind(args.panel, UiPrimKind::MsdfGlyph, 0),
                color_rgba: args.color_rgba,
                border_rgba: 0,
                binding_id: args.binding_id,
            });
            n += 1;
        }
        // Advance only for ASCII codepoints (missing glyph still reserves advance).
        x += args.glyph_w * 0.7;
    }
    n
}

fn icon_bits(id: IconId) -> [u8; GLYPH_H] {
    match id {
        // X
        IconId::Close => [0x11, 0x0A, 0x04, 0x04, 0x0A, 0x11, 0],
        // check mark
        IconId::Check => [0, 0x01, 0x02, 0x14, 0x08, 0, 0],
        // >
        IconId::ChevronRight => [0x08, 0x04, 0x02, 0x04, 0x08, 0, 0],
        // v
        IconId::ChevronDown => [0, 0x11, 0x0A, 0x04, 0, 0, 0],
    }
}

/// Compact 5×7 bits for Tier-1 ASCII (mirrors Dash font coverage; Lens-local copy).
fn glyph_bits(c: u8) -> Option<[u8; GLYPH_H]> {
    Some(match c {
        b' ' => [0, 0, 0, 0, 0, 0, 0],
        b'!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
        b'"' => [0x0A, 0x0A, 0, 0, 0, 0, 0],
        b'#' => [0x0A, 0x1F, 0x0A, 0x0A, 0x1F, 0x0A, 0],
        b'$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        b'%' => [0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13],
        b'&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        b'\'' => [0x04, 0x04, 0, 0, 0, 0, 0],
        b'(' => [0x04, 0x08, 0x10, 0x10, 0x10, 0x08, 0x04],
        b')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        b'*' => [0, 0x04, 0x15, 0x0E, 0x15, 0x04, 0],
        b'+' => [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
        b',' => [0, 0, 0, 0, 0x04, 0x04, 0x08],
        b'-' => [0, 0, 0, 0x1F, 0, 0, 0],
        b'.' => [0, 0, 0, 0, 0, 0x04, 0],
        b'/' => [0x01, 0x02, 0x04, 0x04, 0x08, 0x10, 0x10],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        b'3' => [0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        b':' => [0, 0x04, 0, 0x04, 0, 0, 0],
        b';' => [0, 0x04, 0, 0x04, 0x04, 0x08, 0],
        b'<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        b'=' => [0, 0, 0x1F, 0, 0x1F, 0, 0],
        b'>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        b'?' => [0x0E, 0x11, 0x01, 0x06, 0x04, 0, 0x04],
        b'@' => [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E],
        b'A' | b'a' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' | b'b' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' | b'c' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' | b'd' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        b'E' | b'e' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' | b'f' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' | b'g' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' | b'h' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' | b'i' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' | b'j' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0E],
        b'K' | b'k' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' | b'l' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' | b'm' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' | b'n' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' | b'o' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' | b'p' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' | b'q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' | b'r' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' | b's' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' | b't' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' | b'u' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' | b'v' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' | b'w' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        b'X' | b'x' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' | b'y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        b'Z' | b'z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'[' => [0x1E, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1E],
        b'\\' => [0x10, 0x10, 0x08, 0x04, 0x04, 0x02, 0x01],
        b']' => [0x1E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x1E],
        b'^' => [0x04, 0x0A, 0x11, 0, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 0x1F],
        b'`' => [0x08, 0x04, 0, 0, 0, 0, 0],
        b'{' => [0x06, 0x08, 0x08, 0x10, 0x08, 0x08, 0x06],
        b'|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'}' => [0x0C, 0x02, 0x02, 0x01, 0x02, 0x02, 0x0C],
        b'~' => [0, 0x08, 0x15, 0x02, 0, 0, 0],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_size_and_coverage() {
        let atlas = bake_static_atlas();
        assert_eq!(atlas.len(), (ATLAS_SIZE * ATLAS_SIZE * 4) as usize);
        // Second call must hit the process-wide cache (same pointer).
        assert!(std::ptr::eq(atlas.as_ptr(), bake_static_atlas().as_ptr()));
        // Digit '0' cell should have mid-gray SDF values (not all zero / all 255).
        let uv = uv_for_ascii(b'0').expect("digit 0");
        assert!(sample_coverage(&atlas, uv) > 8);
        let uv_a = uv_for_ascii(b'A').expect("A");
        assert!(sample_coverage(&atlas, uv_a) > 8);
        let uv_m = uv_for_ascii(b'-').expect("minus");
        assert!(sample_coverage(&atlas, uv_m) > 2);
    }

    #[test]
    fn ascii_uv_and_unknown() {
        assert!(uv_for_ascii(b'9').is_some());
        assert!(uv_for_ascii(0x01).is_none());
        assert!(uv_for_ascii(0x7F).is_none());
        let icon = uv_for_icon(IconId::Close);
        assert!(icon[2] > icon[0] && icon[3] > icon[1]);
    }

    #[test]
    fn aa_half_wgsl_literal_matches_f32() {
        let parsed: f32 = MSDF_AA_HALF_WIDTH_WGSL.parse().expect("AA literal");
        assert_eq!(parsed, MSDF_AA_HALF_WIDTH);
        assert_eq!(
            atlas_aa_half_wgsl_const(),
            format!("const MSDF_AA_HALF: f32 = {MSDF_AA_HALF_WIDTH_WGSL};")
        );
    }

    #[test]
    fn push_ascii_emits_msdf_prims() {
        let mut out = Vec::new();
        let n = push_ascii_glyphs(
            &mut out,
            AsciiGlyphBlit {
                panel: 0,
                origin_x: 0.0,
                origin_y: 0.0,
                glyph_w: 12.0,
                glyph_h: 16.0,
                clip: [0.0, 0.0, 100.0, 100.0],
                color_rgba: 0xffff_ffff,
                binding_id: 0,
                text: "ID",
            },
        );
        assert_eq!(n, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(
            super::super::prim::unpack_kind(out[0].panel_and_kind),
            UiPrimKind::MsdfGlyph
        );
    }

    fn sample_coverage(atlas: &[u8], uv: [f32; 4]) -> usize {
        let s = ATLAS_SIZE as f32;
        let x0 = (uv[0] * s) as u32;
        let y0 = (uv[1] * s) as u32;
        let x1 = (uv[2] * s) as u32;
        let y1 = (uv[3] * s) as u32;
        let mut mid = 0usize;
        for y in y0..y1.min(ATLAS_SIZE) {
            for x in x0..x1.min(ATLAS_SIZE) {
                let i = ((y * ATLAS_SIZE + x) * 4) as usize;
                let v = atlas[i];
                if (64..192).contains(&v) {
                    mid += 1;
                }
            }
        }
        mid
    }
}
