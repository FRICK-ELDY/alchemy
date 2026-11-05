//! Zenoh room bus: subscribers / publisher / wire helpers (no room lifecycle).
//!
//! Guarantees:
//! - `bus::*` does **not** depend on `crate::room` (room → bus only; no cycles)
//! - Each unit documents its own gates (see `wire`, `manifest`, `frame`, `presence`, `input`,
//!   `liveliness`)
//! - Zenoh keys come from protocol `*_key` helpers (x0017); callers (`room::ensure_room_bus`) own
//!   key construction and thread spawn order
//!
//! Non-guarantees: session open/close, join/leave, token refresh (see `room`).

use std::time::Duration;

pub mod frame;
pub mod input;
pub mod liveliness;
pub mod manifest;
pub mod presence;
pub mod wire;

/// Interval for aggregated "gate" drop-counter logs in bus subscribers.
pub(crate) const GATE_LOG_INTERVAL: Duration = Duration::from_secs(5);
