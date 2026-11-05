//! x0039: canonical ManifestSeal body hash for world-server.
//! x0075: `--serve` Erlang `{:packet, 4}` loop (Port worker; one-shot path kept).
//!
//! One-shot: JSON from file path (argv[1]) or stdin (`-` / no arg) → lowercase BLAKE3 hex.
//! `--serve`: length-prefixed JSON requests → hex or soft-error replies.
//!
//! Guarantees: same encode path as peers (`manifest_seal_blake3_hex`).
//! Non-guarantees: HTTP / JWT verification (caller supplies attested fields).

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use alchemy_protocol::port_framing::{serve_loop, ServeReply};
use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;
use alchemy_protocol::{
    normalize_scene_entry_trs, LocomotionConfig, LocomotionMode, SceneEntry, SceneManifest,
    PROTOCOL_PIN,
};
use serde::Deserialize;

/// Soft ceiling for seal JSON (scene payloads are small vs blob uploads).
const SERVE_MAX_LEN: u32 = 4 * 1024 * 1024;

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

fn loco_mode(mode: &str, quiet: bool) -> LocomotionMode {
    match mode.trim().to_ascii_lowercase().as_str() {
        "fly" => LocomotionMode::Fly,
        "walk" => LocomotionMode::Walk,
        "noclip" | "" => LocomotionMode::Noclip,
        other => {
            if !quiet {
                eprintln!("manifest-seal-hash: unknown locomotion mode {other:?}; using noclip");
            }
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

fn hash_json_bytes(raw: &[u8], quiet: bool) -> Result<String, String> {
    let text = std::str::from_utf8(raw).map_err(|e| format!("utf8: {e}"))?;
    hash_json_str(text, quiet)
}

fn hash_json_str(raw: &str, quiet: bool) -> Result<String, String> {
    let input: Input = serde_json::from_str(raw).map_err(|err| format!("invalid JSON: {err}"))?;
    let host_pub = decode_host_pub_hex(&input.host_pub_hex)?;
    if input.host_generation == 0 {
        return Err("host_generation must be > 0".into());
    }
    let pin = input
        .protocol_pin
        .unwrap_or_else(|| PROTOCOL_PIN.to_string());
    let loco = input.locomotion.map(|l| LocomotionConfig {
        mode: loco_mode(&l.mode, quiet).into(),
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
    Ok(manifest_seal_blake3_hex(&manifest))
}

fn read_oneshot_raw() -> Result<String, String> {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--serve")
        .collect();
    match args.first().map(|s| s.as_str()) {
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

fn run_serve() -> Result<(), String> {
    let max_len = std::env::var("MANIFEST_SEAL_HASH_MAX_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SERVE_MAX_LEN);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    serve_loop(
        &mut reader,
        &mut writer,
        max_len,
        |payload| match hash_json_bytes(payload, true).and_then(|h| ServeReply::ok_hex(&h)) {
            Ok(r) => r,
            Err(e) => ServeReply::soft_error(e),
        },
    )
    .map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    if std::env::args().any(|a| a == "--serve") {
        if let Err(err) = run_serve() {
            eprintln!("manifest-seal-hash: {err}");
            return ExitCode::from(2);
        }
        return ExitCode::SUCCESS;
    }

    let raw = match read_oneshot_raw() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("manifest-seal-hash: {err}");
            return ExitCode::from(2);
        }
    };
    match hash_json_str(&raw, false) {
        Ok(hex) => {
            println!("{hex}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("manifest-seal-hash: {err}");
            ExitCode::from(2)
        }
    }
}
