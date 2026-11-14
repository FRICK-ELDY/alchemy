//! WGSL sources for MVCC page interpolate (x00ac).

/// HotPhysicsPage SoA interpolate kernel (layout = `GpuTransferableSoA`).
pub const INTERP_WGSL: &str = include_str!("interp.wgsl");
