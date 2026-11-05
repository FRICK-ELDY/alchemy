//! x0039: canonical ManifestSeal body hash for world-server.
//! x0075: `--serve` Erlang `{:packet, 4}` loop (Port worker; one-shot path kept).
//!
//! One-shot: JSON from file path (argv[1]) or stdin (`-` / no arg) → lowercase BLAKE3 hex.
//! `--serve`: length-prefixed JSON requests → hex or soft-error replies.
//!
//! Guarantees: same encode path as peers (`manifest_seal_json` + `manifest_seal_blake3_hex`).
//! Non-guarantees: HTTP / JWT verification (caller supplies attested fields).

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use alchemy_protocol::manifest_seal_json::{blake3_hex_from_json_bytes, blake3_hex_from_json_str};
use alchemy_protocol::port_framing::{serve_loop, ServeReply};

/// Soft ceiling for seal JSON (scene payloads are small vs blob uploads).
const SERVE_MAX_LEN: u32 = 4 * 1024 * 1024;

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
        |payload| match blake3_hex_from_json_bytes(payload, true)
            .and_then(|h| ServeReply::ok_hex(&h))
        {
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
    match blake3_hex_from_json_str(&raw, false) {
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
