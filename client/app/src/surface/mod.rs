//! Presentation surface selection: Desktop (winit) vs OpenXR.
//!
//! Guarantees: `ALCHEMY_FORCE_DESKTOP=1` → Desktop; runtime missing → Desktop;
//! OpenXR boot runs XR session + Desktop companion (x0053 hotswitch / F8);
//! before XR READY, Desktop owns input (no boot dead zone);
//! OpenXR **init** failure demotes to Desktop Solo-like (process stays up);
//! XR thread panic always clears `xr_session_live` (catch_unwind);
//! Zenoh `Session` is owned only by Desktop (XR uses presentation request flags).
//! Non-guarantees: WSL HMD; Quest standalone; Virtual Desktop presence parity;
//! companion window GPU cost while `vr_active` (best-effort mirror every frame).

mod desktop;
mod hotswitch;
mod xr;

pub use hotswitch::{DesktopRole, PresentationGate};

use anyhow::Result;

use crate::mesh::CpuMesh;
use crate::scene::SharedState;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

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

fn hotswitch_auto_from_env() -> bool {
    match std::env::var("ALCHEMY_HOTSWITCH") {
        Ok(v) => {
            let v = v.trim();
            !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        }
        Err(_) => true,
    }
}

pub fn run(
    kind: SurfaceKind,
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
) -> Result<()> {
    match kind {
        SurfaceKind::Desktop => {
            shared.presentation.set_vr_active(false);
            desktop::run(mesh, shared, zenoh, DesktopRole::Solo)
        }
        SurfaceKind::OpenXr => run_xr_with_companion(mesh, shared, zenoh),
    }
}

/// Finish XR thread bookkeeping. Always clears live/started (panic-safe caller).
fn finish_xr_thread(shared: &SharedState, result: Result<()>) -> Result<()> {
    let started = shared.presentation.xr_session_started();
    shared
        .presentation
        .xr_session_live
        .store(false, Ordering::SeqCst);
    shared
        .presentation
        .xr_session_started
        .store(false, Ordering::SeqCst);

    let already_exiting = shared.presentation.exit_requested();
    match &result {
        Ok(()) => {
            if !already_exiting {
                shared.presentation.request_exit();
            }
        }
        Err(err) if started || already_exiting => {
            log::warn!("OpenXR session ended with error: {err:#}");
            if !already_exiting {
                shared.presentation.request_exit();
            }
        }
        Err(err) => {
            shared.presentation.set_vr_active(false);
            shared.clear_move_intent();
            log::warn!(
                "OpenXR failed before session ready ({err:#}); Desktop continues (Solo-like)"
            );
        }
    }
    result
}

/// XR session on a worker thread + Desktop companion on the UI thread (x0053).
///
/// Zenoh stays on Desktop only. XR init failure (never READY) does **not**
/// `request_exit` — companion keeps running Solo-like (`!xr_session_live`).
fn run_xr_with_companion(
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
) -> Result<()> {
    // Preferred present after READY; until then Desktop owns input (`!xr_session_started`).
    shared.presentation.set_vr_active(true);
    shared
        .presentation
        .set_hotswitch_auto(hotswitch_auto_from_env());
    shared
        .presentation
        .auto_suppressed
        .store(false, Ordering::SeqCst);
    shared
        .presentation
        .xr_session_started
        .store(false, Ordering::SeqCst);
    log::info!(
        "surface: OpenXR + Desktop companion (hotswitch auto={}, F8 after READY; Zenoh=Desktop)",
        shared.presentation.hotswitch_auto.load(Ordering::SeqCst)
    );

    let mesh_xr = mesh.clone();
    let shared_xr = Arc::clone(&shared);
    let xr_thread = match thread::Builder::new()
        .name("xr-session".into())
        .spawn(move || {
            shared_xr
                .presentation
                .xr_session_live
                .store(true, Ordering::SeqCst);
            let unwind = catch_unwind(AssertUnwindSafe(|| {
                xr::run(mesh_xr, Arc::clone(&shared_xr))
            }));
            let result = match unwind {
                Ok(r) => r,
                Err(_) => {
                    log::error!("OpenXR thread panicked — clearing session flags");
                    Err(anyhow::anyhow!("xr-session panicked"))
                }
            };
            finish_xr_thread(&shared_xr, result)
        }) {
        Ok(handle) => handle,
        Err(err) => {
            log::warn!("spawn xr-session failed ({err}); falling back to Desktop Solo");
            shared.presentation.set_vr_active(false);
            return desktop::run(mesh, shared, zenoh, DesktopRole::Solo);
        }
    };

    let desktop_result = desktop::run(mesh, Arc::clone(&shared), zenoh, DesktopRole::Companion);
    shared.presentation.request_exit();

    let xr_result = xr_thread
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("xr-session thread panicked")));

    match (desktop_result, xr_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(desktop_err), Ok(())) => Err(desktop_err),
        (Ok(()), Err(xr_err)) => {
            log::debug!("OpenXR thread finished with error after Desktop exit: {xr_err:#}");
            Ok(())
        }
        (Err(desktop_err), Err(xr_err)) => {
            log::warn!("OpenXR session error (with Desktop failure): {xr_err:#}");
            Err(desktop_err)
        }
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
