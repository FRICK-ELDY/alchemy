//! Spatial Lens — Resonite-like spatial inspector without UIX-as-slots (x00ba–x00bd).
//!
//! Guarantees: 1 panel = 1 board; GpuUiPrim packing; Uber-SDF draw; Hierarchy/Inspector
//! extract from Epoch; panel OBB→UV hit + Stickiness; G0 glass tokens (x00ec).
//! Non-guarantees: WorldRoot CRDT sync; Tear-Off; CJK atlas; CustomUIX flex; scene UI entities;
//! background blur (G1); full Dash board migration (U0).

#![forbid(unsafe_code)]

pub mod extract;
pub mod glass;
pub mod gpu;
pub mod hit;
pub mod laser;
pub mod prim;
pub mod shaders;

pub use extract::{
    decode_hierarchy_binding, pack_personal_lens, place_in_front_of_eye, row_is_live,
    scrub_selected_row, SpatialLensState,
};
pub use glass::GlassColor;
pub use gpu::SpatialLensGpu;
pub use hit::pick_panel;
pub use laser::{HandLaserState, InteractionLaserConfig};
pub use prim::{GpuPanelHeader, GpuUiPrim};
