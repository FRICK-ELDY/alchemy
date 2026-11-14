//! naga validation + layout helpers for [`super::shaders::INTERP_WGSL`] (x00ac).

use alchemy_shared_sim::mvcc::GpuTransferableSoA;

use super::shaders::INTERP_WGSL;

/// Compute entry in `interp.wgsl`.
pub const INTERP_ENTRY_POINT: &str = "cs_interpolate_entities";

/// Parse + validate `interp.wgsl` with wgpu's re-exported naga (no new dependency).
pub fn validate_interp_wgsl() -> Result<(), String> {
    let module = wgpu::naga::front::wgsl::parse_str(INTERP_WGSL).map_err(|e| e.to_string())?;
    let entry = module
        .entry_points
        .iter()
        .find(|ep| ep.name == INTERP_ENTRY_POINT);
    if entry.is_none() {
        return Err(format!("missing entry point {INTERP_ENTRY_POINT}"));
    }
    let mut validator = wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    );
    validator.validate(&module).map_err(|e| e.to_string())?;
    Ok(())
}

/// CPU-side size of one SoA face (upload budgeting).
pub fn gpu_soa_bytes() -> usize {
    std::mem::size_of::<GpuTransferableSoA>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn interp_wgsl_naga_validates() {
        validate_interp_wgsl().expect("interp.wgsl must parse under naga");
    }

    #[test]
    fn interp_wgsl_mentions_layout_contract() {
        assert!(INTERP_WGSL.contains("GpuTransferableSoA"));
        assert!(INTERP_WGSL.contains("drot_x"));
        assert!(INTERP_WGSL.contains("inv_inertia_z"));
        assert!(INTERP_WGSL.contains("cell_rel_offset"));
        assert!(INTERP_WGSL.contains(INTERP_ENTRY_POINT));
        assert!(INTERP_WGSL.contains("@compute"));
    }

    #[test]
    fn soa_entity_cap_matches_shader_arrays() {
        assert_eq!(alchemy_shared_sim::mvcc::ENTITIES_PER_PAGE, 1024);
        assert!(INTERP_WGSL.contains("array<f32, 1024>"));
        let _ = GpuTransferableSoA::zeroed();
        assert!(gpu_soa_bytes() > 64 * 1024);
    }
}
