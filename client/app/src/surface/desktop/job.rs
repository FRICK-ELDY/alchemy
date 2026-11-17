//! Desktop auth / world / inventory worker plumbing (x002e / x007c).
//!
//! Guarantees: a worker armed via `arm_auth_job` is tagged with the current
//! `auth_work_epoch`; `send_enter_progress` / `stop_host_if_job_current` act only while
//! that epoch is current, and `discard_auth_job` bumps it so Leave / Logout win any race.
//! Non-guarantee: worker threads are never joined or cancelled (a stale thread runs to
//! completion and its result is dropped); one job per slot is the caller's
//! `auth_job.is_some()` check ("Auth busy…").
//!
//! Leaf: depends only on [`super::support`] (not on session/inventory/hotswitch flows).

use std::sync::mpsc::Receiver;
use std::thread;

use crate::scene::SharedState;
use crate::system_ui::SystemUi;
use crate::world_api;

use super::support::{publish_inventory_mirror_from_ui, stop_supervised_host};

/// Outcome of Auth / world HTTP work off the UI thread (x002e / P3-9 / x0037).
pub(super) enum AuthJobOutcome {
    /// Session only (no auto-join).
    LoginOk {
        username: String,
    },
    RegisterOk {
        username: String,
    },
    LoginFailed(String),
    RegisterFailed(String),
    /// Mid-flight Create/Join/Host progress (does not clear `auth_job`).
    EnterProgress {
        progress: f32,
        title: String,
        log: String,
    },
    /// visibility ensure + guest join (`visibility` says whether the chosen level was applied).
    StartSession {
        visibility: Result<world_api::VisibilityEnsure, String>,
        join: Result<(), String>,
    },
    JoinRoom {
        result: Result<(), String>,
        /// Worlds catalog Join → Waiting shell; Home Join keeps prior UX.
        enter_waiting_shell: bool,
        /// Set `worlds_can_host_rescue` only after successful join (owner guest path).
        can_host_rescue_on_ok: bool,
    },
    WorldsList {
        result: Result<Vec<world_api::RoomSummary>, String>,
    },
    /// Mid-flight paced inventory spawn (does not clear `auth_job`).
    InventoryTakeOutProgress {
        status: String,
    },
    /// Prefab/blob take-out HTTP + paced Zenoh puts finished.
    InventoryTakeOutDone {
        status: String,
    },
}

pub(super) fn send_enter_progress(
    tx: &std::sync::mpsc::Sender<AuthJobOutcome>,
    shared: &SharedState,
    epoch: u64,
    progress: f32,
    title: &str,
    log: &str,
) {
    if shared.auth_work_is_current(epoch) {
        let _ = tx.send(AuthJobOutcome::EnterProgress {
            progress,
            title: title.into(),
            log: log.into(),
        });
    }
}

pub(super) fn spawn_auth_http<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Err(err) = thread::Builder::new().name("auth-http".into()).spawn(f) {
        log::error!("spawn auth-http failed: {err}");
    }
}

/// Discard in-flight auth/catalog/inventory worker and bump epoch so the thread cannot commit.
/// Clears sticky take_out / spawn-put status so Leave/Logout does not leave Mirror mid-flight.
pub(super) fn discard_auth_job(
    ui: &mut SystemUi,
    shared: &SharedState,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    shared.bump_auth_work_epoch();
    *auth_job = None;
    let sticky = ui.state.inventory_status.starts_with("take_out")
        || ui.state.inventory_status.starts_with("spawn put");
    if sticky {
        ui.state.inventory_status = "take_out cancelled".into();
        ui.state.inventory_take_out_override = None;
        publish_inventory_mirror_from_ui(ui, shared);
    }
}

/// Arm a new worker against the current epoch (call only when `auth_job` is empty).
pub(super) fn arm_auth_job(
    shared: &SharedState,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    rx: Receiver<AuthJobOutcome>,
) -> u64 {
    let epoch = shared.auth_work_epoch();
    *auth_job = Some((epoch, rx));
    epoch
}

/// Stop local host only while this job's epoch is still current.
/// Stale workers must not touch host — Leave keeps access and a newer Host may already run.
pub(super) fn stop_host_if_job_current(shared: &SharedState, epoch: u64) {
    if shared.auth_work_is_current(epoch) {
        stop_supervised_host(shared);
    }
}

impl AuthJobOutcome {
    /// Mid-flight progress keeps `auth_job` armed (does not clear the slot).
    pub(super) fn is_mid_flight_progress(&self) -> bool {
        matches!(
            self,
            Self::EnterProgress { .. } | Self::InventoryTakeOutProgress { .. }
        )
    }

    /// Inventory take-out terminal/progress — applied by `inventory_flow`, not session.
    pub(super) fn is_inventory_outcome(&self) -> bool {
        matches!(
            self,
            Self::InventoryTakeOutProgress { .. } | Self::InventoryTakeOutDone { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_job_mid_flight_vs_terminal() {
        assert!(AuthJobOutcome::EnterProgress {
            progress: 0.5,
            title: "t".into(),
            log: "l".into(),
        }
        .is_mid_flight_progress());
        assert!(
            AuthJobOutcome::InventoryTakeOutProgress { status: "s".into() }
                .is_mid_flight_progress()
        );
        assert!(!AuthJobOutcome::LoginOk {
            username: "u".into(),
        }
        .is_mid_flight_progress());
        assert!(!AuthJobOutcome::InventoryTakeOutDone {
            status: "done".into(),
        }
        .is_mid_flight_progress());
        assert!(!AuthJobOutcome::WorldsList { result: Ok(vec![]) }.is_mid_flight_progress());
        assert!(!AuthJobOutcome::LoginFailed("x".into()).is_mid_flight_progress());
    }

    #[test]
    fn inventory_outcomes_are_classified() {
        assert!(AuthJobOutcome::InventoryTakeOutDone {
            status: "ok".into(),
        }
        .is_inventory_outcome());
        assert!(
            AuthJobOutcome::InventoryTakeOutProgress { status: "p".into() }.is_inventory_outcome()
        );
        assert!(!AuthJobOutcome::LoginOk {
            username: "u".into(),
        }
        .is_inventory_outcome());
    }
}
