//! Official default Facet set layout (x000f + x0018 Home). Personal DashRoot only.
//!
//! Facets: Chrome (top) · Home · Settings · Exit (Pinned). Button colors react to hover/press.
//! Home holds account form strings only — no auth-server HTTP here.

use super::facet::{facet_of, DashRoot, FacetId};
use super::layout::{Color, LayoutTree, Rect, WidgetId};
use crate::turn::{TurnSettings, DEFAULT_SMOOTH_TURN_DEG_PER_SEC, DEFAULT_SNAP_DEGREES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashTab {
    /// Account shell (login / register / logout).
    Home,
    /// Pinned Exit Facet body.
    Exit,
    /// Official Settings Facet body.
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeMode {
    Login,
    Register,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeField {
    Identifier,
    Password,
    Username,
    Email,
    Birthday,
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
    /// Front-most hit under pointer / laser (Button hover).
    pub hover: Option<WidgetId>,
    /// Hit while primary is down (Button press).
    pub pressed: Option<WidgetId>,
    /// Home / account shell (x0018).
    pub home_mode: HomeMode,
    pub home_focus: Option<HomeField>,
    pub home_identifier: String,
    pub home_password: String,
    pub home_username: String,
    pub home_email: String,
    pub home_birthday: String,
    pub home_remember_me: bool,
    pub home_status: String,
    pub home_logged_in_as: Option<String>,
}

impl Default for DashState {
    fn default() -> Self {
        let max_speed = alchemy_protocol::DEFAULT_MOVE_SPEED;
        let turn = TurnSettings::default();
        Self {
            open: false,
            tab: DashTab::Home,
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
            hover: None,
            pressed: None,
            home_mode: HomeMode::Login,
            home_focus: Some(HomeField::Identifier),
            home_identifier: String::new(),
            home_password: String::new(),
            home_username: String::new(),
            home_email: String::new(),
            home_birthday: "2000-01-01".into(),
            home_remember_me: true,
            home_status: "Sign in to join the room".into(),
            home_logged_in_as: None,
        }
    }
}

/// Button / hit target color: press darkens, hover lightens.
pub fn interact_color(base: Color, hovered: bool, pressed: bool) -> Color {
    if pressed {
        Color::rgba(
            (base.r * 0.72).clamp(0.0, 1.0),
            (base.g * 0.72).clamp(0.0, 1.0),
            (base.b * 0.72).clamp(0.0, 1.0),
            base.a,
        )
    } else if hovered {
        Color::rgba(
            (base.r + 0.18).min(1.0),
            (base.g + 0.18).min(1.0),
            (base.b + 0.22).min(1.0),
            base.a,
        )
    } else {
        base
    }
}

impl DashState {
    fn interact(&self, id: WidgetId, base: Color) -> Color {
        interact_color(base, self.hover == Some(id), self.pressed == Some(id))
    }

    pub fn refresh_turn_labels(&mut self) {
        self.turn_mode_label = format!("Turn: {} (tap to toggle)", self.turn.mode.label());
        self.snap_degrees_label = format!("Snap angle: {:.0}°", self.turn.snap_degrees);
        self.smooth_turn_label = format!("Smooth rate: {:.0}°/s", self.turn.smooth_deg_per_sec);
    }

    pub fn toggle_turn_mode(&mut self) {
        self.turn.mode = self.turn.mode.toggle();
        self.refresh_turn_labels();
    }

    pub fn append_home_text(&mut self, text: &str) {
        let Some(field) = self.home_focus else {
            return;
        };
        let target = match field {
            HomeField::Identifier => &mut self.home_identifier,
            HomeField::Password => &mut self.home_password,
            HomeField::Username => &mut self.home_username,
            HomeField::Email => &mut self.home_email,
            HomeField::Birthday => &mut self.home_birthday,
        };
        for ch in text.chars() {
            if !ch.is_control() && target.len() < 64 {
                target.push(ch);
            }
        }
    }

    pub fn home_backspace(&mut self) {
        let Some(field) = self.home_focus else {
            return;
        };
        let target = match field {
            HomeField::Identifier => &mut self.home_identifier,
            HomeField::Password => &mut self.home_password,
            HomeField::Username => &mut self.home_username,
            HomeField::Email => &mut self.home_email,
            HomeField::Birthday => &mut self.home_birthday,
        };
        target.pop();
    }

    pub fn cycle_home_focus(&mut self) {
        let fields: &[HomeField] = match self.home_mode {
            HomeMode::Login => &[HomeField::Identifier, HomeField::Password],
            HomeMode::Register => &[
                HomeField::Username,
                HomeField::Email,
                HomeField::Password,
                HomeField::Birthday,
            ],
        };
        let next = match self.home_focus {
            None => fields.first().copied(),
            Some(cur) => {
                let i = fields.iter().position(|f| *f == cur).unwrap_or(0);
                Some(fields[(i + 1) % fields.len()])
            }
        };
        self.home_focus = next;
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
const BTN: Color = Color::rgba(0.14, 0.16, 0.20, 1.0);
const BTN_DANGER: Color = Color::rgba(0.22, 0.12, 0.12, 1.0);
const CLOSE_BTN: Color = Color::rgba(0.18, 0.19, 0.22, 1.0);
const ICON_HOT: Color = Color::rgba(0.22, 0.24, 0.30, 0.95);
const FIELD: Color = Color::rgba(0.10, 0.11, 0.14, 1.0);
const FIELD_FOCUS: Color = Color::rgba(0.16, 0.22, 0.34, 1.0);

fn field_value(state: &DashState, field: HomeField) -> String {
    match field {
        HomeField::Identifier => state.home_identifier.clone(),
        HomeField::Password => "*".repeat(state.home_password.chars().count()),
        HomeField::Username => state.home_username.clone(),
        HomeField::Email => state.home_email.clone(),
        HomeField::Birthday => state.home_birthday.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_field(
    tree: &mut LayoutTree,
    state: &DashState,
    x: f32,
    y: f32,
    w: f32,
    label: &str,
    field: HomeField,
    id: WidgetId,
) -> f32 {
    let h = 40.0;
    tree.push_label(Rect::new(x, y, w, 16.0), label, MUTED, 1.0);
    let r = Rect::new(x, y + 16.0, w, h);
    let focused = state.home_focus == Some(field);
    let base = if focused { FIELD_FOCUS } else { FIELD };
    tree.push_rect(r, state.interact(id, base));
    let mut shown = field_value(state, field);
    if shown.is_empty() {
        shown = if focused { "_".into() } else { String::new() };
    } else if focused {
        shown.push('_');
    }
    tree.push_label(
        Rect::new(x + 10.0, y + 26.0, w - 20.0, 20.0),
        &shown,
        TEXT,
        1.5,
    );
    tree.register_hit(id, r);
    y + 16.0 + h + 10.0
}

fn rebuild_home(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    let panel = Rect::new(cx, cy0, cw, ch.min(380.0));
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 16.0, cw - 48.0, 24.0),
        "Home",
        TEXT,
        2.5,
    );
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 44.0, cw - 48.0, 18.0),
        &state.home_status,
        MUTED,
        1.0,
    );

    let x = cx + 24.0;
    let mut y = cy0 + 72.0;
    let fw = (cw - 48.0).min(520.0);

    if let Some(name) = state.home_logged_in_as.as_ref() {
        tree.push_label(
            Rect::new(x, y, fw, 22.0),
            format!("Signed in as {name}"),
            TEXT,
            1.5,
        );
        y += 36.0;
        let logout = Rect::new(x, y, 200.0, 44.0);
        tree.push_rect(logout, state.interact(WidgetId::HomeLogout, BTN_DANGER));
        tree.push_icon(Rect::new(x + 12.0, y + 8.0, 28.0, 28.0), "logout");
        tree.push_label(
            Rect::new(x + 48.0, y + 12.0, 140.0, 22.0),
            "Log out",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeLogout, logout);
        return;
    }

    // Mode tabs
    let mode_w = 120.0;
    let login_r = Rect::new(x, y, mode_w, 32.0);
    let reg_r = Rect::new(x + mode_w + 8.0, y, mode_w, 32.0);
    let login_base = if state.home_mode == HomeMode::Login {
        ACCENT
    } else {
        BTN
    };
    let reg_base = if state.home_mode == HomeMode::Register {
        ACCENT
    } else {
        BTN
    };
    tree.push_rect(login_r, state.interact(WidgetId::HomeModeLogin, login_base));
    tree.push_label(
        Rect::new(login_r.x + 16.0, login_r.y + 8.0, mode_w - 32.0, 18.0),
        "Login",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::HomeModeLogin, login_r);
    tree.push_rect(reg_r, state.interact(WidgetId::HomeModeRegister, reg_base));
    tree.push_label(
        Rect::new(reg_r.x + 8.0, reg_r.y + 8.0, mode_w - 16.0, 18.0),
        "Register",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::HomeModeRegister, reg_r);
    y += 44.0;

    match state.home_mode {
        HomeMode::Login => {
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Username or email",
                HomeField::Identifier,
                WidgetId::HomeFieldIdentifier,
            );
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Password",
                HomeField::Password,
                WidgetId::HomeFieldPassword,
            );
        }
        HomeMode::Register => {
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Username",
                HomeField::Username,
                WidgetId::HomeFieldUsername,
            );
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Email",
                HomeField::Email,
                WidgetId::HomeFieldEmail,
            );
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Password",
                HomeField::Password,
                WidgetId::HomeFieldPassword,
            );
            y = push_field(
                tree,
                state,
                x,
                y,
                fw,
                "Birthday (YYYY-MM-DD)",
                HomeField::Birthday,
                WidgetId::HomeFieldBirthday,
            );
        }
    }

    let remember = Rect::new(x, y, 220.0, 28.0);
    let rem_label = if state.home_remember_me {
        "[x] Remember me"
    } else {
        "[ ] Remember me"
    };
    tree.push_rect(remember, state.interact(WidgetId::HomeRememberMe, BTN));
    tree.push_label(
        Rect::new(x + 8.0, y + 6.0, 200.0, 18.0),
        rem_label,
        TEXT,
        1.0,
    );
    tree.register_hit(WidgetId::HomeRememberMe, remember);
    y += 40.0;

    let submit = Rect::new(x, y, 200.0, 44.0);
    let submit_label = match state.home_mode {
        HomeMode::Login => "Sign in",
        HomeMode::Register => "Create account",
    };
    tree.push_rect(submit, state.interact(WidgetId::HomeSubmit, ACCENT));
    tree.push_label(
        Rect::new(x + 16.0, y + 12.0, 168.0, 22.0),
        submit_label,
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::HomeSubmit, submit);

    tree.push_label(
        Rect::new(x + 220.0, y + 14.0, fw - 220.0, 18.0),
        "Type while field focused (Tab cycles)",
        MUTED,
        1.0,
    );
}

