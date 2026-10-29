//! GLB → CPU mesh (Slice C-B / x000b flatten / x000c baseColor / x000d rig / x0048).
//!
//! Guarantees: BLAKE3 verify before decode; first mesh positions (+ indices if present);
//! `load_flattened_mesh_from_glb` bakes node transforms across primitives;
//! `load_flattened_textured_from_glb` adds TEXCOORD_0 + optional baseColor RGBA + factor;
//! `load_textured_rigged_from_glb` keeps a node hierarchy for runtime-replaced locals;
//! [`placeholder_error_mesh`] is visually distinct from the Coin fixture (auth asset failure).
//! Non-guarantees: PBR maps, skins, morphs, external URI textures, Coin textured draw;
//! live mesh swap after boot.

use std::collections::HashMap;

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

/// RGBA8 image decoded from an embedded glTF image (x000c).
#[derive(Clone, Debug)]
pub struct CpuRgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// One node in a textured rig (x000d). World = parent_world * local.
#[derive(Clone, Copy, Debug)]
pub struct RigNode {
    pub parent: Option<usize>,
    /// Index into the runtime animatable-state array. `None` → use [`Self::asset_local`].
    pub anim_index: Option<u32>,
    /// glTF local transform. Replaced by runtime pose when `anim_index` is `Some`.
    pub asset_local: Mat4,
}

/// One glTF primitive (x000c / x000d).
#[derive(Clone, Debug)]
pub struct CpuSubmesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Present only when baseColorTexture + TEXCOORD_0 were both available.
    pub base_color: Option<CpuRgbaImage>,
    pub base_color_factor: [f32; 4],
    /// Index into [`CpuTexturedMesh::nodes`] (mesh-local verts).
    pub node_index: usize,
}

/// Textured GLB as per-primitive submeshes + optional rig (OpenXR controller path).
#[derive(Clone, Debug)]
pub struct CpuTexturedMesh {
    pub nodes: Vec<RigNode>,
    pub submeshes: Vec<CpuSubmesh>,
    /// Framing radius (parity with [`CpuMesh`]; XR draws at root pose).
    #[allow(dead_code)]
    pub radius: f32,
}

