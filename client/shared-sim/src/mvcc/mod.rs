//! Ring-buffer MVCC pages for shared-sim (x00a3–x00a9).
//!
//! Guarantees: fixed SoA page layout (incl. drot/holder reservation); NaN/Inf quarantine;
//! pre-allocated arena pool; true CoW [`MvccPageArena::make_mut`]; fixed [`SnapshotSlot`];
//! [`EntityDirectory`] cell cross; page-path XPBD contacts via [`adapter`]; inequality
//! contacts (x00a8); SoA inertia Safe SIMD ([`inertia::step_inertia`]; **not on tick path**);
//! ArcSwap publish foundation ([`publish::PublishedPair`]; app jitter/WGSL later).
//! Non-guarantees: host/app wiring; dirty-delta codec (x0024; not V_ack/XOR); full solver SIMD;
//! wgpu interp.

mod adapter;
mod arena;
mod directory;
mod inertia;
mod math_det;
mod page;
mod publish;

pub use adapter::{page_fingerprint, pages_bit_equal, PageColliderTable, PageXpbdAdapter};
pub use arena::{ArenaError, MvccPageArena, PageId, SnapshotSlot, WorldSnapshot};
pub use directory::{CellId, DirectoryError, EntityDirectory, PageSlot};
pub use inertia::{step_inertia, step_inertia_scalar, step_inertia_simd};
pub use math_det::{cell_rel_offset_f32, det_normalize3, sanitize_f32};
pub use page::{
    HotPhysicsPage, KinematicPoseF32, BITMASK_WORDS, CELL_HYSTERESIS_M, CELL_SIZE_M,
    ENTITIES_PER_PAGE, HISTORY_TICKS, MAX_PAGES_PER_SNAPSHOT, NO_HOLDER,
};
pub use publish::{PublishedFace, PublishedPair};
