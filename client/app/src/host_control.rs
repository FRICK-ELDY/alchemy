//! Host shutdown (Slice E / x001e / x0032). Isolated from system_ui layout knowledge.
//!
//! Guarantees: best-effort stop of local host via launcher IPC, then PID, then opt-in Zenoh.
//! Non-guarantees: remote hosts, multi-host.
//!
//! Order (x0032):
//! 1. `ALCHEMY_HOST_IPC_ADDR` + `ALCHEMY_HOST_IPC_TOKEN` → TCP `SHUTDOWN <token>` (loopback only)
//! 2. `ALCHEMY_HOST_PID` SIGTERM (Unix)
//! 3. Zenoh `host_control` only when `ALCHEMY_HOST_CONTROL=1`

use alchemy_protocol::host_ipc::{self, ENV_ADDR, ENV_TOKEN};
use alchemy_protocol::{host_control_key, host_control_zenoh_enabled, HOST_CONTROL_SHUTDOWN};
use anyhow::Result;
use zenoh::Wait;

/// Try launcher IPC, then `ALCHEMY_HOST_PID`, else optional Zenoh `shutdown`.
pub fn request_host_shutdown(session: &zenoh::Session, room_id: &str) {
    if try_launcher_ipc() {
        return;
    }
    if try_kill_host_pid() {
        return;
    }
    if !host_control_zenoh_enabled() {
        log::warn!(
            "host shutdown: no launcher IPC / usable ALCHEMY_HOST_PID and Zenoh host_control disabled (set ALCHEMY_HOST_CONTROL=1 for local stub)"
        );
        return;
    }
    if let Err(err) = publish_shutdown(session, room_id) {
        log::warn!("host_control publish failed: {err:#}");
    }
}

fn try_launcher_ipc() -> bool {
    let Ok(addr) = std::env::var(ENV_ADDR) else {
        return false;
    };
    let Ok(token) = std::env::var(ENV_TOKEN) else {
        return false;
    };
    let addr = addr.trim();
    let token = token.trim();
    if addr.is_empty() || token.is_empty() {
        return false;
    }
    if !host_ipc::is_loopback_addr(addr) {
        log::warn!("{ENV_ADDR} refused (not loopback): {addr}");
        return false;
    }
    match host_ipc::request_shutdown(addr, token) {
        Ok(()) => {
            log::info!("sent launcher IPC shutdown to {addr}");
            true
        }
        Err(err) => {
            log::warn!("launcher IPC shutdown failed: {err}");
            false
        }
    }
}

fn try_kill_host_pid() -> bool {
    let Ok(raw) = std::env::var("ALCHEMY_HOST_PID") else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<i32>() else {
        log::warn!("ALCHEMY_HOST_PID invalid: {raw}");
        return false;
    };
    if pid <= 1 {
        log::warn!("ALCHEMY_HOST_PID refused: {pid}");
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: pid is a positive user-supplied process id; SIGTERM is a standard signal.
        let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
        if rc == 0 {
            log::info!("sent SIGTERM to host pid={pid}");
            true
        } else {
            log::warn!("SIGTERM pid={pid} failed");
            false
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        log::warn!(
            "ALCHEMY_HOST_PID kill is Unix-only; prefer launcher IPC ({ENV_ADDR}) on Windows"
        );
        false
    }
}

fn publish_shutdown(session: &zenoh::Session, room_id: &str) -> Result<()> {
    let key = host_control_key(room_id);
    let publisher = session
        .declare_publisher(key.as_str())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher host_control: {e}"))?;
    publisher
        .put(HOST_CONTROL_SHUTDOWN)
        .wait()
        .map_err(|e| anyhow::anyhow!("host_control put: {e}"))?;
    log::info!("published shutdown on {key}");
    Ok(())
}
