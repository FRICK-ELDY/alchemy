//! Zenoh payload wire helpers: AEAD open/seal at the bus boundary + client wall clock.
//!
//! Guarantees:
//! - `open_zenoh_payload`: ciphertext size ceiling on `ZBytes::len` → contiguous copy → AEAD open;
//!   fail-closed **before** protobuf / signature checks (x003b)
//! - `seal_zenoh_payload`: AEAD seal with `room_key` + `room_id` binding; missing `room_key` → `Err`
//!
//! Non-guarantees: protobuf decode, signature verification, decode-limit checks;
//! KeyStale overlay (callers); dual-key grace is host-only. AEAD open failure rate-limits a
//! Room Token refresh nudge so guests detect host epoch rotate before FRAME_STALL (x007e).
//! `client_time_ms` is a best-effort wall clock.

use std::time::{SystemTime, UNIX_EPOCH};

use alchemy_protocol::wire_aead::{self, AeadKind};
use zenoh::bytes::ZBytes;

use crate::scene::SharedState;

pub(crate) fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Ciphertext ceiling (on `ZBytes::len`) → contiguous copy → AEAD open → plaintext.
/// Fail-closed before protobuf / signature checks.
pub(crate) fn open_zenoh_payload(
    shared: &SharedState,
    kind: AeadKind,
    bytes: &ZBytes,
) -> Result<Vec<u8>, String> {
    if let Err(err) = wire_aead::check_aead_envelope_bytes(kind, bytes.len()) {
        return Err(err.to_string());
    }
    let (room_key, epoch) = shared
        .live_room_aead()
        .ok_or_else(|| "room_key missing".to_string())?;
    let envelope = bytes.to_bytes();
    wire_aead::open(
        room_key.as_ref(),
        shared.get_room_id().as_ref(),
        epoch,
        kind,
        envelope.as_ref(),
    )
    .map_err(|e| {
        // Host may have rotated ≈15s ahead of soft-TTL refresh; nudge Token so KeyStale
        // fires before FRAME_STALL (x007e). Non-epoch AEAD fails (corrupt / kind mismatch)
        // also nudge — capped at 2s in SharedState (acceptable misdetect cost).
        shared.nudge_room_token_refresh_after_aead_fail();
        e.to_string()
    })
}

pub(crate) fn seal_zenoh_payload(
    shared: &SharedState,
    kind: AeadKind,
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let (room_key, epoch) = shared
        .live_room_aead()
        .ok_or_else(|| "room_key missing".to_string())?;
    wire_aead::seal(
        room_key.as_ref(),
        shared.get_room_id().as_ref(),
        epoch,
        kind,
        plaintext,
    )
    .map_err(|e| e.to_string())
}
