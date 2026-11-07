//! Official default Facet set layout (x000f + x0018 Home + x0037 create panel).
//!
//! Facets: Chrome (top) · Home · Settings · Exit (Pinned). Button colors react to hover/press.
//! Home holds account / create-world form strings only — no auth/world HTTP here.
//! Create panel visuals are placeholders (swap later).
//! Onboarding shell: Step1 operator → Step2 auth → Main hub (minimal chrome).
//!
//! x0053: logical fields live in [`SharedDashLogic`] (SharedState); LayoutTree / hover /
//! canvas / `hotswitch_available` / derived `hotswitch_label` stay per-surface.

use super::facet::{facet_of, DashRoot, FacetId};
use super::layout::{Color, LayoutTree, Rect, WidgetId};
use crate::access_level::AccessLevel;
use crate::operator_prefs::{OperatorKind, CUSTOM_DEFAULT_HOST};
use crate::turn::{TurnSettings, DEFAULT_SMOOTH_TURN_DEG_PER_SEC, DEFAULT_SNAP_DEGREES};

/// Full-screen onboarding / hub phases (before or instead of classic Dash tabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingShell {
    Step1Operator,
    Step1FederationStub,
    Step2Auth,
    MainHub,
    /// Classic Home / Settings / Exit after Host a room / Settings.
    Dash,
}

/// Session display-name cap (input + status; keep in sync with desktop join status).
pub const SESSION_NAME_MAX: usize = 48;
/// Login / register field cap (identifier, password, email, …).
const HOME_ACCOUNT_FIELD_MAX: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashTab {
    /// Account shell (login / register / logout).
    Home,
    /// Pinned Exit Facet body.
    Exit,
    /// Official Settings Facet body.
    Settings,
    /// Inventory take-out (x006d).
    Inventory,
    /// Worlds catalog / Join·Host (x007c).
    Worlds,
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
    /// Create-world session display name (x0037).
    SessionName,
    /// Settings advanced: fixed `ROOM_ID` for debug Join/Start (x007c Phase 4).
    RoomId,
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
    /// XR companion present (x0053); when false, hotswitch row is muted.
    pub hotswitch_available: bool,
    pub hotswitch_auto: bool,
    pub hotswitch_suppressed: bool,
    pub hotswitch_label: String,
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
    /// Register: ToS checkbox (must be true to submit; x P3 hygiene).
    pub home_tos_agreed: bool,
    pub home_status: String,
    pub home_logged_in_as: Option<String>,
    /// When true, Home shows create-world placeholder panel (x0037).
    pub home_create_open: bool,
    pub home_session_name: String,
    pub home_access_level: AccessLevel,
    /// Zenoh / world `room_id` (from env); shown read-only on create panel.
    pub home_room_id: String,
    /// When false (XR), Start/Join hits are omitted — Desktop HTTP only (x0037).
    pub home_session_actions_enabled: bool,
    /// Onboarding / main-hub shell (Step1–3).
    pub shell: OnboardingShell,
    pub operator: OperatorKind,
    pub custom_host: String,
    /// Focus for Step1 custom host field.
    pub custom_host_focus: bool,
    /// Inventory Drive rows (x006d); filled by surface after refresh.
    pub inventory_nodes: Vec<(String, String, String)>, // node_id, kind, name
    pub inventory_selected: Option<usize>,
    /// Scene mesh rows for put-back (x006e); `(entity_id, blake3)`.
    pub scene_meshes: Vec<(u32, String)>,
    pub scene_selected: Option<usize>,
    pub inventory_status: String,
    /// Worlds catalog rows (x007c): room_id, visibility, role_hint, owner_sub.
    pub worlds_rooms: Vec<(String, String, String, String)>,
    pub worlds_selected: Option<usize>,
    pub worlds_mine_only: bool,
    pub worlds_status: String,
    /// In-room shell: waiting for host frames after Join (x007c).
    pub worlds_waiting_host: bool,
    /// Waiting overlay may show Host Rescue only when viewer owns the room (x007c).
    pub worlds_can_host_rescue: bool,
    /// Live → WaitingHost after frames stall (distinct copy from first join wait).
    pub worlds_host_disconnect: bool,
    /// Last observed `last_frame_tick` used to detect stall / Live transition.
    pub worlds_seen_frame_tick: Option<u64>,
    /// Wall time of last *new* host frame tick (Live stall detection · x007c).
    pub worlds_last_frame_at: Option<std::time::Instant>,
    /// True after successful Join/Host until Leave/Logout (gates stall UX).
    pub worlds_in_room: bool,
    /// Room key epoch advanced on Token refresh — Rejoin / Leave required (x007e).
    pub worlds_key_stale: bool,
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
            hotswitch_available: false,
            hotswitch_auto: true,
            hotswitch_suppressed: false,
            hotswitch_label: "VR hotswitch: Desktop-only".into(),
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
            home_tos_agreed: false,
            home_status: "Sign in to create or join a world".into(),
            home_logged_in_as: None,
            home_create_open: false,
            home_session_name: "FRICK World".into(),
            home_access_level: AccessLevel::Public,
            home_room_id: "local".into(),
            home_session_actions_enabled: true,
            shell: OnboardingShell::Step1Operator,
            operator: OperatorKind::Official,
            custom_host: CUSTOM_DEFAULT_HOST.into(),
            custom_host_focus: false,
            inventory_nodes: Vec::new(),
            inventory_selected: None,
            scene_meshes: Vec::new(),
            scene_selected: None,
            inventory_status: String::new(),
            worlds_rooms: Vec::new(),
            worlds_selected: None,
            worlds_mine_only: false,
            worlds_status: String::new(),
            worlds_waiting_host: false,
            worlds_can_host_rescue: false,
            worlds_host_disconnect: false,
            worlds_seen_frame_tick: None,
            worlds_last_frame_at: None,
            worlds_in_room: false,
            worlds_key_stale: false,
        }
    }
}

/// Logical dash fields shared Desktop ↔ XR (x0053).
///
/// Conflict policy:
/// - **Chrome** (`open` / `tab` / `shell` / Settings): own `chrome_revision`.
///   Pull while not chrome-dirty (form-dirty OK). Push is CAS on
///   `chrome_revision` and writes chrome fields only.
/// - **Form** (Home / onboarding strings): own `revision`. Pull blocked while
///   form-dirty; push writes form fields only (never clobbers peer chrome).
///
/// Not shared (per-surface): `time_label` / `fps_label` / `canvas_*` / `hover` /
/// `pressed` / `home_session_actions_enabled` / `hotswitch_available` /
/// derived `hotswitch_label` / `home_password`.
/// Hotswitch auto/suppressed SoT is [`crate::surface::PresentationGate`], not here.
#[derive(Clone, Debug)]
pub struct SharedDashLogic {
    pub open: bool,
    pub tab: DashTab,
    pub shell: OnboardingShell,
    pub max_speed: f32,
    pub max_speed_label: String,
    pub turn: TurnSettings,
    pub turn_mode_label: String,
    pub snap_degrees_label: String,
    pub smooth_turn_label: String,
    pub home_mode: HomeMode,
    pub home_focus: Option<HomeField>,
    pub home_identifier: String,
    pub home_username: String,
    pub home_email: String,
    pub home_birthday: String,
    pub home_remember_me: bool,
    pub home_tos_agreed: bool,
    pub home_status: String,
    pub home_logged_in_as: Option<String>,
    pub home_create_open: bool,
    pub home_session_name: String,
    pub home_access_level: AccessLevel,
    pub home_room_id: String,
    pub operator: OperatorKind,
    pub custom_host: String,
    pub custom_host_focus: bool,
}

impl Default for SharedDashLogic {
    fn default() -> Self {
        Self::from_state(&DashState::default())
    }
}

impl SharedDashLogic {
    pub fn from_state(s: &DashState) -> Self {
        Self {
            open: s.open,
            tab: s.tab,
            shell: s.shell,
            max_speed: s.max_speed,
            max_speed_label: s.max_speed_label.clone(),
            turn: s.turn,
            turn_mode_label: s.turn_mode_label.clone(),
            snap_degrees_label: s.snap_degrees_label.clone(),
            smooth_turn_label: s.smooth_turn_label.clone(),
            home_mode: s.home_mode,
            home_focus: s.home_focus,
            home_identifier: s.home_identifier.clone(),
            home_username: s.home_username.clone(),
            home_email: s.home_email.clone(),
            home_birthday: s.home_birthday.clone(),
            home_remember_me: s.home_remember_me,
            home_tos_agreed: s.home_tos_agreed,
            home_status: s.home_status.clone(),
            home_logged_in_as: s.home_logged_in_as.clone(),
            home_create_open: s.home_create_open,
            home_session_name: s.home_session_name.clone(),
            home_access_level: s.home_access_level,
            home_room_id: s.home_room_id.clone(),
            operator: s.operator,
            custom_host: s.custom_host.clone(),
            custom_host_focus: s.custom_host_focus,
        }
    }

