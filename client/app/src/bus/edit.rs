//! EditCommit subscriber (x00b2 / x00b3).
//!
//! Guarantees: AEAD open as `EditCommit` → decode → **`host_pub` required** (fail-closed,
//! same as Frame/Presence) → verify `host_sig` → update live tip watermark on accept.
//! On accept tip gap (`scene_rev > last+1`) → `fetch_snapshot` and adopt snapshot `scene_rev`.
//!
//! Non-guarantees: Undo UI; predictive local apply of Reparent/SetField; full offline VCS.

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::verify_edit_commit;
use alchemy_protocol::{EditCommit, Message};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use zenoh::bytes::ZBytes;
use zenoh::Wait;

use super::frame::fetch_snapshot;
use super::wire::open_zenoh_payload;
use crate::scene::SharedState;

const EDIT_GAP_SNAPSHOT_COOLDOWN: Duration = Duration::from_millis(500);

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

    thread::Builder::new()
        .name("zenoh-edit-commit".into())
        .spawn(move || {
            log::info!("subscribed to {key} (x00b3 EditCommit)");
            let mut gap_cooldown_until = Instant::now();
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
                    let gap = edit_scene_rev_gap(&shared, commit.scene_rev);
                    if gap {
                        let now = Instant::now();
                        if now >= gap_cooldown_until {
                            gap_cooldown_until = now + EDIT_GAP_SNAPSHOT_COOLDOWN;
                            let room_id = shared.get_room_id();
                            if let Some(wire_cap) = shared.wire_cap() {
                                log::info!(
                                    "EditCommit gap tip={} → snapshot resync",
                                    commit.scene_rev
                                );
                                let _ = fetch_snapshot(
                                    &session,
                                    room_id.as_ref(),
                                    wire_cap.as_ref(),
                                    &shared,
                                );
                                // Watermark may already be advanced by fetch_snapshot;
                                // still fold this commit tip.
                            }
                        }
                    }
                    if let Ok(mut slot) = shared.last_edit_scene_rev.lock() {
                        let next = match *slot {
                            Some(prev) => prev.max(commit.scene_rev),
                            None => commit.scene_rev,
                        };
                        *slot = Some(next);
                    }
                    note_save_commit(&shared, &commit, true);
                    log::info!(
                        "EditCommit accepted op_id={} scene_rev={}",
                        commit.op_id,
                        commit.scene_rev
                    );
                } else {
                    // APPLY = Ready/queue/barrier fail (tip unchanged) → immediate retry OK.
                    // STALE_PRECONDITION → refresh expected_* / snapshot before retry.
                    let hint = match commit.reject_reason {
                        alchemy_protocol::edit_reject::APPLY => "apply_fail_retry_ok",
                        alchemy_protocol::edit_reject::STALE_PRECONDITION => "stale_refresh_first",
                        alchemy_protocol::edit_reject::SAVE_FORBIDDEN => "save_forbidden",
                        alchemy_protocol::edit_reject::SAVE_CONFLICT => "save_conflict",
                        _ => "other",
                    };
                    note_save_commit(&shared, &commit, false);
                    log::info!(
                        "EditCommit rejected op_id={} reason={} scene_rev={} hint={hint}",
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

/// True when `tip` skips at least one live_scene_rev after `prev` (0/None = no tip yet).
pub(crate) fn edit_scene_rev_gap_from_prev(prev: Option<u64>, tip: u64) -> bool {
    match prev {
        Some(p) => tip > p.saturating_add(1),
        None => tip > 1,
    }
}

fn edit_scene_rev_gap(shared: &SharedState, tip: u64) -> bool {
    let Ok(slot) = shared.last_edit_scene_rev.lock() else {
        return false;
    };
    edit_scene_rev_gap_from_prev(*slot, tip)
}

/// If `commit.op_id` matches a pending Room Save, publish Dash status (x00b7).
///
/// Writes status first, then compare-and-clears `pending_save_op_id` only while it still
/// equals `commit.op_id`. Status lock failure leaves pending set so busy stays (no
/// silent unlock to a stuck `saving…` with pending already cleared).
fn note_save_commit(shared: &SharedState, commit: &EditCommit, accepted: bool) {
    let matches = match shared.pending_save_op_id.lock() {
        Ok(g) => *g == Some(commit.op_id),
        Err(_) => return,
    };
    if !matches {
        return;
    }

    let status = if accepted {
        "saved".to_string()
    } else {
        match commit.reject_reason {
            alchemy_protocol::edit_reject::SAVE_FORBIDDEN => {
                "forbidden — active host must be scene writer (owner/editor)".into()
            }
            alchemy_protocol::edit_reject::SAVE_CONFLICT => "conflict — try Save again".into(),
            alchemy_protocol::edit_reject::CYCLE => "cycle — parent loop rejected".into(),
            alchemy_protocol::edit_reject::PAYLOAD => "invalid save patch".into(),
            alchemy_protocol::edit_reject::WORLD => "save failed — try again".into(),
            alchemy_protocol::edit_reject::AUTH => "forbidden — not authorized".into(),
            other => format!("save rejected (reason={other})"),
        }
    };
    let Ok(mut slot) = shared.room_save_status.lock() else {
        return;
    };
    *slot = Some(status);
    drop(slot);

    if let Ok(mut pending) = shared.pending_save_op_id.lock() {
        if *pending == Some(commit.op_id) {
            *pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::edit_scene_rev_gap_from_prev;
    use std::sync::Mutex;

    #[test]
    fn scene_rev_gap_triggers_when_skipping() {
        assert!(edit_scene_rev_gap_from_prev(None, 2));
        assert!(!edit_scene_rev_gap_from_prev(None, 1));
        assert!(!edit_scene_rev_gap_from_prev(Some(5), 6));
        assert!(edit_scene_rev_gap_from_prev(Some(5), 7));
    }

    #[test]
    fn watermark_max_is_monotonic() {
        let slot = Mutex::new(Some(3u64));
        let tip = 5u64;
        let mut g = slot.lock().unwrap();
        *g = Some(g.unwrap().max(tip));
        assert_eq!(*g, Some(5));
    }
}
