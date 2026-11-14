//! Ring-buffer MVCC pages for shared-sim (x00a3 / Phase 1 DoD).
//!
//! Guarantees: fixed SoA page layout; NaN/Inf quarantine on pose writes; pre-allocated arena pool;
//! CoW `fork_page`; [`MvccPageArena::commit_transfer`] / [`commit_share`] retain-then-release
//! (dead / duplicate / history-only transfer rejected); page-path XPBD contacts via [`adapter`].
//! Non-guarantees: host/app wiring; dirty-delta codec (x0024; not V_ack/XOR); `wide` SIMD.

mod adapter;
mod arena;
mod page;

pub use adapter::{page_fingerprint, pages_bit_equal, PageColliderTable, PageXpbdAdapter};
pub use arena::{ArenaError, MvccPageArena, PageId, WorldSnapshot};
pub use page::{HotPhysicsPage, KinematicPoseF32, BITMASK_WORDS, ENTITIES_PER_PAGE, HISTORY_TICKS};
