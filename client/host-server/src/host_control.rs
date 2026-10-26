//! Host shutdown control (Slice E / x001e).
//!
//! Guarantees: when enabled, process exit on payload `shutdown`.
//! Non-guarantees: auth on the Zenoh control channel, multi-tenant rooms, launcher.
//!
//! Default (x001e): Zenoh `host_control` is **off**. WAN peers must not be able to stop host.
//! Opt-in only via `ALCHEMY_HOST_CONTROL=1` (local/dev convenience until launcher IPC).

use alchemy_protocol::{
    host_control_key, host_control_zenoh_enabled, HOST_CONTROL_SHUTDOWN,
};
use anyhow::{Context, Result};
use zenoh::Wait;

pub use alchemy_protocol::host_control_zenoh_enabled as zenoh_control_enabled;

/// Subscribe to `host_control` only when `ALCHEMY_HOST_CONTROL=1`.
/// Otherwise no-op (WAN cannot shut down this process via Zenoh).
pub fn spawn_listener(session: zenoh::Session, room_id: &str) -> Result<()> {
    if !host_control_zenoh_enabled() {
        log::info!(
            "host_control Zenoh listener disabled (set ALCHEMY_HOST_CONTROL=1 to enable; prefer ALCHEMY_HOST_PID / launcher)"
        );
        return Ok(());
    }

    log::warn!(
        "ALCHEMY_HOST_CONTROL=1: unauthenticated Zenoh shutdown is enabled — local/dev only"
    );

    let key = host_control_key(room_id);
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber host_control: {e}"))?;

    std::thread::Builder::new()
        .name("zenoh-host-control".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key} (shutdown stub)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                if bytes.as_ref() == HOST_CONTROL_SHUTDOWN {
                    log::info!("host_control: shutdown received — exiting");
                    std::process::exit(0);
                }
            }
        })
        .context("spawn host_control thread")?;
    Ok(())
}
