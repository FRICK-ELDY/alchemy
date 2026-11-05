//! x0075: raw-bytes BLAKE3 for assets-server Port worker (replaces one-shot `b3sum`).
//!
//! Modes:
//! - `--serve`: Erlang `{:packet, 4}` loop on stdin/stdout
//! - one-shot: file path or `-` / no arg → lowercase hex on stdout
//!
//! Guarantees: hex digest of input bytes; soft oversize/errors keep `--serve` alive.
//! Non-guarantees: streaming API; parallel clients on one process.

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use alchemy_protocol::port_framing::{serve_loop, ServeReply};

/// Align with assets `:max_upload_bytes` default (100 MiB).
const DEFAULT_MAX_LEN: u32 = 100_000_000;

fn hash_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

fn read_oneshot() -> Result<Vec<u8>, String> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("-") => {
            let mut raw = Vec::new();
            io::stdin()
                .read_to_end(&mut raw)
                .map_err(|e| format!("read stdin: {e}"))?;
            Ok(raw)
        }
        Some("--serve") => unreachable!("handled in main"),
        Some(path) => fs::read(Path::new(path)).map_err(|e| format!("read {path}: {e}")),
    }
}

fn run_serve() -> Result<(), String> {
    let max_len = std::env::var("BLAKE3_HASH_MAX_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_LEN);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    serve_loop(
        &mut reader,
        &mut writer,
        max_len,
        |payload| match ServeReply::ok_hex(&hash_hex(payload)) {
            Ok(r) => r,
            Err(e) => ServeReply::soft_error(e),
        },
    )
    .map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let serve = std::env::args().any(|a| a == "--serve");
    if serve {
        if let Err(err) = run_serve() {
            eprintln!("blake3-hash: {err}");
            return ExitCode::from(2);
        }
        return ExitCode::SUCCESS;
    }

    match read_oneshot() {
        Ok(data) => {
            println!("{}", hash_hex(&data));
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("blake3-hash: {err}");
            ExitCode::from(2)
        }
    }
}