    pub fn chrome_eq(&self, s: &DashState) -> bool {
        self.open == s.open
            && self.tab == s.tab
            && self.shell == s.shell
            && self.max_speed == s.max_speed
            && self.max_speed_label == s.max_speed_label
            && self.turn == s.turn
            && self.turn_mode_label == s.turn_mode_label
            && self.snap_degrees_label == s.snap_degrees_label
            && self.smooth_turn_label == s.smooth_turn_label
    }

    pub fn form_eq(&self, s: &DashState) -> bool {
        self.home_mode == s.home_mode
            && self.home_focus == s.home_focus
            && self.home_identifier == s.home_identifier
            && self.home_username == s.home_username
            && self.home_email == s.home_email
            && self.home_birthday == s.home_birthday
            && self.home_remember_me == s.home_remember_me
            && self.home_tos_agreed == s.home_tos_agreed
            && self.home_status == s.home_status
            && self.home_logged_in_as == s.home_logged_in_as
            && self.home_create_open == s.home_create_open
            && self.home_session_name == s.home_session_name
            && self.home_access_level == s.home_access_level
            && self.home_room_id == s.home_room_id
            && self.operator == s.operator
            && self.custom_host == s.custom_host
            && self.custom_host_focus == s.custom_host_focus
    }

    /// Write chrome fields only (push path — does not touch form).
    pub fn write_chrome_from(&mut self, s: &DashState) {
        self.open = s.open;
        self.tab = s.tab;
        self.shell = s.shell;
        self.max_speed = s.max_speed;
        self.max_speed_label = s.max_speed_label.clone();
        self.turn = s.turn;
        self.turn_mode_label = s.turn_mode_label.clone();
        self.snap_degrees_label = s.snap_degrees_label.clone();
        self.smooth_turn_label = s.smooth_turn_label.clone();
    }

    /// Write form fields only (push path — does not touch chrome / password).
    pub fn write_form_from(&mut self, s: &DashState) {
        self.home_mode = s.home_mode;
        self.home_focus = s.home_focus;
        self.home_identifier = s.home_identifier.clone();
        self.home_username = s.home_username.clone();
        self.home_email = s.home_email.clone();
        self.home_birthday = s.home_birthday.clone();
        self.home_remember_me = s.home_remember_me;
        self.home_tos_agreed = s.home_tos_agreed;
        self.home_status = s.home_status.clone();
        self.home_logged_in_as = s.home_logged_in_as.clone();
        self.home_create_open = s.home_create_open;
        self.home_session_name = s.home_session_name.clone();
        self.home_access_level = s.home_access_level;
        self.home_room_id = s.home_room_id.clone();
        self.operator = s.operator;
        self.custom_host = s.custom_host.clone();
        self.custom_host_focus = s.custom_host_focus;
    }

    /// Apply Esc/Menu chrome + Settings. Clears hover when layout identity changes.
    pub fn apply_chrome_to(&self, s: &mut DashState) -> bool {
        let layout_changed = s.open != self.open || s.tab != self.tab || s.shell != self.shell;
        let mut changed = layout_changed;
        if s.open != self.open {
            s.open = self.open;
        }
        if s.tab != self.tab {
            s.tab = self.tab;
        }
        if s.shell != self.shell {
            s.shell = self.shell;
        }
        if s.max_speed != self.max_speed {
            s.max_speed = self.max_speed;
            changed = true;
        }
        if s.max_speed_label != self.max_speed_label {
            s.max_speed_label = self.max_speed_label.clone();
            changed = true;
        }
        if s.turn != self.turn {
            s.turn = self.turn;
            changed = true;
        }
        if s.turn_mode_label != self.turn_mode_label {
            s.turn_mode_label = self.turn_mode_label.clone();
            changed = true;
        }
        if s.snap_degrees_label != self.snap_degrees_label {
            s.snap_degrees_label = self.snap_degrees_label.clone();
            changed = true;
        }
        if s.smooth_turn_label != self.smooth_turn_label {
            s.smooth_turn_label = self.smooth_turn_label.clone();
            changed = true;
        }
        if layout_changed || !s.open {
            s.hover = None;
            s.pressed = None;
        }
        changed
    }

    /// Apply Home / onboarding form fields (not password).
    pub fn apply_form_to(&self, s: &mut DashState) -> bool {
        let mut changed = false;
        if s.home_mode != self.home_mode {
            s.home_mode = self.home_mode;
            changed = true;
        }
        if s.home_focus != self.home_focus {
            s.home_focus = self.home_focus;
            changed = true;
        }
        if s.home_identifier != self.home_identifier {
            s.home_identifier = self.home_identifier.clone();
            changed = true;
        }
        if s.home_username != self.home_username {
            s.home_username = self.home_username.clone();
            changed = true;
        }
        if s.home_email != self.home_email {
            s.home_email = self.home_email.clone();
            changed = true;
        }
        if s.home_birthday != self.home_birthday {
            s.home_birthday = self.home_birthday.clone();
            changed = true;
        }
        if s.home_remember_me != self.home_remember_me {
            s.home_remember_me = self.home_remember_me;
            changed = true;
        }
        if s.home_tos_agreed != self.home_tos_agreed {
            s.home_tos_agreed = self.home_tos_agreed;
            changed = true;
        }
        if s.home_status != self.home_status {
            s.home_status = self.home_status.clone();
            changed = true;
        }
        if s.home_logged_in_as != self.home_logged_in_as {
            s.home_logged_in_as = self.home_logged_in_as.clone();
            changed = true;
        }
        if s.home_create_open != self.home_create_open {
            s.home_create_open = self.home_create_open;
            changed = true;
        }
        if s.home_session_name != self.home_session_name {
            s.home_session_name = self.home_session_name.clone();
            changed = true;
        }
        if s.home_access_level != self.home_access_level {
            s.home_access_level = self.home_access_level;
            changed = true;
        }
        if s.home_room_id != self.home_room_id {
            s.home_room_id = self.home_room_id.clone();
            changed = true;
        }
        if s.operator != self.operator {
            s.operator = self.operator;
            changed = true;
        }
        if s.custom_host != self.custom_host {
            s.custom_host = self.custom_host.clone();
            changed = true;
        }
        if s.custom_host_focus != self.custom_host_focus {
            s.custom_host_focus = self.custom_host_focus;
            changed = true;
        }
        changed
    }

    /// Full apply (chrome + form). Per-surface fields untouched.
    pub fn apply_to(&self, s: &mut DashState) {
        self.apply_chrome_to(s);
        self.apply_form_to(s);
    }
}

/// Revisioned store for [`SharedDashLogic`] inside `SharedState`.
#[derive(Clone, Debug, Default)]
pub struct SharedDashStore {
    /// Form-field revision. Pull skipped while local form-dirty; push bumps only
    /// when form content actually changes (`form_eq` short-circuit).
    pub revision: u64,
    /// Chrome / Settings revision. Pull skipped while local chrome-dirty; push
    /// is CAS on this value and writes chrome fields only.
    pub chrome_revision: u64,
    pub logic: SharedDashLogic,
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

    pub fn refresh_hotswitch_label(&mut self) {
        self.hotswitch_label = if !self.hotswitch_available {
            "VR hotswitch: Desktop-only (needs OpenXR)".into()
        } else if self.hotswitch_suppressed {
            "VR hotswitch auto: suppressed (F8 still works)".into()
        } else if self.hotswitch_auto {
            "VR hotswitch auto: ON (F8 manual)".into()
        } else {
            "VR hotswitch auto: OFF (F8 manual)".into()
        };
    }

    pub fn toggle_hotswitch_auto(&mut self) {
        if !self.hotswitch_available {
            return;
        }
        self.hotswitch_auto = !self.hotswitch_auto;
        self.refresh_hotswitch_label();
    }

    pub fn append_home_text(&mut self, text: &str) {
        if self.shell == OnboardingShell::Step1Operator && self.custom_host_focus {
            for ch in text.chars() {
                if !ch.is_control() && self.custom_host.chars().count() < HOME_ACCOUNT_FIELD_MAX {
                    self.custom_host.push(ch);
                }
            }
            return;
        }
        if !self.home_field_edit_allowed() {
            return;
        }
        let Some(field) = self.home_focus else {
            return;
        };
        let target = match field {
            HomeField::Identifier => &mut self.home_identifier,
            HomeField::Password => &mut self.home_password,
            HomeField::Username => &mut self.home_username,
            HomeField::Email => &mut self.home_email,
            HomeField::Birthday => &mut self.home_birthday,
            HomeField::SessionName => &mut self.home_session_name,
            HomeField::RoomId => &mut self.home_room_id,
        };
        let max = match field {
            HomeField::SessionName => SESSION_NAME_MAX,
            HomeField::RoomId => HOME_ACCOUNT_FIELD_MAX,
            _ => HOME_ACCOUNT_FIELD_MAX,
        };
        for ch in text.chars() {
            // Count Unicode scalars (same unit as desktop `.chars().take(SESSION_NAME_MAX)`).
            if !ch.is_control() && target.chars().count() < max {
                target.push(ch);
            }
        }
    }

