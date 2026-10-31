//! Official default Facet set layout (x000f + x0018 Home + x0037 create + x0066 Connect).
//!
//! Facets: Chrome (top) · Home · Connect · Settings · Exit (Pinned). Button colors react to hover/press.
//! Home / Connect hold form strings only — no auth/world/zenoh HTTP here.
//! Create panel visuals are placeholders (swap later).

use super::facet::{facet_of, DashRoot, FacetId};
use super::layout::{Color, LayoutTree, Rect, WidgetId};
use crate::access_level::AccessLevel;
use crate::turn::{TurnSettings, DEFAULT_SMOOTH_TURN_DEG_PER_SEC, DEFAULT_SNAP_DEGREES};
use alchemy_connect_config::{expand_simple_domain, ConnectProfile};

/// Session display-name cap (input + status; keep in sync with desktop join status).
pub const SESSION_NAME_MAX: usize = 48;
/// Login / register field cap (identifier, password, email, …).
const HOME_ACCOUNT_FIELD_MAX: usize = 64;
const CONNECT_FIELD_MAX: usize = 128;
/// Visible rows only — list length is unbounded (scroll · x0066).
const CONNECT_RELAY_UI_SLOTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashTab {
    /// Account shell (login / register / logout).
    Home,
    /// Home-node endpoints (x0066).
    Connect,
    /// Pinned Exit Facet body.
    Exit,
    /// Official Settings Facet body.
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectMode {
    Simple,
    Detail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectField {
    Domain,
    Auth,
    Assets,
    World,
    Relay(usize),
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
    /// When true, Home shows create-world placeholder panel (x0037).
    pub home_create_open: bool,
    pub home_session_name: String,
    pub home_access_level: AccessLevel,
    /// Zenoh / world `room_id` (from env); shown read-only on create panel.
    pub home_room_id: String,
    /// When false (XR), Start/Join hits are omitted — Desktop HTTP only (x0037).
    pub home_session_actions_enabled: bool,
    /// Connect tab draft (x0066) — Apply persists via surface.
    pub connect_mode: ConnectMode,
    pub connect_focus: Option<ConnectField>,
    pub connect_domain: String,
    pub connect_auth: String,
    pub connect_assets: String,
    pub connect_world: String,
    pub connect_relays: Vec<String>,
    pub connect_allocate: Vec<String>,
    pub connect_selected: usize,
    /// First visible relay index in the detail list (viewport of [`CONNECT_RELAY_UI_SLOTS`]).
    pub connect_relay_scroll: usize,
    pub connect_status: String,
    /// Last successfully applied world URL (allocate re-derive on change · x0066).
    pub connect_last_world: String,
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
            home_status: "Sign in to create or join a world".into(),
            home_logged_in_as: None,
            home_create_open: false,
            home_session_name: "FRICK World".into(),
            home_access_level: AccessLevel::Public,
            home_room_id: "local".into(),
            home_session_actions_enabled: true,
            connect_mode: ConnectMode::Simple,
            connect_focus: Some(ConnectField::Domain),
            connect_domain: String::new(),
            connect_auth: alchemy_connect_config::DEFAULT_AUTH_URL.into(),
            connect_assets: alchemy_connect_config::DEFAULT_ASSETS_URL.into(),
            connect_world: alchemy_connect_config::DEFAULT_WORLD_URL.into(),
            connect_relays: vec![alchemy_connect_config::DEFAULT_ZENOH_CONNECT.into()],
            connect_allocate: vec![String::new()],
            connect_selected: 0,
            connect_relay_scroll: 0,
            connect_status: "Home node — Apply before login".into(),
            connect_last_world: alchemy_connect_config::DEFAULT_WORLD_URL.into(),
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
            HomeField::SessionName => &mut self.home_session_name,
        };
        let max = match field {
            HomeField::SessionName => SESSION_NAME_MAX,
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
        // Logged-in hub: no account fields visible — ignore.
        if self.home_logged_in_as.is_some() && !self.home_create_open {
            return;
        }
        // Create panel: only session name is editable.
        if self.home_logged_in_as.is_some()
            && self.home_create_open
            && self.home_focus != Some(HomeField::SessionName)
        {
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
        };
        target.pop();
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

    pub fn load_connect_profile(&mut self, profile: &ConnectProfile) {
        let mut profile = profile.clone();
        profile.sanitize();
        self.connect_auth = profile.auth_url.clone();
        self.connect_assets = profile.assets_url.clone();
        self.connect_world = profile.world_url.clone();
        self.connect_last_world = profile.world_url.clone();
        self.connect_relays = profile.relays.clone();
        self.connect_allocate = profile.relay_allocate_urls.clone();
        self.connect_selected = profile
            .selected_relay
            .min(profile.relays.len().saturating_sub(1));
        self.ensure_relay_visible(self.connect_selected);
        if let Some(domain) = &profile.simple_domain {
            if profile.matches_simple_expand(domain) {
                self.connect_mode = ConnectMode::Simple;
                self.connect_domain = domain.clone();
                self.connect_focus = Some(ConnectField::Domain);
                return;
            }
        }
        self.connect_mode = ConnectMode::Detail;
        self.connect_domain.clear();
        // Keep Auth/Assets/World (and a still-valid Relay) after Apply; only default
        // to selected relay when focus was Domain / None / stale Relay (startup / mode flip).
        self.connect_focus = match self.connect_focus {
            Some(ConnectField::Auth | ConnectField::Assets | ConnectField::World) => {
                self.connect_focus
            }
            Some(ConnectField::Relay(i)) if i < self.connect_relays.len() => {
                Some(ConnectField::Relay(i))
            }
            Some(ConnectField::Relay(_)) => Some(ConnectField::Relay(self.connect_selected)),
            Some(ConnectField::Domain) | None => Some(ConnectField::Relay(self.connect_selected)),
        };
    }

    pub fn connect_draft_profile(&self) -> ConnectProfile {
        match self.connect_mode {
            ConnectMode::Simple => {
                let domain = self.connect_domain.trim();
                if domain.is_empty() {
                    ConnectProfile::default()
                } else {
                    expand_simple_domain(domain)
                }
            }
            ConnectMode::Detail => {
                let mut p = ConnectProfile {
                    version: alchemy_connect_config::PROFILE_VERSION,
                    simple_domain: None,
                    auth_url: self.connect_auth.clone(),
                    assets_url: self.connect_assets.clone(),
                    world_url: self.connect_world.clone(),
                    relays: self.connect_relays.clone(),
                    selected_relay: self.connect_selected,
                    relay_allocate_urls: self.connect_allocate.clone(),
                };
                p.sanitize();
                let domain = self.connect_domain.trim();
                if !domain.is_empty() && p.matches_simple_expand(domain) {
                    p.simple_domain = Some(domain.to_string());
                }
                p
            }
        }
    }

    pub fn set_connect_simple_mode(&mut self) {
        self.connect_mode = ConnectMode::Simple;
        self.connect_focus = Some(ConnectField::Domain);
        if self.connect_domain.trim().is_empty() {
            self.connect_status = "Enter example.tld then Apply".into();
        }
    }

    pub fn set_connect_detail_mode(&mut self) {
        if self.connect_mode == ConnectMode::Simple {
            let domain = self.connect_domain.trim();
            if !domain.is_empty() {
                let expanded = expand_simple_domain(domain);
                self.connect_auth = expanded.auth_url;
                self.connect_assets = expanded.assets_url;
                self.connect_world = expanded.world_url;
                self.connect_relays = expanded.relays;
                self.connect_allocate = expanded.relay_allocate_urls;
                self.connect_selected = 0;
            }
        }
        self.connect_mode = ConnectMode::Detail;
        self.connect_focus = Some(ConnectField::Auth);
    }

    pub fn append_connect_text(&mut self, text: &str) {
        let Some(field) = self.connect_focus else {
            return;
        };
        let target = self.connect_field_mut(field);
        for ch in text.chars() {
            if !ch.is_control() && target.chars().count() < CONNECT_FIELD_MAX {
                target.push(ch);
            }
        }
        self.after_connect_field_edit(field);
    }

    pub fn connect_backspace(&mut self) {
        let Some(field) = self.connect_focus else {
            return;
        };
        self.connect_field_mut(field).pop();
        self.after_connect_field_edit(field);
    }

    fn after_connect_field_edit(&mut self, field: ConnectField) {
        match field {
            ConnectField::Relay(i) => {
                while self.connect_allocate.len() <= i {
                    self.connect_allocate.push(String::new());
                }
                self.connect_allocate[i] =
                    alchemy_connect_config::allocate_hint_from_zenoh(&self.connect_relays[i]);
            }
            ConnectField::World
                if !self.connect_last_world.is_empty()
                    && self.connect_world.trim_end_matches('/') != self.connect_last_world =>
            {
                self.connect_status =
                    "World edited — Apply will re-derive relay allocate URLs".into();
            }
            _ => {}
        }
    }

    pub fn cycle_connect_focus(&mut self) {
        let fields: Vec<ConnectField> = match self.connect_mode {
            ConnectMode::Simple => vec![ConnectField::Domain],
            ConnectMode::Detail => {
                let mut v = vec![
                    ConnectField::Auth,
                    ConnectField::Assets,
                    ConnectField::World,
                ];
                self.clamp_relay_scroll();
                let start = self.connect_relay_scroll;
                let end = (start + CONNECT_RELAY_UI_SLOTS).min(self.connect_relays.len());
                for i in start..end {
                    v.push(ConnectField::Relay(i));
                }
                v
            }
        };
        let next = match self.connect_focus {
            None => fields.first().copied(),
            Some(cur) => {
                let i = fields.iter().position(|f| *f == cur).unwrap_or(0);
                Some(fields[(i + 1) % fields.len()])
            }
        };
        self.connect_focus = next;
    }

    pub fn add_connect_relay(&mut self) {
        self.connect_relays
            .push(alchemy_connect_config::DEFAULT_ZENOH_CONNECT.into());
        self.connect_allocate.push(String::new());
        let idx = self.connect_relays.len() - 1;
        self.connect_selected = idx;
        self.connect_focus = Some(ConnectField::Relay(idx));
        self.ensure_relay_visible(idx);
        self.connect_status =
            format!("Added relay #{idx} (priority list · top=preferred) — edit then Apply");
    }

    pub fn select_connect_relay(&mut self, idx: usize) {
        if idx < self.connect_relays.len() {
            self.connect_selected = idx;
            self.ensure_relay_visible(idx);
            self.connect_status = format!("Selected relay #{idx} (manual · auto/failover later)");
        }
    }

    pub fn scroll_relays_up(&mut self) {
        if self.connect_relay_scroll > 0 {
            self.connect_relay_scroll -= 1;
        }
    }

    pub fn scroll_relays_down(&mut self) {
        self.connect_relay_scroll = self.connect_relay_scroll.saturating_add(1);
        self.clamp_relay_scroll();
    }

    fn clamp_relay_scroll(&mut self) {
        let len = self.connect_relays.len();
        let max_scroll = len.saturating_sub(CONNECT_RELAY_UI_SLOTS);
        if self.connect_relay_scroll > max_scroll {
            self.connect_relay_scroll = max_scroll;
        }
    }

    fn ensure_relay_visible(&mut self, idx: usize) {
        if idx < self.connect_relay_scroll {
            self.connect_relay_scroll = idx;
        } else if idx >= self.connect_relay_scroll + CONNECT_RELAY_UI_SLOTS {
            self.connect_relay_scroll = idx + 1 - CONNECT_RELAY_UI_SLOTS;
        }
        self.clamp_relay_scroll();
    }

    /// Absolute relay index for visible viewport row `row` (0..CONNECT_RELAY_UI_SLOTS).
    pub fn connect_relay_abs(&self, row: usize) -> Option<usize> {
        let abs = self.connect_relay_scroll.saturating_add(row);
        (abs < self.connect_relays.len()).then_some(abs)
    }

    fn connect_field_mut(&mut self, field: ConnectField) -> &mut String {
        match field {
            ConnectField::Domain => &mut self.connect_domain,
            ConnectField::Auth => &mut self.connect_auth,
            ConnectField::Assets => &mut self.connect_assets,
            ConnectField::World => &mut self.connect_world,
            ConnectField::Relay(i) => {
                while self.connect_relays.len() <= i {
                    self.connect_relays
                        .push(alchemy_connect_config::DEFAULT_ZENOH_CONNECT.into());
                    self.connect_allocate.push(String::new());
                }
                &mut self.connect_relays[i]
            }
        }
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

fn rebuild_connect(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    let panel = Rect::new(cx, cy0, cw, ch.min(420.0));
    tree.push_rect(panel, Color::rgba(0.12, 0.13, 0.16, 1.0));
    tree.push_label(
        Rect::new(cx + 20.0, cy0 + 12.0, cw - 40.0, 22.0),
        "Connect (home node)",
        TEXT,
        2.0,
    );

    let mode_y = cy0 + 42.0;
    let mode_w = 100.0;
    let simple_r = Rect::new(cx + 20.0, mode_y, mode_w, 28.0);
    let detail_r = Rect::new(cx + 28.0 + mode_w, mode_y, mode_w, 28.0);
    let simple_base = if state.connect_mode == ConnectMode::Simple {
        ACCENT
    } else {
        BTN
    };
    let detail_base = if state.connect_mode == ConnectMode::Detail {
        ACCENT
    } else {
        BTN
    };
    tree.push_rect(
        simple_r,
        state.interact(WidgetId::ConnectModeSimple, simple_base),
    );
    tree.push_label(
        Rect::new(simple_r.x + 8.0, simple_r.y + 6.0, mode_w - 16.0, 18.0),
        "Simple",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::ConnectModeSimple, simple_r);
    tree.push_rect(
        detail_r,
        state.interact(WidgetId::ConnectModeDetail, detail_base),
    );
    tree.push_label(
        Rect::new(detail_r.x + 8.0, detail_r.y + 6.0, mode_w - 16.0, 18.0),
        "Detail",
        TEXT,
        1.2,
    );
    tree.register_hit(WidgetId::ConnectModeDetail, detail_r);

    let mut y = mode_y + 40.0;
    let field_h = 30.0;
    let label_w = 72.0;
    let input_x = cx + 20.0 + label_w;
    let input_w = (cw - 48.0 - label_w).max(120.0);

    let push_field = |tree: &mut LayoutTree,
                      state: &DashState,
                      y: f32,
                      label: &str,
                      value: &str,
                      field: ConnectField,
                      id: WidgetId| {
        tree.push_label(
            Rect::new(cx + 20.0, y + 6.0, label_w - 4.0, 18.0),
            label,
            MUTED,
            1.0,
        );
        let r = Rect::new(input_x, y, input_w, field_h);
        let focused = state.connect_focus == Some(field);
        tree.push_rect(r, if focused { FIELD_FOCUS } else { FIELD });
        let shown = if value.is_empty() {
            "…".to_string()
        } else {
            value.chars().take(48).collect()
        };
        tree.push_label(
            Rect::new(r.x + 8.0, r.y + 6.0, r.w - 16.0, 18.0),
            &shown,
            TEXT,
            1.1,
        );
        tree.register_hit(id, r);
    };

    match state.connect_mode {
        ConnectMode::Simple => {
            push_field(
                tree,
                state,
                y,
                "Domain",
                &state.connect_domain,
                ConnectField::Domain,
                WidgetId::ConnectFieldDomain,
            );
            y += field_h + 10.0;
            tree.push_label(
                Rect::new(cx + 20.0, y, cw - 40.0, 36.0),
                "expands to auth/assets/world + relay.<domain>",
                MUTED,
                1.0,
            );
            y += 40.0;
        }
        ConnectMode::Detail => {
            push_field(
                tree,
                state,
                y,
                "Auth",
                &state.connect_auth,
                ConnectField::Auth,
                WidgetId::ConnectFieldAuth,
            );
            y += field_h + 6.0;
            push_field(
                tree,
                state,
                y,
                "Assets",
                &state.connect_assets,
                ConnectField::Assets,
                WidgetId::ConnectFieldAssets,
            );
            y += field_h + 6.0;
            push_field(
                tree,
                state,
                y,
                "World",
                &state.connect_world,
                ConnectField::World,
                WidgetId::ConnectFieldWorld,
            );
            y += field_h + 10.0;
            tree.push_label(
                Rect::new(cx + 20.0, y, cw - 40.0, 18.0),
                "Relays (priority top→bottom · select one for now)",
                MUTED,
                1.0,
            );
            y += 22.0;
            let relay_ids = [
                WidgetId::ConnectRelay0,
                WidgetId::ConnectRelay1,
                WidgetId::ConnectRelay2,
                WidgetId::ConnectRelay3,
            ];
            let select_ids = [
                WidgetId::ConnectSelectRelay0,
                WidgetId::ConnectSelectRelay1,
                WidgetId::ConnectSelectRelay2,
                WidgetId::ConnectSelectRelay3,
            ];
            let scroll = state.connect_relay_scroll.min(
                state
                    .connect_relays
                    .len()
                    .saturating_sub(CONNECT_RELAY_UI_SLOTS),
            );
            let visible = state.connect_relays.len().saturating_sub(scroll);
            let n = visible.min(CONNECT_RELAY_UI_SLOTS);
            let relay_input_x = cx + 52.0;
            let relay_input_w = (cw - 100.0).max(120.0);
            for row in 0..n {
                let abs = scroll + row;
                let sel = Rect::new(cx + 20.0, y, 28.0, field_h);
                let sel_base = if state.connect_selected == abs {
                    ACCENT
                } else {
                    BTN
                };
                tree.push_rect(sel, state.interact(select_ids[row], sel_base));
                tree.push_label(
                    Rect::new(sel.x + 6.0, sel.y + 6.0, 20.0, 18.0),
                    if state.connect_selected == abs {
                        "●"
                    } else {
                        "○"
                    },
                    TEXT,
                    1.2,
                );
                tree.register_hit(select_ids[row], sel);
                let value = state
                    .connect_relays
                    .get(abs)
                    .map(String::as_str)
                    .unwrap_or("");
                let r = Rect::new(relay_input_x, y, relay_input_w, field_h);
                let focused = state.connect_focus == Some(ConnectField::Relay(abs));
                tree.push_rect(r, if focused { FIELD_FOCUS } else { FIELD });
                let shown: String = if value.is_empty() {
                    format!("#{abs} …")
                } else {
                    format!("#{abs} {}", value.chars().take(40).collect::<String>())
                };
                tree.push_label(
                    Rect::new(r.x + 8.0, r.y + 6.0, r.w - 16.0, 18.0),
                    shown,
                    TEXT,
                    1.1,
                );
                tree.register_hit(relay_ids[row], r);
                y += field_h + 4.0;
            }
            let add = Rect::new(cx + 20.0, y, 72.0, 28.0);
            tree.push_rect(add, state.interact(WidgetId::ConnectAddRelay, BTN));
            tree.push_label(
                Rect::new(add.x + 16.0, add.y + 6.0, 40.0, 18.0),
                "+",
                TEXT,
                1.5,
            );
            tree.register_hit(WidgetId::ConnectAddRelay, add);
            let can_up = scroll > 0;
            let can_down = scroll + CONNECT_RELAY_UI_SLOTS < state.connect_relays.len();
            if can_up {
                let up = Rect::new(cx + 100.0, y, 28.0, 28.0);
                tree.push_rect(up, state.interact(WidgetId::ConnectRelayScrollUp, BTN));
                tree.push_label(
                    Rect::new(up.x + 6.0, up.y + 6.0, 16.0, 16.0),
                    "▲",
                    TEXT,
                    1.2,
                );
                tree.register_hit(WidgetId::ConnectRelayScrollUp, up);
            }
            if can_down {
                let down = Rect::new(cx + 132.0, y, 28.0, 28.0);
                tree.push_rect(down, state.interact(WidgetId::ConnectRelayScrollDown, BTN));
                tree.push_label(
                    Rect::new(down.x + 6.0, down.y + 6.0, 16.0, 16.0),
                    "▼",
                    TEXT,
                    1.2,
                );
                tree.register_hit(WidgetId::ConnectRelayScrollDown, down);
            }
            tree.push_label(
                Rect::new(cx + 168.0, y + 6.0, cw - 188.0, 18.0),
                format!(
                    "{} relays · selected #{} (auto/failover later)",
                    state.connect_relays.len(),
                    state.connect_selected
                ),
                MUTED,
                1.0,
            );
            y += 36.0;
        }
    }

    let apply = Rect::new(cx + 20.0, y.min(cy0 + ch - 56.0), 120.0, 36.0);
    tree.push_rect(apply, state.interact(WidgetId::ConnectApply, ACCENT));
    tree.push_label(
        Rect::new(apply.x + 28.0, apply.y + 8.0, 80.0, 20.0),
        "Apply",
        TEXT,
        1.5,
    );
    tree.register_hit(WidgetId::ConnectApply, apply);
    tree.push_label(
        Rect::new(cx + 152.0, apply.y + 8.0, cw - 180.0, 40.0),
        &state.connect_status,
        MUTED,
        1.0,
    );
}

fn rebuild_home(tree: &mut LayoutTree, state: &DashState, cx: f32, cy0: f32, cw: f32, ch: f32) {
    if state.home_logged_in_as.is_some() && state.home_create_open {
        rebuild_home_create(tree, state, cx, cy0, cw, ch);
        return;
    }

    let panel = Rect::new(cx, cy0, cw, ch.min(420.0));
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

        let new_w = Rect::new(x, y, 240.0, 48.0);
        tree.push_rect(new_w, state.interact(WidgetId::HomeNewWorld, ACCENT));
        tree.push_label(
            Rect::new(x + 16.0, y + 14.0, 208.0, 22.0),
            "Create new world",
            TEXT,
            1.5,
        );
        tree.register_hit(WidgetId::HomeNewWorld, new_w);

        let join = Rect::new(x + 256.0, y, 180.0, 48.0);
        if state.home_session_actions_enabled {
            tree.push_rect(join, state.interact(WidgetId::HomeJoinRoom, BTN));
            tree.push_label(
                Rect::new(x + 272.0, y + 14.0, 148.0, 22.0),
                "Join room",
                TEXT,
                1.5,
            );
            tree.register_hit(WidgetId::HomeJoinRoom, join);
        } else {
            tree.push_rect(join, BTN);
            tree.push_label(
                Rect::new(x + 272.0, y + 14.0, 148.0, 22.0),
                "Join (Desktop)",
                MUTED,
                1.5,
            );
        }
        y += 64.0;
        if !state.home_session_actions_enabled {
            tree.push_label(
                Rect::new(x, y, fw, 16.0),
                "Start/Join HTTP is Desktop-only (XR can browse panel only)",
                MUTED,
                1.0,
            );
            y += 24.0;
        }

        tree.push_label(
            Rect::new(x, y, fw, 18.0),
            format!("room_id={} (ROOM_ID / Zenoh)", state.home_room_id),
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
        "Create new world (placeholder UI)",
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
        format!("room_id={} (fixed)", state.home_room_id),
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

    // --- Bottom bar: Home · Connect · Settings · Exit ---
    let by = h - BOTTOM_H;
    tree.push_rect(Rect::new(0.0, by, w, BOTTOM_H), BAR);

    let tab_w = 78.0;
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
        DashTab::Connect,
        WidgetId::TabConnect,
        "sync",
        FacetId::Connect.label(),
        Color::rgba(0.14, 0.32, 0.36, 0.95),
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
        DashTab::Connect => {
            rebuild_connect(tree, state, cx, cy0, cw, ch);
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
                "Host exit: ALCHEMY_HOST_PID (Unix) or ALCHEMY_HOST_CONTROL=1",
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
    fn logged_in_hub_has_new_world() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
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
            .any(|(id, _)| *id == WidgetId::HomeNewWorld));
        assert!(tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeJoinRoom));
    }

    #[test]
    fn create_panel_has_all_access_levels_and_start_at_default_canvas() {
        let mut tree = LayoutTree::default();
        let state = DashState {
            open: true,
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
