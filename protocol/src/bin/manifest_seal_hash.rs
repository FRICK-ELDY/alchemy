//! x0039: canonical ManifestSeal body hash for world-server.
//!
//! Reads one JSON object from a file path (argv[1]) or stdin (`-` / no arg),
//! builds `SceneManifest` with empty `world_seal`, prints lowercase BLAKE3 hex.
//!
//! Guarantees: same encode path as peers (`manifest_seal_blake3_hex`).
//! Non-guarantees: HTTP / JWT verification (caller supplies attested fields).

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;
use alchemy_protocol::{
    normalize_scene_entry_trs, LocomotionConfig, LocomotionMode, SceneEntry, SceneManifest,
    PROTOCOL_PIN,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Input {
    room_id: String,
    revision: u64,
    /// World-assigned host generation (x003a). Required (>0) for canonical seal.
    host_generation: u64,
    #[serde(default)]
    protocol_pin: Option<String>,
    /// 64-char hex of 32-byte Ed25519 host_pub.
    host_pub_hex: String,
    host_attestation: String,
    #[serde(default)]
    locomotion: Option<LocoJson>,
    #[serde(default)]
    entries: Vec<EntryJson>,
}

#[derive(Debug, Deserialize)]
struct LocoJson {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    move_speed: f32,
}

#[derive(Debug, Deserialize)]
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

fn loco_mode(mode: &str) -> LocomotionMode {
    match mode.trim().to_ascii_lowercase().as_str() {
        "fly" => LocomotionMode::Fly,
        "walk" => LocomotionMode::Walk,
        "noclip" | "" => LocomotionMode::Noclip,
        other => {
            eprintln!("manifest-seal-hash: unknown locomotion mode {other:?}; using noclip");
            LocomotionMode::Noclip
        }
    }
}

fn decode_host_pub_hex(hex_str: &str) -> Result<Vec<u8>, String> {
    let down = hex_str.trim().to_ascii_lowercase();
    if down.len() != 64 || !down.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "host_pub_hex must be 64 hex chars (got len={})",
            down.len()
        ));
    }
    hex::decode(&down).map_err(|e| format!("host_pub_hex decode: {e}"))
}

fn read_input() -> Result<String, String> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("-") => {
            let mut raw = String::new();
            io::stdin()
                .read_to_string(&mut raw)
                .map_err(|e| format!("read stdin: {e}"))?;
            Ok(raw)
        }
        Some(path) => fs::read_to_string(Path::new(path)).map_err(|e| format!("read {path}: {e}")),
    }
}

fn main() {
    let raw = match read_input() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("manifest-seal-hash: {err}");
            std::process::exit(2);
        }
    };
    let input: Input = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("manifest-seal-hash: invalid JSON: {err}");
            std::process::exit(2);
        }
    };
    let host_pub = match decode_host_pub_hex(&input.host_pub_hex) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("manifest-seal-hash: {err}");
            std::process::exit(2);
        }
    };
    if input.host_generation == 0 {
        eprintln!("manifest-seal-hash: host_generation must be > 0");
        std::process::exit(2);
    }
    let pin = input
        .protocol_pin
        .unwrap_or_else(|| PROTOCOL_PIN.to_string());
    let loco = input.locomotion.map(|l| LocomotionConfig {
        mode: loco_mode(&l.mode).into(),
        move_speed: l.move_speed,
    });
    let entries: Vec<SceneEntry> = input
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
        .collect();

    let manifest = SceneManifest {
        room_id: input.room_id,
        revision: input.revision,
        entries,
        locomotion: loco,
        protocol_pin: pin,
        host_pub,
        host_attestation: input.host_attestation,
        world_seal: String::new(),
        host_generation: input.host_generation,
    };
    println!("{}", manifest_seal_blake3_hex(&manifest));
}