    pub fn home_backspace(&mut self) {
        if self.shell == OnboardingShell::Step1Operator && self.custom_host_focus {
            self.custom_host.pop();
            return;
        }
        if !self.home_field_edit_allowed() {
            return;
        }
        let Some(field) = self.home_focus else {
            return;
        };
        let target = match field {
            HomeField::Identifier => &mut self.home_identifier,
            HomeField::Password => &mut self.home_password,
            HomeField::Username => &mut self.home_username,
            HomeField::Email => &mut self.home_email,
            HomeField::Birthday => &mut self.home_birthday,
            HomeField::SessionName => &mut self.home_session_name,
            HomeField::RoomId => &mut self.home_room_id,
        };
        target.pop();
    }

    /// Defense in depth for hub / create / RoomId (SystemUi still gates Settings tab).
    fn home_field_edit_allowed(&self) -> bool {
        let Some(field) = self.home_focus else {
            return false;
        };
        if self.home_logged_in_as.is_none() {
            return !matches!(field, HomeField::SessionName | HomeField::RoomId);
        }
        match field {
            HomeField::SessionName => self.home_create_open,
            // RoomId: DashState cannot see tab; SystemUi must already restrict to Settings.
            HomeField::RoomId => !self.home_create_open,
            // Logged-in hub has no account fields.
            HomeField::Identifier
            | HomeField::Password
            | HomeField::Username
            | HomeField::Email
            | HomeField::Birthday => false,
        }
    }

    pub fn cycle_home_focus(&mut self) {
        if self.home_logged_in_as.is_some() {
            if self.home_create_open {
                self.home_focus = Some(HomeField::SessionName);
            }
            // Hub: no focus cycle (would walk invisible login fields).
            return;
        }
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
        HomeField::SessionName => state.home_session_name.clone(),
        HomeField::RoomId => state.home_room_id.clone(),
    }
}

pub fn access_widget(level: AccessLevel) -> WidgetId {
    match level {
        AccessLevel::Private => WidgetId::HomeAccessPrivate,
        AccessLevel::Lan => WidgetId::HomeAccessLan,
        AccessLevel::Friends => WidgetId::HomeAccessFriends,
        AccessLevel::FriendsPlus => WidgetId::HomeAccessFriendsPlus,
        AccessLevel::Followers => WidgetId::HomeAccessFollowers,
        AccessLevel::FollowersPlus => WidgetId::HomeAccessFollowersPlus,
        AccessLevel::Public => WidgetId::HomeAccessPublic,
    }
}

