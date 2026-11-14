//! EditCommit subscriber (x00b2).
//!
//! Guarantees: AEAD open as `EditCommit` → decode → **`host_pub` required** (fail-closed,
//! same as Frame/Presence) → verify `host_sig` → log accept/reject. Does not apply scene
//! locally (host Ready path + Manifest/frame remain physics/scene truth).
//!
//! Non-guarantees: Undo UI; predictive apply; snapshot `scene_rev` gap recovery.

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::verify_edit_commit;
use alchemy_protocol::{EditCommit, Message};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::thread;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

use super::wire::open_zenoh_payload;
use crate::scene::SharedState;

/// Subscribe to `…/edit` and decode host `EditCommit` envelopes (proposals ignored via AEAD kind).
pub fn spawn_edit_commit_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber edit failed: {e}"))?;

    drop(session);

    thread::Builder::new()
        .name("zenoh-edit-commit".into())
        .spawn(move || {
            log::info!("subscribed to {key} (x00b2 EditCommit)");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = match open_zenoh_payload(&shared, AeadKind::EditCommit, &bytes) {
                    Ok(pt) => pt,
                    Err(_) => continue, // EditProposal or AEAD fail
                };
                let Ok(commit) = EditCommit::decode(payload.as_ref()) else {
                    log::warn!("drop EditCommit: decode failed");
                    continue;
                };
                let Some(host_pub) = shared.host_pub.lock().ok().and_then(|g| g.clone()) else {
                    log::warn!("drop EditCommit: host_pub not yet known from Manifest");
                    continue;
                };
                if let Err(err) = verify_edit_commit(&host_pub, &commit) {
                    log::warn!("drop EditCommit: host_sig: {err}");
                    continue;
                }
                if commit.accepted {
                    // Tip watermark: accept only, monotonic max (reject.scene_rev can lag CAS tip).
                    if let Ok(mut slot) = shared.last_edit_scene_rev.lock() {
                        let next = match *slot {
                            Some(prev) => prev.max(commit.scene_rev),
                            None => commit.scene_rev,
                        };
                        *slot = Some(next);
                    }
                    log::info!(
                        "EditCommit accepted op_id={} scene_rev={}",
                        commit.op_id,
                        commit.scene_rev
                    );
                } else {
                    log::info!(
                        "EditCommit rejected op_id={} reason={} scene_rev={}",
                        commit.op_id,
                        commit.reject_reason,
                        commit.scene_rev
                    );
                }
            }
        })
        .context("spawn edit-commit thread")?;
    Ok(())
}