pub fn rebuild(tree: &mut LayoutTree, state: &DashState) {
    tree.clear();
    if !state.open {
        return;
    }

    let w = state.canvas_w.max(1.0);
    let h = state.canvas_h.max(1.0);

    // Dimmed full-canvas backdrop (still part of one Canvas / DashRoot).
    tree.push_rect(Rect::new(0.0, 0.0, w, h), BG);

    // --- Facet: Chrome (top bar) ---
    debug_assert_eq!(facet_of(WidgetId::Mic), FacetId::Chrome);
    debug_assert!(DashRoot::official_facets().contains(&FacetId::Chrome));
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
        if state.hover == Some(id) || state.pressed == Some(id) {
            tree.push_rect(r, state.interact(id, ICON_HOT));
        }
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
    tree.push_rect(sync_r, state.interact(WidgetId::SyncBadge, ACCENT));
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
    tree.push_rect(close_r, state.interact(WidgetId::CloseSession, CLOSE_BTN));
    tree.push_label(
        Rect::new(close_x + 8.0, 14.0, close_w - 50.0, 18.0),
        "Close session",
        MUTED,
        1.0,
    );
    tree.register_hit(WidgetId::CloseSession, close_r);

    let x_r = Rect::new(w - 36.0, 8.0, 28.0, TOP_H - 16.0);
    tree.push_rect(x_r, state.interact(WidgetId::CloseSession, DANGER));
    tree.push_icon(Rect::new(w - 32.0, iy, icon * 0.7, icon * 0.7), "close");
    tree.register_hit(WidgetId::CloseSession, x_r);

    // --- Bottom bar: Home · Settings · Exit ---
    let by = h - BOTTOM_H;
    tree.push_rect(Rect::new(0.0, by, w, BOTTOM_H), BAR);

    let tab_w = 88.0;
    let gap = 16.0;
    let tabs_total = tab_w * 3.0 + gap * 2.0;
    let tab0_x = (w - tabs_total) * 0.5;

    let draw_tab = |tree: &mut LayoutTree,
                    state: &DashState,
                    x: f32,
                    tab: DashTab,
                    id: WidgetId,
                    icon: &'static str,
                    label: &str,
                    active_color: Color| {
        let r = Rect::new(x, by + 8.0, tab_w, BOTTOM_H - 16.0);
        let hot = state.hover == Some(id) || state.pressed == Some(id);
        if state.tab == tab || hot {
            let base = if state.tab == tab {
                active_color
            } else {
                ICON_HOT
            };
            tree.push_rect(r, state.interact(id, base));
        }
        tree.push_icon(
            Rect::new(x + (tab_w - 32.0) * 0.5, by + 12.0, 32.0, 32.0),
            icon,
        );
        tree.push_label(
            Rect::new(x + 4.0, by + 46.0, tab_w - 8.0, 14.0),
            label,
            TEXT,
            1.0,
        );
        tree.register_hit(id, r);
    };

    draw_tab(
        tree,
        state,
        tab0_x,
        DashTab::Home,
        WidgetId::TabHome,
        "home",
        FacetId::Home.label(),
        Color::rgba(0.18, 0.28, 0.42, 0.95),
    );
    draw_tab(
        tree,
        state,
        tab0_x + tab_w + gap,
        DashTab::Settings,
        WidgetId::TabSettings,
        "settings",
        FacetId::Settings.label(),
        Color::rgba(0.20, 0.22, 0.28, 0.95),
    );
    draw_tab(
        tree,
        state,
        tab0_x + (tab_w + gap) * 2.0,
        DashTab::Exit,
        WidgetId::TabExit,
        "power",
        FacetId::Exit.label(),
        TAB_ON,
    );

    // --- Center ---
    let cy0 = TOP_H + 24.0;
    let cy1 = by - 24.0;
    let ch = (cy1 - cy0).max(80.0);
    let cx = w * 0.12;
    let cw = w * 0.76;

    match state.tab {
        DashTab::Home => {
            rebuild_home(tree, state, cx, cy0, cw, ch);
        }
        // Pinned Exit Facet body
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
            tree.push_rect(left, state.interact(WidgetId::ExitClientOnly, BTN));
            tree.push_icon(Rect::new(cx + 16.0, btn_y + 16.0, 32.0, 32.0), "logout");
            tree.push_label(
                Rect::new(cx + 56.0, btn_y + 22.0, btn_w - 70.0, 24.0),
                "Exit client only",
                TEXT,
                2.0,
            );
            debug_assert!(DashRoot::is_pinned(facet_of(WidgetId::ExitClientOnly)));
            tree.register_hit(WidgetId::ExitClientOnly, left);

            let right = Rect::new(cx + btn_w + btn_gap, btn_y, btn_w, btn_h);
            tree.push_rect(
                right,
                state.interact(WidgetId::ExitClientAndHost, BTN_DANGER),
            );
            debug_assert!(facet_of(WidgetId::ExitClientAndHost).pinned());
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
        // Official Settings Facet body
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
            tree.push_rect(toggle, state.interact(WidgetId::TurnModeToggle, ACCENT));
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
    use super::super::layout::DrawCmd;
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
        assert!(tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
        assert!(tree.hit_test(100.0, 100.0).is_some() || tree.draws.len() > 5);
    }

    #[test]
    fn open_home_tab_has_submit() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            tab: DashTab::Home,
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree.hits.iter().any(|(id, _)| *id == WidgetId::HomeSubmit));
        assert!(tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
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

    #[test]
    fn interact_color_hover_lightens_press_darkens() {
        let base = Color::rgba(0.40, 0.40, 0.40, 1.0);
        let h = interact_color(base, true, false);
        assert!(h.r > base.r && h.g > base.g);
        let p = interact_color(base, true, true);
        assert!(p.r < base.r);
    }

    #[test]
    fn hover_exit_button_changes_draw_color() {
        let mut tree = LayoutTree::default();
        let mut state = DashState {
            open: true,
            tab: DashTab::Exit,
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        let id = WidgetId::ExitClientOnly;
        let hit_r = tree
            .hits
            .iter()
            .find(|(w, _)| *w == id)
            .map(|(_, r)| *r)
            .expect("exit button hit");
        let color_at = |tree: &LayoutTree| {
            tree.draws.iter().rev().find_map(|d| match d {
                DrawCmd::Rect { rect, color } if *rect == hit_r => Some(*color),
                _ => None,
            })
        };
        let color_before = color_at(&tree).expect("button rect");
        state.hover = Some(id);
        rebuild(&mut tree, &state);
        let color_after = color_at(&tree).expect("hovered button rect");
        assert!(
            color_after.r > color_before.r || color_after.g > color_before.g,
            "hover should lighten button before={color_before:?} after={color_after:?}"
        );
    }
}
