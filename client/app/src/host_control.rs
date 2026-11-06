//! Host shutdown helpers (Slice E / x001e / x0032; discovery LOOKUP retired with launcher).
//!
//! Guarantees: best-effort stop of a local host via IPC addr/token env, then PID, then opt-in Zenoh.
//! Preferred path for supervised hosts is [`crate::host_supervisor::HostSupervisor::shutdown_graceful`];
//! this module covers external / CI hosts and falls through when supervisor did not own the child.
//!
//! Non-guarantees: remote hosts, multi-host, launcher discovery LOOKUP (abolished).
//!
//! Order:
//! 1. `ALCHEMY_HOST_IPC_ADDR` (+ token) → TCP `SHUTDOWN <token>` (loopback only)
//! 2. `ALCHEMY_HOST_PID` SIGTERM (Unix)
//! 3. Zenoh `host_control` only when `ALCHEMY_HOST_CONTROL=1`

use alchemy_protocol::host_ipc::{self, ENV_ADDR, ENV_TOKEN};
use alchemy_protocol::{host_control_key, host_control_zenoh_enabled, HOST_CONTROL_SHUTDOWN};
use anyhow::Result;
use zenoh::Wait;

/// Try IPC env, then `ALCHEMY_HOST_PID`, else optional Zenoh `shutdown`.
pub fn request_host_shutdown(session: &zenoh::Session, room_id: &str, wire_cap: Option<&str>) {
    if try_host_ipc() {
        return;
    }
    if try_kill_host_pid() {
        return;
    }
    if !host_control_zenoh_enabled() {
        log::warn!(
            "host shutdown: no host IPC / usable ALCHEMY_HOST_PID and Zenoh host_control disabled (set ALCHEMY_HOST_CONTROL=1 for local stub)"
        );
        return;
    }
    let Some(cap) = wire_cap else {
        log::warn!("host_control Zenoh shutdown skipped: no wire_cap (x004e)");
        return;
    };
    if let Err(err) = publish_shutdown(session, room_id, cap) {
        log::warn!("host_control publish failed: {err:#}");
    }
}

fn try_host_ipc() -> bool {
    let Ok(token) = std::env::var(ENV_TOKEN) else {
        return false;
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return false;
    }

    let addr = std::env::var(ENV_ADDR)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let Some(addr) = addr else {
        return false;
    };
    if !host_ipc::is_loopback_addr(&addr) {
        log::warn!("host IPC addr refused (not loopback): {addr}");
        return false;
    }
    match host_ipc::request_shutdown(&addr, &token) {
        Ok(()) => {
            log::info!("sent host IPC shutdown to {addr}");
            true
        }
        Err(err) => {
            log::warn!("host IPC shutdown failed: {err}");
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
        log::warn!("ALCHEMY_HOST_PID kill is Unix-only; prefer host IPC ({ENV_ADDR}) on Windows");
        false
    }
}

fn publish_shutdown(session: &zenoh::Session, room_id: &str, wire_cap: &str) -> Result<()> {
    let key = host_control_key(room_id, wire_cap);
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

#[cfg(test)]
mod tests {
    #[test]
    fn module_loads() {
        // Discovery LOOKUP policy tests removed with launcher; IPC helpers live in protocol.
        assert!(alchemy_protocol::host_ipc::is_loopback_addr("127.0.0.1:9"));
    }
}
