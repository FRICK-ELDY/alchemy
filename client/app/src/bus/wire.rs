//! Zenoh payload wire helpers: AEAD open/seal at the bus boundary + client wall clock.
//!
//! Guarantees:
//! - `open_zenoh_payload`: ciphertext size ceiling on `ZBytes::len` → contiguous copy → AEAD open;
//!   fail-closed **before** protobuf / signature checks (x003b)
//! - Dual-open pending→live while seamless epoch switch is in flight (x00d5)
//! - `seal_zenoh_payload`: AEAD seal with pending (if any) else live `room_key` + `room_id` binding
//! - `bus_generation` snapshot: open/seal fail-closed if generation changes mid-call (x00d5).
//!   Bumped on pending commit / KeyStale / Leave / epoch-changing Join — **not** same-epoch
//!   soft-TTL `Commit` (that would drop frames on every Token refresh).
//!
//! Non-guarantees: protobuf decode, signature verification, decode-limit checks;
//! KeyStale overlay (callers). AEAD open failure rate-limits a Room Token refresh nudge so
//! guests detect host epoch rotate before FRAME_STALL (x007e).
//! `client_time_ms` is a best-effort wall clock.

use std::time::{SystemTime, UNIX_EPOCH};

use alchemy_protocol::wire_aead::{self, AeadKind, WireAeadError};
use zenoh::bytes::ZBytes;

use crate::scene::SharedState;

pub(crate) fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Ciphertext ceiling → contiguous copy → AEAD dual-open → `(plaintext, opened_epoch)`.
/// Fail-closed before protobuf / signature checks. `opened_epoch` is the key epoch that
/// decrypted (pending/current preferred over live/prev).
pub(crate) fn open_zenoh_payload(
    shared: &SharedState,
    kind: AeadKind,
    bytes: &ZBytes,
) -> Result<(Vec<u8>, u64), String> {
    // Soft grace poll: pending deadline → hard KeyStale before open attempt.
    let _ = shared.poll_pending_room_aead_grace();

    if let Err(err) = wire_aead::check_aead_envelope_bytes(kind, bytes.len()) {
        return Err(err.to_string());
    }
    let (current_key, current_epoch, prev, gen) = shared
        .room_aead_open_view()
        .ok_or_else(|| "room_key missing".to_string())?;
    let envelope = bytes.to_bytes();
    let room_id = shared.get_room_id();
    let opened = open_with_epoch(
        current_key.as_ref().as_slice(),
        current_epoch,
        prev.as_ref().map(|(k, e)| (k.as_ref().as_slice(), *e)),
        room_id.as_ref(),
        kind,
        envelope.as_ref(),
    );
    if shared.bus_generation() != gen {
        return Err("bus_generation changed during open".to_string());
    }
    match opened {
        Ok(pair) => Ok(pair),
        Err(e) => {
            // Host may have rotated ≈15s ahead of soft-TTL refresh; nudge Token so pending
            // or KeyStale fires before FRAME_STALL (x007e / x00d5).
            shared.nudge_room_token_refresh_after_aead_fail();
            Err(e.to_string())
        }
    }
}

/// Same order as [`wire_aead::open_current_or_prev`], but returns which epoch opened.
fn open_with_epoch(
    current_key: &[u8],
    current_epoch: u64,
    prev: Option<(&[u8], u64)>,
    room_id: &str,
    kind: AeadKind,
    envelope: &[u8],
) -> Result<(Vec<u8>, u64), WireAeadError> {
    match wire_aead::open(current_key, room_id, current_epoch, kind, envelope) {
        Ok(pt) => Ok((pt, current_epoch)),
        Err(WireAeadError::CryptographicFailure) => {
            if let Some((prev_key, prev_epoch)) = prev {
                wire_aead::open(prev_key, room_id, prev_epoch, kind, envelope)
                    .map(|pt| (pt, prev_epoch))
            } else {
                Err(WireAeadError::CryptographicFailure)
            }
        }
        Err(other) => Err(other),
    }
}

pub(crate) fn seal_zenoh_payload(
    shared: &SharedState,
    kind: AeadKind,
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let (room_key, epoch, gen) = shared
        .room_aead_seal_view()
        .ok_or_else(|| "room_key missing".to_string())?;
    let sealed = wire_aead::seal(
        room_key.as_ref(),
        shared.get_room_id().as_ref(),
        epoch,
        kind,
        plaintext,
    )
    .map_err(|e| e.to_string())?;
    if shared.bus_generation() != gen {
        return Err("bus_generation changed during seal".to_string());
    }
    Ok(sealed)
}

#[cfg(test)]
mod tests {
    use super::open_with_epoch;
    use alchemy_protocol::wire_aead::{self, AeadKind, ROOM_KEY_LEN};

    #[test]
    fn dual_open_prefers_current_then_prev() {
        let room = "room-x00d5";
        let old_key = [7u8; ROOM_KEY_LEN];
        let new_key = [9u8; ROOM_KEY_LEN];
        let old_ct = wire_aead::seal(&old_key, room, 3, AeadKind::Frame, b"old-frame").unwrap();
        let new_ct = wire_aead::seal(&new_key, room, 4, AeadKind::Frame, b"new-frame").unwrap();

        let (pt, epoch) = open_with_epoch(
            &new_key,
            4,
            Some((old_key.as_slice(), 3)),
            room,
            AeadKind::Frame,
            &new_ct,
        )
        .expect("new opens with current");
        assert_eq!(epoch, 4);
        assert_eq!(pt, b"new-frame");

        let (pt, epoch) = open_with_epoch(
            &new_key,
            4,
            Some((old_key.as_slice(), 3)),
            room,
            AeadKind::Frame,
            &old_ct,
        )
        .expect("old opens with prev");
        assert_eq!(epoch, 3);
        assert_eq!(pt, b"old-frame");
    }

    #[test]
    fn after_commit_style_single_key_rejects_old() {
        let room = "room-x00d5";
        let old_key = [7u8; ROOM_KEY_LEN];
        let new_key = [9u8; ROOM_KEY_LEN];
        let old_ct = wire_aead::seal(&old_key, room, 3, AeadKind::Frame, b"old-frame").unwrap();
        let err = open_with_epoch(&new_key, 4, None, room, AeadKind::Frame, &old_ct);
        assert!(err.is_err(), "old ciphertext must fail without prev");
    }
}
