//! Host shutdown stub (Slice E / x001e). Isolated from system_ui layout knowledge.
//!
//! Guarantees: best-effort stop of local host via PID; Zenoh control only when opted in.
//! Non-guarantees: remote hosts, multi-host, launcher-managed lifecycle.
//!
//! Default (x001e): do **not** publish Zenoh `host_control` (unauthenticated). Prefer
//! `ALCHEMY_HOST_PID` (Unix). Set `ALCHEMY_HOST_CONTROL=1` only for local/dev.

use alchemy_protocol::{host_control_key, host_control_zenoh_enabled, HOST_CONTROL_SHUTDOWN};
use anyhow::Result;
use zenoh::Wait;

/// Try `ALCHEMY_HOST_PID` SIGTERM (Unix), else optional Zenoh `shutdown` when enabled.
pub fn request_host_shutdown(session: &zenoh::Session, room_id: &str) {
    if try_kill_host_pid() {
        return;
    }
    if !host_control_zenoh_enabled() {
        log::warn!(
            "host shutdown: no usable ALCHEMY_HOST_PID and Zenoh host_control disabled (set ALCHEMY_HOST_CONTROL=1 for local stub; PID kill is Unix-only)"
        );
        return;
    }
    if let Err(err) = publish_shutdown(session, room_id) {
        log::warn!("host_control publish failed: {err:#}");
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
        log::warn!("ALCHEMY_HOST_PID kill is Unix-only; use ALCHEMY_HOST_CONTROL=1 for Zenoh stub on Windows");
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
