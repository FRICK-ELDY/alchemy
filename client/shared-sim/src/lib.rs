//! Shared scene classification for host-server and client (x0030 / P4-2).
//!
//! Guarantees: [`EntityKind`] + [`classify_scene_entry`] priority matches x0027;
//! [`ManifestIndex`] one-pass mesh / box / grabbable maps; shared interaction +
//! demo id constants; [`scene_entry_color_rgba`].
//! Non-guarantees: Rapier / Formula / prediction; Elixir `DemoScene` codegen
//! (manual sync via [`demo`] constants).

use std::collections::{HashMap, HashSet};

use alchemy_protocol::{scene_entry_grabbable, scene_entry_is_user_root, SceneEntry};

pub mod demo;

/// Max Grab start / client pick ray distance (meters). Host `finite` + client `world_hit`.
pub const MAX_GRAB_HIT_DISTANCE: f32 = 5.0;

/// Host mesh ball collider radius (client visual scale pairs with this).
pub const PHYS_BALL_RADIUS: f32 = 0.25;

/// Default cuboid half-extent when Manifest omits per-entity size (draw fallback).
pub const DEFAULT_BOX_HALF_EXTENT: f32 = 0.25;

/// Logical kind of a SceneManifest entry (not a wire field).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityKind {
    UserRoot,
    Mesh,
    Box,
    Ignored,
}

/// Classify one entry. Priority (host + client must match, x0027):
/// 1. user_root id band → [`EntityKind::UserRoot`]
/// 2. `blake3` non-empty → [`EntityKind::Mesh`] (`half_extent` / Grab ignored for box path)
/// 3. `half_extent > 0` → [`EntityKind::Box`]
/// 4. else → [`EntityKind::Ignored`]
pub fn classify_scene_entry(entry: &SceneEntry) -> EntityKind {
    if scene_entry_is_user_root(entry.entity_id) {
        EntityKind::UserRoot
    } else if !entry.blake3.is_empty() {
        EntityKind::Mesh
    } else if entry.half_extent > 0.0 {
        // blake3 already empty here; same as protocol `scene_entry_is_box`.
        EntityKind::Box
    } else {
        EntityKind::Ignored
    }
}

/// One-pass index of Manifest entries for draw / Grab / mesh id (no poses).
#[derive(Clone, Debug, Default)]
pub struct ManifestIndex {
    /// First mesh entry id (blake3 non-empty), if any.
    pub mesh_entity_id: Option<u32>,
    pub half_extents: HashMap<u32, f32>,
    pub grabbable: HashSet<u32>,
}

impl ManifestIndex {
    /// Build index with the same classification priority as [`classify_scene_entry`].
    pub fn from_entries(entries: &[SceneEntry]) -> Self {
        let mut mesh_entity_id = None;
        let mut half_extents = HashMap::new();
        let mut grabbable = HashSet::new();
        for entry in entries {
            match classify_scene_entry(entry) {
                EntityKind::Mesh => {
                    if mesh_entity_id.is_none() {
                        mesh_entity_id = Some(entry.entity_id);
                    }
                }
                EntityKind::Box => {
                    half_extents.insert(entry.entity_id, entry.half_extent);
                    if scene_entry_grabbable(entry.flags) {
                        grabbable.insert(entry.entity_id);
                    }
                }
                EntityKind::UserRoot | EntityKind::Ignored => {}
            }
        }
        Self {
            mesh_entity_id,
            half_extents,
            grabbable,
        }
    }
}

/// Tint from Manifest; `a == 0` → opaque white (host / client agreement).
pub fn scene_entry_color_rgba(entry: &SceneEntry) -> (f32, f32, f32, f32) {
    if entry.a > 0.0 {
        (entry.r, entry.g, entry.b, entry.a)
    } else {
        (1.0, 1.0, 1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::{
        scene_entry_is_box, scene_entry_is_user_root, SCENE_FLAG_GRABBABLE, USER_ROOT_ENTITY_FIRST,
    };

    fn entry(
        entity_id: u32,
        blake3: &str,
        half_extent: f32,
        flags: u32,
    ) -> SceneEntry {
        SceneEntry {
            entity_id,
            blake3: blake3.into(),
            half_extent,
            flags,
            a: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn classify_matches_protocol_helpers() {
        let root = entry(USER_ROOT_ENTITY_FIRST, "", 0.25, SCENE_FLAG_GRABBABLE);
        assert_eq!(classify_scene_entry(&root), EntityKind::UserRoot);
        assert!(scene_entry_is_user_root(root.entity_id));

        let mesh = entry(1, "abc", 0.25, 0);
        assert_eq!(classify_scene_entry(&mesh), EntityKind::Mesh);
        assert!(!scene_entry_is_box(&mesh.blake3, mesh.half_extent, mesh.entity_id));

        let cuboid = entry(10, "", 0.25, SCENE_FLAG_GRABBABLE);
        assert_eq!(classify_scene_entry(&cuboid), EntityKind::Box);
        assert!(scene_entry_is_box(
            &cuboid.blake3,
            cuboid.half_extent,
            cuboid.entity_id
        ));

        let ignored = entry(99, "", 0.0, 0);
        assert_eq!(classify_scene_entry(&ignored), EntityKind::Ignored);
    }

    #[test]
    fn manifest_index_first_mesh_and_grabbable_boxes() {
        let entries = vec![
            entry(USER_ROOT_ENTITY_FIRST, "", 0.0, 0),
            entry(1, "mesh-a", 0.0, 0),
            entry(2, "mesh-b", 0.0, 0),
            entry(10, "", 0.25, SCENE_FLAG_GRABBABLE),
            entry(11, "", 0.3, 0),
        ];
        let idx = ManifestIndex::from_entries(&entries);
        assert_eq!(idx.mesh_entity_id, Some(1));
        assert_eq!(idx.half_extents.get(&10), Some(&0.25));
        assert_eq!(idx.half_extents.get(&11), Some(&0.3));
        assert!(idx.grabbable.contains(&10));
        assert!(!idx.grabbable.contains(&11));
        assert!(!idx.half_extents.contains_key(&1));
    }

    #[test]
    fn color_rgba_defaults_when_a_zero() {
        let mut e = entry(1, "", 0.0, 0);
        e.r = 0.2;
        e.g = 0.3;
        e.b = 0.4;
        e.a = 0.0;
        assert_eq!(scene_entry_color_rgba(&e), (1.0, 1.0, 1.0, 1.0));
        e.a = 0.5;
        assert_eq!(scene_entry_color_rgba(&e), (0.2, 0.3, 0.4, 0.5));
    }
}
