// Spatial Lens Uber-SDF (x00ba) + Static MSDF (x00ed). Layout matches GpuUiPrim / GpuPanelHeader.
// Entry: vs_panel_quad / fs_uber_sdf
// Atlas layout must match spatial_lens::msdf (512², cell 32, ASCII 0x20 base).
// Board UV: bottom-left origin, y up. Output: premultiplied RGBA.

struct GpuUiPrim {
    rect: vec4<f32>,
    clip_rect: vec4<f32>,
    payload: vec4<u32>,
    panel_and_kind: u32,
    color_rgba: u32,
    border_rgba: u32,
    binding_id: u32,
}

struct GpuPanelHeader {
    world_matrix: mat4x4<f32>,
    dimensions: vec4<f32>, // w_px, h_px, w_m, h_m
    prim_start_index: u32,
    prim_count: u32,
    lod_level: u32,
    _pad: u32,
}

struct FrameUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<storage, read> panels: array<GpuPanelHeader>;
@group(0) @binding(2) var<storage, read> prims: array<GpuUiPrim>;
@group(0) @binding(3) var msdf_atlas: texture_2d<f32>;
@group(0) @binding(4) var msdf_samp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) panel_uv: vec2<f32>, // 0..1 on board
    @location(1) @interpolate(flat) panel_index: u32,
}

fn unpack_rgba(c: u32) -> vec4<f32> {
    let r = f32(c & 0xffu) / 255.0;
    let g = f32((c >> 8u) & 0xffu) / 255.0;
    let b = f32((c >> 16u) & 0xffu) / 255.0;
    let a = f32((c >> 24u) & 0xffu) / 255.0;
    return vec4(r, g, b, a);
}

fn to_premul(c: vec4<f32>) -> vec4<f32> {
    return vec4(c.rgb * c.a, c.a);
}

// Premultiplied src over dst (both premul). Matches wgpu PREMULTIPLIED_ALPHA_BLENDING.
fn over_premul(dst: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    return src + dst * (1.0 - src.a);
}

