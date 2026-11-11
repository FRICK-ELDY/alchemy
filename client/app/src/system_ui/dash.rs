//! Official default Facet set layout (x000f + x0018 Home + x0037 create panel).
//!
//! Facets: Chrome (top) · Home · Settings · Exit (Pinned). Button colors react to hover/press.
//! Home holds account / create-world form strings only — no auth/world HTTP here.
//! Create panel visuals are placeholders (swap later).
//! Onboarding shell: Step1 operator → Step2 auth → MainHub (Participation | Organizing).
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

/// MainHub intent tabs: join sessions vs create/host worlds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MainHubTab {
    #[default]
    Participation,
    Organizing,
}

/// Session display-name cap (input + status; keep in sync with desktop join status).
pub const SESSION_NAME_MAX: usize = 48;

/// New `room_id` for Create → Start (16 lowercase hex, same shape as world-server
/// `Rooms.random_room_id/0`). Charset `[A-Za-z0-9_-]+`, max 64 bytes.
///
/// Not CSPRNG: time+seq mix is enough to avoid accidental reuse across Create opens;
/// collision/prediction risk is negligible for local room ids (no secrecy).
pub fn fresh_create_room_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Mix time + a process-local counter so rapid Create opens do not collide.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mixed = nanos
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u128::from(seq) << 48);
    format!("{:016x}", mixed as u64)
}
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
    /// Legacy / Join waiting shell carrier (no bottom tab; catalog is MainHub).
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
    /// MainHub: Participation (default) vs Organizing.
    /// Not in [`SharedDashLogic`] — Desktop↔XR do not sync this sub-tab (same as `worlds_*`).
    pub main_hub_tab: MainHubTab,
    pub operator: OperatorKind,
    pub custom_host: String,
    /// Focus for Step1 custom host field.
    pub custom_host_focus: bool,
    /// Local host / Organizing gate (Settings). Distinct from Step1 [`OperatorKind`].
    pub server_operator: bool,
    /// Inventory Drive rows (x006d); filled by surface after refresh.
    pub inventory_nodes: Vec<(String, String, String)>, // node_id, kind, name
    pub inventory_selected: Option<usize>,
    /// One-shot take-out target (XR peer / lag); not mirrored.
    /// Consumed at InventoryTakeOut entry (discarded on busy/blocked — XR re-fires).
    pub inventory_take_out_override: Option<(String, String)>,
    /// Scene mesh rows for put-back (x006e); `(entity_id, blake3)`.
    pub scene_meshes: Vec<(u32, String)>,
    /// Multi-select indices into `scene_meshes` (x007f put-back). Despawn uses first.
    pub scene_selected: Vec<usize>,
    pub inventory_status: String,
    /// Worlds catalog rows (x007c): room_id, visibility, role_hint, owner_sub.
    pub worlds_rooms: Vec<(String, String, String, String)>,
    /// Selected catalog room_id (stable across Refresh reorder).
    pub worlds_selected: Option<String>,
    pub worlds_mine_only: bool,
    /// Participation catalog: first visible room index (windowed list).
    pub worlds_catalog_scroll: usize,
    /// Trackpad/wheel pixel accumulator before stepping one catalog row.
    pub worlds_catalog_scroll_accum: f32,
    pub worlds_status: String,
    /// Desktop-authored in-session flag mirrored via SharedDashLogic (XR Inventory gate).
    pub in_session_peer: bool,
    /// Desktop-authored Live flag (Inventory allowed · x007f); XR echoes via SharedDashLogic.
    pub inventory_live_peer: bool,
    /// In-room shell: waiting for host frames after Join (x007c).
    pub worlds_waiting_host: bool,
    /// Waiting overlay may show Host Rescue only when viewer owns the room (x007c).
    pub worlds_can_host_rescue: bool,
    /// Waiting overlay may show Takeover when session is dormant / lease-expired (x0080).
    pub worlds_can_takeover: bool,
    /// Local sub was nominated as `active_host_sub` (x0080 auto-host when Server Operator ON).
    pub worlds_nominated_host: bool,
    /// Last Waiting host-session poll (x0080 nomination / takeover CTA).
    pub worlds_session_poll_at: Option<std::time::Instant>,
    /// Live → WaitingHost after frames stall (distinct copy from first join wait).
    pub worlds_host_disconnect: bool,
    /// Last observed `last_frame_tick` used for first-Frame Live transition.
    pub worlds_seen_frame_tick: Option<u64>,
    /// Wall time of last *new* frame tick (enter UX). Stall uses SharedState host-alive.
    pub worlds_last_frame_at: Option<std::time::Instant>,
    /// True after successful Join/Host until Leave/Logout (gates stall UX).
    pub worlds_in_room: bool,
    /// Room key epoch advanced on Token refresh — Rejoin / Leave required (x007e).
    pub worlds_key_stale: bool,
    /// Create / Join / Host: modal until first host frame (or Cancel).
    pub session_enter_active: bool,
    /// 0.0..=1.0 enter progress (discrete steps from auth worker).
    pub session_enter_progress: f32,
    pub session_enter_title: String,
    /// Newest last; capped in [`DashState::push_session_enter_log`].
    pub session_enter_logs: Vec<String>,
    /// Wall time when post-join Frame wait began (heartbeat elapsed).
    pub session_enter_wait_since: Option<std::time::Instant>,
    /// True after Manifest watermark observed during enter wait.
    pub session_enter_seen_manifest: bool,
    /// Last enter-wait detail / heartbeat log.
    pub session_enter_last_detail_at: Option<std::time::Instant>,
    /// Settings body scroll offset (px); content above bottom tabs is clipped.
    pub settings_scroll: f32,
    /// Tape / Bitmask X-Ray panel text (local; not mirrored Desktop↔XR).
    pub epoch_xray_text: String,
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
            main_hub_tab: MainHubTab::Participation,
            operator: OperatorKind::Official,
            custom_host: CUSTOM_DEFAULT_HOST.into(),
            custom_host_focus: false,
            server_operator: false,
            inventory_nodes: Vec::new(),
            inventory_selected: None,
            inventory_take_out_override: None,
            scene_meshes: Vec::new(),
            scene_selected: Vec::new(),
            inventory_status: String::new(),
            worlds_rooms: Vec::new(),
            worlds_selected: None,
            worlds_mine_only: false,
            worlds_catalog_scroll: 0,
            worlds_catalog_scroll_accum: 0.0,
            worlds_status: String::new(),
            in_session_peer: false,
            inventory_live_peer: false,
            worlds_waiting_host: false,
            worlds_can_host_rescue: false,
            worlds_can_takeover: false,
            worlds_nominated_host: false,
            worlds_session_poll_at: None,
            worlds_host_disconnect: false,
            worlds_seen_frame_tick: None,
            worlds_last_frame_at: None,
            worlds_in_room: false,
            worlds_key_stale: false,
            session_enter_active: false,
            session_enter_progress: 0.0,
            session_enter_title: String::new(),
            session_enter_logs: Vec::new(),
            session_enter_wait_since: None,
            session_enter_seen_manifest: false,
            session_enter_last_detail_at: None,
            settings_scroll: 0.0,
            epoch_xray_text: String::new(),
        }
    }
}

