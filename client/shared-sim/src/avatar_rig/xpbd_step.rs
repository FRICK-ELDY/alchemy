//! Local soft-distance XPBD over [`AvatarRigBuffer`] (x00e8).
//!
//! Same distance + compliance projection idea as [`crate::xpbd`], but on the
//! rig SoA only — no EpochSheet, topology coloring, LBVH, or Grab.
//!
//! Frame-to-frame Verlet: `prev_*` is updated inside each substep of [`step`]
//! and retained on the buffer across ticks. There is no separate
//! `commit_prev_from_curr` — secondary bones are Ring3-unpublished local state.

use super::buffer::{AvatarAnchorKind, AvatarAnchorWorld, AvatarRigBuffer, BONE_PINNED};

/// One Ring-2 Step-2 tick: Verlet predict → pin snap → soft distance solve.
///
/// `substeps` mirrors host XPBD (typically 4). Gravity is world m/s².
/// Host / client tick wiring is intentionally out of scope for x00e8.
pub fn step(
    rig: &mut AvatarRigBuffer,
    anchors: &AvatarAnchorWorld,
    dt: f32,
    substeps: u32,
    gravity: [f32; 3],
) {
    if rig.bone_count() == 0 || !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let n_sub = substeps.max(1);
    let sub_dt = dt / n_sub as f32;
    let sub_dt2 = sub_dt * sub_dt;
    let alpha_scale = if sub_dt > 0.0 {
        1.0 / (sub_dt * sub_dt)
    } else {
        0.0
    };

    for _ in 0..n_sub {
        predict(rig, sub_dt2, gravity);
        snap_pins(rig, anchors);
        solve_distances(rig, alpha_scale);
        snap_pins(rig, anchors);
    }
}

fn predict(rig: &mut AvatarRigBuffer, dt2: f32, gravity: [f32; 3]) {
    let n = rig.bone_count();
    let px = &mut rig.pos_x;
    let py = &mut rig.pos_y;
    let pz = &mut rig.pos_z;
    let prev_x = &mut rig.prev_x;
    let prev_y = &mut rig.prev_y;
    let prev_z = &mut rig.prev_z;
    let inv_mass = &rig.inv_mass;
    let parent = &rig.parent;
    for i in 0..n {
        if parent[i] == BONE_PINNED || inv_mass[i] == 0.0 {
            continue;
        }
        let cx = px[i];
        let cy = py[i];
        let cz = pz[i];
        let mut nx = cx + (cx - prev_x[i]) + gravity[0] * dt2;
        let mut ny = cy + (cy - prev_y[i]) + gravity[1] * dt2;
        let mut nz = cz + (cz - prev_z[i]) + gravity[2] * dt2;
        if !(nx.is_finite() && ny.is_finite() && nz.is_finite()) {
            nx = cx;
            ny = cy;
            nz = cz;
        }
        prev_x[i] = cx;
        prev_y[i] = cy;
        prev_z[i] = cz;
        px[i] = nx;
        py[i] = ny;
        pz[i] = nz;
    }
}

fn snap_pins(rig: &mut AvatarRigBuffer, anchors: &AvatarAnchorWorld) {
    let n = rig.bone_count();
    let px = &mut rig.pos_x;
    let py = &mut rig.pos_y;
    let pz = &mut rig.pos_z;
    let prev_x = &mut rig.prev_x;
    let prev_y = &mut rig.prev_y;
    let prev_z = &mut rig.prev_z;
    let parent = &rig.parent;
    let pin_anchor = &rig.pin_anchor;
    for i in 0..n {
        if parent[i] != BONE_PINNED {
            continue;
        }
        // Invalid discriminant: keep last pose (push_pinned rejects these;
        // defense in depth if SoA is ever patched by hand).
        let Some(kind) = AvatarAnchorKind::from_discriminant(pin_anchor[i]) else {
            continue;
        };
        let Some(p) = anchors.get(kind) else {
            continue;
        };
        // Non-finite anchor: keep last finite pin pose (do not yank to NaN).
        if !p.iter().all(|v| v.is_finite()) {
            continue;
        }
        px[i] = p[0];
        py[i] = p[1];
        pz[i] = p[2];
        prev_x[i] = p[0];
        prev_y[i] = p[1];
        prev_z[i] = p[2];
    }
}

fn solve_distances(rig: &mut AvatarRigBuffer, alpha_scale: f32) {
    let n = rig.bone_count();
    let px = &mut rig.pos_x;
    let py = &mut rig.pos_y;
    let pz = &mut rig.pos_z;
    let inv_mass = &rig.inv_mass;
    let parent = &rig.parent;
    let rest_len = &rig.rest_len;
    let compliance = &rig.compliance;
    // Two projection passes per substep (minimal path; coloring deferred).
    for _ in 0..2 {
        for (i, &par) in parent.iter().enumerate() {
            if par == BONE_PINNED {
                continue;
            }
            let a = par as usize;
            if a >= n {
                continue;
            }
            project_pair_slices(
                px,
                py,
                pz,
                inv_mass,
                rest_len,
                compliance,
                a,
                i,
                alpha_scale,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn project_pair_slices(
    px: &mut [f32],
    py: &mut [f32],
    pz: &mut [f32],
    inv_mass: &[f32],
    rest_len: &[f32],
    compliance: &[f32],
    a: usize,
    b: usize,
    alpha_scale: f32,
) {
    let rest = rest_len[b];
    let wa = inv_mass[a];
    let wb = inv_mass[b];
    if wa + wb == 0.0 {
        return;
    }
    let mut dx = px[a] - px[b];
    let mut dy = py[a] - py[b];
    let mut dz = pz[a] - pz[b];
    let dist_sq = dx * dx + dy * dy + dz * dz;
    let dist = dist_sq.sqrt();
    if dist < 1e-8 {
        // Degenerate: prefer hang axis (sway bones sit under gravity).
        dx = 0.0;
        dy = -1.0;
        dz = 0.0;
    } else {
        let inv = 1.0 / dist;
        dx *= inv;
        dy *= inv;
        dz *= inv;
    }
    let c_err = dist - rest;
    let alpha = compliance[b] * alpha_scale;
    let denom = wa + wb + alpha;
    if denom == 0.0 {
        return;
    }
    let lambda = -c_err / denom;
    let corr_x = dx * lambda;
    let corr_y = dy * lambda;
    let corr_z = dz * lambda;
    if wa > 0.0 {
        let nx = px[a] + corr_x * wa;
        let ny = py[a] + corr_y * wa;
        let nz = pz[a] + corr_z * wa;
        if nx.is_finite() && ny.is_finite() && nz.is_finite() {
            px[a] = nx;
            py[a] = ny;
            pz[a] = nz;
        }
    }
    if wb > 0.0 {
        let nx = px[b] - corr_x * wb;
        let ny = py[b] - corr_y * wb;
        let nz = pz[b] - corr_z * wb;
        if nx.is_finite() && ny.is_finite() && nz.is_finite() {
            px[b] = nx;
            py[b] = ny;
            pz[b] = nz;
        }
    }
}
