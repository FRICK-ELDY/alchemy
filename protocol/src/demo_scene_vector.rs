//! Shared DemoScene ManifestSeal test vector (x0079).
//!
//! Elixir SSoT: `World.DemoScene` → `World.CanonicalManifest.build_payload/6`.
//! Rust mirror: `alchemy-shared-sim::demo::demo_scene_entries` (+ host `demo_entries_for_test`).
//!
//! Both sides assemble a SceneManifest with these fixed seal fields and must
//! produce [`DEMO_SCENE_VECTOR_BLAKE3`]. Update the hex when DemoScene layout
//! changes (recompute via either side's vector test / `manifest-seal-hash`).

use crate::{LocomotionConfig, LocomotionMode, SceneManifest, DEFAULT_MOVE_SPEED, PROTOCOL_PIN};

/// Room id used only for the shared vector (not a live Room).
pub const DEMO_SCENE_VECTOR_ROOM_ID: &str = "demo-scene-vector";

/// DemoScene `revision` (always 1 for a fresh build).
pub const DEMO_SCENE_VECTOR_REVISION: u64 = 1;

/// Fixed host generation for the vector.
pub const DEMO_SCENE_VECTOR_HOST_GENERATION: u64 = 1;

/// 32-byte host_pub as lowercase hex (64 chars). Not a real key.
pub const DEMO_SCENE_VECTOR_HOST_PUB_HEX: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";

/// Placeholder attestation string (hash input only; not a JWT).
pub const DEMO_SCENE_VECTOR_HOST_ATTESTATION: &str = "x0079-demo-scene-vector-attestation";

/// Canonical ManifestSeal blake3 (lowercase hex) for DemoScene + the fields above.
///
/// Recompute when `World.DemoScene` / Rust mirror / [`PROTOCOL_PIN`] change.
pub const DEMO_SCENE_VECTOR_BLAKE3: &str =
    "3c3b29b829830ee00a41a384d8f86f193795020bd7b35096ba47e3618f84dc4f";

/// Decode [`DEMO_SCENE_VECTOR_HOST_PUB_HEX`] to 32 bytes.
pub fn demo_scene_vector_host_pub() -> [u8; 32] {
    let mut out = [0u8; 32];
    // Fixed hex → infallible for the constant above.
    let bytes = hex::decode(DEMO_SCENE_VECTOR_HOST_PUB_HEX).expect("vector host_pub hex");
    out.copy_from_slice(&bytes);
    out
}

/// Build the seal-hash SceneManifest shell (caller supplies DemoScene entries).
pub fn demo_scene_vector_manifest(entries: Vec<crate::SceneEntry>) -> SceneManifest {
    SceneManifest {
        room_id: DEMO_SCENE_VECTOR_ROOM_ID.into(),
        revision: DEMO_SCENE_VECTOR_REVISION,
        entries,
        locomotion: Some(LocomotionConfig {
            mode: LocomotionMode::Noclip.into(),
            move_speed: DEFAULT_MOVE_SPEED,
        }),
        protocol_pin: PROTOCOL_PIN.into(),
        host_pub: demo_scene_vector_host_pub().to_vec(),
        host_attestation: DEMO_SCENE_VECTOR_HOST_ATTESTATION.into(),
        world_seal: String::new(),
        host_generation: DEMO_SCENE_VECTOR_HOST_GENERATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_crypto::manifest_seal_blake3_hex;
    use crate::{normalize_scene_entry_trs, SceneEntry};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Input {
        #[serde(default)]
        entries: Vec<EntryJson>,
    }

    #[derive(Deserialize)]
    struct EntryJson {
        entity_id: u32,
        #[serde(default)]
        blake3: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        z: f64,
        #[serde(default)]
        half_extent: f32,
        #[serde(default)]
        flags: u32,
        #[serde(default)]
        r: f32,
        #[serde(default)]
        g: f32,
        #[serde(default)]
        b: f32,
        #[serde(default)]
        a: f32,
        #[serde(default)]
        qx: f64,
        #[serde(default)]
        qy: f64,
        #[serde(default)]
        qz: f64,
        #[serde(default)]
        qw: f64,
        #[serde(default)]
        sx: f64,
        #[serde(default)]
        sy: f64,
        #[serde(default)]
        sz: f64,
    }

    fn fixture_entries() -> Vec<SceneEntry> {
        let raw = include_str!("../testdata/demo_scene_seal.json");
        let input: Input = serde_json::from_str(raw).expect("fixture json");
        input
            .entries
            .into_iter()
            .map(|e| {
                let mut entry = SceneEntry {
                    entity_id: e.entity_id,
                    blake3: e.blake3,
                    x: e.x,
                    y: e.y,
                    z: e.z,
                    half_extent: e.half_extent,
                    flags: e.flags,
                    r: e.r,
                    g: e.g,
                    b: e.b,
                    a: e.a,
                    qx: e.qx,
                    qy: e.qy,
                    qz: e.qz,
                    qw: e.qw,
                    sx: e.sx,
                    sy: e.sy,
                    sz: e.sz,
                };
                normalize_scene_entry_trs(&mut entry);
                entry
            })
            .collect()
    }

    #[test]
    fn elixir_fixture_json_matches_expected_blake3() {
        let entries = fixture_entries();
        let manifest = demo_scene_vector_manifest(entries);
        let got = manifest_seal_blake3_hex(&manifest);
        assert_eq!(
            got, DEMO_SCENE_VECTOR_BLAKE3,
            "update DEMO_SCENE_VECTOR_BLAKE3 from Elixir/CLI fixture hash"
        );
    }
}
