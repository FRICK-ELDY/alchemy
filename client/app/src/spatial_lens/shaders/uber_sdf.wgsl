// Spatial Lens Uber-SDF (x00ba). Layout matches GpuUiPrim / GpuPanelHeader in prim.rs.
// Entry: vs_panel_quad / fs_uber_sdf

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

fn digit_glyph(d: u32, uv: vec2<f32>) -> f32 {
    // Tiny 3x5 bitmap digits 0-9 (procedural, no atlas).
    var bits: u32 = 0u;
    switch d {
        case 0u: { bits = 0x7B6Fu; }
        case 1u: { bits = 0x2492u; }
        case 2u: { bits = 0x73E7u; }
        case 3u: { bits = 0x73CFu; }
        case 4u: { bits = 0x5BC9u; }
        case 5u: { bits = 0x79CFu; }
        case 6u: { bits = 0x79EFu; }
        case 7u: { bits = 0x7249u; }
        case 8u: { bits = 0x7BEFu; }
        case 9u: { bits = 0x7BCFu; }
        default: { bits = 0u; }
    }
    let x = i32(clamp(floor(uv.x * 3.0), 0.0, 2.0));
    let y = i32(clamp(floor((1.0 - uv.y) * 5.0), 0.0, 4.0));
    let bit = u32(y * 3 + x);
    let on = (bits >> bit) & 1u;
    return select(0.0, 1.0, on == 1u);
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
        // Encode '-' as 10
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
        // Emit up to `decimals` leading digits of frac_x10000.
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
    if (code == 10u) {
        // minus
        return select(0.0, 1.0, abs(guv.y - 0.5) < 0.12 && guv.x > 0.15 && guv.x < 0.85);
    }
    if (code == 11u) {
        return select(0.0, 1.0, length(guv - vec2(0.5, 0.15)) < 0.12);
    }
    return digit_glyph(code, guv);
}

@fragment
fn fs_uber_sdf(in: VsOut) -> @location(0) vec4<f32> {
    let p = panels[in.panel_index];
    if (p.lod_level >= 2u) {
        return vec4(0.05, 0.06, 0.08, 0.85);
    }
    let px = in.panel_uv * p.dimensions.xy;
    var color = vec4(0.0);

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
        if (px.x < rect.x || px.y < rect.y || px.x > rect.x + rect.z || px.y > rect.y + rect.w) {
            i = i + 1u;
            continue;
        }

        if (kind == 0u) {
            // RoundedBox SDF in prim-local px.
            let half = rect.zw * 0.5;
            let center = rect.xy + half;
            let local = px - center;
            let radius = bitcast<f32>(prim.payload.x);
            let d = sd_rounded_box(local, half, radius);
            let fill = unpack_rgba(prim.color_rgba);
            let border = unpack_rgba(prim.border_rgba);
            let border_t = bitcast<f32>(prim.payload.y);
            let inside = 1.0 - smoothstep(-0.75, 0.75, d);
            let edge = 1.0 - smoothstep(-0.75, 0.75, abs(d) - border_t);
            var c = fill * inside;
            c = mix(c, border, edge * border.a);
            color = mix(color, c, c.a);
        } else if (kind == 2u) {
            let local_uv = (px - rect.xy) / max(rect.zw, vec2(1.0));
            let g = sample_direct_number(prim.payload, local_uv);
            let fill = unpack_rgba(prim.color_rgba);
            color = mix(color, vec4(fill.rgb, fill.a * g), fill.a * g);
        }
        i = i + 1u;
    }

    if (color.a < 0.01) {
        discard;
    }
    return color;
}