impl CpuTexturedMesh {
    pub fn vertex_count(&self) -> usize {
        self.submeshes.iter().map(|s| s.positions.len()).sum()
    }
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

/// Unit cube (half-extent 0.5) used when authenticated assets GET / blake3 fails (x0048).
///
/// Guarantees: never the Coin fixture bytes — failure stays visually obvious.
pub fn placeholder_error_mesh() -> CpuMesh {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let indices = [
        0, 1, 2, 0, 2, 3, // -Z
        4, 6, 5, 4, 7, 6, // +Z
        0, 4, 5, 0, 5, 1, // -Y
        3, 2, 6, 3, 6, 7, // +Y
        0, 3, 7, 0, 7, 4, // -X
        1, 5, 6, 1, 6, 2, // +X
    ];
    CpuMesh {
        positions: positions.to_vec(),
        indices: indices.to_vec(),
        radius: (0.5_f32 * 3.0_f32.sqrt()),
    }
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

fn framing_radius_rigged(nodes: &[RigNode], submeshes: &[CpuSubmesh]) -> f32 {
    let worlds = rest_pose_worlds(nodes);
    let mut max_sq = 0.0_f32;
    for s in submeshes {
        let world = worlds.get(s.node_index).copied().unwrap_or(Mat4::IDENTITY);
        for p in &s.positions {
            let v = world.transform_point3(Vec3::from(*p));
            let sq = v.length_squared();
            if sq > max_sq {
                max_sq = sq;
            }
        }
    }
    max_sq.sqrt().max(0.1)
}

/// Rest-pose worlds using each node's `asset_local` (including animatable nodes).
pub fn rest_pose_worlds(nodes: &[RigNode]) -> Vec<Mat4> {
    let mut worlds = vec![Mat4::IDENTITY; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        let local = n.asset_local;
        worlds[i] = match n.parent {
            Some(p) => worlds[p] * local,
            None => local,
        };
    }
    worlds
}

fn node_asset_local(node: &gltf::Node<'_>) -> Mat4 {
    let (t, r, s) = node.transform().decomposed();
    Mat4::from_scale_rotation_translation(
        Vec3::from(s),
        Quat::from_xyzw(r[0], r[1], r[2], r[3]),
        Vec3::from(t),
    )
}

fn root_only_nodes() -> Vec<RigNode> {
    vec![RigNode {
        parent: None,
        anim_index: None,
        asset_local: Mat4::IDENTITY,
    }]
}

/// Flatten every mesh primitive under the default scene, baking node local transforms.
/// Retained for Coin / x000b callers and tests; XR controllers use
/// [`load_textured_rigged_from_glb`]. Skins / morphs are ignored.
#[allow(dead_code)]
pub fn load_flattened_mesh_from_glb(bytes: &[u8]) -> Result<CpuMesh> {
    let textured = load_flattened_textured_from_glb(bytes)?;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for sub in textured.submeshes {
        let base = positions.len() as u32;
        positions.extend(sub.positions);
        indices.extend(sub.indices.into_iter().map(|i| base + i));
    }
    if positions.is_empty() {
        return load_mesh_from_glb(bytes);
    }
    Ok(CpuMesh {
        radius: framing_radius(&positions),
        positions,
        indices,
    })
}

/// Flatten with per-primitive UVs and optional embedded baseColor (x000c).
/// Produces a single identity rig node; verts are already in root space.
pub fn load_flattened_textured_from_glb(bytes: &[u8]) -> Result<CpuTexturedMesh> {
    let (doc, buffers, images) = gltf::import_slice(bytes).context("gltf import_slice")?;

    let mut submeshes = Vec::new();

    let Some(scene) = doc.default_scene().or_else(|| doc.scenes().next()) else {
        return textured_from_first_primitive(&doc, &buffers, &images);
    };

    for node in scene.nodes() {
        flatten_textured_node(node, Mat4::IDENTITY, &buffers, &images, &mut submeshes)?;
    }

    if submeshes.is_empty() {
        return textured_from_first_primitive(&doc, &buffers, &images);
    }

    let radius = framing_radius_rigged(&root_only_nodes(), &submeshes);
    Ok(CpuTexturedMesh {
        nodes: root_only_nodes(),
        submeshes,
        radius,
    })
}

/// Keep glTF node hierarchy; verts stay mesh-local. Names in `animatable_names` map to
/// `RigNode::anim_index` (runtime replaces `asset_local`). Empty names → static hierarchy.
pub fn load_textured_rigged_from_glb(
    bytes: &[u8],
    animatable_names: &[&str],
) -> Result<CpuTexturedMesh> {
    if animatable_names.is_empty() {
        return load_flattened_textured_from_glb(bytes);
    }

    let (doc, buffers, images) = gltf::import_slice(bytes).context("gltf import_slice")?;
    let name_to_anim: HashMap<&str, u32> = animatable_names
        .iter()
        .enumerate()
        .map(|(i, n)| (*n, i as u32))
        .collect();

    let mut nodes = Vec::new();
    let mut submeshes = Vec::new();

    let Some(scene) = doc.default_scene().or_else(|| doc.scenes().next()) else {
        return textured_from_first_primitive(&doc, &buffers, &images);
    };

    for node in scene.nodes() {
        walk_rigged_node(
            node,
            None,
            &name_to_anim,
            &buffers,
            &images,
            &mut nodes,
            &mut submeshes,
        )?;
    }

    if submeshes.is_empty() {
        return textured_from_first_primitive(&doc, &buffers, &images);
    }

    let radius = framing_radius_rigged(&nodes, &submeshes);
    Ok(CpuTexturedMesh {
        nodes,
        submeshes,
        radius,
    })
}

fn textured_from_first_primitive(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<CpuTexturedMesh> {
    let mesh = doc.meshes().next().context("GLB contained no meshes")?;
    let prim = mesh
        .primitives()
        .next()
        .context("mesh contained no primitives")?;
    let Some(mut sub) = primitive_to_submesh(&prim, Mat4::IDENTITY, buffers, images)? else {
        bail!("GLB contained no mesh positions");
    };
    sub.node_index = 0;
    let radius = framing_radius(&sub.positions);
    Ok(CpuTexturedMesh {
        nodes: root_only_nodes(),
        submeshes: vec![sub],
        radius,
    })
}

fn flatten_textured_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    out: &mut Vec<CpuSubmesh>,
) -> Result<()> {
    let local = node_asset_local(&node);
    let world = parent * local;

    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            if let Some(mut sub) = primitive_to_submesh(&prim, world, buffers, images)? {
                sub.node_index = 0;
                out.push(sub);
            }
        }
    }

    for child in node.children() {
        flatten_textured_node(child, world, buffers, images, out)?;
    }
    Ok(())
}

