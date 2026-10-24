//! Host shutdown stub (Slice E). Isolated from system_ui layout knowledge.
//!
//! Guarantees: best-effort stop of local host via PID or Zenoh control key.
//! Non-guarantees: remote hosts, multi-host, launcher-managed lifecycle.

use alchemy_protocol::{HOST_CONTROL_KEY_LOCAL, HOST_CONTROL_SHUTDOWN};
use anyhow::Result;
use zenoh::Wait;

/// Try `ALCHEMY_HOST_PID` SIGTERM (Unix), else publish `shutdown` on host_control.
pub fn request_host_shutdown(session: &zenoh::Session) {
    if try_kill_host_pid() {
        return;
    }
    if let Err(err) = publish_shutdown(session) {
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
        log::warn!("ALCHEMY_HOST_PID kill is Unix-only in Slice E");
        false
    }
}

fn publish_shutdown(session: &zenoh::Session) -> Result<()> {
    let publisher = session
        .declare_publisher(HOST_CONTROL_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher host_control: {e}"))?;
    publisher
        .put(HOST_CONTROL_SHUTDOWN)
        .wait()
        .map_err(|e| anyhow::anyhow!("host_control put: {e}"))?;
    log::info!("published shutdown on {HOST_CONTROL_KEY_LOCAL}");
    Ok(())
}