pub fn access_level_from_widget(id: WidgetId) -> Option<AccessLevel> {
    AccessLevel::ALL
        .into_iter()
        .find(|level| access_widget(*level) == id)
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

const INVENTORY_ROW_IDS: [WidgetId; 8] = [
    WidgetId::InventoryRow0,
    WidgetId::InventoryRow1,
    WidgetId::InventoryRow2,
    WidgetId::InventoryRow3,
    WidgetId::InventoryRow4,
    WidgetId::InventoryRow5,
    WidgetId::InventoryRow6,
    WidgetId::InventoryRow7,
];

const WORLDS_ROW_IDS: [WidgetId; 8] = [
    WidgetId::WorldsRow0,
    WidgetId::WorldsRow1,
    WidgetId::WorldsRow2,
    WidgetId::WorldsRow3,
    WidgetId::WorldsRow4,
    WidgetId::WorldsRow5,
    WidgetId::WorldsRow6,
    WidgetId::WorldsRow7,
];

const SCENE_MESH_ROW_IDS: [WidgetId; 4] = [
    WidgetId::SceneMeshRow0,
    WidgetId::SceneMeshRow1,
    WidgetId::SceneMeshRow2,
    WidgetId::SceneMeshRow3,
];

/// Worlds catalog Master-Detail (x007c).
fn rebuild_worlds(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    tree.push_label(Rect::new(cx, cy0, cw, 22.0), "Worlds", TEXT, 2.0);
    tree.push_label(
        Rect::new(cx, cy0 + 24.0, cw, 16.0),
        "Catalog = visible only. Join = guest ticket. Host = run local host-server.",
        MUTED,
        1.0,
    );

    let mut y = cy0 + 48.0;
    let btn_h = 32.0;
    let gap = 8.0;
    let refresh = Rect::new(cx, y, 100.0, btn_h);
    tree.push_rect(refresh, state.interact(WidgetId::WorldsRefresh, BTN));
    tree.push_label(
        Rect::new(refresh.x + 8.0, refresh.y + 8.0, 84.0, 16.0),
        "Refresh",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::WorldsRefresh, refresh);

    let all_r = Rect::new(cx + 108.0, y, 72.0, btn_h);
    let all_c = if !state.worlds_mine_only { ACCENT } else { BTN };
    tree.push_rect(all_r, state.interact(WidgetId::WorldsFilterAll, all_c));
    tree.push_label(
        Rect::new(all_r.x + 8.0, all_r.y + 8.0, 56.0, 16.0),
        "All",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::WorldsFilterAll, all_r);

    let mine_r = Rect::new(cx + 188.0, y, 100.0, btn_h);
    let mine_c = if state.worlds_mine_only { ACCENT } else { BTN };
    tree.push_rect(mine_r, state.interact(WidgetId::WorldsFilterMine, mine_c));
    tree.push_label(
        Rect::new(mine_r.x + 8.0, mine_r.y + 8.0, 84.0, 16.0),
        "My rooms",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::WorldsFilterMine, mine_r);

    let create = Rect::new(cx + 300.0, y, 120.0, btn_h);
    tree.push_rect(create, state.interact(WidgetId::WorldsCreate, ACCENT));
    tree.push_label(
        Rect::new(create.x + 8.0, create.y + 8.0, 104.0, 16.0),
        "Create world",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::WorldsCreate, create);

    y += btn_h + gap + 4.0;
    if state.worlds_key_stale {
        tree.push_label(
            Rect::new(cx, y, cw, 18.0),
            "Room updated — reconnect required (key rotated).",
            Color::rgba(0.95, 0.55, 0.35, 1.0),
            1.1,
        );
        y += 22.0;
        let rejoin = Rect::new(cx, y, 160.0, btn_h);
        tree.push_rect(rejoin, state.interact(WidgetId::WorldsRejoin, ACCENT));
        tree.push_label(
            Rect::new(rejoin.x + 8.0, rejoin.y + 8.0, 144.0, 16.0),
            "Rejoin room",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsRejoin, rejoin);
        let leave = Rect::new(cx + 168.0, y, 160.0, btn_h);
        tree.push_rect(leave, state.interact(WidgetId::WorldsLeave, BTN_DANGER));
        tree.push_label(
            Rect::new(leave.x + 8.0, leave.y + 8.0, 144.0, 16.0),
            "Leave to catalog",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsLeave, leave);
        y += btn_h + gap;
    } else if state.worlds_waiting_host {
        let wait_msg = if state.worlds_host_disconnect {
            "Host connection lost — wait for return or Leave."
        } else {
            "Waiting for host — ticket OK; space starts when a host publishes."
        };
        tree.push_label(
            Rect::new(cx, y, cw, 18.0),
            wait_msg,
            Color::rgba(0.95, 0.75, 0.35, 1.0),
            1.1,
        );
        y += 22.0;
        let leave = Rect::new(cx, y, 160.0, btn_h);
        tree.push_rect(leave, state.interact(WidgetId::WorldsLeave, BTN_DANGER));
        tree.push_label(
            Rect::new(leave.x + 8.0, leave.y + 8.0, 144.0, 16.0),
            "Leave to catalog",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsLeave, leave);
        if state.worlds_can_host_rescue {
            let rescue = Rect::new(cx + 168.0, y, 200.0, btn_h);
            tree.push_rect(rescue, state.interact(WidgetId::WorldsHostRescue, ACCENT));
            tree.push_label(
                Rect::new(rescue.x + 8.0, rescue.y + 8.0, 184.0, 16.0),
                "Host this room now",
                TEXT,
                1.2,
            );
            tree.register_hit(WidgetId::WorldsHostRescue, rescue);
        }
        y += btn_h + gap;
    }

    let list_w = (cw * 0.48).max(160.0);
    let detail_x = cx + list_w + 12.0;
    let detail_w = (cw - list_w - 12.0).max(160.0);
    let list_h = (ch - (y - cy0) - 36.0).max(80.0);

    tree.push_rect(
        Rect::new(cx, y, list_w, list_h),
        Color::rgba(0.10, 0.11, 0.14, 1.0),
    );

    if state.worlds_rooms.is_empty() {
        tree.push_label(
            Rect::new(cx + 12.0, y + 24.0, list_w - 24.0, 40.0),
            "No visible rooms. Create a world or Refresh.",
            MUTED,
            1.1,
        );
    } else {
        let row_h = 36.0;
        for (i, (room_id, visibility, role_hint, _owner)) in
            state.worlds_rooms.iter().take(8).enumerate()
        {
            let id = WORLDS_ROW_IDS[i];
            let ry = y + 8.0 + i as f32 * (row_h + 4.0);
            if ry + row_h > y + list_h {
                break;
            }
            let r = Rect::new(cx + 8.0, ry, list_w - 16.0, row_h);
            let selected = state.worlds_selected == Some(i);
            let base = if selected { ACCENT } else { BTN };
            tree.push_rect(r, state.interact(id, base));
            let role = if role_hint == "owner" { " [owner]" } else { "" };
            let label = format!("{room_id} · {visibility}{role}");
            tree.push_label(
                Rect::new(r.x + 8.0, r.y + 10.0, r.w - 16.0, 16.0),
                &label,
                TEXT,
                1.1,
            );
            tree.register_hit(id, r);
        }
    }

    tree.push_rect(
        Rect::new(detail_x, y, detail_w, list_h),
        Color::rgba(0.12, 0.13, 0.16, 1.0),
    );
    if let Some(i) = state.worlds_selected {
        if let Some((room_id, visibility, role_hint, owner)) = state.worlds_rooms.get(i) {
            tree.push_label(
                Rect::new(detail_x + 12.0, y + 12.0, detail_w - 24.0, 20.0),
                room_id,
                TEXT,
                1.8,
            );
            tree.push_label(
                Rect::new(detail_x + 12.0, y + 36.0, detail_w - 24.0, 16.0),
                format!("{visibility} · {role_hint}"),
                MUTED,
                1.1,
            );
            tree.push_label(
                Rect::new(detail_x + 12.0, y + 54.0, detail_w - 24.0, 28.0),
                format!("owner {owner}"),
                MUTED,
                1.0,
            );

            let is_owner = role_hint == "owner";
            let mut by = y + 96.0;
            if is_owner {
                let host = Rect::new(detail_x + 12.0, by, detail_w - 24.0, 40.0);
                tree.push_rect(host, state.interact(WidgetId::WorldsHostRoom, ACCENT));
                tree.push_label(
                    Rect::new(host.x + 10.0, host.y + 12.0, host.w - 20.0, 16.0),
                    "Host this room",
                    TEXT,
                    1.3,
                );
                tree.register_hit(WidgetId::WorldsHostRoom, host);
                by += 48.0;
                let join = Rect::new(detail_x + 12.0, by, detail_w - 24.0, 36.0);
                tree.push_rect(join, state.interact(WidgetId::WorldsJoinGuest, BTN));
                tree.push_label(
                    Rect::new(join.x + 10.0, join.y + 10.0, join.w - 20.0, 16.0),
                    "Join as guest",
                    TEXT,
                    1.2,
                );
                tree.register_hit(WidgetId::WorldsJoinGuest, join);
            } else {
                let join = Rect::new(detail_x + 12.0, by, detail_w - 24.0, 40.0);
                tree.push_rect(join, state.interact(WidgetId::WorldsJoinGuest, ACCENT));
                tree.push_label(
                    Rect::new(join.x + 10.0, join.y + 12.0, join.w - 20.0, 16.0),
                    "Join as guest",
                    TEXT,
                    1.3,
                );
                tree.register_hit(WidgetId::WorldsJoinGuest, join);
            }
            tree.push_label(
                Rect::new(detail_x + 12.0, y + list_h - 48.0, detail_w - 24.0, 36.0),
                "Join does not start a host. Without a host you will wait.",
                MUTED,
                1.0,
            );
        }
    } else {
        tree.push_label(
            Rect::new(detail_x + 12.0, y + 24.0, detail_w - 24.0, 40.0),
            "Select a room on the left.",
            MUTED,
            1.1,
        );
    }

    let status_y = y + list_h + 8.0;
    if !state.worlds_status.is_empty() {
        tree.push_label(
            Rect::new(cx, status_y, cw, 20.0),
            &state.worlds_status,
            MUTED,
            1.0,
        );
    }
}

fn rebuild_inventory(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    ch: f32,
) {
    // Drive 8 + Scene 4 with compact rows; clip hits to panel (x006e).
    let panel_h = ch.min(520.0);
    let panel = Rect::new(cx, cy0, cw, panel_h);
    let panel_bottom = cy0 + panel_h;
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 12.0, cw - 48.0, 22.0),
        "Inventory",
        TEXT,
        2.5,
    );
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 36.0, cw - 48.0, 16.0),
        if state.inventory_status.is_empty() {
            "Drive→Take out. Scene→Put back (copy) / Despawn removes from room (host)."
        } else {
            &state.inventory_status
        },
        MUTED,
        1.0,
    );

    let refresh = Rect::new(cx + 24.0, cy0 + 58.0, 88.0, 36.0);
    tree.push_rect(refresh, state.interact(WidgetId::InventoryRefresh, BTN));
    tree.push_label(
        Rect::new(cx + 32.0, cy0 + 68.0, 72.0, 18.0),
        "Refresh",
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::InventoryRefresh, refresh);

    let take = Rect::new(cx + 120.0, cy0 + 58.0, 100.0, 36.0);
    tree.push_rect(take, state.interact(WidgetId::InventoryTakeOut, ACCENT));
    tree.push_label(
        Rect::new(cx + 128.0, cy0 + 68.0, 84.0, 18.0),
        "Take out",
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::InventoryTakeOut, take);

    let put = Rect::new(cx + 228.0, cy0 + 58.0, 100.0, 36.0);
    tree.push_rect(put, state.interact(WidgetId::InventoryPutBack, ACCENT));
    tree.push_label(
        Rect::new(cx + 236.0, cy0 + 68.0, 84.0, 18.0),
        "Put back",
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::InventoryPutBack, put);

    // Destructive: remove from room (not Drive). Distinct from Take out / Put back.
    let despawn = Rect::new(cx + 336.0, cy0 + 58.0, 100.0, 36.0);
    tree.push_rect(
        despawn,
        state.interact(WidgetId::InventoryDespawn, BTN_DANGER),
    );
    tree.push_label(
        Rect::new(cx + 344.0, cy0 + 68.0, 84.0, 18.0),
        "Despawn",
        DANGER,
        1.5,
    );
    tree.register_hit(WidgetId::InventoryDespawn, despawn);

    let mut y = cy0 + 102.0;
    let row_h = 26.0;
    tree.push_label(
        Rect::new(cx + 24.0, y, cw - 48.0, 14.0),
        "Drive",
        MUTED,
        1.0,
    );
    y += 16.0;
    for (i, (node_id, kind, name)) in state.inventory_nodes.iter().take(8).enumerate() {
        if y + row_h > panel_bottom {
            break;
        }
        let id = INVENTORY_ROW_IDS[i];
        let row = Rect::new(cx + 24.0, y, cw - 48.0, row_h);
        let selected = state.inventory_selected == Some(i);
        let base = if selected {
            Color::rgba(0.22, 0.36, 0.30, 1.0)
        } else {
            BTN
        };
        tree.push_rect(row, state.interact(id, base));
        tree.push_label(
            Rect::new(cx + 36.0, y + 4.0, cw - 72.0, 18.0),
            format!("[{kind}] {name} ({})", &node_id[..8.min(node_id.len())]),
            TEXT,
            1.0,
        );
        tree.register_hit(id, row);
        y += row_h + 3.0;
    }

    y += 4.0;
    if y + 14.0 <= panel_bottom {
        tree.push_label(
            Rect::new(cx + 24.0, y, cw - 48.0, 14.0),
            "Scene meshes",
            MUTED,
            1.0,
        );
        y += 16.0;
    }
    for (i, (entity_id, blake3)) in state.scene_meshes.iter().take(4).enumerate() {
        if y + row_h > panel_bottom {
            break;
        }
        let id = SCENE_MESH_ROW_IDS[i];
        let row = Rect::new(cx + 24.0, y, cw - 48.0, row_h);
        let selected = state.scene_selected == Some(i);
        let base = if selected {
            Color::rgba(0.28, 0.30, 0.42, 1.0)
        } else {
            BTN
        };
        tree.push_rect(row, state.interact(id, base));
        tree.push_label(
            Rect::new(cx + 36.0, y + 4.0, cw - 72.0, 18.0),
            format!("[mesh] #{entity_id} {}", &blake3[..8.min(blake3.len())]),
            TEXT,
            1.0,
        );
        tree.register_hit(id, row);
        y += row_h + 3.0;
    }
}

