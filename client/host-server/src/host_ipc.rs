//! Localhost TCP shutdown for alchemy-launcher (x0032).
//!
//! Guarantees: when `ALCHEMY_HOST_IPC_TOKEN` is set, bind `127.0.0.1:0` (no TOCTOU),
//! accept matching `SHUTDOWN <token>\n`, set [`shutdown_requested`] for main to leave
//! cleanly (exit 0 — intentional; launcher must not treat as crash-restart).
//! Non-guarantees: remote hosts, TLS, multi-command IPC, Zenoh host_control (separate).

use alchemy_protocol::host_ipc::{authenticates_shutdown, ENV_TOKEN};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static BOUND_ADDR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn bound_slot() -> &'static Mutex<Option<String>> {
    BOUND_ADDR.get_or_init(|| Mutex::new(None))
}

/// True after an authenticated IPC SHUTDOWN (main should break and return Ok).
pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

/// Bound `127.0.0.1:port` after [`spawn_listener`], if IPC is enabled.
pub fn bound_addr() -> Option<String> {
    bound_slot().lock().ok().and_then(|g| g.clone())
}

/// Bind loopback ephemeral port when token env is set. Holds the listener until process end.
pub fn spawn_listener() -> Result<()> {
    let Ok(token) = std::env::var(ENV_TOKEN) else {
        log::info!("{ENV_TOKEN} unset — launcher IPC shutdown disabled");
        return Ok(());
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        log::info!("{ENV_TOKEN} empty — launcher IPC shutdown disabled");
        return Ok(());
    }

    // Bind and *keep* the socket — launcher never pre-reserves the port (x0032 TOCTOU fix).
    let listener = TcpListener::bind("127.0.0.1:0").context("bind launcher IPC 127.0.0.1:0")?;
    let bound = listener
        .local_addr()
        .context("launcher IPC local_addr")?
        .to_string();
    {
        let mut slot = bound_slot().lock().expect("BOUND_ADDR mutex");
        *slot = Some(bound.clone());
    }
    log::info!("launcher IPC listening on {bound}");

    std::thread::Builder::new()
        .name("host-ipc".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        if handle_client(stream, &token) {
                            log::info!(
                                "launcher IPC: authenticated shutdown — signaling main loop"
                            );
                            SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                    Err(err) => log::warn!("launcher IPC accept: {err}"),
                }
            }
        })
        .context("spawn host-ipc thread")?;
    Ok(())
}

fn handle_client(stream: TcpStream, expected_token: &str) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    if authenticates_shutdown(&line, expected_token) {
        true
    } else {
        log::warn!("launcher IPC: ignored unauthenticated or malformed line");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::host_ipc::request_shutdown;
    use std::net::TcpListener;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn shutdown_line_accepts_matching_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let b2 = Arc::clone(&barrier);
        let handle = thread::spawn(move || {
            b2.wait();
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream, "secret-token")
        });
        barrier.wait();
        thread::sleep(Duration::from_millis(20));
        request_shutdown(&addr.to_string(), "secret-token").unwrap();
        assert!(handle.join().unwrap());
    }

    #[test]
    fn shutdown_rejects_bad_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let b2 = Arc::clone(&barrier);
        let handle = thread::spawn(move || {
            b2.wait();
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream, "secret-token")
        });
        barrier.wait();
        thread::sleep(Duration::from_millis(20));
        request_shutdown(&addr.to_string(), "wrong").unwrap();
        assert!(!handle.join().unwrap());
    }
}
