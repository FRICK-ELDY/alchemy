//! Host crash restart policy (x0047 / P3-2).
//!
//! Guarantees: exit success (IPC SHUTDOWN / clean leave) stops the launcher without
//! respawn; non-success exits restart with exponential backoff up to
//! [`MAX_HOST_RESTARTS`] attempts. Shutdown-in-progress ignores further restarts.
//!
//! Non-guarantees: OS-specific exit codes beyond success/failure; refreshing the
//! client's `ALCHEMY_HOST_IPC_ADDR` after restart; **ready-wait failure after a
//! respawn** (crash-before-ready / timeout) — that path stops the launcher without
//! another [`decide_host_exit`] (see `main` / architecture/launcher.md).

use std::time::Duration;

/// Maximum number of **crash** restarts (non-zero exits) before the launcher gives up.
pub const MAX_HOST_RESTARTS: u32 = 3;

/// What the launcher should do after `host.try_wait()` observes an exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostExitAction {
    /// Ctrl+C / shutdown already in progress — do not restart.
    IgnoreDuringShutdown,
    /// Exit code 0 — launcher exits; do not restart.
    StopLauncherIntentionally,
    /// Non-zero crash — respawn after backoff (`attempt` is 1-based crash count).
    Restart { attempt: u32, backoff: Duration },
    /// Crash count would exceed [`MAX_HOST_RESTARTS`].
    BudgetExhausted { attempt: u32 },
}

/// Decide the next launcher action for a host process exit.
///
/// `prior_restarts` is how many crash-restarts have **already** completed (0 before
/// the first crash). On restart, store `attempt` back into that counter.
pub fn decide_host_exit(
    exit_success: bool,
    shutting_down: bool,
    prior_restarts: u32,
) -> HostExitAction {
    if shutting_down {
        return HostExitAction::IgnoreDuringShutdown;
    }
    if exit_success {
        return HostExitAction::StopLauncherIntentionally;
    }
    let attempt = prior_restarts.saturating_add(1);
    if attempt > MAX_HOST_RESTARTS {
        return HostExitAction::BudgetExhausted { attempt };
    }
    HostExitAction::Restart {
        attempt,
        backoff: backoff_for_attempt(attempt),
    }
}

/// Backoff for a 1-based crash `attempt`: 1s, 2s, 4s (capped at attempt 3).
pub fn backoff_for_attempt(attempt: u32) -> Duration {
    let capped = attempt.clamp(1, 3);
    Duration::from_secs(1u64 << (capped - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intentional_exit_stops_launcher() {
        assert_eq!(
            decide_host_exit(true, false, 0),
            HostExitAction::StopLauncherIntentionally
        );
        // Even after prior crashes, exit 0 must not restart.
        assert_eq!(
            decide_host_exit(true, false, 2),
            HostExitAction::StopLauncherIntentionally
        );
    }

    #[test]
    fn shutdown_in_progress_ignores_exit() {
        assert_eq!(
            decide_host_exit(false, true, 0),
            HostExitAction::IgnoreDuringShutdown
        );
        assert_eq!(
            decide_host_exit(true, true, 0),
            HostExitAction::IgnoreDuringShutdown
        );
    }

    #[test]
    fn crash_restarts_with_backoff_until_budget() {
        assert_eq!(
            decide_host_exit(false, false, 0),
            HostExitAction::Restart {
                attempt: 1,
                backoff: Duration::from_secs(1),
            }
        );
        assert_eq!(
            decide_host_exit(false, false, 1),
            HostExitAction::Restart {
                attempt: 2,
                backoff: Duration::from_secs(2),
            }
        );
        assert_eq!(
            decide_host_exit(false, false, 2),
            HostExitAction::Restart {
                attempt: 3,
                backoff: Duration::from_secs(4),
            }
        );
        assert_eq!(
            decide_host_exit(false, false, 3),
            HostExitAction::BudgetExhausted { attempt: 4 }
        );
    }

    #[test]
    fn backoff_caps_at_four_seconds() {
        assert_eq!(backoff_for_attempt(1), Duration::from_secs(1));
        assert_eq!(backoff_for_attempt(2), Duration::from_secs(2));
        assert_eq!(backoff_for_attempt(3), Duration::from_secs(4));
        assert_eq!(backoff_for_attempt(99), Duration::from_secs(4));
        assert_eq!(backoff_for_attempt(0), Duration::from_secs(1));
    }
}
