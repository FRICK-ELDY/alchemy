// HotPhysicsPage GpuTransferableSoA interpolate (x00ac / mvcc_xpbd.md §4.6).
// Layout must match alchemy_shared_sim::mvcc::page::GpuTransferableSoA field order.

struct GpuTransferableSoA {
    pos_x: array<f32, 1024>,
    pos_y: array<f32, 1024>,
    pos_z: array<f32, 1024>,
    inv_mass: array<f32, 1024>,

    delta_x: array<f32, 1024>,
    delta_y: array<f32, 1024>,
    delta_z: array<f32, 1024>,
    damping: array<f32, 1024>,

    rot_x: array<f32, 1024>,
    rot_y: array<f32, 1024>,
    rot_z: array<f32, 1024>,
    rot_w: array<f32, 1024>,

    // Reserved rotation / inertia columns (x00a6); unused by this kernel.
    drot_x: array<f32, 1024>,
    drot_y: array<f32, 1024>,
    drot_z: array<f32, 1024>,
    inv_inertia_x: array<f32, 1024>,
    inv_inertia_y: array<f32, 1024>,
    inv_inertia_z: array<f32, 1024>,
};

struct InterpUniforms {
    /// Camera cell origin − page cell origin (CPU f64 subtract → f32).
    cell_rel_offset: vec3<f32>,
    interp_alpha: f32,
    entity_count: u32,
    _pad0: vec3<u32>,
};

@group(0) @binding(0) var<storage, read> state_prev: GpuTransferableSoA;
@group(0) @binding(1) var<storage, read> state_curr: GpuTransferableSoA;
@group(0) @binding(2) var<storage, read_write> out_matrices: array<mat4x4<f32>, 1024>;
@group(0) @binding(3) var<uniform> uniforms: InterpUniforms;

fn quat_to_mat4(pos: vec3<f32>, q: vec4<f32>) -> mat4x4<f32> {
    let x2 = q.x + q.x;
    let y2 = q.y + q.y;
    let z2 = q.z + q.z;
    let xx = q.x * x2;
    let xy = q.x * y2;
    let xz = q.x * z2;
    let yy = q.y * y2;
    let yz = q.y * z2;
    let zz = q.z * z2;
    let wx = q.w * x2;
    let wy = q.w * y2;
    let wz = q.w * z2;

    return mat4x4<f32>(
        vec4<f32>(1.0 - (yy + zz), xy + wz, xz - wy, 0.0),
        vec4<f32>(xy - wz, 1.0 - (xx + zz), yz + wx, 0.0),
        vec4<f32>(xz + wy, yz - wx, 1.0 - (xx + yy), 0.0),
        vec4<f32>(pos.x, pos.y, pos.z, 1.0)
    );
}

@compute @workgroup_size(64)
fn cs_interpolate_entities(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= uniforms.entity_count) {
        return;
    }

    let alpha = clamp(uniforms.interp_alpha, 0.0, 1.0);

    let p0 = vec3<f32>(state_prev.pos_x[idx], state_prev.pos_y[idx], state_prev.pos_z[idx]);
    let p1 = vec3<f32>(state_curr.pos_x[idx], state_curr.pos_y[idx], state_curr.pos_z[idx]);
    let camera_rel_pos = mix(p0, p1, alpha) + uniforms.cell_rel_offset;

    let q0 = vec4<f32>(state_prev.rot_x[idx], state_prev.rot_y[idx], state_prev.rot_z[idx], state_prev.rot_w[idx]);
    var q1 = vec4<f32>(state_curr.rot_x[idx], state_curr.rot_y[idx], state_curr.rot_z[idx], state_curr.rot_w[idx]);
    if (dot(q0, q1) < 0.0) {
        q1 = -q1;
    }
    let rot = normalize(mix(q0, q1, alpha));

    out_matrices[idx] = quat_to_mat4(camera_rel_pos, rot);
}
