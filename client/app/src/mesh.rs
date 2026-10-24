//! GLB → CPU mesh (Slice C-B).
//!
//! Guarantees: BLAKE3 verify before decode; first mesh positions (+ indices if present).
//! Non-guarantees: materials, skins, animations, multi-mesh scenes.

use anyhow::{bail, Context, Result};

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
}
