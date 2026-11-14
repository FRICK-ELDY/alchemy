//! Ring-buffer MVCC pages for shared-sim (x00a3 / Phase 1 DoD / x00a5 SIMD).
//!
//! Guarantees: fixed SoA page layout; NaN/Inf quarantine on pose writes; pre-allocated arena pool;
//! CoW `fork_page`; [`MvccPageArena::commit_transfer`] / [`commit_share`] retain-then-release
//! (dead / duplicate / history-only transfer rejected); page-path XPBD contacts via [`adapter`];
//! SoA inertia Safe SIMD building block ([`inertia::step_inertia`]; scalar ref kept; **not on tick path**).
//! Non-guarantees: host/app wiring; dirty-delta codec (x0024; not V_ack/XOR); full solver SIMD.

mod adapter;
mod arena;
mod inertia;
mod math_det;
mod page;

pub use adapter::{page_fingerprint, pages_bit_equal, PageColliderTable, PageXpbdAdapter};
pub use arena::{ArenaError, MvccPageArena, PageId, WorldSnapshot};
pub use inertia::{step_inertia, step_inertia_scalar, step_inertia_simd};
pub use math_det::{cell_rel_offset_f32, det_normalize3, sanitize_f32};
pub use page::{HotPhysicsPage, KinematicPoseF32, BITMASK_WORDS, ENTITIES_PER_PAGE, HISTORY_TICKS};
