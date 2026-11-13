//! Ring-buffer MVCC pages for shared-sim (x00a3 / mvcc_xpbd Phase 1 scaffold).
//!
//! Guarantees: fixed SoA page layout; NaN/Inf quarantine on pose writes; pre-allocated arena pool;
//! `release_page` never underflows refcount.
//! Non-guarantees: host/app wiring; XOR delta codec; XPBD contact replacement; `wide` SIMD;
//! `commit_snapshot` does not retain page refs (history may dangle after `release_page`).

mod arena;
mod page;

pub use arena::{MvccPageArena, PageId, WorldSnapshot};
pub use page::{HotPhysicsPage, KinematicPoseF32, BITMASK_WORDS, ENTITIES_PER_PAGE, HISTORY_TICKS};
