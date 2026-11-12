// SceneGpu world pass (x0091): mesh/box TRS from epoch_out storage when pack_index
// is valid; floor / fallback use model_override. No GPU→CPU readback.

struct EpochRow {
    pos: vec3<f32>,
    _pad0: f32,
    rot: vec4<f32>,
    scale: vec3<f32>,
    row_id: u32,
}

struct Uniforms {
    view_proj: mat4x4<f32>,
    color: vec4<f32>,
    scale_mul: vec3<f32>,
    // 0xFFFFFFFFu → use model_override (floor / pre-epoch fallback).
    pack_index: u32,
    extra_rot: vec4<f32>,
    model_override: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> epoch_out: array<EpochRow>;

struct VertexInput {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

fn quat_to_mat3(q: vec4<f32>) -> mat3x3<f32> {
    // glam/xyzw; tolerate near-zero as identity.
    let len2 = dot(q, q);
    if (len2 < 1e-12) {
        return mat3x3<f32>(
            vec3<f32>(1.0, 0.0, 0.0),
            vec3<f32>(0.0, 1.0, 0.0),
            vec3<f32>(0.0, 0.0, 1.0),
        );
    }
    let n = q * inverseSqrt(len2);
    let x = n.x;
    let y = n.y;
    let z = n.z;
    let w = n.w;
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    return mat3x3<f32>(
        vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
        vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
        vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy)),
    );
}

fn trs_model(pos: vec3<f32>, rot: vec4<f32>, scale: vec3<f32>) -> mat4x4<f32> {
    let r = quat_to_mat3(rot);
    let sx = scale.x;
    let sy = scale.y;
    let sz = scale.z;
    return mat4x4<f32>(
        vec4<f32>(r[0] * sx, 0.0),
        vec4<f32>(r[1] * sy, 0.0),
        vec4<f32>(r[2] * sz, 0.0),
        vec4<f32>(pos, 1.0),
    );
}

fn quat_mul(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
        a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
        a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
    );
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    var model: mat4x4<f32>;
    if (uniforms.pack_index == 0xffffffffu) {
        model = uniforms.model_override;
    } else {
        let row = epoch_out[uniforms.pack_index];
        // Mirror / EntityPose treat near-zero scale as identity, then apply scale_mul.
        var scale = row.scale;
        if (dot(scale, scale) < 1e-12) {
            scale = vec3<f32>(1.0, 1.0, 1.0);
        }
        scale = scale * uniforms.scale_mul;
        let rot = quat_mul(row.rot, uniforms.extra_rot);
        model = trs_model(row.pos, rot, scale);
    }
    out.clip_position = uniforms.view_proj * model * vec4<f32>(in.position, 1.0);
    return out;
}

@fragment
fn fs_main(_in: VertexOutput) -> @location(0) vec4<f32> {
    return uniforms.color;
}
