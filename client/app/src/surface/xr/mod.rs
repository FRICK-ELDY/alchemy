//! OpenXR surface (SteamVR). Falls back to Desktop on probe/session failure.
//!
//! Guarantees: menu toggle, ray click → system_ui UiAction, Exit ends session,
//! dash UI + aim laser in the same projection (x000e; tip in front of panel),
//! laser UV → system_ui `on_pointer` hover/press (x000f; no XR types in system_ui),
//! thumbstick Noclip Intent (aim-relative), comfort turn (snap/smooth),
//! SceneGpu solid floor (Desktop 相当) + mesh/boxes (x0051/x0052),
//! XR_EXT_render_model controllers when runtime supports them (SteamVR 2.12+),
//! embedded baseColor textures when present in the GLB (x000c),
//! animatable node pose/visibility (x000d).
//! Grab hold: squeeze (or Select when dash closed) → InputEvent GRAB + aim (x0011).
//! XR 描画は wgpu SceneGpu + overlays 既定（x0052）。ash は Vulkan enable／hal interop のみ。
//! Non-guarantees: WSL HMD, Quest standalone, Vive trackpad-only,
//! Facet rearrange / WorldRoot, Steal, CompositionLayerQuad dash,
//! PBR maps, SteamVR rendermodels path loading; multiview; Bevy.

mod input;
mod loader;
mod loco;
mod math;
mod render_model;
mod wgpu_interop;
mod wgpu_overlays;
mod wgpu_session;

use anyhow::{bail, Context, Result};

use crate::mesh::CpuMesh;
use crate::scene::SharedState;
use std::sync::Arc;

/// Returns Ok(true) if an HMD form-factor system is available.
pub fn probe() -> Result<bool> {
    // Safety: OpenXR loader is process-global; probe is best-effort.
    let entry = unsafe { loader::load_entry() }.context("load OpenXR loader")?;
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
            &(),
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
    log::info!("surface/xr: wgpu SceneGpu session (x0052)");
    wgpu_session::run_session(mesh, shared, zenoh)
}
