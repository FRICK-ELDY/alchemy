//! Slice E stub: listen for `host_control` shutdown (room-scoped; x0017).
//!
//! Guarantees: process exit on payload `shutdown`.
//! Non-guarantees: auth on control channel, multi-tenant rooms, launcher.

use alchemy_protocol::{host_control_key, HOST_CONTROL_SHUTDOWN};
use anyhow::{Context, Result};
use zenoh::Wait;

pub fn spawn_listener(session: zenoh::Session, room_id: &str) -> Result<()> {
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