/// Logical dash fields shared Desktop ↔ XR (x0053).
///
/// Conflict policy:
/// - **Chrome** (`open` / `tab` / `shell` / Settings / `in_session`): own
///   `chrome_revision`. Pull while not chrome-dirty (form-dirty OK). Push is CAS
///   on `chrome_revision` and writes chrome fields only.
/// - **`in_session`**: Desktop authors from `worlds_*` only; XR echoes into
///   `in_session_peer`. Desktop never latches peer `in_session` into
///   `in_session_peer`. CAS miss with `store.in_session != local` restores
///   Desktop shell/tab/open landing and retries push (Leave false / Join true).
/// - **Form** (Home / onboarding strings): own `revision`. Pull blocked while
///   form-dirty; push writes form fields only (never clobbers peer chrome).
///
/// Not shared (per-surface): `time_label` / `fps_label` / `canvas_*` / `hover` /
/// `pressed` / `home_session_actions_enabled` / `hotswitch_available` /
/// derived `hotswitch_label` / `home_password` / `worlds_*` / `in_session_peer` /
/// `inventory_live_peer`.
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
    pub server_operator: bool,
    /// Desktop session shell active — XR uses this to block Inventory without local worlds_*.
    pub in_session: bool,
    /// Desktop Live (Inventory allowed · x007f). XR echoes into `inventory_live_peer`.
    pub inventory_live: bool,
}

impl Default for SharedDashLogic {
    fn default() -> Self {
        Self::from_state(&DashState::default())
    }
}

/// Desktop authors `in_session` from `worlds_*` only. XR echoes `in_session_peer`
/// so a CAS-miss adopt cannot re-latch Desktop after Leave.
fn dash_in_session_for_store(s: &DashState) -> bool {
    if s.home_session_actions_enabled {
        s.session_shell_active()
    } else {
        s.in_session_peer
    }
}

