//! Presentation surface selection: Desktop (winit) vs OpenXR.
//!
//! Guarantees: `ALCHEMY_FORCE_DESKTOP=1` → Desktop; runtime missing → Desktop.
//! Non-guarantees: WSL HMD; Quest standalone.

mod desktop;
mod xr;

use anyhow::Result;

use crate::mesh::CpuMesh;
use crate::scene::SharedState;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Desktop,
    OpenXr,
}

/// Pick surface. OpenXR probe failure or force-desktop → Desktop.
pub fn choose() -> SurfaceKind {
    if std::env::var_os("ALCHEMY_FORCE_DESKTOP").is_some() {
        log::info!("surface: ALCHEMY_FORCE_DESKTOP set → Desktop");
        return SurfaceKind::Desktop;
    }
    match xr::probe() {
        Ok(true) => {
            log::info!("surface: OpenXR runtime present → trying XR");
            SurfaceKind::OpenXr
        }
        Ok(false) => {
            log::info!("surface: no OpenXR HMD system → Desktop");
            SurfaceKind::Desktop
        }
        Err(err) => {
            log::info!("surface: OpenXR probe failed ({err:#}) → Desktop");
            SurfaceKind::Desktop
        }
    }
}

pub fn run(
    kind: SurfaceKind,
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
) -> Result<()> {
    match kind {
        SurfaceKind::Desktop => desktop::run(mesh, shared, zenoh),
        SurfaceKind::OpenXr => match xr::run(mesh.clone(), Arc::clone(&shared), zenoh.clone()) {
            Ok(()) => Ok(()),
            Err(err) => {
                log::warn!("OpenXR surface failed ({err:#}); falling back to Desktop");
                desktop::run(mesh, shared, zenoh)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_desktop_env_selects_desktop() {
        // Safety: test process only; restore after.
        let prev = std::env::var_os("ALCHEMY_FORCE_DESKTOP");
        std::env::set_var("ALCHEMY_FORCE_DESKTOP", "1");
        assert_eq!(choose(), SurfaceKind::Desktop);
        match prev {
            Some(v) => std::env::set_var("ALCHEMY_FORCE_DESKTOP", v),
            None => std::env::remove_var("ALCHEMY_FORCE_DESKTOP"),
        }
    }
}
