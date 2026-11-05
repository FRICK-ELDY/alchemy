//! Host shutdown (Slice E / x001e / x0032 / x0072). Isolated from system_ui layout knowledge.
//!
//! Guarantees: best-effort stop of local host via launcher IPC, then PID, then opt-in Zenoh.
//! Prefer discovery LOOKUP (x0072) so Exit Client+Host still hits a restarted host.
//! When discovery is configured, LOOKUP failure does **not** fall back to spawn-time
//! `ALCHEMY_HOST_IPC_ADDR` (stale after restart). Env addr is only for discovery-less launches.
//! Non-guarantees: remote hosts, multi-host.
//!
//! Order (x0032 / x0072):
//! 1. Optional `ALCHEMY_LAUNCHER_DISCOVERY_ADDR` LOOKUP → fresh IPC addr (required if set)
//! 2. Else `ALCHEMY_HOST_IPC_ADDR` (+ token) → TCP `SHUTDOWN <token>` (loopback only)
//! 3. `ALCHEMY_HOST_PID` SIGTERM (Unix)
//! 4. Zenoh `host_control` only when `ALCHEMY_HOST_CONTROL=1`

use alchemy_protocol::host_ipc::{self, LookupOk, ENV_ADDR, ENV_DISCOVERY_ADDR, ENV_TOKEN};
use alchemy_protocol::{host_control_key, host_control_zenoh_enabled, HOST_CONTROL_SHUTDOWN};
use anyhow::Result;
use zenoh::Wait;

/// Try launcher IPC, then `ALCHEMY_HOST_PID`, else optional Zenoh `shutdown`.
pub fn request_host_shutdown(session: &zenoh::Session, room_id: &str, wire_cap: Option<&str>) {
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
    let Some(cap) = wire_cap else {
        log::warn!("host_control Zenoh shutdown skipped: no wire_cap (x004e)");
        return;
    };
    if let Err(err) = publish_shutdown(session, room_id, cap) {
        log::warn!("host_control publish failed: {err:#}");
    }
}

fn try_launcher_ipc() -> bool {
    let Ok(token) = std::env::var(ENV_TOKEN) else {
        return false;
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return false;
    }

    let addr = resolve_host_ipc_addr(&token);
    let Some(addr) = addr else {
        return false;
    };
    if !host_ipc::is_loopback_addr(&addr) {
        log::warn!("host IPC addr refused (not loopback): {addr}");
        return false;
    }
    match host_ipc::request_shutdown(&addr, &token) {
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

/// Resolve host IPC for shutdown.
///
/// - Discovery env set → LOOKUP only (no stale `ENV_ADDR` fallback).
/// - Discovery unset → spawn-time `ENV_ADDR` (manual / discovery-less launch).
fn resolve_host_ipc_addr(token: &str) -> Option<String> {
    let discovery = std::env::var(ENV_DISCOVERY_ADDR)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let env_addr = std::env::var(ENV_ADDR)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref discovery) = discovery {
        if !host_ipc::is_loopback_addr(discovery) {
            log::warn!("{ENV_DISCOVERY_ADDR} refused (not loopback): {discovery}");
            return choose_host_ipc_addr(Some(Err(())), env_addr.as_deref());
        }
        let lookup = match host_ipc::request_lookup(discovery, token) {
            Ok(ok) => {
                log::info!("discovery LOOKUP gen={} ipc={}", ok.gen, ok.ipc_addr);
                Ok(ok)
            }
            Err(err) => {
                log::warn!(
                    "discovery LOOKUP failed ({err}); not falling back to {ENV_ADDR} (x0072)"
                );
                Err(())
            }
        };
        return match lookup {
            Ok(ok) => choose_host_ipc_addr(Some(Ok(&ok)), env_addr.as_deref()),
            Err(()) => choose_host_ipc_addr(Some(Err(())), env_addr.as_deref()),
        };
    }

    choose_host_ipc_addr(None, env_addr.as_deref())
}

/// Pure policy for addr selection (used by [`resolve_host_ipc_addr`], unit-tested).
///
/// `lookup` is `Some` only when discovery was configured:
/// - `Ok` → use ipc (`parse_lookup_ok` already requires loopback)
/// - `Err` → no fallback to `env_addr`
///
/// When `lookup` is `None` (discovery unset), use `env_addr`.
fn choose_host_ipc_addr(
    lookup: Option<Result<&LookupOk, ()>>,
    env_addr: Option<&str>,
) -> Option<String> {
    match lookup {
        Some(Ok(ok)) => Some(ok.ipc_addr.clone()),
        Some(Err(())) => None,
        None => env_addr
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
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
    use super::*;

    #[test]
    fn choose_prefers_discovery_ok_over_env() {
        let ok = LookupOk {
            gen: 2,
            ipc_addr: "127.0.0.1:9".into(),
        };
        assert_eq!(
            choose_host_ipc_addr(Some(Ok(&ok)), Some("127.0.0.1:1")).as_deref(),
            Some("127.0.0.1:9")
        );
    }

    #[test]
    fn choose_discovery_err_does_not_fallback_to_env() {
        assert!(choose_host_ipc_addr(Some(Err(())), Some("127.0.0.1:1")).is_none());
    }

    #[test]
    fn choose_without_discovery_uses_env() {
        assert_eq!(
            choose_host_ipc_addr(None, Some("127.0.0.1:45555")).as_deref(),
            Some("127.0.0.1:45555")
        );
        assert!(choose_host_ipc_addr(None, Some("")).is_none());
        assert!(choose_host_ipc_addr(None, None).is_none());
    }
}
