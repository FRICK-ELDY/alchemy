//! OpenXR surface (SteamVR). Falls back to Desktop on probe/session failure.
//!
//! Guarantees: menu toggle, ray click → system_ui UiAction, Exit ends session.
//! Non-guarantees: WSL HMD, Quest standalone, locomotion.

mod input;
mod vk_loop;

use anyhow::{bail, Context, Result};

use crate::mesh::CpuMesh;
use crate::scene::SharedState;
use std::sync::Arc;

/// Returns Ok(true) if an HMD form-factor system is available.
pub fn probe() -> Result<bool> {
    // Safety: OpenXR loader is process-global; probe is best-effort.
    let entry = unsafe { openxr::Entry::load() }.context("load OpenXR loader")?;
    let exts = entry
        .enumerate_extensions()
        .context("enumerate OpenXR extensions")?;
    if !exts.khr_vulkan_enable2 {
        bail!("runtime lacks XR_KHR_vulkan_enable2");
    }
    let mut enabled = openxr::ExtensionSet::default();
    enabled.khr_vulkan_enable2 = true;
    let instance = entry
        .create_instance(
            &openxr::ApplicationInfo {
                application_name: "alchemy-probe",
                application_version: 1,
                engine_name: "alchemy",
                engine_version: 1,
                api_version: openxr::Version::new(1, 0, 0),
            },
            &enabled,
            &[],
        )
        .context("create OpenXR instance (probe)")?;
    match instance.system(openxr::FormFactor::HEAD_MOUNTED_DISPLAY) {
        Ok(_) => Ok(true),
        Err(openxr::sys::Result::ERROR_FORM_FACTOR_UNAVAILABLE)
        | Err(openxr::sys::Result::ERROR_FORM_FACTOR_UNSUPPORTED) => Ok(false),
        Err(err) => Err(anyhow::anyhow!("OpenXR system(): {err}")),
    }
}

pub fn run(mesh: CpuMesh, shared: Arc<SharedState>, zenoh: Option<zenoh::Session>) -> Result<()> {
    vk_loop::run_openxr(mesh, shared, zenoh)
}