fn rebuild_home(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    ch: f32,
    step2: bool,
) {
    if state.home_logged_in_as.is_some() && state.home_create_open {
        rebuild_home_create(tree, state, cx, cy0, cw, ch);
        return;
    }

    let panel = Rect::new(cx, cy0, cw, ch.min(420.0));
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
    let title = if step2 {
        "STEP 2 / 3 — LOGIN & REGISTRATION"
    } else {
        "Home"
    };
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 16.0, cw - 48.0, 24.0),
        title,
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

        // x007c Phase 4: Worlds is the primary Join/Host path (not Home Join/Start).
        let worlds = Rect::new(x, y, 240.0, 48.0);
        tree.push_rect(worlds, state.interact(WidgetId::HomeOpenWorlds, ACCENT));
        tree.push_label(
            Rect::new(x + 16.0, y + 14.0, 208.0, 22.0),
            "Open Worlds",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeOpenWorlds, worlds);

        let create = Rect::new(x + 256.0, y, 200.0, 48.0);
        tree.push_rect(create, state.interact(WidgetId::HomeNewWorld, BTN));
        tree.push_label(
            Rect::new(x + 272.0, y + 14.0, 168.0, 22.0),
            "Create world",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeNewWorld, create);
        y += 64.0;

        tree.push_label(
            Rect::new(x, y, fw, 32.0),
            "Catalog Join/Host live on the Worlds tab. Fixed ROOM_ID Join/Start → Settings (advanced).",
            MUTED,
            1.0,
        );
        y += 40.0;

        tree.push_label(
            Rect::new(x, y, fw, 18.0),
            format!("current room_id={} (after Join/Host)", state.home_room_id),
            MUTED,
            1.0,
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
    y += 36.0;

    if matches!(state.home_mode, HomeMode::Register) {
        let tos = Rect::new(x, y, 280.0, 28.0);
        let tos_label = if state.home_tos_agreed {
            "[x] I agree to the Terms of Service"
        } else {
            "[ ] I agree to the Terms of Service"
        };
        tree.push_rect(tos, state.interact(WidgetId::HomeTosAgreed, BTN));
        tree.push_label(
            Rect::new(x + 8.0, y + 6.0, 260.0, 18.0),
            tos_label,
            TEXT,
            1.0,
        );
        tree.register_hit(WidgetId::HomeTosAgreed, tos);
        y += 40.0;
    } else {
        y += 4.0;
    }

    let submit = Rect::new(x, y, 200.0, 44.0);
    let submit_label = if step2 {
        match state.home_mode {
            HomeMode::Login => "Proceed to Step 3",
            HomeMode::Register => "Create & proceed",
        }
    } else {
        match state.home_mode {
            HomeMode::Login => "Sign in",
            HomeMode::Register => "Create account",
        }
    };
    tree.push_rect(submit, state.interact(WidgetId::HomeSubmit, ACCENT));
    tree.push_label(
        Rect::new(x + 16.0, y + 12.0, 168.0, 22.0),
        submit_label,
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::HomeSubmit, submit);

    if step2 {
        let back = Rect::new(x + 220.0, y, 160.0, 44.0);
        tree.push_rect(back, state.interact(WidgetId::Step2Back, BTN));
        tree.push_label(
            Rect::new(x + 236.0, y + 12.0, 128.0, 22.0),
            "Back to Step 1",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::Step2Back, back);
    } else {
        tree.push_label(
            Rect::new(x + 220.0, y + 14.0, fw - 220.0, 18.0),
            "Type while field focused (Tab cycles)",
            MUTED,
            1.0,
        );
    }
}

/// Placeholder create-world panel (Resonite-like information structure only).
///
/// Layout guarantee (960×540 default): all 7 access levels + Start are hit-registered.
/// Start lives on the left column so the right column can hold the full ACL list.
fn rebuild_home_create(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    ch: f32,
) {
    let panel_h = ch.min(460.0);
    let panel = Rect::new(cx, cy0, cw, panel_h);
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));

    let back = Rect::new(cx + 16.0, cy0 + 12.0, 88.0, 32.0);
    tree.push_rect(back, state.interact(WidgetId::HomeCreateBack, BTN));
    tree.push_label(
        Rect::new(back.x + 16.0, back.y + 8.0, 56.0, 18.0),
        "Back",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::HomeCreateBack, back);

    tree.push_label(
        Rect::new(cx + 120.0, cy0 + 16.0, cw - 140.0, 24.0),
        "Create world — pick access, then Start",
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 48.0, cw - 48.0, 16.0),
        &state.home_status,
        MUTED,
        1.0,
    );

    let gap = 16.0;
    let left_w = (cw - 48.0 - gap) * 0.38;
    let right_w = (cw - 48.0 - gap) - left_w;
    let left_x = cx + 24.0;
    let right_x = left_x + left_w + gap;
    let body_y = cy0 + 72.0;
    let body_h = panel_h - 88.0;
    let start_h = 40.0;

    // Left: template (Grid only) + Start (bottom) so right column can fit all 7 levels.
    tree.push_rect(
        Rect::new(left_x, body_y, left_w, body_h),
        Color::rgba(0.10, 0.11, 0.14, 1.0),
    );
    tree.push_label(
        Rect::new(left_x + 12.0, body_y + 12.0, left_w - 24.0, 20.0),
        "Template",
        TEXT,
        1.5,
    );
    let templates = [
        ("Grid", true, Some(WidgetId::HomeTemplateGrid)),
        ("Platform", false, None),
        ("Blank", false, None),
        ("DebugWorld", false, None),
    ];
    let mut ty = body_y + 40.0;
    for (label, selected, hit) in templates {
        let row = Rect::new(left_x + 12.0, ty, left_w - 24.0, 28.0);
        let base = if selected { ACCENT } else { BTN };
        tree.push_rect(
            row,
            if let Some(id) = hit {
                state.interact(id, base)
            } else {
                base
            },
        );
        let mark = if selected { "(*)" } else { "( )" };
        let color = if selected { TEXT } else { MUTED };
        tree.push_label(
            Rect::new(row.x + 10.0, row.y + 6.0, row.w - 20.0, 18.0),
            format!("{mark} {label}"),
            color,
            1.1,
        );
        if let Some(id) = hit {
            tree.register_hit(id, row);
        }
        ty += 34.0;
    }
    tree.push_label(
        Rect::new(left_x + 12.0, ty + 4.0, left_w - 24.0, 32.0),
        "Only Grid is wired (DemoScene).",
        MUTED,
        1.0,
    );

    let start_y = body_y + body_h - start_h - 8.0;
    let start = Rect::new(left_x + 12.0, start_y, left_w - 24.0, start_h);
    if state.home_session_actions_enabled {
        tree.push_rect(start, state.interact(WidgetId::HomeStartSession, ACCENT));
        tree.push_label(
            Rect::new(start.x + 12.0, start.y + 10.0, start.w - 24.0, 22.0),
            "Start session",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeStartSession, start);
    } else {
        tree.push_rect(start, BTN);
        tree.push_label(
            Rect::new(start.x + 12.0, start.y + 10.0, start.w - 24.0, 22.0),
            "Start (Desktop)",
            MUTED,
            1.5,
        );
    }

    // Right: session name + full 7-level access list (no clipping).
    tree.push_rect(
        Rect::new(right_x, body_y, right_w, body_h),
        Color::rgba(0.10, 0.11, 0.14, 1.0),
    );
    tree.push_label(
        Rect::new(right_x + 12.0, body_y + 10.0, right_w - 24.0, 18.0),
        "Session",
        TEXT,
        1.4,
    );

    let mut ry = push_field(
        tree,
        state,
        right_x + 12.0,
        body_y + 32.0,
        right_w - 24.0,
        "Session name (display)",
        HomeField::SessionName,
        WidgetId::HomeCreateName,
    );
    tree.push_label(
        Rect::new(right_x + 12.0, ry, right_w - 24.0, 14.0),
        format!(
            "room_id={} (Start uses this; edit in Settings Advanced)",
            state.home_room_id
        ),
        MUTED,
        1.0,
    );
    ry += 20.0;

    tree.push_label(
        Rect::new(right_x + 12.0, ry, right_w - 24.0, 16.0),
        "Access level (who can enter)",
        TEXT,
        1.1,
    );
    ry += 18.0;

    let access_rows = AccessLevel::ALL.len().div_ceil(2);
    let avail = (body_y + body_h - 8.0 - ry).max(24.0 * access_rows as f32);
    let row_step = (avail / access_rows as f32).clamp(22.0, 28.0);
    let row_h = (row_step - 2.0).max(20.0);
    let col_w = (right_w - 24.0 - 8.0) * 0.5;
    for (i, level) in AccessLevel::ALL.iter().copied().enumerate() {
        let id = access_widget(level);
        let selected = state.home_access_level == level;
        let col = i % 2;
        let row_i = i / 2;
        let row = Rect::new(
            right_x + 12.0 + col as f32 * (col_w + 8.0),
            ry + row_i as f32 * row_step,
            col_w,
            row_h,
        );
        let base = if selected { ACCENT } else { BTN };
        tree.push_rect(row, state.interact(id, base));
        let mark = if selected { "(*)" } else { "( )" };
        tree.push_label(
            Rect::new(row.x + 6.0, row.y + 4.0, row.w - 12.0, 16.0),
            format!("{mark} {}", level.label()),
            TEXT,
            0.9,
        );
        tree.register_hit(id, row);
    }
}

