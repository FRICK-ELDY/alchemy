//! XR ↔ Desktop present decision (x0053).
//!
//! Guarantees: manual F8 flip; optional auto from `XR_EXT_user_presence` with
//! hysteresis; broken-sensor heuristic suppresses auto (manual still works).
//! Non-guarantees: Quest/WebXR; Virtual Desktop presence parity; perfect
//! near-sensor match across runtimes.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Shared present / exit flags between XR thread and Desktop companion window.
///
/// Zenoh session ownership: **Desktop only** (companion or Solo). XR must not
/// `clone` / `bind_room_after_join` / `leave_room` / `close` — use the request
/// flags below so Desktop performs wire side-effects.
#[derive(Debug)]
pub struct PresentationGate {
    /// When true and XR session is live, HMD owns Intent / Grab.
    pub vr_active: AtomicBool,
    /// Settings / env: apply presence events to `vr_active`.
    pub hotswitch_auto: AtomicBool,
    /// XR `run_session` thread is past spawn and not yet finished.
    pub xr_session_live: AtomicBool,
    /// OpenXR session reached READY at least once (distinguishes init fail vs mid-run).
    pub xr_session_started: AtomicBool,
    /// Either surface requested process exit.
    pub exit_requested: AtomicBool,
    /// XR UI asked Desktop to run `leave_room` (Desktop owns Zenoh).
    pub leave_requested: AtomicBool,
    /// XR UI asked Desktop to publish host shutdown then exit.
    pub host_shutdown_requested: AtomicBool,
    /// Last presence sample (EXT event); informational + auto input.
    pub user_present: AtomicBool,
    /// Auto path disabled by broken-sensor heuristic (F8 still works).
    pub auto_suppressed: AtomicBool,
    /// Bumped on F8 / Settings manual present flip so XR controller clears pending auto.
    pub manual_epoch: AtomicU64,
}

impl Default for PresentationGate {
    fn default() -> Self {
        Self {
            vr_active: AtomicBool::new(true),
            hotswitch_auto: AtomicBool::new(true),
            xr_session_live: AtomicBool::new(false),
            xr_session_started: AtomicBool::new(false),
            exit_requested: AtomicBool::new(false),
            leave_requested: AtomicBool::new(false),
            host_shutdown_requested: AtomicBool::new(false),
            user_present: AtomicBool::new(true),
            auto_suppressed: AtomicBool::new(false),
            manual_epoch: AtomicU64::new(0),
        }
    }
}

impl PresentationGate {
    pub fn vr_active(&self) -> bool {
        self.vr_active.load(Ordering::SeqCst)
    }

    pub fn set_vr_active(&self, active: bool) {
        self.vr_active.store(active, Ordering::SeqCst);
    }

    pub fn toggle_vr_active(&self) -> bool {
        // fetch_xor returns previous; new = !previous.
        let now = !self.vr_active.fetch_xor(true, Ordering::SeqCst);
        self.manual_epoch.fetch_add(1, Ordering::SeqCst);
        now
    }

    pub fn bump_manual_epoch(&self) {
        self.manual_epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn hotswitch_auto(&self) -> bool {
        self.hotswitch_auto.load(Ordering::SeqCst) && !self.auto_suppressed.load(Ordering::SeqCst)
    }

    pub fn set_hotswitch_auto(&self, enabled: bool) {
        self.hotswitch_auto.store(enabled, Ordering::SeqCst);
    }

    pub fn xr_session_live(&self) -> bool {
        self.xr_session_live.load(Ordering::SeqCst)
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::SeqCst)
    }

    pub fn request_exit(&self) {
        self.exit_requested.store(true, Ordering::SeqCst);
    }

    pub fn request_leave(&self) {
        self.leave_requested.store(true, Ordering::SeqCst);
    }

    pub fn take_leave_requested(&self) -> bool {
        self.leave_requested.swap(false, Ordering::SeqCst)
    }

    pub fn request_host_shutdown(&self) {
        self.host_shutdown_requested.store(true, Ordering::SeqCst);
    }

    pub fn take_host_shutdown_requested(&self) -> bool {
        self.host_shutdown_requested.swap(false, Ordering::SeqCst)
    }

    pub fn mark_xr_started(&self) {
        self.xr_session_started.store(true, Ordering::SeqCst);
    }

