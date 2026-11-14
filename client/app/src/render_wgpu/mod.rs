//! MVCC page GPU interpolate path (x00ac).
//!
//! Guarantees: [`shaders::INTERP_WGSL`] matches [`alchemy_shared_sim::mvcc::GpuTransferableSoA`]
//! field order; naga parses the module with entry `cs_interpolate_entities`.
//! Non-guarantees: replacing Epoch [`crate::epoch_view::INTERPOLATE_WGSL`] draw path;
//! host page publish; multi-page upload.
//!
//! Draw wiring lands in Phase 3; items are validated in unit tests until then.

#![allow(dead_code, unused_imports)]

pub mod compute_interp;
pub mod shaders;

pub use compute_interp::{validate_interp_wgsl, INTERP_ENTRY_POINT};
pub use shaders::INTERP_WGSL;