const OFFICIAL_CARD: Color = Color::rgba(0.12, 0.28, 0.22, 1.0);
const FED_CARD: Color = Color::rgba(0.22, 0.14, 0.32, 1.0);
const CUSTOM_CARD: Color = Color::rgba(0.12, 0.22, 0.32, 1.0);
const SELECT_RING: Color = Color::rgba(0.45, 0.85, 0.95, 1.0);

fn rebuild_step1_operator(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    ch: f32,
) {
    tree.push_label(Rect::new(cx, cy0, cw, 22.0), "STEP 1 / 3", MUTED, 1.2);
    tree.push_label(
        Rect::new(cx, cy0 + 28.0, cw, 28.0),
        "SELECT SERVER OPERATOR",
        TEXT,
        2.5,
    );

    let gap = 16.0;
    let card_w = ((cw - gap * 2.0) / 3.0).max(120.0);
    let card_h = (ch - 120.0).clamp(180.0, 280.0);
    let card_y = cy0 + 72.0;
    let cards = [
        (
            WidgetId::OperatorOfficial,
            OperatorKind::Official,
            "OFFICIAL SERVER",
            "Pre-configured & Recommended",
            OFFICIAL_CARD,
        ),
        (
            WidgetId::OperatorFederation,
            OperatorKind::Federation,
            "COMMUNITY FEDERATION",
            "Connect with Federated Worlds",
            FED_CARD,
        ),
        (
            WidgetId::OperatorCustom,
            OperatorKind::Custom,
            "CUSTOM URL",
            "Self-Hosted or Localhost",
            CUSTOM_CARD,
        ),
    ];
    for (i, (id, kind, title, sub, base)) in cards.iter().enumerate() {
        let x = cx + i as f32 * (card_w + gap);
        let r = Rect::new(x, card_y, card_w, card_h);
        let selected = state.operator == *kind;
        if selected {
            tree.push_rect(
                Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0),
                SELECT_RING,
            );
        }
        tree.push_rect(r, state.interact(*id, *base));
        tree.push_label(
            Rect::new(x + 12.0, card_y + 24.0, card_w - 24.0, 40.0),
            *title,
            TEXT,
            1.4,
        );
        tree.push_label(
            Rect::new(x + 12.0, card_y + card_h - 40.0, card_w - 24.0, 28.0),
            *sub,
            MUTED,
            1.0,
        );
        tree.register_hit(*id, r);

        if *kind == OperatorKind::Custom {
            let field = Rect::new(x + 12.0, card_y + 90.0, card_w - 24.0, 36.0);
            let focused = state.custom_host_focus;
            let fc = if focused { FIELD_FOCUS } else { FIELD };
            tree.push_rect(field, state.interact(WidgetId::OperatorCustomHost, fc));
            let mut shown = state.custom_host.clone();
            if shown.is_empty() {
                shown = if focused {
                    "_".into()
                } else {
                    CUSTOM_DEFAULT_HOST.into()
                };
            } else if focused {
                shown.push('_');
            }
            tree.push_label(
                Rect::new(field.x + 8.0, field.y + 8.0, field.w - 16.0, 20.0),
                &shown,
                TEXT,
                1.2,
            );
            tree.register_hit(WidgetId::OperatorCustomHost, field);
        }
    }

    let next = Rect::new(cx + cw - 140.0, cy0 + ch - 52.0, 120.0, 40.0);
    // Federation stub / XR (no Desktop session actions): look disabled and no hit.
    let next_enabled =
        state.operator != OperatorKind::Federation && state.home_session_actions_enabled;
    let next_base = if next_enabled { ACCENT } else { BTN };
    tree.push_rect(next, state.interact(WidgetId::Step1Next, next_base));
    tree.push_label(
        Rect::new(next.x + 28.0, next.y + 10.0, 80.0, 20.0),
        "NEXT",
        if next_enabled { TEXT } else { MUTED },
        1.5,
    );
    if next_enabled {
        tree.register_hit(WidgetId::Step1Next, next);
    }

    // BACK is a no-op on first launch (no prior shell); still drawn for parity.
    let back = Rect::new(cx + cw - 280.0, cy0 + ch - 52.0, 120.0, 40.0);
    tree.push_rect(back, state.interact(WidgetId::Step1Back, BTN));
    tree.push_label(
        Rect::new(back.x + 28.0, back.y + 10.0, 80.0, 20.0),
        "BACK",
        MUTED,
        1.5,
    );
    tree.register_hit(WidgetId::Step1Back, back);
}

fn rebuild_federation_stub(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    ch: f32,
) {
    tree.push_label(
        Rect::new(cx, cy0, cw, 22.0),
        "STEP 1 / 3 — COMMUNITY FEDERATION",
        MUTED,
        1.2,
    );
    tree.push_label(
        Rect::new(cx, cy0 + 36.0, cw, 24.0),
        "Federation directory (stub)",
        TEXT,
        2.0,
    );
    let panel = Rect::new(cx, cy0 + 72.0, cw * 0.62, ch - 140.0);
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.18, 1.0));
    tree.push_label(
        Rect::new(panel.x + 16.0, panel.y + 16.0, panel.w - 32.0, 20.0),
        "Search or enter URL… (unavailable)",
        MUTED,
        1.0,
    );
    tree.push_label(
        Rect::new(panel.x + 16.0, panel.y + 56.0, panel.w - 32.0, 60.0),
        "No federation hubs listed yet.\nComing soon.",
        TEXT,
        1.3,
    );
    let detail = Rect::new(cx + cw * 0.66, cy0 + 72.0, cw * 0.34, ch - 140.0);
    tree.push_rect(detail, Color::rgba(0.10, 0.11, 0.15, 1.0));
    tree.push_label(
        Rect::new(detail.x + 12.0, detail.y + 16.0, detail.w - 24.0, 40.0),
        "alchemy-node-info.json\n(stub)",
        MUTED,
        1.0,
    );
    tree.push_label(
        Rect::new(
            detail.x + 12.0,
            detail.y + detail.h - 80.0,
            detail.w - 24.0,
            40.0,
        ),
        "CONNECT disabled",
        MUTED,
        1.2,
    );
    let back = Rect::new(cx + cw - 140.0, cy0 + ch - 52.0, 120.0, 40.0);
    tree.push_rect(back, state.interact(WidgetId::FederationStubBack, BTN));
    tree.push_label(
        Rect::new(back.x + 28.0, back.y + 10.0, 80.0, 20.0),
        "BACK",
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::FederationStubBack, back);
}

