//! Slice E stub: listen for local `host_control` shutdown.
//!
//! Guarantees: process exit on payload `shutdown`.
//! Non-guarantees: auth on control channel, multi-tenant rooms, launcher.

use alchemy_protocol::{HOST_CONTROL_KEY_LOCAL, HOST_CONTROL_SHUTDOWN};
use anyhow::{Context, Result};
use zenoh::Wait;

pub fn spawn_listener(session: zenoh::Session) -> Result<()> {
    let subscriber = session
        .declare_subscriber(HOST_CONTROL_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber host_control: {e}"))?;

    std::thread::Builder::new()
        .name("zenoh-host-control".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {HOST_CONTROL_KEY_LOCAL} (shutdown stub)");
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
