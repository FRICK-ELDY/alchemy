//! Dash menu screen definition (Resonite-like chrome). Layout only — no host knowledge.

use super::layout::{Color, LayoutTree, Rect, WidgetId};
use crate::turn::{TurnSettings, DEFAULT_SMOOTH_TURN_DEG_PER_SEC, DEFAULT_SNAP_DEGREES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashTab {
    Exit,
    Settings,
}

#[derive(Clone, Debug)]
pub struct DashState {
    pub open: bool,
    pub tab: DashTab,
    pub time_label: String,
    pub fps_label: String,
    pub canvas_w: f32,
    pub canvas_h: f32,
    /// Personal max move speed (m/s). Displayed in Settings; edit via virtual keyboard later.
    pub max_speed: f32,
    pub max_speed_label: String,
    pub turn: TurnSettings,
    pub turn_mode_label: String,
    pub snap_degrees_label: String,
    pub smooth_turn_label: String,
}

impl Default for DashState {
    fn default() -> Self {
        let max_speed = alchemy_protocol::DEFAULT_MOVE_SPEED;
        let turn = TurnSettings::default();
        Self {
            open: false,
            tab: DashTab::Exit,
            time_label: "00:00:00".into(),
            fps_label: "0 FPS".into(),
            canvas_w: 960.0,
            canvas_h: 540.0,
            max_speed,
            max_speed_label: format!("Max speed: {max_speed:.1} m/s"),
            turn,
            turn_mode_label: format!("Turn: {} (tap to toggle)", turn.mode.label()),
            snap_degrees_label: format!("Snap angle: {DEFAULT_SNAP_DEGREES:.0}°"),
            smooth_turn_label: format!("Smooth rate: {DEFAULT_SMOOTH_TURN_DEG_PER_SEC:.0}°/s"),
        }
    }
}

impl DashState {
    pub fn refresh_turn_labels(&mut self) {
        self.turn_mode_label = format!("Turn: {} (tap to toggle)", self.turn.mode.label());
        self.snap_degrees_label = format!("Snap angle: {:.0}°", self.turn.snap_degrees);
        self.smooth_turn_label = format!("Smooth rate: {:.0}°/s", self.turn.smooth_deg_per_sec);
    }

    pub fn toggle_turn_mode(&mut self) {
        self.turn.mode = self.turn.mode.toggle();
        self.refresh_turn_labels();
    }
}

const TOP_H: f32 = 44.0;
const BOTTOM_H: f32 = 72.0;
const BG: Color = Color::rgba(0.06, 0.07, 0.09, 0.92);
const BAR: Color = Color::rgba(0.10, 0.11, 0.14, 0.96);
const ACCENT: Color = Color::rgba(0.35, 0.45, 0.95, 1.0);
const DANGER: Color = Color::rgba(0.75, 0.20, 0.22, 1.0);
const TEXT: Color = Color::rgba(0.92, 0.93, 0.95, 1.0);
const MUTED: Color = Color::rgba(0.65, 0.68, 0.72, 1.0);
const TAB_ON: Color = Color::rgba(0.55, 0.15, 0.18, 0.95);

pub fn rebuild(tree: &mut LayoutTree, state: &DashState) {
    tree.clear();
    if !state.open {
        return;
    }

    let w = state.canvas_w.max(1.0);
    let h = state.canvas_h.max(1.0);

    // Dimmed full-canvas backdrop (still part of one Canvas).
    tree.push_rect(Rect::new(0.0, 0.0, w, h), BG);

    // --- Top bar ---
    tree.push_rect(Rect::new(0.0, 0.0, w, TOP_H), BAR);
    let icon = 28.0_f32;
    let mut x = 12.0_f32;
    let iy = (TOP_H - icon) * 0.5;
    for (name, id) in [
        ("mic", WidgetId::Mic),
        ("mic_off", WidgetId::MicOff),
        ("volume_up", WidgetId::Volume),
        ("hearing", WidgetId::Hearing),
    ] {
        let r = Rect::new(x, iy, icon, icon);
        tree.push_icon(r, name);
        tree.register_hit(id, r);
        x += icon + 8.0;
    }

    // Param stub bar
    let bar_x = x + 8.0;
    let bar_w = 120.0_f32.min(w * 0.15);
    tree.push_rect(
        Rect::new(bar_x, TOP_H * 0.35, bar_w, TOP_H * 0.3),
        Color::rgba(0.15, 0.55, 0.25, 1.0),
    );

    // Time + FPS (right-ish center)
    let time_w = 110.0;
    let fps_w = 90.0;
    let sync_w = 100.0;
    let close_w = 150.0;
    let right = w - 12.0;
    let close_x = right - close_w;
    let sync_x = close_x - 12.0 - sync_w;
    let fps_x = sync_x - 12.0 - fps_w;
    let time_x = fps_x - 12.0 - time_w;

    tree.push_label(
        Rect::new(time_x, 12.0, time_w, 20.0),
        &state.time_label,
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(fps_x, 12.0, fps_w, 20.0),
        &state.fps_label,
        TEXT,
        2.0,
    );

    let sync_r = Rect::new(sync_x, 8.0, sync_w, TOP_H - 16.0);
    tree.push_rect(sync_r, ACCENT);
    tree.push_icon(
        Rect::new(sync_x + 6.0, iy, icon * 0.75, icon * 0.75),
        "sync",
    );
    tree.push_label(
        Rect::new(sync_x + 28.0, 14.0, sync_w - 32.0, 18.0),
        "Synced",
        TEXT,
        2.0,
    );
    tree.register_hit(WidgetId::SyncBadge, sync_r);

    let close_r = Rect::new(close_x, 8.0, close_w - 36.0, TOP_H - 16.0);
    tree.push_rect(close_r, Color::rgba(0.18, 0.19, 0.22, 1.0));
    tree.push_label(
        Rect::new(close_x + 8.0, 14.0, close_w - 50.0, 18.0),
        "Close session",
        MUTED,
        1.0,
    );
    tree.register_hit(WidgetId::CloseSession, close_r);

    let x_r = Rect::new(w - 36.0, 8.0, 28.0, TOP_H - 16.0);
    tree.push_rect(x_r, DANGER);
    tree.push_icon(Rect::new(w - 32.0, iy, icon * 0.7, icon * 0.7), "close");
    tree.register_hit(WidgetId::CloseSession, x_r);

    // --- Bottom bar ---
    let by = h - BOTTOM_H;
    tree.push_rect(Rect::new(0.0, by, w, BOTTOM_H), BAR);

    let tab_w = 88.0;
    let gap = 24.0;
    let tabs_total = tab_w * 2.0 + gap;
    let tab0_x = (w - tabs_total) * 0.5;

    // Left: Settings, Right: Exit (Resonite-like: power/exit on the right).
    let set_r = Rect::new(tab0_x, by + 8.0, tab_w, BOTTOM_H - 16.0);
    if state.tab == DashTab::Settings {
        tree.push_rect(set_r, Color::rgba(0.20, 0.22, 0.28, 0.95));
    }
    tree.push_icon(
        Rect::new(tab0_x + (tab_w - 32.0) * 0.5, by + 12.0, 32.0, 32.0),
        "settings",
    );
    tree.push_label(
        Rect::new(tab0_x + 4.0, by + 46.0, tab_w - 8.0, 14.0),
        "Settings",
        TEXT,
        1.0,
    );
    tree.register_hit(WidgetId::TabSettings, set_r);

    let exit_x = tab0_x + tab_w + gap;
    let exit_r = Rect::new(exit_x, by + 8.0, tab_w, BOTTOM_H - 16.0);
    if state.tab == DashTab::Exit {
        tree.push_rect(exit_r, TAB_ON);
    }
    tree.push_icon(
        Rect::new(exit_x + (tab_w - 32.0) * 0.5, by + 12.0, 32.0, 32.0),
        "power",
    );
    tree.push_label(
        Rect::new(exit_x + 8.0, by + 46.0, tab_w - 16.0, 14.0),
        "Exit",
        TEXT,
        2.0,
    );
    tree.register_hit(WidgetId::TabExit, exit_r);

    // --- Center ---
    let cy0 = TOP_H + 24.0;
    let cy1 = by - 24.0;
    let ch = (cy1 - cy0).max(80.0);
    let cx = w * 0.12;
    let cw = w * 0.76;

    match state.tab {
        DashTab::Exit => {
            tree.push_label(
                Rect::new(cx, cy0, cw, 28.0),
                "Exit Alchemy",
                Color::rgba(0.70, 0.45, 0.95, 1.0),
                3.0,
            );

            let btn_h = 64.0;
            let btn_y = cy0 + 56.0;
            let btn_gap = 24.0;
            let btn_w = (cw - btn_gap) * 0.5;

            let left = Rect::new(cx, btn_y, btn_w, btn_h);
            tree.push_rect(left, Color::rgba(0.14, 0.16, 0.20, 1.0));
            tree.push_icon(Rect::new(cx + 16.0, btn_y + 16.0, 32.0, 32.0), "logout");
            tree.push_label(
                Rect::new(cx + 56.0, btn_y + 22.0, btn_w - 70.0, 24.0),
                "Exit client only",
                TEXT,
                2.0,
            );
            tree.register_hit(WidgetId::ExitClientOnly, left);

            let right = Rect::new(cx + btn_w + btn_gap, btn_y, btn_w, btn_h);
            tree.push_rect(right, Color::rgba(0.22, 0.12, 0.12, 1.0));
            tree.push_icon(
                Rect::new(cx + btn_w + btn_gap + 16.0, btn_y + 16.0, 32.0, 32.0),
                "exit_to_app",
            );
            tree.push_label(
                Rect::new(
                    cx + btn_w + btn_gap + 56.0,
                    btn_y + 22.0,
                    btn_w - 70.0,
                    24.0,
                ),
                "Exit client + host",
                TEXT,
                2.0,
            );
            tree.register_hit(WidgetId::ExitClientAndHost, right);

            tree.push_label(
                Rect::new(cx, btn_y + btn_h + 32.0, cw, 20.0),
                "Host exit is a stub ALCHEMY_HOST_PID or host_control",
                MUTED,
                1.0,
            );
        }
        DashTab::Settings => {
            let panel = Rect::new(cx, cy0, cw, ch.min(320.0));
            tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 24.0, cw - 48.0, 24.0),
                "Locomotion",
                TEXT,
                2.0,
            );
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 60.0, cw - 48.0, 24.0),
                &state.max_speed_label,
                TEXT,
                1.5,
            );
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 92.0, cw - 48.0, 18.0),
                "speed edit later (virtual keyboard)",
                MUTED,
                1.0,
            );

            let toggle = Rect::new(cx + 24.0, cy0 + 130.0, (cw - 48.0).min(420.0), 44.0);
            tree.push_rect(toggle, ACCENT);
            tree.push_label(
                Rect::new(toggle.x + 16.0, toggle.y + 10.0, toggle.w - 32.0, 24.0),
                &state.turn_mode_label,
                TEXT,
                1.5,
            );
            tree.register_hit(WidgetId::TurnModeToggle, toggle);

            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 190.0, cw - 48.0, 22.0),
                &state.snap_degrees_label,
                TEXT,
                1.5,
            );
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 220.0, cw - 48.0, 22.0),
                &state.smooth_turn_label,
                TEXT,
                1.5,
            );
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 252.0, cw - 48.0, 18.0),
                "snap angle / smooth rate: edit later",
                MUTED,
                1.0,
            );
            tree.push_label(
                Rect::new(cx + 24.0, cy0 + 280.0, cw - 48.0, 18.0),
                "VR: both sticks Y=move (aim), X=turn",
                MUTED,
                1.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_exit_tab_has_exit_hits() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            tab: DashTab::Exit,
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ExitClientOnly));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ExitClientAndHost));
        assert!(tree.hit_test(100.0, 100.0).is_some() || tree.draws.len() > 5);
    }

    #[test]
    fn closed_has_no_hits() {
        let mut tree = LayoutTree::default();
        rebuild(&mut tree, &DashState::default());
        assert!(tree.hits.is_empty());
        assert!(tree.draws.is_empty());
    }

    #[test]
    fn settings_tab_shows_max_speed() {
        let state = DashState {
            open: true,
            tab: DashTab::Settings,
            ..Default::default()
        };
        assert!(
            state.max_speed_label.contains("Max speed"),
            "label={}",
            state.max_speed_label
        );
        assert!((state.max_speed - alchemy_protocol::DEFAULT_MOVE_SPEED).abs() < f32::EPSILON);
        assert!(state.turn_mode_label.contains("Snap"));
        assert!(state.snap_degrees_label.contains("30"));
        let mut tree = LayoutTree::default();
        rebuild(&mut tree, &state);
        assert!(tree.draws.len() > 5);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::TurnModeToggle));
    }

    #[test]
    fn toggle_turn_mode_snap_to_smooth() {
        let mut state = DashState::default();
        assert_eq!(state.turn.mode, crate::turn::TurnMode::Snap);
        state.toggle_turn_mode();
        assert_eq!(state.turn.mode, crate::turn::TurnMode::Smooth);
        assert!(state.turn_mode_label.contains("Smooth"));
    }
}