    pub fn xr_session_started(&self) -> bool {
        self.xr_session_started.load(Ordering::SeqCst)
    }
}

/// Desktop surface role relative to an XR session (x0053).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopRole {
    /// Sole surface (no live XR).
    Solo,
    /// Companion window beside a live OpenXR session.
    Companion,
}

impl DesktopRole {
    /// Desktop owns look / Intent / Grab.
    ///
    /// Companion yields only when XR is **READY** (`xr_session_started`) and
    /// `vr_active`. Before READY (SteamVR wait) Desktop keeps input — no dead zone.
    pub fn owns_input(self, gate: &PresentationGate) -> bool {
        match self {
            Self::Solo => true,
            Self::Companion => {
                !gate.xr_session_live() || !gate.xr_session_started() || !gate.vr_active()
            }
        }
    }
}

/// Tunables for presence → present auto path.
#[derive(Clone, Copy, Debug)]
pub struct HotswitchConfig {
    pub hysteresis: Duration,
    pub presence_grace: Duration,
    pub flip_window: Duration,
    pub max_flips_per_window: u32,
}

impl Default for HotswitchConfig {
    fn default() -> Self {
        Self {
            hysteresis: Duration::from_millis(300),
            presence_grace: Duration::from_secs(30),
            flip_window: Duration::from_secs(10),
            max_flips_per_window: 8,
        }
    }
}

/// Local XR-thread controller: presence debounce + broken-sensor suppress.
pub struct HotswitchController {
    config: HotswitchConfig,
    ext_enabled: bool,
    started: Instant,
    presence_seen_true: bool,
    pending: Option<(bool, Instant)>,
    flip_times: VecDeque<Instant>,
    last_applied_present: bool,
    last_manual_epoch: u64,
}

impl HotswitchController {
    pub fn new(ext_enabled: bool, config: HotswitchConfig) -> Self {
        Self {
            config,
            ext_enabled,
            started: Instant::now(),
            presence_seen_true: false,
            pending: None,
            flip_times: VecDeque::new(),
            last_applied_present: true,
            last_manual_epoch: 0,
        }
    }

    /// Ingest a presence sample (from `UserPresenceChangedEXT` or tests).
    pub fn on_presence(&mut self, is_present: bool, now: Instant) {
        if is_present {
            self.presence_seen_true = true;
        }
        match self.pending {
            Some((target, _)) if target == is_present => {}
            _ => self.pending = Some((is_present, now)),
        }
    }

    /// Apply pending presence / heuristics into `gate`. Manual F8 writes are preserved
    /// until a stable presence sample differs and auto is allowed.
    pub fn tick(&mut self, gate: &PresentationGate, now: Instant) {
        let epoch = gate.manual_epoch.load(Ordering::SeqCst);
        if epoch != self.last_manual_epoch {
            self.last_manual_epoch = epoch;
            self.note_manual(gate.vr_active());
        }

        if self.ext_enabled
            && !self.presence_seen_true
            && now.duration_since(self.started) >= self.config.presence_grace
            && !gate.auto_suppressed.load(Ordering::SeqCst)
        {
            gate.auto_suppressed.store(true, Ordering::SeqCst);
            log::warn!(
                "hotswitch: presence never became true within {:?} — auto suppressed (F8 still works)",
                self.config.presence_grace
            );
        }

        let Some((want, since)) = self.pending else {
            return;
        };
        if now.duration_since(since) < self.config.hysteresis {
            return;
        }
        self.pending = None;
        gate.user_present.store(want, Ordering::SeqCst);

        if !gate.hotswitch_auto() {
            return;
        }
        if want == self.last_applied_present {
            return;
        }

        self.note_flip(now);
        // Suppress on the (max+1)-th flip inside the window (strictly more than max).
        if self.flip_times.len() as u32 > self.config.max_flips_per_window {
            gate.auto_suppressed.store(true, Ordering::SeqCst);
            log::warn!(
                "hotswitch: presence flipped more than {} times in {:?} — auto suppressed (F8 still works)",
                self.config.max_flips_per_window,
                self.config.flip_window
            );
            return;
        }

        self.last_applied_present = want;
        gate.set_vr_active(want);
        log::info!("hotswitch: auto → {}", if want { "VR" } else { "Desktop" });
    }