fn rebuild_main_hub(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    let who = state.home_logged_in_as.as_deref().unwrap_or("signed in");
    tree.push_label(Rect::new(cx, cy0, cw * 0.5, 24.0), "Alchemy", TEXT, 2.5);
    tree.push_label(
        Rect::new(cx + cw * 0.5, cy0, cw * 0.5, 20.0),
        format!("{who} · server operator"),
        MUTED,
        1.1,
    );
    tree.push_label(
        Rect::new(cx, cy0 + 36.0, cw, 18.0),
        &state.home_status,
        MUTED,
        1.0,
    );

    let gap = 24.0;
    let card_w = ((cw - gap) * 0.5).max(160.0);
    let card_h = (ch - 100.0).clamp(160.0, 300.0);
    let y = cy0 + 72.0;

    let play = Rect::new(cx, y, card_w, card_h);
    tree.push_rect(play, state.interact(WidgetId::MainJoinHost, CUSTOM_CARD));
    tree.push_label(
        Rect::new(cx + 20.0, y + 28.0, card_w - 40.0, 28.0),
        "Join a host",
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(cx + 20.0, y + 68.0, card_w - 40.0, 20.0),
        "Enter as guest",
        MUTED,
        1.2,
    );
    tree.push_label(
        Rect::new(cx + 20.0, y + 100.0, card_w - 40.0, 40.0),
        "Join a room someone else is hosting",
        MUTED,
        1.0,
    );
    tree.push_label(
        Rect::new(cx + 20.0, y + card_h - 48.0, card_w - 40.0, 28.0),
        "Worlds → Join as guest",
        MUTED,
        1.0,
    );
    tree.register_hit(WidgetId::MainJoinHost, play);

    let solo = Rect::new(cx + card_w + gap, y, card_w, card_h);
    tree.push_rect(
        solo,
        state.interact(WidgetId::MainHostRoom, Color::rgba(0.32, 0.18, 0.10, 1.0)),
    );
    tree.push_label(
        Rect::new(solo.x + 20.0, y + 28.0, card_w - 40.0, 28.0),
        "Host a room",
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(solo.x + 20.0, y + 68.0, card_w - 40.0, 20.0),
        "Run space on this PC",
        MUTED,
        1.2,
    );
    tree.push_label(
        Rect::new(solo.x + 20.0, y + 100.0, card_w - 40.0, 40.0),
        "Create world or Host this room",
        MUTED,
        1.0,
    );
    tree.push_label(
        Rect::new(solo.x + 20.0, y + card_h - 48.0, card_w - 40.0, 28.0),
        "Worlds → Create / Host",
        TEXT,
        1.3,
    );
    tree.register_hit(WidgetId::MainHostRoom, solo);
}

pub fn rebuild(tree: &mut LayoutTree, state: &DashState) {
    tree.clear();
    if !state.open {
        return;
    }

    let w = state.canvas_w.max(1.0);
    let h = state.canvas_h.max(1.0);
    let onboarding = matches!(
        state.shell,
        OnboardingShell::Step1Operator
            | OnboardingShell::Step1FederationStub
            | OnboardingShell::Step2Auth
            | OnboardingShell::MainHub
    );

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

    let show_bottom_tabs = matches!(
        state.shell,
        OnboardingShell::Dash | OnboardingShell::MainHub
    );
    let by = if show_bottom_tabs { h - BOTTOM_H } else { h };
    if show_bottom_tabs {
        // --- Bottom bar: Home · Worlds · Inventory · Settings · Exit ---
        tree.push_rect(Rect::new(0.0, by, w, BOTTOM_H), BAR);

        let tab_w = 68.0;
        let gap = 8.0;
        let tabs_total = tab_w * 5.0 + gap * 4.0;
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
            DashTab::Worlds,
            WidgetId::TabWorlds,
            "public",
            FacetId::Worlds.label(),
            Color::rgba(0.18, 0.32, 0.34, 0.95),
        );
        draw_tab(
            tree,
            state,
            tab0_x + (tab_w + gap) * 2.0,
            DashTab::Inventory,
            WidgetId::TabInventory,
            "inventory_2",
            FacetId::Inventory.label(),
            Color::rgba(0.16, 0.30, 0.26, 0.95),
        );
        draw_tab(
            tree,
            state,
            tab0_x + (tab_w + gap) * 3.0,
            DashTab::Settings,
            WidgetId::TabSettings,
            "settings",
            FacetId::Settings.label(),
            Color::rgba(0.20, 0.22, 0.28, 0.95),
        );
        draw_tab(
            tree,
            state,
            tab0_x + (tab_w + gap) * 4.0,
            DashTab::Exit,
            WidgetId::TabExit,
            "power",
            FacetId::Exit.label(),
            TAB_ON,
        );
    }

    // --- Center ---
    let cy0 = TOP_H + 24.0;
    let cy1 = by - 24.0;
    let ch = (cy1 - cy0).max(80.0);
    let cx = w * 0.12;
    let cw = w * 0.76;

    if onboarding {
        match state.shell {
            OnboardingShell::Step1Operator => {
                rebuild_step1_operator(tree, state, cx, cy0, cw, ch);
            }
            OnboardingShell::Step1FederationStub => {
                rebuild_federation_stub(tree, state, cx, cy0, cw, ch);
            }
            OnboardingShell::Step2Auth => {
                rebuild_home(tree, state, cx, cy0, cw, ch, true);
            }
            OnboardingShell::MainHub => {
                rebuild_main_hub(tree, state, cx, cy0, cw, ch);
            }
            OnboardingShell::Dash => {}
        }
        return;
    }

    match state.tab {
        DashTab::Home => {
            rebuild_home(tree, state, cx, cy0, cw, ch, false);
        }
        DashTab::Worlds => {
            rebuild_worlds(tree, state, cx, cy0, cw, ch);
        }
        DashTab::Inventory => {
            rebuild_inventory(tree, state, cx, cy0, cw, ch);
        }
        // Pinned Exit Facet body — single Exit (Exit Client only abolished).
        DashTab::Exit => {
            tree.push_label(
                Rect::new(cx, cy0, cw, 28.0),
                "Exit Alchemy",
                Color::rgba(0.70, 0.45, 0.95, 1.0),
                3.0,
            );

            let btn_h = 64.0;
            let btn_y = cy0 + 56.0;
            let btn_w = cw.min(420.0);

            let exit_btn = Rect::new(cx, btn_y, btn_w, btn_h);
            tree.push_rect(
                exit_btn,
                state.interact(WidgetId::ExitClientAndHost, BTN_DANGER),
            );
            debug_assert!(DashRoot::is_pinned(facet_of(WidgetId::ExitClientAndHost)));
            tree.push_icon(
                Rect::new(cx + 16.0, btn_y + 16.0, 32.0, 32.0),
                "exit_to_app",
            );
            tree.push_label(
                Rect::new(cx + 56.0, btn_y + 22.0, btn_w - 70.0, 24.0),
                "Exit",
                TEXT,
                2.0,
            );
            tree.register_hit(WidgetId::ExitClientAndHost, exit_btn);

            tree.push_label(
                Rect::new(cx, btn_y + btn_h + 32.0, cw, 40.0),
                "Stops this client's supervised host only (if Start session spawned one). External/CI hosts are untouched. Leave/Logout keep host.",
                MUTED,
                1.0,
            );
        }
        // Official Settings Facet body
        DashTab::Settings => {
            rebuild_settings(tree, state, cx, cy0, cw, ch);
        }
    }
}

