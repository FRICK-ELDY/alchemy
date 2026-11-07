//! Shared DemoScene ManifestSeal **test vector** (x0079).
//!
//! Not a wire API. Constants + helpers exist so Elixir / Rust CI can pin the
//! same seal blake3. Do not use these room_id / attestation values in production.
//!
//! Elixir SSoT: `World.DemoScene` → `World.CanonicalManifest.build_payload/6`.
//! Rust mirror: `alchemy-shared-sim::demo::demo_scene_entries`.
//!
//! Regen (cwd-independent): `mix run world-server/scripts/regen_demo_scene_vector.exs`
//! — writes `protocol/testdata/demo_scene_seal.json`, prints `blake3=`, and
//! (via `demo-scene-vector-yaw-bits`) paste-ready `YAW45_*` `from_bits` lines.

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
/// Recompute via `mix run world-server/scripts/regen_demo_scene_vector.exs`
/// when `World.DemoScene` / Rust mirror / [`PROTOCOL_PIN`] change.
pub const DEMO_SCENE_VECTOR_BLAKE3: &str =
    "a7ee5e89ce73320052854cd0811089af8d0e9d47120bcf49b351adf659eb380a";

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
    use crate::manifest_seal_json::{
        blake3_hex_from_json_str, scene_manifest_from_json_str, ManifestSealJson,
    };
    use crate::wire_crypto::manifest_seal_blake3_hex;

    const FIXTURE: &str = include_str!("../testdata/demo_scene_seal.json");

    #[test]
    fn fixture_metadata_matches_vector_constants() {
        let input: ManifestSealJson = serde_json::from_str(FIXTURE).expect("fixture json");
        assert_eq!(input.room_id, DEMO_SCENE_VECTOR_ROOM_ID);
        assert_eq!(input.revision, DEMO_SCENE_VECTOR_REVISION);
        assert_eq!(input.host_generation, DEMO_SCENE_VECTOR_HOST_GENERATION);
        assert_eq!(input.host_pub_hex, DEMO_SCENE_VECTOR_HOST_PUB_HEX);
        assert_eq!(input.host_attestation, DEMO_SCENE_VECTOR_HOST_ATTESTATION);
        assert_eq!(
            input.protocol_pin.as_deref(),
            Some(PROTOCOL_PIN),
            "fixture protocol_pin drifted from PROTOCOL_PIN"
        );
        let loco = input.locomotion.expect("fixture locomotion");
        assert_eq!(loco.mode.to_ascii_lowercase(), "noclip");
        assert_eq!(loco.move_speed, DEFAULT_MOVE_SPEED);
    }

    #[test]
    fn fixture_json_hashes_via_seal_json_path() {
        let got = blake3_hex_from_json_str(FIXTURE, true).expect("hash fixture");
        assert_eq!(
            got, DEMO_SCENE_VECTOR_BLAKE3,
            "update DEMO_SCENE_VECTOR_BLAKE3 via regen_demo_scene_vector.exs"
        );
    }

    #[test]
    fn fixture_manifest_matches_vector_shell_hash() {
        let from_json = scene_manifest_from_json_str(FIXTURE, true).expect("parse fixture");
        let via_shell = demo_scene_vector_manifest(from_json.entries.clone());
        assert_eq!(
            manifest_seal_blake3_hex(&from_json),
            manifest_seal_blake3_hex(&via_shell)
        );
        assert_eq!(from_json.room_id, via_shell.room_id);
        assert_eq!(from_json.protocol_pin, via_shell.protocol_pin);
        assert_eq!(from_json.host_generation, via_shell.host_generation);
        assert_eq!(from_json.host_pub, via_shell.host_pub);
        assert_eq!(from_json.host_attestation, via_shell.host_attestation);
    }
}