    /// Keep controller in sync when user toggles manually (avoids immediate auto-revert).
    pub fn note_manual(&mut self, vr_active: bool) {
        self.last_applied_present = vr_active;
        self.pending = None;
    }

    fn note_flip(&mut self, now: Instant) {
        let window = self.config.flip_window;
        self.flip_times.push_back(now);
        while self
            .flip_times
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            self.flip_times.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_role_input_ownership() {
        let gate = PresentationGate::default();
        gate.xr_session_live.store(true, Ordering::SeqCst);
        gate.set_vr_active(true);
        // LIVE but not READY → Desktop still owns (no boot dead zone).
        assert!(DesktopRole::Companion.owns_input(&gate));
        gate.mark_xr_started();
        assert!(!DesktopRole::Companion.owns_input(&gate));
        gate.set_vr_active(false);
        assert!(DesktopRole::Companion.owns_input(&gate));
        assert!(DesktopRole::Solo.owns_input(&gate));
        gate.xr_session_live.store(false, Ordering::SeqCst);
        gate.set_vr_active(true);
        assert!(DesktopRole::Companion.owns_input(&gate));
    }

    #[test]
    fn toggle_vr_active_flips() {
        let gate = PresentationGate::default();
        assert!(gate.vr_active());
        assert!(!gate.toggle_vr_active());
        assert!(!gate.vr_active());
        assert!(gate.toggle_vr_active());
        assert!(gate.vr_active());
    }

    #[test]
    fn hysteresis_delays_auto_switch() {
        let gate = PresentationGate::default();
        let cfg = HotswitchConfig {
            hysteresis: Duration::from_millis(100),
            ..HotswitchConfig::default()
        };
        let mut ctrl = HotswitchController::new(true, cfg);
        let t0 = Instant::now();
        ctrl.on_presence(false, t0);
        ctrl.tick(&gate, t0);
        assert!(gate.vr_active(), "should still be VR before hysteresis");
        ctrl.tick(&gate, t0 + Duration::from_millis(150));
        assert!(!gate.vr_active());
    }

    #[test]
    fn auto_off_ignores_presence() {
        let gate = PresentationGate::default();
        gate.set_hotswitch_auto(false);
        let mut ctrl = HotswitchController::new(true, HotswitchConfig::default());
        let t0 = Instant::now();
        ctrl.on_presence(false, t0);
        ctrl.tick(&gate, t0 + Duration::from_secs(1));
        assert!(gate.vr_active());
    }

    #[test]
    fn never_present_suppresses_auto() {
        let gate = PresentationGate::default();
        let cfg = HotswitchConfig {
            presence_grace: Duration::from_millis(50),
            ..HotswitchConfig::default()
        };
        let mut ctrl = HotswitchController::new(true, cfg);
        let t0 = Instant::now();
        ctrl.tick(&gate, t0 + Duration::from_millis(60));
        assert!(gate.auto_suppressed.load(Ordering::SeqCst));
        assert!(!gate.hotswitch_auto());
        // Manual still works.
        gate.set_vr_active(false);
        assert!(!gate.vr_active());
    }

    #[test]
    fn rapid_flips_suppress_auto() {
        let gate = PresentationGate::default();
        let cfg = HotswitchConfig {
            hysteresis: Duration::ZERO,
            max_flips_per_window: 3,
            flip_window: Duration::from_secs(10),
            ..HotswitchConfig::default()
        };
        let mut ctrl = HotswitchController::new(true, cfg);
        let t0 = Instant::now();
        for (i, present) in [false, true, false, true].into_iter().enumerate() {
            let t = t0 + Duration::from_millis(i as u64 * 10);
            ctrl.on_presence(present, t);
            ctrl.tick(&gate, t);
        }
        assert!(gate.auto_suppressed.load(Ordering::SeqCst));
    }

    #[test]
    fn manual_note_clears_pending_auto_revert() {
        let gate = PresentationGate::default();
        let cfg = HotswitchConfig {
            hysteresis: Duration::from_millis(50),
            ..HotswitchConfig::default()
        };
        let mut ctrl = HotswitchController::new(true, cfg);
        let t0 = Instant::now();
        ctrl.on_presence(false, t0);
        // User manually stays in VR before debounce fires.
        gate.set_vr_active(true);
        ctrl.note_manual(true);
        ctrl.tick(&gate, t0 + Duration::from_millis(100));
        assert!(gate.vr_active());
    }
}
