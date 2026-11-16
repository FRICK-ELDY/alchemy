//! x00c8: Spatial VCS Tree/Commit codec Port for assets-server (wiring in later slice).
//!
//! Modes:
//! - `--serve`: Erlang `{:packet, 4}` loop; JSON request → JSON reply (or `!` soft-error)
//! - one-shot: JSON from file path / `-` / stdin → JSON on stdout
//!
//! Guarantees: same encode/verify path as `alchemy_protocol::vcs_object` via
//! [`alchemy_protocol::vcs_object_port`]. Oneshot input capped like `--serve`.
//! Non-guarantees: assets Port pool; DB writes; push negotiation.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use alchemy_protocol::port_framing::{serve_loop, ServeReply};
use alchemy_protocol::vcs_object::MAX_OBJECT_BYTES;
use alchemy_protocol::vcs_object_port::handle_json;

const SERVE_MAX_LEN: u32 = MAX_OBJECT_BYTES as u32 + 64 * 1024;

fn main() -> ExitCode {
    if std::env::args().any(|a| a == "--serve") {
        if let Err(err) = run_serve() {
            eprintln!("vcs-object: {err}");
            return ExitCode::from(2);
        }
        return ExitCode::SUCCESS;
    }

    let raw = match read_oneshot_raw() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("vcs-object: {err}");
            return ExitCode::from(2);
        }
    };
    match handle_json(&raw) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("vcs-object: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_serve() -> Result<(), String> {
    let max_len = oneshot_max_len();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    serve_loop(&mut reader, &mut writer, max_len, |payload| {
        let text = match std::str::from_utf8(payload) {
            Ok(s) => s,
            Err(e) => return ServeReply::soft_error(format!("utf8: {e}")),
        };
        match handle_json(text) {
            Ok(out) => match ServeReply::ok_bytes(out.into_bytes()) {
                Ok(r) => r,
                Err(e) => ServeReply::soft_error(e),
            },
            Err(e) => ServeReply::soft_error(e),
        }
    })
    .map_err(|e| e.to_string())
}

fn oneshot_max_len() -> u32 {
    std::env::var("VCS_OBJECT_MAX_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SERVE_MAX_LEN)
}

fn read_oneshot_raw() -> Result<String, String> {
    let max = oneshot_max_len() as usize;
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--serve")
        .collect();
    let bytes = match args.first().map(|s| s.as_str()) {
        None | Some("-") => read_limited(&mut io::stdin().lock(), max)?,
        Some(path) => {
            let mut f = File::open(Path::new(path)).map_err(|e| format!("open {path}: {e}"))?;
            if let Ok(meta) = f.metadata() {
                if meta.len() > max as u64 {
                    return Err(format!("oversize file len={} max={max}", meta.len()));
                }
            }
            read_limited(&mut f, max)?
        }
    };
    String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
}

fn read_limited<R: Read>(reader: &mut R, max: usize) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > max {
            return Err(format!("oversize input max={max}"));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}
