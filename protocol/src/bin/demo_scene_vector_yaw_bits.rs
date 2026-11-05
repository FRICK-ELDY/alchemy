//! Print paste-ready center-box yaw `f64::from_bits` for the DemoScene vector (x0079).
//!
//! Used by `world-server/scripts/regen_demo_scene_vector.exs` after rewriting the
//! golden JSON. Args: optional path to `demo_scene_seal.json` (default:
//! `protocol/testdata/demo_scene_seal.json` next to this crate).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use alchemy_protocol::manifest_seal_json::scene_manifest_from_json_str;

fn default_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/demo_scene_seal.json")
}

fn main() -> ExitCode {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_fixture);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("demo-scene-vector-yaw-bits: read {}: {err}", path.display());
            return ExitCode::from(2);
        }
    };
    let manifest = match scene_manifest_from_json_str(&raw, true) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("demo-scene-vector-yaw-bits: {err}");
            return ExitCode::from(2);
        }
    };
    let Some(center) = manifest.entries.iter().find(|e| e.entity_id == 14) else {
        eprintln!("demo-scene-vector-yaw-bits: entity_id=14 missing");
        return ExitCode::from(2);
    };
    println!(
        "YAW45_QY: f64 = f64::from_bits(0x{:x});",
        center.qy.to_bits()
    );
    println!(
        "YAW45_QW: f64 = f64::from_bits(0x{:x});",
        center.qw.to_bits()
    );
    ExitCode::SUCCESS
}