fn rebuild_settings(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    let panel = Rect::new(cx, cy0, cw, ch.min(520.0));
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 16.0, cw - 48.0, 24.0),
        "Locomotion",
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 48.0, cw - 48.0, 22.0),
        &state.max_speed_label,
        TEXT,
        1.5,
    );

    let toggle = Rect::new(cx + 24.0, cy0 + 80.0, (cw - 48.0).min(420.0), 40.0);
    tree.push_rect(toggle, state.interact(WidgetId::TurnModeToggle, ACCENT));
    tree.push_label(
        Rect::new(toggle.x + 16.0, toggle.y + 10.0, toggle.w - 32.0, 22.0),
        &state.turn_mode_label,
        TEXT,
        1.4,
    );
    tree.register_hit(WidgetId::TurnModeToggle, toggle);

    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 132.0, cw - 48.0, 18.0),
        &state.snap_degrees_label,
        TEXT,
        1.2,
    );
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 154.0, cw - 48.0, 18.0),
        &state.smooth_turn_label,
        TEXT,
        1.2,
    );

    let hs = Rect::new(cx + 24.0, cy0 + 184.0, (cw - 48.0).min(520.0), 40.0);
    let hs_color = if state.hotswitch_available {
        state.interact(WidgetId::HotswitchAutoToggle, ACCENT)
    } else {
        Color::rgba(0.25, 0.26, 0.28, 1.0)
    };
    tree.push_rect(hs, hs_color);
    tree.push_label(
        Rect::new(hs.x + 16.0, hs.y + 10.0, hs.w - 32.0, 22.0),
        &state.hotswitch_label,
        TEXT,
        1.4,
    );
    if state.hotswitch_available {
        tree.register_hit(WidgetId::HotswitchAutoToggle, hs);
    }

    // Advanced: fixed ROOM_ID Join/Start (prefer Worlds for normal use · x007c Phase 4).
    let adv_y = cy0 + 240.0;
    tree.push_label(
        Rect::new(cx + 24.0, adv_y, cw - 48.0, 22.0),
        "Advanced room (debug)",
        TEXT,
        1.8,
    );
    tree.push_label(
        Rect::new(cx + 24.0, adv_y + 26.0, cw - 48.0, 28.0),
        "Prefer Worlds tab for catalog Join/Host. These use a fixed ROOM_ID (env retarget).",
        MUTED,
        1.0,
    );

    let room_field = Rect::new(cx + 24.0, adv_y + 60.0, (cw - 48.0).min(420.0), 36.0);
    let room_editable = state.home_logged_in_as.is_some();
    let room_focused = room_editable && state.home_focus == Some(HomeField::RoomId);
    tree.push_rect(
        room_field,
        if room_focused {
            ACCENT
        } else if room_editable {
            state.interact(WidgetId::SettingsRoomId, BTN)
        } else {
            BTN
        },
    );
    let room_label = if !room_editable {
        if state.home_room_id.is_empty() {
            "room_id (sign in to edit)".into()
        } else {
            format!("room_id: {} (sign in to edit)", state.home_room_id)
        }
    } else if state.home_room_id.is_empty() {
        "room_id (click + type)".into()
    } else {
        format!("room_id: {}", state.home_room_id)
    };
    tree.push_label(
        Rect::new(
            room_field.x + 12.0,
            room_field.y + 8.0,
            room_field.w - 24.0,
            20.0,
        ),
        room_label,
        if room_editable { TEXT } else { MUTED },
        1.2,
    );
    if room_editable {
        tree.register_hit(WidgetId::SettingsRoomId, room_field);
    }

    let btn_y = adv_y + 108.0;
    let join = Rect::new(cx + 24.0, btn_y, 160.0, 40.0);
    let start = Rect::new(cx + 200.0, btn_y, 180.0, 40.0);
    if state.home_logged_in_as.is_some() && state.home_session_actions_enabled {
        tree.push_rect(join, state.interact(WidgetId::SettingsJoinRoom, BTN));
        tree.push_label(
            Rect::new(join.x + 12.0, join.y + 10.0, join.w - 24.0, 20.0),
            "Join room",
            TEXT,
            1.3,
        );
        tree.register_hit(WidgetId::SettingsJoinRoom, join);

        tree.push_rect(
            start,
            state.interact(WidgetId::SettingsStartSession, ACCENT),
        );
        tree.push_label(
            Rect::new(start.x + 12.0, start.y + 10.0, start.w - 24.0, 20.0),
            "Start session",
            TEXT,
            1.3,
        );
        tree.register_hit(WidgetId::SettingsStartSession, start);
    } else {
        let (join_l, start_l) = if state.home_logged_in_as.is_none() {
            ("Join (sign in)", "Start (sign in)")
        } else {
            // Signed in but XR / browse-only: HTTP Start/Join is Desktop-only.
            ("Join (Desktop)", "Start (Desktop)")
        };
        tree.push_rect(join, BTN);
        tree.push_label(
            Rect::new(join.x + 12.0, join.y + 10.0, join.w - 24.0, 20.0),
            join_l,
            MUTED,
            1.2,
        );
        tree.push_rect(start, BTN);
        tree.push_label(
            Rect::new(start.x + 12.0, start.y + 10.0, start.w - 24.0, 20.0),
            start_l,
            MUTED,
            1.2,
        );
    }
    tree.push_label(
        Rect::new(cx + 24.0, btn_y + 48.0, cw - 48.0, 18.0),
        &state.home_status,
        MUTED,
        1.0,
    );
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
            shell: OnboardingShell::Dash,
            tab: DashTab::Exit,
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
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
            shell: OnboardingShell::Dash,
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
    fn logged_in_hub_points_to_worlds_not_join() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Home,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeOpenWorlds));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeNewWorld));
        assert!(!tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeJoinRoom));
    }

    #[test]
    fn settings_advanced_has_join_start_when_signed_in() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsRoomId));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsJoinRoom));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsStartSession));
    }

    #[test]
    fn settings_advanced_omits_join_start_hits_when_desktop_actions_disabled() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            home_session_actions_enabled: false,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsRoomId));
        assert!(!tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsJoinRoom));
        assert!(!tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsStartSession));
    }

    #[test]
    fn settings_advanced_room_id_not_editable_when_signed_out() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: None,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(!tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsRoomId));
    }

    #[test]
    fn create_panel_has_all_access_levels_and_start_at_default_canvas() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Home,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            home_create_open: true,
            home_access_level: AccessLevel::Friends,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeStartSession));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeCreateBack));
        for level in AccessLevel::ALL {
            let id = access_widget(level);
            assert!(
                tree.hits.iter().any(|(hid, _)| *hid == id),
                "missing hit for {level:?} ({})",
                level.as_str()
            );
        }
    }

    #[test]
    fn create_panel_xr_omits_start_hit() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Home,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            home_create_open: true,
            home_session_actions_enabled: false,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        assert!(!tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeStartSession));
    }

    #[test]
    fn logged_in_hub_focus_cycle_does_not_touch_login_fields() {
        let mut state = DashState {
            home_logged_in_as: Some("alice".into()),
            home_create_open: false,
            home_focus: Some(HomeField::Identifier),
            home_identifier: "keep".into(),
            ..Default::default()
        };
        state.cycle_home_focus();
        assert_eq!(state.home_focus, Some(HomeField::Identifier));
        state.home_backspace();
        assert_eq!(state.home_identifier, "keep");
    }

    #[test]
    fn closed_has_no_hits() {
        let mut tree = LayoutTree::default();
        rebuild(&mut tree, &DashState::default());
        assert!(tree.hits.is_empty());
        assert!(tree.draws.is_empty());
    }

    #[test]
    fn settings_tab_shows_hotswitch_when_available() {
        let mut tree = LayoutTree::default();
        let mut state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            hotswitch_available: true,
            hotswitch_auto: true,
            ..Default::default()
        };
        state.refresh_hotswitch_label();
        rebuild(&mut tree, &state);
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HotswitchAutoToggle));
        state.toggle_hotswitch_auto();
        assert!(!state.hotswitch_auto);
        assert!(state.hotswitch_label.contains("OFF"));
    }

    #[test]
    fn settings_tab_shows_max_speed() {
        let state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
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
            shell: OnboardingShell::Dash,
            tab: DashTab::Exit,
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        rebuild(&mut tree, &state);
        let id = WidgetId::ExitClientAndHost;
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

    #[test]
    fn shared_dash_logic_roundtrip_leaves_surface_locals() {
        let src = DashState {
            open: true,
            tab: DashTab::Settings,
            shell: OnboardingShell::Dash,
            max_speed: 2.5,
            home_status: "from desktop".into(),
            home_logged_in_as: Some("alice".into()),
            home_password: "secret".into(),
            hotswitch_auto: true,
            hotswitch_available: true,
            home_session_actions_enabled: true,
            hover: Some(WidgetId::CloseSession),
            pressed: Some(WidgetId::TabHome),
            time_label: "12:00".into(),
            fps_label: "60 FPS".into(),
            canvas_w: 1280.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        let logic = SharedDashLogic::from_state(&src);

        let mut dst = DashState {
            open: false,
            tab: DashTab::Home,
            shell: OnboardingShell::Step2Auth,
            home_password: "xr-local".into(),
            hotswitch_auto: false,
            hotswitch_available: false,
            home_session_actions_enabled: false,
            hover: Some(WidgetId::TabExit),
            pressed: Some(WidgetId::ExitClientAndHost),
            time_label: "xr-clock".into(),
            fps_label: "90 FPS".into(),
            canvas_w: 960.0,
            canvas_h: 540.0,
            ..Default::default()
        };
        logic.apply_to(&mut dst);

        assert!(dst.open);
        assert_eq!(dst.tab, DashTab::Settings);
        assert_eq!(dst.shell, OnboardingShell::Dash);
        assert!((dst.max_speed - 2.5).abs() < f32::EPSILON);
        assert_eq!(dst.home_status, "from desktop");
        assert_eq!(dst.home_logged_in_as.as_deref(), Some("alice"));
        // Password + hotswitch stay per-surface / Gate — not in SharedDashLogic.
        assert_eq!(dst.home_password, "xr-local");
        assert!(!dst.hotswitch_auto);
        assert!(!dst.hotswitch_available);
        assert!(!dst.home_session_actions_enabled);
        assert_eq!(dst.time_label, "xr-clock");
        assert_eq!(dst.fps_label, "90 FPS");
        assert!((dst.canvas_w - 960.0).abs() < f32::EPSILON);
        // Layout identity changed → hover/pressed cleared.
        assert!(dst.hover.is_none());
        assert!(dst.pressed.is_none());
    }

    #[test]
    fn apply_chrome_clears_hover_on_tab_change_while_open() {
        let logic = SharedDashLogic {
            open: true,
            tab: DashTab::Settings,
            shell: OnboardingShell::Dash,
            ..Default::default()
        };
        let mut s = DashState {
            open: true,
            tab: DashTab::Home,
            shell: OnboardingShell::Dash,
            hover: Some(WidgetId::HomeSubmit),
            pressed: Some(WidgetId::TabHome),
            ..Default::default()
        };
        assert!(logic.apply_chrome_to(&mut s));
        assert_eq!(s.tab, DashTab::Settings);
        assert!(s.hover.is_none());
        assert!(s.pressed.is_none());
    }
}
