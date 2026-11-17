//! Desktop side of the x0053 hotswitch: Dash mirror of the presentation gate, F8 manual
//! flip, Dash auto toggle, and XR-requested leave / host shutdown.
//!
//! Guarantees: the presentation gate is the SoT for auto / suppressed (Dash only mirrors
//! it or stores the toggle); XR `AuthLogout` / Exit requests run here because Desktop owns
//! Zenoh + HTTP teardown.
//! Non-guarantee: input-ownership edges (look / Grab / Intent reset, window title) stay on
//! `App` in `mod.rs`.

use std::sync::mpsc::Receiver;

use crate::leave_room;
use crate::operator_prefs;
use crate::scene::SharedState;
use crate::surface::hotswitch::DesktopRole;
use crate::system_ui::{DashTab, OnboardingShell, SystemUi};

use super::job::{discard_auth_job, AuthJobOutcome};
use super::session_flow::{
    clear_worlds_catalog, clear_worlds_session_shell, exit_stop_supervised_host,
    maybe_handover_before_leave,
};
use super::support::stop_supervised_host;

/// Mirror the presentation gate's hotswitch state into Dash. Returns true when the
/// visible hotswitch view changed (caller rebuilds while the dash is open).
pub(super) fn sync_hotswitch_ui_state(
    ui: &mut SystemUi,
    shared: &SharedState,
    role: DesktopRole,
) -> bool {
    let auto = shared
        .presentation
        .hotswitch_auto
        .load(std::sync::atomic::Ordering::SeqCst);
    let suppressed = shared
        .presentation
        .auto_suppressed
        .load(std::sync::atomic::Ordering::SeqCst);
    let prev_auto = ui.state.hotswitch_auto;
    let prev_suppressed = ui.state.hotswitch_suppressed;
    let prev_available = ui.state.hotswitch_available;
    let prev_label = ui.state.hotswitch_label.clone();
    // Hotswitch controls only after READY (same gate as F8). Local gate only.
    // PresentationGate is SoT for auto/suppressed — not SharedDashLogic.
    ui.state.hotswitch_available =
        role == DesktopRole::Companion && shared.presentation.xr_session_started();
    ui.state.hotswitch_auto = auto;
    ui.state.hotswitch_suppressed = suppressed;
    ui.state.refresh_hotswitch_label();
    prev_auto != auto
        || prev_suppressed != suppressed
        || prev_available != ui.state.hotswitch_available
        || ui.state.hotswitch_label != prev_label
}

/// XR-requested leave / host shutdown (Desktop is the Zenoh owner).
pub(super) fn poll_presentation_requests(
    ui: &mut SystemUi,
    shared: &SharedState,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if shared.presentation.take_leave_requested() {
        let had_zenoh = zenoh.is_some();
        if !had_zenoh {
            log::warn!(
                "hotswitch: leave requested but Zenoh is None — clearing auth without Leave put"
            );
        }
        // Bump epoch before Leave so in-flight join cannot commit past this point.
        discard_auth_job(ui, shared, auth_job);
        if shared.host_supervisor.owns_local_host() {
            maybe_handover_before_leave(shared);
        }
        leave_room(shared, zenoh);
        stop_supervised_host(shared);
        clear_worlds_session_shell(ui);
        clear_worlds_catalog(ui);
        if let Ok(mut gate) = shared.auth.lock() {
            gate.clear_session();
        }
        let key = operator_prefs::clear_session_rmw();
        operator_prefs::delete_refresh_vault_key(key, &crate::refresh_vault::OsRefreshVault);
        ui.state.home_logged_in_as = None;
        ui.state.home_create_open = false;
        ui.state.shell = OnboardingShell::Step1Operator;
        ui.state.home_status = if had_zenoh {
            "Signed out — Leave sent; select a server operator".into()
        } else {
            "Signed out — local only; select a server operator".into()
        };
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        ui.mark_dash_logic_dirty();
        ui.rebuild();
        log::info!("hotswitch: Desktop performed leave_room for XR AuthLogout");
    }
    if shared.presentation.take_host_shutdown_requested() {
        exit_stop_supervised_host(shared);
        log::info!("hotswitch: Desktop graceful host stop for XR Exit");
        shared.presentation.request_exit();
    }
}

/// F8 is allowed only for Companion after XR READY (same gate as Dash hotswitch controls).
pub(super) fn hotswitch_manual_allowed(role: DesktopRole, xr_session_started: bool) -> bool {
    role == DesktopRole::Companion && xr_session_started
}

/// F8 manual present flip (Companion + XR READY only). Returns true when `vr_active`
/// flipped; the caller then runs its input-ownership edge.
pub(super) fn toggle_hotswitch_manual(role: DesktopRole, shared: &SharedState) -> bool {
    if !hotswitch_manual_allowed(role, shared.presentation.xr_session_started()) {
        if role == DesktopRole::Companion {
            log::info!("hotswitch: F8 ignored (XR not READY yet)");
        }
        return false;
    }
    let now_vr = shared.presentation.toggle_vr_active();
    log::info!("hotswitch: F8 → {}", if now_vr { "VR" } else { "Desktop" });
    true
}

/// Dash `HotswitchAutoChanged`: store auto into the presentation gate (SoT).
pub(super) fn apply_hotswitch_auto_changed(ui: &mut SystemUi, shared: &SharedState) {
    shared
        .presentation
        .set_hotswitch_auto(ui.state.hotswitch_auto);
    // Clearing suppress lets the user re-enable auto after a bad sensor episode.
    if ui.state.hotswitch_auto {
        shared
            .presentation
            .auto_suppressed
            .store(false, std::sync::atomic::Ordering::SeqCst);
        ui.state.hotswitch_suppressed = false;
        ui.state.refresh_hotswitch_label();
    }
    ui.rebuild();
    log::info!("hotswitch auto stored → {}", ui.state.hotswitch_auto);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f8_gate_requires_companion_and_ready() {
        assert!(!hotswitch_manual_allowed(DesktopRole::Solo, true));
        assert!(!hotswitch_manual_allowed(DesktopRole::Solo, false));
        assert!(!hotswitch_manual_allowed(DesktopRole::Companion, false));
        assert!(hotswitch_manual_allowed(DesktopRole::Companion, true));
    }
}