fn walk_rigged_node(
    node: gltf::Node<'_>,
    parent: Option<usize>,
    name_to_anim: &HashMap<&str, u32>,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    nodes: &mut Vec<RigNode>,
    out: &mut Vec<CpuSubmesh>,
) -> Result<()> {
    let asset_local = node_asset_local(&node);
    let anim_index = node.name().and_then(|n| name_to_anim.get(n).copied());
    let idx = nodes.len();
    nodes.push(RigNode {
        parent,
        anim_index,
        asset_local,
    });

    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            // Mesh-local verts; node world is applied at draw time.
            if let Some(mut sub) = primitive_to_submesh(&prim, Mat4::IDENTITY, buffers, images)? {
                sub.node_index = idx;
                out.push(sub);
            }
        }
    }

    for child in node.children() {
        walk_rigged_node(child, Some(idx), name_to_anim, buffers, images, nodes, out)?;
    }
    Ok(())
}

fn primitive_to_submesh(
    prim: &gltf::Primitive<'_>,
    world: Mat4,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<Option<CpuSubmesh>> {
    let reader = prim.reader(|buffer| Some(&buffers[buffer.index()].0));
    let Some(pos_iter) = reader.read_positions() else {
        return Ok(None);
    };

    let mut positions = Vec::new();
    for p in pos_iter {
        let v = world.transform_point3(Vec3::from(p));
        positions.push([v.x, v.y, v.z]);
    }
    if positions.is_empty() {
        return Ok(None);
    }

    let mut uvs = Vec::with_capacity(positions.len());
    let has_uv = if let Some(tc) = reader.read_tex_coords(0) {
        for uv in tc.into_f32() {
            uvs.push(uv);
        }
        uvs.len() == positions.len()
    } else {
        false
    };
    if !has_uv {
        uvs.clear();
        uvs.resize(positions.len(), [0.0, 0.0]);
    }

    let mut indices = Vec::new();
    if let Some(idx) = reader.read_indices() {
        for i in idx.into_u32() {
            indices.push(i);
        }
    } else {
        for i in 0..positions.len() as u32 {
            indices.push(i);
        }
    }

    let pbr = prim.material().pbr_metallic_roughness();
    let base_color_factor = pbr.base_color_factor();
    let base_color = if has_uv {
        pbr.base_color_texture()
            .and_then(|info| {
                let idx = info.texture().source().index();
                images.get(idx)
            })
            .and_then(image_to_rgba8)
    } else {
        None
    };

    Ok(Some(CpuSubmesh {
        positions,
        uvs,
        indices,
        base_color,
        base_color_factor,
        node_index: 0,
    }))
}

fn image_to_rgba8(img: &gltf::image::Data) -> Option<CpuRgbaImage> {
    use gltf::image::Format;
    let n = (img.width as usize).checked_mul(img.height as usize)?;
    let pixels = match img.format {
        Format::R8 => {
            if img.pixels.len() < n {
                return None;
            }
            img.pixels[..n]
                .iter()
                .flat_map(|&r| [r, r, r, 255])
                .collect()
        }
        Format::R8G8 => {
            if img.pixels.len() < n * 2 {
                return None;
            }
            img.pixels[..n * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[0], c[0], c[1]])
                .collect()
        }
        Format::R8G8B8 => {
            if img.pixels.len() < n * 3 {
                return None;
            }
            img.pixels[..n * 3]
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2], 255])
                .collect()
        }
        Format::R8G8B8A8 => {
            if img.pixels.len() < n * 4 {
                return None;
            }
            img.pixels[..n * 4].to_vec()
        }
        _ => return None,
    };
    Some(CpuRgbaImage {
        width: img.width,
        height: img.height,
        pixels,
    })
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

    #[test]
    fn coin_textured_flatten_has_submeshes() {
        let mesh = load_flattened_textured_from_glb(FIXTURE).unwrap();
        assert!(!mesh.submeshes.is_empty());
        assert_eq!(mesh.nodes.len(), 1);
        assert!(mesh.vertex_count() > 0);
        for s in &mesh.submeshes {
            assert_eq!(s.positions.len(), s.uvs.len());
            assert!(!s.indices.is_empty());
            assert_eq!(s.node_index, 0);
        }
    }

    #[test]
    fn coin_rigged_empty_names_matches_flatten() {
        let flat = load_flattened_textured_from_glb(FIXTURE).unwrap();
        let rigged = load_textured_rigged_from_glb(FIXTURE, &[]).unwrap();
        assert_eq!(flat.submeshes.len(), rigged.submeshes.len());
        assert_eq!(flat.vertex_count(), rigged.vertex_count());
    }

    #[test]
    fn placeholder_error_mesh_is_not_empty_and_not_coin() {
        let mesh = placeholder_error_mesh();
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.indices.len(), 36);
        assert!(mesh.radius > 0.0);
        // Distinct from Coin fixture vertex count (smoke: not a silent Coin swap).
        let coin = load_mesh_from_glb(FIXTURE).unwrap();
        assert_ne!(mesh.positions.len(), coin.positions.len());
    }
}
