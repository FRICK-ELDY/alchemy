//! host-server library surface for unit tests and cargo-fuzz (x004c / P3-9).
//!
//! Guarantees: Room Token verify path ([`room_token::RoomTokenVerifier`]) is reachable
//! without the physics binary entrypoint.
//! Non-guarantees: Zenoh / Rapier loop; fuzz corpus completeness.
//!
//! `intent_rate` / `metrics` / `session` are `pub` only because `room_token` and its
//! unit tests share this crate root; a thinner `alchemy-room-token` extract is a
//! future option if fuzz CI cost or the public surface becomes painful.

pub mod intent_rate;
pub mod metrics;
pub mod room_token;
pub mod session;
