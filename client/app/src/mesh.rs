//! GLB → CPU mesh (Slice C-B / x000b flatten).
//!
//! Guarantees: BLAKE3 verify before decode; first mesh positions (+ indices if present);
//! `load_flattened_mesh_from_glb` bakes node transforms across primitives.
//! Non-guarantees: materials, skins, animations, textured draw.

use anyhow::{bail, Context, Result};
use glam::{Mat4, Quat, Vec3};

/// Expected BLAKE3 of `client/fixtures/Coin_PolygonalMind.glb`.
pub const COIN_BLAKE3_HEX: &str =
    "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";

#[derive(Clone, Debug)]
pub struct CpuMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Axis-aligned radius used to frame the camera.
    pub radius: f32,
}

pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn verify_blake3(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let got = blake3_hex(bytes);
    if !got.eq_ignore_ascii_case(expected_hex) {
        bail!("BLAKE3 mismatch: expected {expected_hex}, got {got}");
    }
    Ok(())
}

pub fn load_mesh_from_glb(bytes: &[u8]) -> Result<CpuMesh> {
    let (doc, buffers, _images) = gltf::import_slice(bytes).context("gltf import_slice")?;

    let mut positions = Vec::new();
    let mut indices = Vec::new();

    // C-B: first mesh / first primitive only.
    let mesh = doc.meshes().next().context("GLB contained no meshes")?;
    let prim = mesh
        .primitives()
        .next()
        .context("mesh contained no primitives")?;
    let reader = prim.reader(|buffer| Some(&buffers[buffer.index()].0));
    let pos_iter = reader
        .read_positions()
        .context("mesh primitive missing POSITION")?;
    for p in pos_iter {
        positions.push(p);
    }

    if let Some(idx) = reader.read_indices() {
        for i in idx.into_u32() {
            indices.push(i);
        }
    } else {
        for i in 0..positions.len() as u32 {
            indices.push(i);
        }
    }

    if positions.is_empty() {
        bail!("GLB contained no mesh positions");
    }

    let radius = framing_radius(&positions);
    Ok(CpuMesh {
        positions,
        indices,
        radius,
    })
}

fn framing_radius(positions: &[[f32; 3]]) -> f32 {
    let mut max_sq = 0.0_f32;
    for p in positions {
        let sq = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        if sq > max_sq {
            max_sq = sq;
        }
    }
    max_sq.sqrt().max(0.1)
}

/// Flatten every mesh primitive under the default scene, baking node local transforms.
/// Used for OpenXR controller GLBs (x000b). Skins / morphs are ignored.
pub fn load_flattened_mesh_from_glb(bytes: &[u8]) -> Result<CpuMesh> {
    let (doc, buffers, _images) = gltf::import_slice(bytes).context("gltf import_slice")?;

    let mut positions = Vec::new();
    let mut indices = Vec::new();

    let Some(scene) = doc.default_scene().or_else(|| doc.scenes().next()) else {
        // No scene graph — fall back to first primitive.
        return load_mesh_from_glb(bytes);
    };

    for node in scene.nodes() {
        flatten_node(node, Mat4::IDENTITY, &buffers, &mut positions, &mut indices)?;
    }

    if positions.is_empty() {
        return load_mesh_from_glb(bytes);
    }

    let radius = framing_radius(&positions);
    Ok(CpuMesh {
        positions,
        indices,
        radius,
    })
}

fn flatten_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    positions: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) -> Result<()> {
    let local = {
        let (t, r, s) = node.transform().decomposed();
        Mat4::from_scale_rotation_translation(
            Vec3::from(s),
            Quat::from_xyzw(r[0], r[1], r[2], r[3]),
            Vec3::from(t),
        )
    };
    let world = parent * local;

    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            let reader = prim.reader(|buffer| Some(&buffers[buffer.index()].0));
            let Some(pos_iter) = reader.read_positions() else {
                continue;
            };
            let base = positions.len() as u32;
            for p in pos_iter {
                let v = world.transform_point3(Vec3::from(p));
                positions.push([v.x, v.y, v.z]);
            }
            if let Some(idx) = reader.read_indices() {
                for i in idx.into_u32() {
                    indices.push(base + i);
                }
            } else {
                let n = positions.len() as u32 - base;
                for i in 0..n {
                    indices.push(base + i);
                }
            }
        }
    }

    for child in node.children() {
        flatten_node(child, world, buffers, positions, indices)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../fixtures/Coin_PolygonalMind.glb");

    #[test]
    fn coin_blake3_matches() {
        assert_eq!(blake3_hex(FIXTURE), COIN_BLAKE3_HEX);
    }

    #[test]
    fn coin_gltf_loads_vertices() {
        verify_blake3(FIXTURE, COIN_BLAKE3_HEX).unwrap();
        let mesh = load_mesh_from_glb(FIXTURE).unwrap();
        assert!(!mesh.positions.is_empty());
        assert!(!mesh.indices.is_empty());
        assert!(mesh.radius > 0.0);
    }

    #[test]
    fn coin_flatten_loads_vertices() {
        let mesh = load_flattened_mesh_from_glb(FIXTURE).unwrap();
        assert!(!mesh.positions.is_empty());
        assert!(!mesh.indices.is_empty());
    }
}