@vertex
fn vs_panel_quad(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) panel_index: u32,
) -> VsOut {
    // Full-board quad in local meters, Z=0 face.
    var corners = array<vec2<f32>, 6>(
        vec2(-0.5, -0.5),
        vec2(0.5, -0.5),
        vec2(0.5, 0.5),
        vec2(-0.5, -0.5),
        vec2(0.5, 0.5),
        vec2(-0.5, 0.5),
    );
    let c = corners[vid];
    let p = panels[panel_index];
    let local = vec4(c.x * p.dimensions.z, c.y * p.dimensions.w, 0.0, 1.0);
    let world = p.world_matrix * local;
    var out: VsOut;
    out.clip_pos = frame.view_proj * world;
    out.panel_uv = c + vec2(0.5, 0.5);
    out.panel_index = panel_index;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2(r, r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn median3(a: f32, b: f32, c: f32) -> f32 {
    return max(min(a, b), min(max(a, b), c));
}

// Static atlas contract — must match msdf.rs ATLAS_SIZE/CELL/CELL_PAD/ASCII_BASE/GRID
// (enforced by gpu::tests via msdf::atlas_*_wgsl_* helpers).
const MSDF_ATLAS: f32 = 512.0;
const MSDF_CELL: f32 = 32.0;
const MSDF_PAD: f32 = 4.0;
const MSDF_GRID: u32 = 16u;
const MSDF_ASCII_BASE: u32 = 0x20u;
// Constant AA half-width in SDF space (0.5 mid). No screen-space derivatives:
// prim-loop continues break uniformity (WGSL/Vulkan). Match msdf::MSDF_AA_HALF_WIDTH.
const MSDF_AA_HALF: f32 = 0.06;

fn atlas_uv_for_slot(slot: u32) -> vec4<f32> {
    let col = f32(slot % MSDF_GRID);
    let row = f32(slot / MSDF_GRID);
    let u0 = (col * MSDF_CELL + MSDF_PAD) / MSDF_ATLAS;
    let v0 = (row * MSDF_CELL + MSDF_PAD) / MSDF_ATLAS;
    let u1 = (col * MSDF_CELL + MSDF_CELL - MSDF_PAD) / MSDF_ATLAS;
    let v1 = (row * MSDF_CELL + MSDF_CELL - MSDF_PAD) / MSDF_ATLAS;
    return vec4(u0, v0, u1, v1);
}

fn sample_msdf(atlas_uv: vec2<f32>) -> f32 {
    // Explicit LOD 0: safe inside non-uniform prim clip/rect continue paths.
    let s = textureSampleLevel(msdf_atlas, msdf_samp, atlas_uv, 0.0);
    let sd = median3(s.r, s.g, s.b);
    return smoothstep(0.5 - MSDF_AA_HALF, 0.5 + MSDF_AA_HALF, sd);
}

fn sample_msdf_rect(uv_rect: vec4<f32>, local_uv: vec2<f32>) -> f32 {
    // Board local_uv: bottom-left origin, y up. Atlas cell: v0 = glyph top (image row 0).
    // Flip V so panel-bottom samples glyph-bottom.
    let uv = vec2(local_uv.x, 1.0 - local_uv.y);
    let atlas_uv = mix(uv_rect.xy, uv_rect.zw, clamp(uv, vec2(0.0), vec2(1.0)));
    return sample_msdf(atlas_uv);
}

// Digit / sign codes used by DirectNumber: 0-9, 10='-', 11='.'.
fn direct_number_code_uv(code: u32) -> vec4<f32> {
    var ch: u32 = 0x30u; // '0'
    if (code == 10u) {
        ch = 0x2du; // '-'
    } else if (code == 11u) {
        ch = 0x2eu; // '.'
    } else {
        ch = 0x30u + min(code, 9u);
    }
    return atlas_uv_for_slot(ch - MSDF_ASCII_BASE);
}

// Two-limb unsigned /10 for DirectNumber int_hi:int_lo (little-endian u64).
// floor(2^32/10)=429496729, 2^32%10=6.
fn u64_div10(hi: ptr<function, u32>, lo: ptr<function, u32>) -> u32 {
    let h = *hi;
    let l = *lo;
    let q_hi = h / 10u;
    let r_hi = h % 10u;
    let l_q = l / 10u;
    let l_r = l % 10u;
    let rem_raw = r_hi * 6u + l_r;
    let q_lo = r_hi * 429496729u + l_q + rem_raw / 10u;
    let rem = rem_raw % 10u;
    *hi = q_hi;
    *lo = q_lo;
    return rem;
}

fn sample_direct_number(payload: vec4<u32>, local_uv: vec2<f32>) -> f32 {
    let decimals = payload.w & 0xffu;
    let neg = (payload.w >> 8u) & 1u;
    var int_hi = payload.y;
    var int_lo = payload.x;
    // Cap visible digits for shader simplicity (12 incl. sign/dot).
    var digits: array<u32, 12>;
    var n = 0u;
    if (neg == 1u) {
        digits[n] = 10u;
        n = n + 1u;
    }
    if (int_hi == 0u && int_lo == 0u) {
        digits[n] = 0u;
        n = n + 1u;
    } else {
        var tmp: array<u32, 20>;
        var tn = 0u;
        while ((int_hi != 0u || int_lo != 0u) && tn < 20u) {
            tmp[tn] = u64_div10(&int_hi, &int_lo);
            tn = tn + 1u;
        }
        var i = tn;
        while (i > 0u && n < 12u) {
            i = i - 1u;
            digits[n] = tmp[i];
            n = n + 1u;
        }
    }
    if (decimals > 0u && n < 12u) {
        digits[n] = 11u; // '.'
        n = n + 1u;
        var frac = payload.z; // x10000
        var place = 1000u;
        var d = 0u;
        while (d < decimals && d < 4u && n < 12u) {
            let dig = frac / place;
            digits[n] = dig;
            n = n + 1u;
            frac = frac % place;
            place = place / 10u;
            d = d + 1u;
        }
    }

    let cell_w = 1.0 / f32(max(n, 1u));
    let idx = u32(clamp(floor(local_uv.x / cell_w), 0.0, f32(n - 1u)));
    let guv = vec2((local_uv.x - f32(idx) * cell_w) / cell_w, local_uv.y);
    let code = digits[idx];
    return sample_msdf_rect(direct_number_code_uv(code), guv);
}

@fragment
fn fs_uber_sdf(in: VsOut) -> @location(0) vec4<f32> {
    let p = panels[in.panel_index];
    // G0 LOD2: must match glass.rs LOD2_SHELL / lod2_wgsl_vec4_literal(). No blur.
    // Token is straight RGBA; output is premultiplied for the pipeline blend.
    if (p.lod_level >= 2u) {
        let lod2 = vec4(0.06, 0.07, 0.09, 0.85);
        return to_premul(lod2);
    }
    let px = in.panel_uv * p.dimensions.xy;
    var color = vec4(0.0); // premul accumulator

    let start = p.prim_start_index;
    let count = p.prim_count;
    var i = 0u;
    loop {
        if (i >= count) { break; }
        let prim = prims[start + i];
        let kind = (prim.panel_and_kind >> 16u) & 0xffu;
        let rect = prim.rect;
        let clip = prim.clip_rect;
        if (px.x < clip.x || px.y < clip.y || px.x > clip.z || px.y > clip.w) {
            i = i + 1u;
            continue;
        }

        if (kind == 0u) {
            // RoundedBox SDF: fill + inner gradient + border + optional outer glow (G0).
            // Shell prims are CPU-inset by active glow_w so soft edge stays inside panel clip.
            // glow_w stmt must match glass::shell_glow_extent_wgsl_stmt() (code line, not comment).
            let glow = bitcast<f32>(prim.payload.z);
            let border_t = bitcast<f32>(prim.payload.y);
            let glow_w = select(0.0, border_t + 6.0 * glow, glow > 0.0);
            if (px.x < rect.x - glow_w || px.y < rect.y - glow_w
                || px.x > rect.x + rect.z + glow_w || px.y > rect.y + rect.w + glow_w) {
                i = i + 1u;
                continue;
            }
            let half = rect.zw * 0.5;
            let center = rect.xy + half;
            let local = px - center;
            let radius = bitcast<f32>(prim.payload.x);
            let d = sd_rounded_box(local, half, radius);
            let fill = unpack_rgba(prim.color_rgba);
            let border = unpack_rgba(prim.border_rgba);
            // Board local Y is up (bottom-left origin) → high y (top) slightly brighter.
            let local_uv = (px - rect.xy) / max(rect.zw, vec2(1.0));
            let grad = mix(0.94, 1.06, clamp(local_uv.y, 0.0, 1.0));
            let fill_g = vec4(fill.rgb * grad, fill.a);
            let inside = 1.0 - smoothstep(-0.75, 0.75, d);
            let edge = 1.0 - smoothstep(-0.75, 0.75, abs(d) - border_t);
            // Prim-local composite stays premul (one coverage factor, like MsdfGlyph).
            let fill_a = fill_g.a * inside;
            var c = vec4(fill_g.rgb * fill_a, fill_a);
            let edge_a = edge * border.a;
            c = over_premul(c, vec4(border.rgb * edge_a, edge_a));
            if (glow > 0.0) {
                let outer = (1.0 - smoothstep(-0.5, glow_w, d)) * (1.0 - inside);
                // Clamp: glow contract is ≤1, but out-of-band values must not make premul α>1.
                let glow_a = min(border.a * glow * outer, 1.0);
                c = over_premul(c, vec4(border.rgb * glow_a, glow_a));
            }
            color = over_premul(color, c);
        } else if (kind == 1u) {
            // MsdfGlyph: payload = [u0, v0, u1, v1] as f32 bits.
            if (px.x < rect.x || px.y < rect.y || px.x > rect.x + rect.z || px.y > rect.y + rect.w) {
                i = i + 1u;
                continue;
            }
            let local_uv = (px - rect.xy) / max(rect.zw, vec2(1.0));
            let uv_rect = vec4(
                bitcast<f32>(prim.payload.x),
                bitcast<f32>(prim.payload.y),
                bitcast<f32>(prim.payload.z),
                bitcast<f32>(prim.payload.w),
            );
            let g = sample_msdf_rect(uv_rect, local_uv);
            let fill = unpack_rgba(prim.color_rgba);
            let a = fill.a * g;
            color = over_premul(color, vec4(fill.rgb * a, a));
        } else if (kind == 2u) {
            if (px.x < rect.x || px.y < rect.y || px.x > rect.x + rect.z || px.y > rect.y + rect.w) {
                i = i + 1u;
                continue;
            }
            let local_uv = (px - rect.xy) / max(rect.zw, vec2(1.0));
            let g = sample_direct_number(prim.payload, local_uv);
            let fill = unpack_rgba(prim.color_rgba);
            let a = fill.a * g;
            color = over_premul(color, vec4(fill.rgb * a, a));
        } else {
            if (px.x < rect.x || px.y < rect.y || px.x > rect.x + rect.z || px.y > rect.y + rect.w) {
                i = i + 1u;
                continue;
            }
        }
        i = i + 1u;
    }

    if (color.a < 0.01) {
        discard;
    }
    return color;
}