/// Live-only Inventory gate (x007f): in-room, not waiting / KeyStale / entering.
fn dash_inventory_live_for_store(s: &DashState) -> bool {
    if s.home_session_actions_enabled {
        s.inventory_live_allowed()
    } else {
        s.inventory_live_peer
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
            server_operator: s.server_operator,
            in_session: dash_in_session_for_store(s),
            inventory_live: dash_inventory_live_for_store(s),
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
            && self.server_operator == s.server_operator
            && self.in_session == dash_in_session_for_store(s)
            && self.inventory_live == dash_inventory_live_for_store(s)
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
        self.server_operator = s.server_operator;
        self.in_session = dash_in_session_for_store(s);
        self.inventory_live = dash_inventory_live_for_store(s);
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
        // Live Inventory (x007f): peer Inventory tab is allowed when not blocked.
        let peer_inventory = self.tab == DashTab::Inventory;
        if s.tab != self.tab {
            if s.inventory_blocked() && peer_inventory {
                // keep local tab (Home/Settings/Exit) while Waiting / KeyStale / etc.
            } else {
                s.tab = self.tab;
            }
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
        if s.server_operator != self.server_operator {
            s.server_operator = self.server_operator;
            changed = true;
        }
        // Desktop owns worlds_* — never latch peer in_session into in_session_peer (CAS miss heal).
        // XR only: mirror Desktop-authored in_session / inventory_live for Inventory gate (x007f).
        if s.home_session_actions_enabled {
            if s.in_session_peer {
                s.in_session_peer = false;
                changed = true;
            }
            if s.inventory_live_peer {
                s.inventory_live_peer = false;
                changed = true;
            }
        } else {
            if s.in_session_peer != self.in_session {
                s.in_session_peer = self.in_session;
                changed = true;
            }
            if s.inventory_live_peer != self.inventory_live {
                s.inventory_live_peer = self.inventory_live;
                changed = true;
            }
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

const SCENE_MESH_ROW_IDS: [WidgetId; 4] = [
    WidgetId::SceneMeshRow0,
    WidgetId::SceneMeshRow1,
    WidgetId::SceneMeshRow2,
    WidgetId::SceneMeshRow3,
];

impl DashState {
    /// True while Join/Host has an active room session (Waiting / Live / KeyStale / Entering).
    pub fn session_shell_active(&self) -> bool {
        self.session_enter_active
            || self.worlds_in_room
            || self.worlds_waiting_host
            || self.worlds_key_stale
    }

    /// Open the enter modal (Create / Join / Host until first frame or Cancel).
    pub fn begin_session_enter(&mut self, title: &str) {
        self.session_enter_active = true;
        self.session_enter_progress = 0.05;
        self.session_enter_title = title.to_string();
        self.session_enter_logs.clear();
        self.session_enter_wait_since = None;
        self.session_enter_seen_manifest = false;
        self.session_enter_last_detail_at = None;
    }

    pub fn set_session_enter_progress(&mut self, progress: f32) {
        self.session_enter_progress = progress.clamp(0.0, 1.0);
    }

    pub fn push_session_enter_log(&mut self, line: impl Into<String>) {
        const MAX: usize = 14;
        let mut s = line.into();
        if s.chars().count() > 110 {
            s = s.chars().take(107).collect::<String>() + "…";
        }
        if s.is_empty() {
            return;
        }
        self.session_enter_logs.push(s);
        if self.session_enter_logs.len() > MAX {
            let drop_n = self.session_enter_logs.len() - MAX;
            self.session_enter_logs.drain(0..drop_n);
        }
    }

    pub fn apply_session_enter_progress(&mut self, progress: f32, title: &str, log: &str) {
        if !self.session_enter_active {
            self.begin_session_enter(title);
        } else if !title.is_empty() {
            self.session_enter_title = title.to_string();
        }
        self.set_session_enter_progress(progress);
        if !log.is_empty() {
            self.push_session_enter_log(log);
        }
    }

    /// Mark post-join wait (bus bound; listening for Manifest then Frame).
    pub fn mark_session_enter_bus_bound(&mut self) {
        let now = std::time::Instant::now();
        self.session_enter_wait_since = Some(now);
        self.session_enter_seen_manifest = false;
        self.session_enter_last_detail_at = Some(now);
        self.apply_session_enter_progress(
            0.90,
            "Waiting for host publish…",
            "Room bus bound — listening for Manifest then Frame",
        );
    }

    pub fn end_session_enter(&mut self) {
        self.session_enter_active = false;
        self.session_enter_progress = 0.0;
        self.session_enter_title.clear();
        self.session_enter_logs.clear();
        self.session_enter_wait_since = None;
        self.session_enter_seen_manifest = false;
        self.session_enter_last_detail_at = None;
    }

    /// Live session allows Inventory (x007f). Waiting / KeyStale / Entering stay blocked.
    pub fn inventory_live_allowed(&self) -> bool {
        self.worlds_in_room
            && !self.worlds_waiting_host
            && !self.worlds_key_stale
            && !self.session_enter_active
    }

    /// Inventory blocked outside Live while a session shell is active.
    /// Desktop: local `worlds_*`. XR: Desktop-authored flags via `*_peer`.
    pub fn inventory_blocked(&self) -> bool {
        if self.home_session_actions_enabled {
            self.session_shell_active() && !self.inventory_live_allowed()
        } else {
            self.in_session_peer && !self.inventory_live_peer
        }
    }

    /// Value written to SharedDashLogic.in_session for Desktop↔XR chrome sync.
    pub fn in_session_for_store(&self) -> bool {
        dash_in_session_for_store(self)
    }

    /// Settings center panel rect (same geometry as `rebuild` center).
    pub fn settings_panel_rect(&self) -> Rect {
        let show_bottom = matches!(self.shell, OnboardingShell::Dash | OnboardingShell::MainHub);
        let by = if show_bottom {
            self.canvas_h - BOTTOM_H
        } else {
            self.canvas_h
        };
        let cy0 = TOP_H + 24.0;
        let ch = (by - 24.0 - cy0).max(80.0);
        let cx = self.canvas_w * 0.12;
        let cw = self.canvas_w * 0.76;
        Rect::new(cx, cy0, cw, ch)
    }

    /// Max Settings scroll for current canvas / bottom-tab chrome.
    pub fn settings_max_scroll(&self) -> f32 {
        let panel_h = self.settings_panel_rect().h;
        (settings_content_height(self) - panel_h).max(0.0)
    }

    /// Clamp `settings_scroll` after resize / content-height changes.
    pub fn clamp_settings_scroll(&mut self) {
        let max = self.settings_max_scroll();
        self.settings_scroll = self.settings_scroll.clamp(0.0, max);
    }

    /// `delta_y` > 0 scrolls content up (reveal lower widgets). Clamped.
    pub fn scroll_settings_by(&mut self, delta_y: f32) {
        self.clamp_settings_scroll();
        let max = self.settings_max_scroll();
        self.settings_scroll = (self.settings_scroll + delta_y).clamp(0.0, max);
    }

    /// How many Participation rows fit in `list_h` (matches `rebuild_main_hub_participation`).
    pub fn worlds_catalog_visible_rows(list_h: f32) -> usize {
        const ROW_H: f32 = 36.0;
        const GAP: f32 = 4.0;
        (((list_h - 16.0) / (ROW_H + GAP)).floor() as usize).clamp(1, 8)
    }

    pub fn worlds_catalog_max_scroll(&self, list_h: f32) -> usize {
        let visible = Self::worlds_catalog_visible_rows(list_h);
        self.worlds_rooms.len().saturating_sub(visible)
    }

    /// Friends stub + Public header height above the catalog list (single source for layout/scroll).
    pub const WORLDS_CATALOG_HEADER_H: f32 = 24.0 + 40.0 + 28.0; // title + friends + public header
    /// Join bar reserve below the catalog list.
    pub const WORLDS_CATALOG_JOIN_RESERVE: f32 = 52.0;

    /// Participation catalog list height (same offsets as `rebuild_main_hub_participation`).
    pub fn worlds_catalog_list_h(&self) -> f32 {
        let panel_h = self.settings_panel_rect().h;
        let body_h = (panel_h - 80.0).max(80.0);
        (body_h - Self::WORLDS_CATALOG_HEADER_H - Self::WORLDS_CATALOG_JOIN_RESERVE).max(72.0)
    }

    pub fn worlds_catalog_list_rect(&self) -> Rect {
        let panel = self.settings_panel_rect();
        let body_y = panel.y + 80.0;
        let list_y = body_y + Self::WORLDS_CATALOG_HEADER_H;
        Rect::new(panel.x, list_y, panel.w, self.worlds_catalog_list_h())
    }

    /// Organizing "My worlds" window size (matches `MAIN_MY_WORLD_ROW_IDS`).
    pub const WORLDS_ORGANIZING_VISIBLE_ROWS: usize = 8;

    pub fn worlds_organizing_max_scroll(&self) -> usize {
        self.worlds_rooms
            .len()
            .saturating_sub(Self::WORLDS_ORGANIZING_VISIBLE_ROWS)
    }

    /// Active catalog max scroll for the current MainHub tab.
    pub fn worlds_active_catalog_max_scroll(&self) -> usize {
        if self.main_hub_tab == MainHubTab::Organizing && self.server_operator {
            self.worlds_organizing_max_scroll()
        } else {
            self.worlds_catalog_max_scroll(self.worlds_catalog_list_h())
        }
    }

    pub fn clamp_worlds_catalog_scroll(&mut self) {
        let max = self.worlds_active_catalog_max_scroll();
        if self.worlds_catalog_scroll > max {
            self.worlds_catalog_scroll = max;
        }
    }

    /// Scroll catalog by room rows (`delta_rows` > 0 → later rooms).
    pub fn scroll_worlds_catalog_by(&mut self, delta_rows: i32) {
        let max = self.worlds_active_catalog_max_scroll() as i32;
        let next = (self.worlds_catalog_scroll as i32 + delta_rows).clamp(0, max);
        self.worlds_catalog_scroll = next as usize;
    }

    /// Accumulate wheel/trackpad pixels; step one row per ~40px.
    pub fn scroll_worlds_catalog_pixels(&mut self, delta_y: f32) -> bool {
        const STEP_PX: f32 = 40.0;
        self.worlds_catalog_scroll_accum += delta_y;
        let mut stepped = false;
        while self.worlds_catalog_scroll_accum >= STEP_PX {
            self.worlds_catalog_scroll_accum -= STEP_PX;
            let before = self.worlds_catalog_scroll;
            self.scroll_worlds_catalog_by(1);
            stepped |= self.worlds_catalog_scroll != before;
            if self.worlds_catalog_scroll == before {
                self.worlds_catalog_scroll_accum = 0.0;
                break;
            }
        }
        while self.worlds_catalog_scroll_accum <= -STEP_PX {
            self.worlds_catalog_scroll_accum += STEP_PX;
            let before = self.worlds_catalog_scroll;
            self.scroll_worlds_catalog_by(-1);
            stepped |= self.worlds_catalog_scroll != before;
            if self.worlds_catalog_scroll == before {
                self.worlds_catalog_scroll_accum = 0.0;
                break;
            }
        }
        stepped
    }
}

/// Local content height for Settings (from panel top). Derived from layout offsets —
/// includes optional in-session status banner under Server Operator.
fn settings_content_height(state: &DashState) -> f32 {
    let so = 240.0;
    let adv = if state.session_shell_active() && !state.home_status.is_empty() {
        so + 152.0
    } else {
        so + 124.0
    };
    let btn = adv + 108.0;
    let xray = if state.epoch_xray_text.is_empty() {
        0.0
    } else {
        120.0
    };
    // join/start row + status label + optional X-Ray + bottom pad
    btn + 48.0 + 18.0 + xray + 12.0
}

/// In-room session shell only (x007c). Catalog lives on MainHub — no Worlds tab body.
fn rebuild_session_shell(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    cy0: f32,
    cw: f32,
    _ch: f32,
) {
    tree.push_label(Rect::new(cx, cy0, cw, 22.0), "Session", TEXT, 2.0);
    tree.push_label(
        Rect::new(cx, cy0 + 24.0, cw, 16.0),
        format!("room {}", state.home_room_id),
        MUTED,
        1.0,
    );

    let mut y = cy0 + 52.0;
    let btn_h = 36.0;
    let gap = 10.0;

    if state.worlds_key_stale {
        tree.push_label(
            Rect::new(cx, y, cw, 18.0),
            "Room updated — reconnect required (key rotated).",
            Color::rgba(0.95, 0.55, 0.35, 1.0),
            1.1,
        );
        y += 26.0;
        let rejoin = Rect::new(cx, y, 160.0, btn_h);
        tree.push_rect(rejoin, state.interact(WidgetId::WorldsRejoin, ACCENT));
        tree.push_label(
            Rect::new(rejoin.x + 12.0, rejoin.y + 10.0, 136.0, 16.0),
            "Rejoin room",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsRejoin, rejoin);
        let leave = Rect::new(cx + 172.0, y, 160.0, btn_h);
        tree.push_rect(leave, state.interact(WidgetId::WorldsLeave, BTN_DANGER));
        tree.push_label(
            Rect::new(leave.x + 12.0, leave.y + 10.0, 136.0, 16.0),
            "Leave session",
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
            Rect::new(cx, y, cw, 36.0),
            wait_msg,
            Color::rgba(0.95, 0.75, 0.35, 1.0),
            1.1,
        );
        y += 40.0;
        let leave = Rect::new(cx, y, 160.0, btn_h);
        tree.push_rect(leave, state.interact(WidgetId::WorldsLeave, BTN_DANGER));
        tree.push_label(
            Rect::new(leave.x + 12.0, leave.y + 10.0, 136.0, 16.0),
            "Leave session",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsLeave, leave);
        let host_cta = state.worlds_can_host_rescue
            || state.worlds_can_takeover
            || state.worlds_nominated_host;
        if host_cta {
            if state.server_operator {
                let (label, widget) = if state.worlds_can_takeover && !state.worlds_can_host_rescue
                {
                    ("Take over host", WidgetId::WorldsTakeover)
                } else if state.worlds_nominated_host && !state.worlds_can_host_rescue {
                    ("Start as nominated host", WidgetId::WorldsHostRescue)
                } else {
                    ("Host this room now", WidgetId::WorldsHostRescue)
                };
                let rescue = Rect::new(cx + 172.0, y, 220.0, btn_h);
                tree.push_rect(rescue, state.interact(widget, ACCENT));
                tree.push_label(
                    Rect::new(rescue.x + 12.0, rescue.y + 10.0, 196.0, 16.0),
                    label,
                    TEXT,
                    1.2,
                );
                tree.register_hit(widget, rescue);
            } else {
                // Avoid Settings deadlock: enable here (or via Settings tab while in-room).
                let enable = Rect::new(cx + 172.0, y, 260.0, btn_h);
                tree.push_rect(
                    enable,
                    state.interact(WidgetId::ServerOperatorToggle, ACCENT),
                );
                tree.push_label(
                    Rect::new(enable.x + 10.0, enable.y + 10.0, 240.0, 16.0),
                    "Enable Server Operator",
                    TEXT,
                    1.2,
                );
                tree.register_hit(WidgetId::ServerOperatorToggle, enable);
            }
        }
        y += btn_h + gap;
        if host_cta && !state.server_operator {
            tree.push_label(
                Rect::new(cx, y, cw, 32.0),
                "Hosts on this PC (CPU/GPU load). Not recommended on weak machines. Or open Settings.",
                MUTED,
                1.0,
            );
            y += 36.0;
        }
    } else if state.worlds_in_room {
        tree.push_label(
            Rect::new(cx, y, cw, 18.0),
            "Live — space is running.",
            Color::rgba(0.55, 0.85, 0.55, 1.0),
            1.1,
        );
        y += 26.0;
        let leave = Rect::new(cx, y, 160.0, btn_h);
        tree.push_rect(leave, state.interact(WidgetId::WorldsLeave, BTN_DANGER));
        tree.push_label(
            Rect::new(leave.x + 12.0, leave.y + 10.0, 136.0, 16.0),
            "Leave session",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::WorldsLeave, leave);
        y += btn_h + gap;
    }

    if !state.worlds_status.is_empty() {
        tree.push_label(Rect::new(cx, y, cw, 20.0), &state.worlds_status, MUTED, 1.0);
    }
}

/// Modal covering Create/Join/Host until first host frame (or Cancel).
fn rebuild_session_enter_overlay(tree: &mut LayoutTree, state: &DashState, w: f32, h: f32) {
    // Dim + hit sink so hub/create controls cannot fire underneath.
    tree.push_rect(
        Rect::new(0.0, 0.0, w, h),
        Color::rgba(0.02, 0.03, 0.05, 0.88),
    );
    tree.register_hit(WidgetId::SessionEnterBlocker, Rect::new(0.0, 0.0, w, h));

    let pw = (w * 0.62).clamp(360.0, 640.0);
    let ph = (h * 0.55).clamp(260.0, 420.0);
    let px = (w - pw) * 0.5;
    let py = (h - ph) * 0.42;
    let panel = Rect::new(px, py, pw, ph);
    tree.push_rect(panel, Color::rgba(0.11, 0.12, 0.16, 1.0));

    let pad = 20.0;
    tree.push_label(
        Rect::new(px + pad, py + 16.0, pw - pad * 2.0, 24.0),
        if state.session_enter_title.is_empty() {
            "Entering world…"
        } else {
            state.session_enter_title.as_str()
        },
        TEXT,
        1.6,
    );
    tree.push_label(
        Rect::new(px + pad, py + 44.0, pw - pad * 2.0, 16.0),
        format!("room {}", state.home_room_id),
        MUTED,
        1.0,
    );

    let track = Rect::new(px + pad, py + 78.0, pw - pad * 2.0, 14.0);
    tree.push_rect(track, Color::rgba(0.08, 0.09, 0.12, 1.0));
    let progress = state.session_enter_progress.clamp(0.0, 1.0);
    if progress > 0.0 {
        let fill_w = (track.w * progress).max(2.0);
        tree.push_rect(
            Rect::new(track.x, track.y, fill_w, track.h),
            Color::rgba(0.40, 0.55, 0.95, 1.0),
        );
    }
    tree.push_label(
        Rect::new(px + pad, py + 98.0, pw - pad * 2.0, 14.0),
        format!(
            "{:.0}%",
            state.session_enter_progress.clamp(0.0, 1.0) * 100.0
        ),
        MUTED,
        0.95,
    );

    let log_top = py + 122.0;
    let log_h = ph - 122.0 - 56.0;
    tree.push_rect(
        Rect::new(px + pad, log_top, pw - pad * 2.0, log_h),
        Color::rgba(0.07, 0.08, 0.10, 1.0),
    );
    let mut ly = log_top + 8.0;
    let line_h = 16.0;
    let max_lines = ((log_h - 12.0) / line_h).floor() as usize;
    let start = state
        .session_enter_logs
        .len()
        .saturating_sub(max_lines.max(1));
    for line in state.session_enter_logs.iter().skip(start) {
        tree.push_label(
            Rect::new(px + pad + 8.0, ly, pw - pad * 2.0 - 16.0, line_h),
            line,
            MUTED,
            0.95,
        );
        ly += line_h;
    }

    let cancel = Rect::new(px + pad, py + ph - 44.0, 140.0, 32.0);
    tree.push_rect(
        cancel,
        state.interact(WidgetId::SessionEnterCancel, BTN_DANGER),
    );
    tree.push_label(
        Rect::new(cancel.x + 18.0, cancel.y + 8.0, 104.0, 16.0),
        "Cancel",
        TEXT,
        1.1,
    );
    tree.register_hit(WidgetId::SessionEnterCancel, cancel);
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
            "Live only. Drive→Take out (blob/prefab). Scene multi-select→Put back / Despawn (first)."
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
        let selected = state.scene_selected.contains(&i);
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
        "LOGIN & REGISTRATION"
    } else {
        "Home"
    };
    let title_w = if step2 { cw - 48.0 - 100.0 } else { cw - 48.0 };
    tree.push_label(
        Rect::new(cx + 24.0, cy0 + 16.0, title_w.max(120.0), 24.0),
        title,
        TEXT,
        2.5,
    );
    if step2 {
        let back = Rect::new(cx + cw - 24.0 - 88.0, cy0 + 12.0, 88.0, 32.0);
        tree.push_rect(back, state.interact(WidgetId::Step2Back, BTN));
        tree.push_label(
            Rect::new(back.x + 22.0, back.y + 8.0, 48.0, 18.0),
            "Back",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::Step2Back, back);
    }
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

        // Join/Host catalog is MainHub; Home keeps create shortcut (Logout is chrome-only).
        let hub = Rect::new(x, y, 240.0, 48.0);
        tree.push_rect(hub, state.interact(WidgetId::HomeOpenWorlds, ACCENT));
        tree.push_label(
            Rect::new(x + 16.0, y + 14.0, 208.0, 22.0),
            "Open Main hub",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeOpenWorlds, hub);

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
            "Join/Host from Main hub. Fixed ROOM_ID Join/Start → Settings (advanced).",
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
        // Logout lives in chrome (`ChromeLogout`) — avoid a second HomeLogout CTA.
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
            HomeMode::Login => "Start",
            HomeMode::Register => "Create & Start",
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

    if !step2 {
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
            "room_id={} (minted for this Create; DemoScene on first Start)",
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
    tree.push_label(
        Rect::new(cx, cy0, cw, 28.0),
        "SELECT SERVER OPERATOR",
        TEXT,
        2.5,
    );

    let gap = 16.0;
    let card_w = ((cw - gap * 2.0) / 3.0).max(120.0);
    let card_h = (ch - 100.0).clamp(180.0, 280.0);
    let card_y = cy0 + 48.0;
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

const MAIN_PUBLIC_ROW_IDS: [WidgetId; 8] = [
    WidgetId::MainPublicRow0,
    WidgetId::MainPublicRow1,
    WidgetId::MainPublicRow2,
    WidgetId::MainPublicRow3,
    WidgetId::MainPublicRow4,
    WidgetId::MainPublicRow5,
    WidgetId::MainPublicRow6,
    WidgetId::MainPublicRow7,
];

const MAIN_MY_WORLD_ROW_IDS: [WidgetId; 8] = [
    WidgetId::MainMyWorldRow0,
    WidgetId::MainMyWorldRow1,
    WidgetId::MainMyWorldRow2,
    WidgetId::MainMyWorldRow3,
    WidgetId::MainMyWorldRow4,
    WidgetId::MainMyWorldRow5,
    WidgetId::MainMyWorldRow6,
    WidgetId::MainMyWorldRow7,
];

fn rebuild_main_hub(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    tree.push_label(Rect::new(cx, cy0, cw, 18.0), &state.home_status, MUTED, 1.0);

    let tab_y = cy0 + 28.0;
    let tab_h = 36.0;
    let tab_w = (cw * 0.5 - 6.0).max(120.0);
    let part = Rect::new(cx, tab_y, tab_w, tab_h);
    let org = Rect::new(cx + tab_w + 12.0, tab_y, tab_w, tab_h);
    let part_c = if state.main_hub_tab == MainHubTab::Participation {
        ACCENT
    } else {
        BTN
    };
    let org_c = if !state.server_operator {
        Color::rgba(0.22, 0.23, 0.26, 1.0)
    } else if state.main_hub_tab == MainHubTab::Organizing {
        ACCENT
    } else {
        BTN
    };
    tree.push_rect(part, state.interact(WidgetId::MainTabParticipation, part_c));
    tree.push_label(
        Rect::new(part.x + 12.0, part.y + 10.0, part.w - 24.0, 16.0),
        "Participation",
        TEXT,
        1.3,
    );
    tree.register_hit(WidgetId::MainTabParticipation, part);
    tree.push_rect(
        org,
        if state.server_operator {
            state.interact(WidgetId::MainTabOrganizing, org_c)
        } else {
            org_c
        },
    );
    tree.push_label(
        Rect::new(org.x + 12.0, org.y + 10.0, org.w - 24.0, 16.0),
        if state.server_operator {
            "Organizing"
        } else {
            "Organizing (Settings)"
        },
        if state.server_operator { TEXT } else { MUTED },
        1.3,
    );
    // Always hittable so off-state can show the Settings tip.
    tree.register_hit(WidgetId::MainTabOrganizing, org);

    let body_y = tab_y + tab_h + 16.0;
    let body_h = (ch - (body_y - cy0)).max(80.0);
    // Gate body even if chrome sync / create-back left Organizing while flag is OFF.
    let hub_tab = if state.server_operator {
        state.main_hub_tab
    } else {
        MainHubTab::Participation
    };
    match hub_tab {
        MainHubTab::Participation => {
            rebuild_main_hub_participation(tree, state, cx, body_y, cw, body_h);
        }
        MainHubTab::Organizing => {
            rebuild_main_hub_organizing(tree, state, cx, body_y, cw, body_h);
        }
    }
}

fn rebuild_main_hub_participation(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    y0: f32,
    cw: f32,
    _ch: f32,
) {
    let mut y = y0;
    tree.push_label(
        Rect::new(cx, y, cw, 18.0),
        "Friends & Following — Active sessions",
        TEXT,
        1.4,
    );
    y += 24.0;
    // Compact stub so Public sessions keeps more rows at 960×540.
    tree.push_rect(
        Rect::new(cx, y, cw, 32.0),
        Color::rgba(0.10, 0.11, 0.14, 1.0),
    );
    tree.push_label(
        Rect::new(cx + 12.0, y + 8.0, cw - 24.0, 16.0),
        "Friends coming soon.",
        MUTED,
        1.0,
    );
    y += 40.0;

    tree.push_label(
        Rect::new(cx, y, cw * 0.55, 18.0),
        "Public sessions",
        TEXT,
        1.4,
    );
    let refresh = Rect::new(cx + cw - 100.0, y - 4.0, 100.0, 28.0);
    tree.push_rect(refresh, state.interact(WidgetId::MainHubRefresh, BTN));
    tree.push_label(
        Rect::new(refresh.x + 16.0, refresh.y + 6.0, 68.0, 16.0),
        "Refresh",
        TEXT,
        1.1,
    );
    tree.register_hit(WidgetId::MainHubRefresh, refresh);
    y += 28.0;

    let list_h = state.worlds_catalog_list_h();
    let list = Rect::new(cx, y, cw, list_h);
    tree.push_rect(list, Color::rgba(0.10, 0.11, 0.14, 1.0));

    // Catalog API already returns visible/joinable rooms only (x007c).
    let page_label;
    if state.worlds_rooms.is_empty() {
        page_label = None;
        let empty_hint = if state.server_operator {
            "No public sessions yet. Refresh, or create one under Organizing."
        } else {
            "No public sessions yet. Refresh, or enable Server Operator in Settings to create."
        };
        tree.push_label(
            Rect::new(cx + 12.0, y + 20.0, cw - 24.0, 40.0),
            empty_hint,
            MUTED,
            1.1,
        );
    } else {
        let row_h = 36.0;
        let gap = 4.0;
        let visible = DashState::worlds_catalog_visible_rows(list_h);
        let max_scroll = state.worlds_rooms.len().saturating_sub(visible);
        let start = state.worlds_catalog_scroll.min(max_scroll);
        let end = (start + visible).min(state.worlds_rooms.len());
        tree.clip = Some(list);
        for (vis_i, &id) in MAIN_PUBLIC_ROW_IDS.iter().enumerate().take(visible) {
            let room_i = start + vis_i;
            let Some((room_id, visibility, _role, _owner)) = state.worlds_rooms.get(room_i) else {
                break;
            };
            let ry = y + 8.0 + vis_i as f32 * (row_h + gap);
            let r = Rect::new(cx + 8.0, ry, cw - 16.0, row_h);
            let selected = state.worlds_selected.as_deref() == Some(room_id.as_str());
            let base = if selected { ACCENT } else { BTN };
            tree.push_rect(r, state.interact(id, base));
            tree.push_label(
                Rect::new(r.x + 8.0, r.y + 10.0, r.w - 16.0, 16.0),
                format!("{room_id} · {visibility}"),
                TEXT,
                1.1,
            );
            tree.register_hit(id, r);
        }
        tree.clip = None;
        page_label = if max_scroll > 0 {
            Some(format!(
                "{start}–{end}/{n}",
                start = start + 1,
                n = state.worlds_rooms.len()
            ))
        } else {
            None
        };
    }

    let join_y = y + list_h + 8.0;
    let join = Rect::new(cx, join_y, cw.min(280.0), 36.0);
    let join_enabled = state
        .worlds_selected
        .as_ref()
        .is_some_and(|sel| state.worlds_rooms.iter().any(|(id, ..)| id == sel));
    let join_c = if join_enabled { ACCENT } else { BTN };
    tree.push_rect(join, state.interact(WidgetId::MainHubJoin, join_c));
    tree.push_label(
        Rect::new(join.x + 12.0, join.y + 10.0, join.w - 24.0, 16.0),
        "Join",
        TEXT,
        1.3,
    );
    if join_enabled {
        tree.register_hit(WidgetId::MainHubJoin, join);
    }
    if let Some(page) = page_label.as_ref() {
        let page_w = 88.0;
        tree.push_label(
            Rect::new(cx + cw - page_w, join_y + 10.0, page_w, 16.0),
            page,
            MUTED,
            0.9,
        );
    }
    if !state.worlds_status.is_empty() {
        let status_w = if page_label.is_some() {
            (cw - join.w - 12.0 - 96.0).max(40.0)
        } else {
            (cw - join.w - 12.0).max(40.0)
        };
        tree.push_label(
            Rect::new(cx + join.w + 12.0, join_y + 10.0, status_w, 16.0),
            &state.worlds_status,
            MUTED,
            1.0,
        );
    }
}

fn rebuild_main_hub_organizing(
    tree: &mut LayoutTree,
    state: &DashState,
    cx: f32,
    y0: f32,
    cw: f32,
    _ch: f32,
) {
    let mut y = y0;
    tree.push_label(Rect::new(cx, y, cw, 18.0), "Popular worlds", TEXT, 1.4);
    y += 24.0;
    tree.push_rect(
        Rect::new(cx, y, cw, 56.0),
        Color::rgba(0.10, 0.11, 0.14, 1.0),
    );
    tree.push_label(
        Rect::new(cx + 12.0, y + 12.0, cw - 24.0, 16.0),
        "Popular catalog is not connected yet.",
        MUTED,
        1.1,
    );
    tree.push_label(
        Rect::new(cx + 12.0, y + 32.0, cw - 24.0, 16.0),
        "Coming soon — templates to host from will appear here.",
        MUTED,
        1.0,
    );
    y += 68.0;

    tree.push_label(Rect::new(cx, y, cw * 0.55, 18.0), "My worlds", TEXT, 1.4);
    let refresh = Rect::new(cx + cw - 100.0, y - 4.0, 100.0, 28.0);
    tree.push_rect(refresh, state.interact(WidgetId::MainHubRefresh, BTN));
    tree.push_label(
        Rect::new(refresh.x + 16.0, refresh.y + 6.0, 68.0, 16.0),
        "Refresh",
        TEXT,
        1.1,
    );
    tree.register_hit(WidgetId::MainHubRefresh, refresh);
    y += 28.0;

    let row_h = 32.0;
    let gap = 4.0;
    let visible = DashState::WORLDS_ORGANIZING_VISIBLE_ROWS;
    debug_assert_eq!(visible, MAIN_MY_WORLD_ROW_IDS.len());
    let max_scroll = state.worlds_rooms.len().saturating_sub(visible);
    let start = state.worlds_catalog_scroll.min(max_scroll);
    let shown = state.worlds_rooms.len().min(visible).max(1);
    let list_h = 8.0 + shown as f32 * (row_h + gap) + 4.0;
    let list = Rect::new(cx, y, cw, list_h);
    tree.push_rect(list, Color::rgba(0.10, 0.11, 0.14, 1.0));
    if state.worlds_rooms.is_empty() {
        tree.push_label(
            Rect::new(cx + 12.0, y + 20.0, cw - 24.0, 40.0),
            "No owned worlds yet. Create one below, or Refresh after Hosting.",
            MUTED,
            1.1,
        );
    } else {
        for (vis_i, &id) in MAIN_MY_WORLD_ROW_IDS.iter().enumerate().take(visible) {
            let room_i = start + vis_i;
            let Some((room_id, visibility, _role, _owner)) = state.worlds_rooms.get(room_i) else {
                break;
            };
            let ry = y + 8.0 + vis_i as f32 * (row_h + gap);
            let r = Rect::new(cx + 8.0, ry, cw - 16.0, row_h);
            let selected = state.worlds_selected.as_deref() == Some(room_id.as_str());
            let base = if selected { ACCENT } else { BTN };
            tree.push_rect(r, state.interact(id, base));
            tree.push_label(
                Rect::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, 16.0),
                format!("{room_id} · {visibility}"),
                TEXT,
                1.0,
            );
            tree.register_hit(id, r);
        }
        if max_scroll > 0 {
            tree.push_label(
                Rect::new(cx + 12.0, y + list_h - 18.0, cw - 24.0, 14.0),
                format!(
                    "{}–{} / {} (scroll)",
                    start + 1,
                    (start + visible).min(state.worlds_rooms.len()),
                    state.worlds_rooms.len()
                ),
                MUTED,
                0.9,
            );
        }
    }
    y += list_h + 8.0;

    let host = Rect::new(cx, y, cw.min(280.0), 36.0);
    // tuple: (room_id, visibility, role_hint, owner_sub)
    let host_enabled = state.worlds_selected.as_ref().is_some_and(|sel| {
        state
            .worlds_rooms
            .iter()
            .any(|(id, _, role, _)| id == sel && role == "owner")
    });
    let host_c = if host_enabled { ACCENT } else { BTN };
    tree.push_rect(host, state.interact(WidgetId::WorldsHostRoom, host_c));
    tree.push_label(
        Rect::new(host.x + 12.0, host.y + 10.0, host.w - 24.0, 16.0),
        "Host this room",
        TEXT,
        1.3,
    );
    if host_enabled {
        tree.register_hit(WidgetId::WorldsHostRoom, host);
    }
    y += 48.0;

    let create = Rect::new(cx, y, cw.min(320.0), 44.0);
    tree.push_rect(create, state.interact(WidgetId::MainHubCreateWorld, ACCENT));
    tree.push_label(
        Rect::new(create.x + 14.0, create.y + 8.0, create.w - 28.0, 16.0),
        "Create new world",
        TEXT,
        1.4,
    );
    tree.push_label(
        Rect::new(create.x + 14.0, create.y + 26.0, create.w - 28.0, 14.0),
        "Grid space and templates",
        MUTED,
        1.0,
    );
    tree.register_hit(WidgetId::MainHubCreateWorld, create);
}

pub fn rebuild(tree: &mut LayoutTree, state: &DashState) {
    tree.clear();
    if !state.open {
        return;
    }

    let w = state.canvas_w.max(1.0);
    let h = state.canvas_h.max(1.0);
    // Session shell must win over MainHub: do not treat hub as onboarding while in-room.
    let onboarding = !state.session_shell_active()
        && matches!(
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

    // Account affordance left of mic / volume (always visible while dash is open).
    if let Some(name) = state.home_logged_in_as.as_ref() {
        let name_w = (name.chars().count() as f32 * 9.0 + 8.0).clamp(48.0, 140.0);
        tree.push_label(Rect::new(x, 14.0, name_w, 18.0), name, TEXT, 1.2);
        x += name_w + 6.0;
        let logout_w = 88.0;
        let logout = Rect::new(x, 8.0, logout_w, TOP_H - 16.0);
        tree.push_rect(logout, state.interact(WidgetId::ChromeLogout, BTN_DANGER));
        tree.push_label(
            Rect::new(logout.x + 10.0, 14.0, logout_w - 20.0, 18.0),
            "Logout",
            TEXT,
            1.1,
        );
        tree.register_hit(WidgetId::ChromeLogout, logout);
        x += logout_w + 12.0;
    } else {
        let login_w = 128.0;
        let login = Rect::new(x, 8.0, login_w, TOP_H - 16.0);
        tree.push_rect(login, state.interact(WidgetId::ChromeLoginRegister, ACCENT));
        tree.push_label(
            Rect::new(login.x + 8.0, 14.0, login_w - 16.0, 18.0),
            "Login / Register",
            TEXT,
            1.0,
        );
        tree.register_hit(WidgetId::ChromeLoginRegister, login);
        x += login_w + 12.0;
    }

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

    // --- Center (drawn before bottom so tabs mask overflow) ---
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
    } else if state.session_shell_active() && matches!(state.tab, DashTab::Home | DashTab::Worlds) {
        // In-room shell on Home/Worlds only. Inventory is Live-unlocked (x007f) and
        // must render `rebuild_inventory` — not the session Leave/status panel.
        // Settings/Exit stay reachable (Server Operator / quit).
        rebuild_session_shell(tree, state, cx, cy0, cw, ch);
    } else {
        match state.tab {
            DashTab::Home => {
                rebuild_home(tree, state, cx, cy0, cw, ch, false);
            }
            // Legacy tab value after Join — treat as Home until Leave clears session.
            DashTab::Worlds => {
                rebuild_home(tree, state, cx, cy0, cw, ch, false);
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

    // Bottom bar last: paints over center overflow and wins hit tests in the strip.
    if show_bottom_tabs {
        tree.clip = None;
        tree.push_rect(Rect::new(0.0, by, w, BOTTOM_H), BAR);

        let tab_w = 76.0;
        let gap = 10.0;
        let tabs_total = tab_w * 4.0 + gap * 3.0;
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
            DashTab::Inventory,
            WidgetId::TabInventory,
            "inventory_2",
            FacetId::Inventory.label(),
            Color::rgba(0.16, 0.30, 0.26, 0.95),
        );
        draw_tab(
            tree,
            state,
            tab0_x + (tab_w + gap) * 2.0,
            DashTab::Settings,
            WidgetId::TabSettings,
            "settings",
            FacetId::Settings.label(),
            Color::rgba(0.20, 0.22, 0.28, 0.95),
        );
        draw_tab(
            tree,
            state,
            tab0_x + (tab_w + gap) * 3.0,
            DashTab::Exit,
            WidgetId::TabExit,
            "power",
            FacetId::Exit.label(),
            TAB_ON,
        );
    }

    // Enter modal last: covers hub/session chrome until first frame or Cancel.
    if state.session_enter_active {
        rebuild_session_enter_overlay(tree, state, w, h);
    }
}

fn rebuild_settings(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    let panel_h = ch;
    let panel = Rect::new(cx, cy0, cw, panel_h);
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));

    let content_h = settings_content_height(state);
    let max_scroll = (content_h - panel_h).max(0.0);
    let scroll = state.settings_scroll.clamp(0.0, max_scroll);
    let y0 = cy0 - scroll;

    // Primary overflow mask: scissor body to the Settings panel.
    tree.clip = Some(panel);

    tree.push_label(
        Rect::new(cx + 24.0, y0 + 16.0, cw - 48.0, 24.0),
        "Locomotion",
        TEXT,
        2.0,
    );
    tree.push_label(
        Rect::new(cx + 24.0, y0 + 48.0, cw - 48.0, 22.0),
        &state.max_speed_label,
        TEXT,
        1.5,
    );

    let toggle = Rect::new(cx + 24.0, y0 + 80.0, (cw - 48.0).min(420.0), 40.0);
    tree.push_rect(toggle, state.interact(WidgetId::TurnModeToggle, ACCENT));
    tree.push_label(
        Rect::new(toggle.x + 16.0, toggle.y + 10.0, toggle.w - 32.0, 22.0),
        &state.turn_mode_label,
        TEXT,
        1.4,
    );
    tree.register_hit(WidgetId::TurnModeToggle, toggle);

    tree.push_label(
        Rect::new(cx + 24.0, y0 + 132.0, cw - 48.0, 18.0),
        &state.snap_degrees_label,
        TEXT,
        1.2,
    );
    tree.push_label(
        Rect::new(cx + 24.0, y0 + 154.0, cw - 48.0, 18.0),
        &state.smooth_turn_label,
        TEXT,
        1.2,
    );

    let hs = Rect::new(cx + 24.0, y0 + 184.0, (cw - 48.0).min(520.0), 40.0);
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

    // Server Operator: local HostSupervisor / Organizing (distinct from Step1 OperatorKind).
    let so_y = y0 + 240.0;
    tree.push_label(
        Rect::new(cx + 24.0, so_y, cw - 48.0, 22.0),
        "Server Operator",
        TEXT,
        1.8,
    );
    tree.push_label(
        Rect::new(cx + 24.0, so_y + 26.0, cw - 48.0, 36.0),
        "Hosts a world on this PC (CPU/GPU load). Not recommended on weak machines.",
        MUTED,
        1.0,
    );
    let so = Rect::new(cx + 24.0, so_y + 68.0, (cw - 48.0).min(520.0), 40.0);
    tree.push_rect(so, state.interact(WidgetId::ServerOperatorToggle, ACCENT));
    tree.push_label(
        Rect::new(so.x + 16.0, so.y + 10.0, so.w - 32.0, 22.0),
        if state.server_operator {
            "Server Operator: ON — Organizing enabled"
        } else {
            "Server Operator: OFF — join only"
        },
        TEXT,
        1.4,
    );
    tree.register_hit(WidgetId::ServerOperatorToggle, so);
    let mut adv_y = so_y + 124.0;
    // Visible while Settings is open during an in-room session (worlds_status is Home-only).
    if state.session_shell_active() && !state.home_status.is_empty() {
        tree.push_label(
            Rect::new(cx + 24.0, so_y + 116.0, cw - 48.0, 28.0),
            &state.home_status,
            Color::rgba(0.95, 0.75, 0.35, 1.0),
            1.1,
        );
        adv_y = so_y + 152.0;
    }

    if !state.epoch_xray_text.is_empty() {
        tree.push_label(
            Rect::new(cx + 24.0, adv_y, cw - 48.0, 22.0),
            "Tape X-Ray",
            TEXT,
            1.8,
        );
        tree.push_label(
            Rect::new(cx + 24.0, adv_y + 26.0, cw - 48.0, 88.0),
            &state.epoch_xray_text,
            MUTED,
            0.95,
        );
        adv_y += 120.0;
    }

    // Advanced: fixed ROOM_ID Join/Start (prefer Worlds for normal use · x007c Phase 4).
    tree.push_label(
        Rect::new(cx + 24.0, adv_y, cw - 48.0, 22.0),
        "Advanced room (debug)",
        TEXT,
        1.8,
    );
    tree.push_label(
        Rect::new(cx + 24.0, adv_y + 26.0, cw - 48.0, 28.0),
        "Prefer Main hub for catalog Join/Host. These use a fixed ROOM_ID (env retarget).",
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
    let advanced_session_ok = state.home_logged_in_as.is_some()
        && state.home_session_actions_enabled
        && !state.session_shell_active();
    if advanced_session_ok {
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
        let (join_l, start_l) = if state.session_shell_active() {
            ("Join (Leave first)", "Start (Leave first)")
        } else if state.home_logged_in_as.is_none() {
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

    tree.clip = None;

    // Scroll affordance (wheel on Desktop; buttons for XR / no-wheel).
    if max_scroll > 1.0 {
        let up = Rect::new(cx + cw - 44.0, cy0 + 8.0, 36.0, 28.0);
        let down = Rect::new(cx + cw - 44.0, cy0 + panel_h - 36.0, 36.0, 28.0);
        tree.push_rect(up, state.interact(WidgetId::SettingsScrollUp, BTN));
        tree.push_label(
            Rect::new(up.x + 10.0, up.y + 6.0, 16.0, 16.0),
            "^",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::SettingsScrollUp, up);
        tree.push_rect(down, state.interact(WidgetId::SettingsScrollDown, BTN));
        tree.push_label(
            Rect::new(down.x + 10.0, down.y + 6.0, 16.0, 16.0),
            "v",
            TEXT,
            1.2,
        );
        tree.register_hit(WidgetId::SettingsScrollDown, down);
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
        let mut state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            ..Default::default()
        };
        state.settings_scroll = state.settings_max_scroll();
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
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsScrollUp));
    }

    #[test]
    fn settings_advanced_omits_join_start_hits_when_desktop_actions_disabled() {
        let mut tree = LayoutTree::default();
        let mut state = DashState {
            open: true,
            shell: OnboardingShell::Dash,
            tab: DashTab::Settings,
            canvas_w: 960.0,
            canvas_h: 540.0,
            home_logged_in_as: Some("alice".into()),
            home_session_actions_enabled: false,
            ..Default::default()
        };
        state.settings_scroll = state.settings_max_scroll();
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
