//! Demo content ids + layout mirror of Elixir `World.DemoScene` (x0030 / x0079).
//!
//! Production layout SSoT is Elixir `World.DemoScene`
//! (`world-server/lib/world/demo_scene.ex`). Rust mirror is checked by the
//! shared ManifestSeal blake3 vector in [`alchemy_protocol::demo_scene_vector`].
//!
//! Color / half_extent literals use `f32` (not `f64 as f32`) so they match the
//! JSON → `manifest-seal-hash` path that world uses for the shared vector.

use alchemy_protocol::{
    normalize_scene_entry_trs, SceneEntry, ENTITY_QUAT_IDENTITY, ENTITY_SCALE_ONE,
    SCENE_FLAG_GRABBABLE, USER_ROOT_ENTITY_FIRST,
};

use crate::DEFAULT_BOX_HALF_EXTENT;

/// Coin / primary mesh entity id in DemoScene.
pub const DEMO_MESH_ENTITY_ID: u32 = 1;

/// First grabbable box entity id; boxes are `DEMO_BOX_ENTITY_BASE + 0..8` → 10..=18.
pub const DEMO_BOX_ENTITY_BASE: u32 = 10;

/// Coin blake3 hex used by DemoScene (assets fixture).
pub const DEMO_COIN_BLAKE3: &str =
    "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";

/// 45° yaw (xyzw) for center grabbable box.
///
/// Do **not** paste Elixir `@yaw45_*` decimals — Jason→serde differs by 1 ULP.
/// Regen prints paste-ready lines via `demo-scene-vector-yaw-bits`
/// (`mix run world-server/scripts/regen_demo_scene_vector.exs`).
const YAW45_QY: f64 = f64::from_bits(0x3fd8_7de2_a6ae_a963);
const YAW45_QW: f64 = f64::from_bits(0x3fed_906b_cf32_8d47);

/// Rust mirror of `World.DemoScene.build()["entries"]` (x0027 / x002c / x004d / x0079).
///
/// Guarantees: entity ids, poses, TRS, tints, coin blake3, grabbable flags match Elixir.
/// Non-guarantees: UGC scenes; runtime edits after room create.
pub fn demo_scene_entries() -> Vec<SceneEntry> {
    let coin = {
        let mut e = SceneEntry {
            entity_id: DEMO_MESH_ENTITY_ID,
            blake3: DEMO_COIN_BLAKE3.into(),
            x: 0.0,
            y: 5.0,
            z: 0.0,
            half_extent: 0.0,
            flags: 0,
            r: 0.2,
            g: 0.9,
            b: 0.4,
            a: 1.0,
            qx: ENTITY_QUAT_IDENTITY[0],
            qy: ENTITY_QUAT_IDENTITY[1],
            qz: ENTITY_QUAT_IDENTITY[2],
            qw: ENTITY_QUAT_IDENTITY[3],
            sx: ENTITY_SCALE_ONE[0],
            sy: ENTITY_SCALE_ONE[1],
            sz: ENTITY_SCALE_ONE[2],
        };
        normalize_scene_entry_trs(&mut e);
        e
    };

    // Spawn/color pairs — keep order identical to `World.DemoScene` `@user_roots`.
    let roots = [
        ((0.0, 1.6, 6.0), (0.4_f32, 0.7_f32, 1.0_f32)),
        ((1.5, 1.6, 6.0), (1.0_f32, 0.55_f32, 0.25_f32)),
        ((-1.5, 1.6, 6.0), (0.45_f32, 0.9_f32, 0.5_f32)),
        ((3.0, 1.6, 6.0), (0.9_f32, 0.45_f32, 0.85_f32)),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, ((x, y, z), (r, g, b)))| {
        let mut e = SceneEntry {
            entity_id: USER_ROOT_ENTITY_FIRST + i as u32,
            blake3: String::new(),
            x,
            y,
            z,
            half_extent: 0.0,
            flags: 0,
            r,
            g,
            b,
            a: 1.0,
            qx: ENTITY_QUAT_IDENTITY[0],
            qy: ENTITY_QUAT_IDENTITY[1],
            qz: ENTITY_QUAT_IDENTITY[2],
            qw: ENTITY_QUAT_IDENTITY[3],
            sx: ENTITY_SCALE_ONE[0],
            sy: ENTITY_SCALE_ONE[1],
            sz: ENTITY_SCALE_ONE[2],
        };
        normalize_scene_entry_trs(&mut e);
        e
    });

    // f32 palette — must match Jason/serde f32 parse of DemoScene floats (x0079).
    let palette: [(f32, f32, f32); 9] = [
        (0.90, 0.35, 0.30),
        (0.90, 0.60, 0.20),
        (0.85, 0.80, 0.25),
        (0.35, 0.75, 0.35),
        (0.25, 0.70, 0.75),
        (0.30, 0.45, 0.90),
        (0.55, 0.35, 0.85),
        (0.85, 0.35, 0.70),
        (0.70, 0.70, 0.75),
    ];
    let boxes = palette.into_iter().enumerate().map(|(i, (r, g, b))| {
        let row = (i / 3) as f64;
        let col = (i % 3) as f64;
        let (qx, qy, qz, qw, sx, sy, sz) = if i == 4 {
            (0.0, YAW45_QY, 0.0, YAW45_QW, 1.5, 1.0, 1.0)
        } else {
            (
                ENTITY_QUAT_IDENTITY[0],
                ENTITY_QUAT_IDENTITY[1],
                ENTITY_QUAT_IDENTITY[2],
                ENTITY_QUAT_IDENTITY[3],
                ENTITY_SCALE_ONE[0],
                ENTITY_SCALE_ONE[1],
                ENTITY_SCALE_ONE[2],
            )
        };
        let mut e = SceneEntry {
            entity_id: DEMO_BOX_ENTITY_BASE + i as u32,
            blake3: String::new(),
            x: (col - 1.0) * 0.8,
            y: f64::from(DEFAULT_BOX_HALF_EXTENT),
            z: 3.5 + (row - 1.0) * 0.8,
            half_extent: DEFAULT_BOX_HALF_EXTENT,
            flags: SCENE_FLAG_GRABBABLE,
            r,
            g,
            b,
            a: 1.0,
            qx,
            qy,
            qz,
            qw,
            sx,
            sy,
            sz,
        };
        normalize_scene_entry_trs(&mut e);
        e
    });

    std::iter::once(coin).chain(roots).chain(boxes).collect()
}

#[cfg(test)]
mod tests {
    use alchemy_protocol::demo_scene_vector::{
        demo_scene_vector_manifest, DEMO_SCENE_VECTOR_BLAKE3,
    };
    use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;

    use super::demo_scene_entries;

    #[test]
    fn demo_scene_seal_matches_shared_vector() {
        let manifest = demo_scene_vector_manifest(demo_scene_entries());
        let got = manifest_seal_blake3_hex(&manifest);
        assert_eq!(
            got, DEMO_SCENE_VECTOR_BLAKE3,
            "Rust DemoScene mirror drifted from protocol vector / Elixir SSoT (x0079)"
        );
    }
}
