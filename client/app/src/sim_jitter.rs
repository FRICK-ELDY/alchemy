//! `sim_tick` / `host_time_ms` based interpolation clock with a 1–2 frame jitter delay (x00aa).
//!
//! Guarantees: α ∈ [0, 1] from a playhead that lags the latest published host time by
//! `delay_frames` × wire period; monotonic as playhead advances between prev/curr;
//! clamps outside the window. Falls back to [`None`] until two publishes exist.
//! Non-guarantees: proto `sim_tick` field (Phase 3); until then callers pass net `tick`
//! as the sim clock; wall-clock α ([`crate::epoch_mirror::epoch_interp_alpha`]) remains
//! the fallback when this clock is cold.

use std::time::{Duration, Instant};

use crate::epoch_mirror::EPOCH_FRAME_PERIOD;

/// Default jitter delay in wire frames (spec: 1–2).
pub const DEFAULT_JITTER_DELAY_FRAMES: u32 = 1;

/// Pure α from host-time anchors and a playhead (all ms).
pub fn alpha_from_host_times(prev_host_ms: u64, curr_host_ms: u64, playhead_ms: u64) -> f32 {
    if curr_host_ms <= prev_host_ms {
        return 1.0;
    }
    let span = (curr_host_ms - prev_host_ms) as f64;
    let t = playhead_ms.saturating_sub(prev_host_ms) as f64;
    (t / span).clamp(0.0, 1.0) as f32
}

/// Pure α from discrete sim ticks and a continuous playhead tick.
#[cfg_attr(not(test), allow(dead_code))]
pub fn alpha_from_sim_ticks(prev_tick: u64, curr_tick: u64, playhead_tick: f64) -> f32 {
    if curr_tick <= prev_tick {
        return 1.0;
    }
    let span = (curr_tick - prev_tick) as f64;
    ((playhead_tick - prev_tick as f64) / span).clamp(0.0, 1.0) as f32
}

/// Tracks the last two published faces for sim-based α.
#[derive(Debug)]
pub struct SimJitterState {
    pub prev_sim_tick: u64,
    pub curr_sim_tick: u64,
    pub prev_host_time_ms: u64,
    pub curr_host_time_ms: u64,
    curr_received_at: Option<Instant>,
    pub delay_frames: u32,
    publishes: u64,
}

impl Default for SimJitterState {
    fn default() -> Self {
        Self::new(DEFAULT_JITTER_DELAY_FRAMES)
    }
}

impl SimJitterState {
    pub fn new(delay_frames: u32) -> Self {
        Self {
            prev_sim_tick: 0,
            curr_sim_tick: 0,
            prev_host_time_ms: 0,
            curr_host_time_ms: 0,
            curr_received_at: None,
            delay_frames: delay_frames.clamp(1, 2),
            publishes: 0,
        }
    }

    /// Record a verified Applied Frame publish (`sim_tick` may equal wire `tick` until Phase 3).
    pub fn on_publish(&mut self, sim_tick: u64, host_time_ms: u64, received_at: Instant) {
        self.prev_sim_tick = self.curr_sim_tick;
        self.prev_host_time_ms = self.curr_host_time_ms;
        self.curr_sim_tick = sim_tick;
        self.curr_host_time_ms = host_time_ms;
        self.curr_received_at = Some(received_at);
        self.publishes = self.publishes.saturating_add(1);
    }

    pub fn has_pair(&self) -> bool {
        self.publishes >= 2
    }

    pub fn delay_duration(&self) -> Duration {
        EPOCH_FRAME_PERIOD.saturating_mul(self.delay_frames)
    }

    /// Estimated host clock = last published `host_time_ms` + local elapsed since receive.
    pub fn estimated_host_now_ms(&self, now: Instant) -> Option<u64> {
        let at = self.curr_received_at?;
        let elapsed_ms = now.saturating_duration_since(at).as_millis() as u64;
        Some(self.curr_host_time_ms.saturating_add(elapsed_ms))
    }

    /// α for display, or [`None`] until two publishes (caller uses wall-clock fallback).
    pub fn alpha_at(&self, now: Instant) -> Option<f32> {
        if !self.has_pair() {
            return None;
        }
        let host_now = self.estimated_host_now_ms(now)?;
        let delay_ms = self.delay_duration().as_millis() as u64;
        let playhead = host_now.saturating_sub(delay_ms);
        Some(alpha_from_host_times(
            self.prev_host_time_ms,
            self.curr_host_time_ms,
            playhead,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_clamps_outside_window() {
        assert!((alpha_from_host_times(1000, 1033, 900) - 0.0).abs() < 1e-5);
        assert!((alpha_from_host_times(1000, 1033, 1033) - 1.0).abs() < 1e-5);
        assert!((alpha_from_host_times(1000, 1033, 2000) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn alpha_monotone_inside_window() {
        let a0 = alpha_from_host_times(1000, 1033, 1000);
        let a1 = alpha_from_host_times(1000, 1033, 1016);
        let a2 = alpha_from_host_times(1000, 1033, 1033);
        assert!(a0 <= a1 && a1 <= a2);
        assert!((a1 - 0.5).abs() < 0.05);
    }

    #[test]
    fn sim_tick_alpha_matches_host_span() {
        assert!((alpha_from_sim_ticks(10, 11, 10.0) - 0.0).abs() < 1e-5);
        assert!((alpha_from_sim_ticks(10, 11, 10.5) - 0.5).abs() < 1e-5);
        assert!((alpha_from_sim_ticks(10, 11, 11.0) - 1.0).abs() < 1e-5);
        assert!((alpha_from_sim_ticks(10, 10, 10.5) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn jitter_needs_two_publishes() {
        let mut j = SimJitterState::new(1);
        let t0 = Instant::now();
        j.on_publish(1, 1000, t0);
        assert!(j.alpha_at(t0).is_none());
        j.on_publish(2, 1033, t0);
        let a = j.alpha_at(t0).expect("pair ready");
        // playhead = 1033 - ~33ms → near prev → α near 0
        assert!(a < 0.15, "expected delayed α near 0, got {a}");
    }

    #[test]
    fn jitter_alpha_advances_with_local_time() {
        let mut j = SimJitterState::new(1);
        let t0 = Instant::now();
        j.on_publish(1, 1000, t0);
        j.on_publish(2, 1033, t0);
        let early = j.alpha_at(t0).unwrap();
        let later = j.alpha_at(t0 + Duration::from_millis(40)).unwrap();
        assert!(later >= early);
        assert!(later <= 1.0);
    }
}
