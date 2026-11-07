//! system_ui — personal DashRoot + retained Canvas (x000f + x0018 Home + x0037 create).
//!
//! Guarantees: ESC/Menu toggle (via surface), hit-tested chrome, Exit Pinned Facet,
//! onboarding shell (Step1 operator → Step2 auth → Main hub), Home account /
//! create-world form (Desktop text input), Button hover/press color, client exit /
//! auth / start-session action plumbing, Settings defaults (read-only).
//! Dash is personal — never published on Room `frame`.
//! x0053: logical [`SharedDashLogic`] is SharedState-owned; LayoutTree / UiGpu /
//! hover / password stay per-surface. Chrome/form push are field-split with
//! chrome CAS; `PresentationGate` is SoT for hotswitch auto/suppressed.
//! Non-guarantees: OpenXR types inside this module, user Facet rearrange, Vault layout
//! persist, WorldRoot, XR virtual keyboard, auth/world HTTP (surface modules), final UI art,
//! Federation directory / Join-a-host session list (future).

mod dash;
pub mod facet;
mod font;
mod gpu;
mod icons;
mod layout;

#[allow(unused_imports)] // public re-export; store.logic type for callers
pub use dash::SharedDashLogic;
pub use dash::{
    access_level_from_widget, DashState, DashTab, HomeField, HomeMode, OnboardingShell,
    SharedDashStore, SESSION_NAME_MAX,
};
pub use gpu::UiGpu;
pub use layout::{LayoutTree, WidgetId};

use std::sync::Mutex;

use crate::operator_prefs::OperatorKind;
use crate::scene::SharedState;
use dash::rebuild;

/// Owns dash state + retained layout tree.
pub struct SystemUi {
    pub state: DashState,
    pub tree: LayoutTree,
    pub gpu: Option<UiGpu>,
    /// Keeps wgpu device alive when canvas is used CPU-only (OpenXR path).
    _gpu_keepalive: Option<(wgpu::Device, wgpu::Queue)>,
    last_fps_instant: std::time::Instant,
    frames_in_window: u32,
    fps: f32,
    /// Last applied / published [`SharedDashStore::revision`] (form).
    dash_logic_rev: u64,
    /// Last applied / published [`SharedDashStore::chrome_revision`].
    dash_chrome_rev: u64,
    /// Local chrome edits not yet CAS-pushed.
    dash_chrome_dirty: bool,
    /// Local form edits not yet pushed.
    dash_form_dirty: bool,
}

impl SystemUi {
    pub fn new() -> Self {
        Self {
            state: DashState::default(),
            tree: LayoutTree::default(),
            gpu: None,
            _gpu_keepalive: None,
            last_fps_instant: std::time::Instant::now(),
            frames_in_window: 0,
            fps: 0.0,
            dash_logic_rev: 0,
            dash_chrome_rev: 0,
            dash_chrome_dirty: false,
            dash_form_dirty: false,
        }
    }

    /// Mark chrome (`open`/`tab`/`shell`/Settings) dirty.
    pub fn mark_dash_chrome_dirty(&mut self) {
        self.dash_chrome_dirty = true;
    }

    /// Mark form (Home / onboarding) dirty.
    pub fn mark_dash_form_dirty(&mut self) {
        self.dash_form_dirty = true;
    }

    /// Mark both chrome and form dirty (mixed shell+status updates).
    pub fn mark_dash_logic_dirty(&mut self) {
        self.dash_chrome_dirty = true;
        self.dash_form_dirty = true;
    }

    /// Pull from a dash store (testable without full [`SharedState`]).
    ///
    /// Chrome merges while form-dirty (Esc/Menu during typing). Chrome pull is
    /// skipped while chrome-dirty so a pending Esc is not wiped before CAS push.
    /// Form merges only when form-clean.
    pub fn pull_dash_store(&mut self, dash: &Mutex<SharedDashStore>) -> bool {
        let Ok(store) = dash.lock() else {
            return false;
        };
        let mut changed = false;
        if !self.dash_chrome_dirty && store.chrome_revision != self.dash_chrome_rev {
            changed |= store.logic.apply_chrome_to(&mut self.state);
            self.dash_chrome_rev = store.chrome_revision;
        }
        if !self.dash_form_dirty && store.revision != self.dash_logic_rev {
            changed |= store.logic.apply_form_to(&mut self.state);
            self.dash_logic_rev = store.revision;
        }
        drop(store);
        if changed {
            self.state.refresh_hotswitch_label();
            self.rebuild();
        }
        changed
    }

    pub fn pull_dash_logic(&mut self, shared: &SharedState) -> bool {
        self.pull_dash_store(&shared.dash)
    }

    /// Push dirty chrome/form separately. Chrome uses CAS on `chrome_revision`.
    pub fn push_dash_store(&mut self, dash: &Mutex<SharedDashStore>) {
        if !self.dash_chrome_dirty && !self.dash_form_dirty {
            return;
        }
        let Ok(mut store) = dash.lock() else {
            return;
        };
        let mut rebuild_after = false;

        if self.dash_form_dirty {
            if !store.logic.form_eq(&self.state) {
                store.logic.write_form_from(&self.state);
                store.revision = store.revision.saturating_add(1);
            }
            self.dash_logic_rev = store.revision;
            self.dash_form_dirty = false;
        }

        if self.dash_chrome_dirty {
            if store.chrome_revision == self.dash_chrome_rev {
                if !store.logic.chrome_eq(&self.state) {
                    store.logic.write_chrome_from(&self.state);
                    store.chrome_revision = store.chrome_revision.saturating_add(1);
                }
                self.dash_chrome_rev = store.chrome_revision;
                self.dash_chrome_dirty = false;
            } else {
                // Peer chrome advanced: adopt it; do not clobber with stale local.
                // Concurrent Esc/Menu: the side that lost the CAS drops its intent.
                log::info!(
                    "dash chrome CAS miss: local_rev={} store_rev={} — adopting peer chrome",
                    self.dash_chrome_rev,
                    store.chrome_revision
                );
                rebuild_after = store.logic.apply_chrome_to(&mut self.state);
                self.dash_chrome_rev = store.chrome_revision;
                self.dash_chrome_dirty = false;
            }
        }
        drop(store);
        if rebuild_after {
            self.state.refresh_hotswitch_label();
            self.rebuild();
        }
    }

    pub fn push_dash_logic(&mut self, shared: &SharedState) {
        self.push_dash_store(&shared.dash)
    }

    pub fn init_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.state.canvas_w = width as f32;
        self.state.canvas_h = height as f32;
        self.gpu = Some(UiGpu::new(device, queue, format, width, height)?);
        self.rebuild();
        Ok(())
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.state.canvas_w = width.max(1) as f32;
        self.state.canvas_h = height.max(1) as f32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(device, width, height);
        }
        self.rebuild();
    }

    pub fn toggle(&mut self) {
        self.state.open = !self.state.open;
        if !self.state.open {
            self.state.hover = None;
            self.state.pressed = None;
        }
        self.mark_dash_chrome_dirty();
        self.rebuild();
    }

    /// Pointer / laser UV in canvas pixels. `primary_down` = mouse LMB or XR Select.
    /// Surfaces pass coordinates only — this module does not know OpenXR / winit.
    pub fn on_pointer(&mut self, px: f32, py: f32, primary_down: bool) {
        if !self.state.open {
            if self.state.hover.is_some() || self.state.pressed.is_some() {
                self.state.hover = None;
                self.state.pressed = None;
                self.rebuild();
            }
            return;
        }
        let hit = self.tree.hit_test(px, py);
        let pressed = if primary_down { hit } else { None };
        if hit != self.state.hover || pressed != self.state.pressed {
            self.state.hover = hit;
            self.state.pressed = pressed;
            self.rebuild();
        }
    }

    pub fn clear_pointer(&mut self) {
        if self.state.hover.is_some() || self.state.pressed.is_some() {
            self.state.hover = None;
            self.state.pressed = None;
            if self.state.open {
                self.rebuild();
            }
        }
    }

    pub fn set_time_now(&mut self) {
        let now = chrono_lite_now();
        if self.state.time_label != now {
            self.state.time_label = now;
            if self.state.open {
                self.rebuild();
            }
        }
    }

    pub fn tick_fps(&mut self) {
        self.frames_in_window += 1;
        let elapsed = self.last_fps_instant.elapsed();
        if elapsed >= std::time::Duration::from_secs(1) {
            self.fps = self.frames_in_window as f32 / elapsed.as_secs_f32();
            self.frames_in_window = 0;
            self.last_fps_instant = std::time::Instant::now();
            let label = format!("{:.0} FPS", self.fps);
            if self.state.fps_label != label {
                self.state.fps_label = label;
                if self.state.open {
                    self.rebuild();
                }
            }
        }
    }

    pub fn rebuild(&mut self) {
        rebuild(&mut self.tree, &self.state);
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.mark_dirty();
        }
    }

    pub fn on_click(&mut self, px: f32, py: f32) -> UiAction {
        if !self.state.open {
            return UiAction::None;
        }
        // Hit against current tree (hover rebuild already applied this frame when wired).
        let Some(hit) = self.tree.hit_test(px, py) else {
            return UiAction::None;
        };
        if let Some(level) = access_level_from_widget(hit) {
            self.state.home_access_level = level;
            self.mark_dash_form_dirty();
            self.rebuild();
            return UiAction::None;
        }
        match hit {
            WidgetId::TabExit | WidgetId::CloseSession => {
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Exit;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabSettings => {
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Settings;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabWorlds => {
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Worlds;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::WorldsRefresh
            }
            WidgetId::TabInventory => {
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Inventory;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                // Desktop syncs Manifest scene mesh list without Drive HTTP (x006e).
                UiAction::InventoryOpen
            }
            WidgetId::WorldsRefresh => UiAction::WorldsRefresh,
            WidgetId::WorldsFilterAll => {
                self.state.worlds_mine_only = false;
                self.rebuild();
                UiAction::WorldsRefresh
            }
            WidgetId::WorldsFilterMine => {
                self.state.worlds_mine_only = true;
                self.rebuild();
                UiAction::WorldsRefresh
            }
            WidgetId::WorldsCreate | WidgetId::HomeNewWorld => {
                self.state.home_create_open = true;
                self.state.tab = DashTab::Home;
                self.state.shell = OnboardingShell::Dash;
                self.state.home_focus = Some(crate::system_ui::HomeField::SessionName);
                self.state.home_status = if self.state.home_session_actions_enabled {
                    "Pick access level, then Start session".into()
                } else {
                    "Create panel (browse only) — Start HTTP is Desktop-only".into()
                };
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeOpenWorlds => {
                self.clear_settings_room_focus();
                self.state.home_create_open = false;
                self.state.tab = DashTab::Worlds;
                self.state.shell = OnboardingShell::Dash;
                self.state.home_status = "Worlds catalog — Join as guest or Host this room".into();
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::WorldsJoinGuest => UiAction::WorldsJoinGuest,
            WidgetId::WorldsHostRoom => UiAction::WorldsHostRoom,
            WidgetId::WorldsHostRescue => UiAction::WorldsHostRescue,
            WidgetId::WorldsLeave => UiAction::WorldsLeave,
            WidgetId::WorldsRow0
            | WidgetId::WorldsRow1
            | WidgetId::WorldsRow2
            | WidgetId::WorldsRow3
            | WidgetId::WorldsRow4
            | WidgetId::WorldsRow5
            | WidgetId::WorldsRow6
            | WidgetId::WorldsRow7 => {
                let idx = match hit {
                    WidgetId::WorldsRow0 => 0,
                    WidgetId::WorldsRow1 => 1,
                    WidgetId::WorldsRow2 => 2,
                    WidgetId::WorldsRow3 => 3,
                    WidgetId::WorldsRow4 => 4,
                    WidgetId::WorldsRow5 => 5,
                    WidgetId::WorldsRow6 => 6,
                    WidgetId::WorldsRow7 => 7,
                    _ => 0,
                };
                if idx < self.state.worlds_rooms.len() {
                    // Preview only — do not retarget home_room_id / rescue / SharedState.
                    // Host Rescue uses the actually joined room (`shared.get_room_id`).
                    self.state.worlds_selected = Some(idx);
                    self.rebuild();
                }
                UiAction::None
            }
            WidgetId::InventoryRefresh => UiAction::InventoryRefresh,
            WidgetId::InventoryTakeOut => UiAction::InventoryTakeOut,
            WidgetId::InventoryPutBack => UiAction::InventoryPutBack,
            WidgetId::InventoryDespawn => UiAction::InventoryDespawn,
            WidgetId::InventoryRow0
            | WidgetId::InventoryRow1
            | WidgetId::InventoryRow2
            | WidgetId::InventoryRow3
            | WidgetId::InventoryRow4
            | WidgetId::InventoryRow5
            | WidgetId::InventoryRow6
            | WidgetId::InventoryRow7 => {
                let idx = match hit {
                    WidgetId::InventoryRow0 => 0,
                    WidgetId::InventoryRow1 => 1,
                    WidgetId::InventoryRow2 => 2,
                    WidgetId::InventoryRow3 => 3,
                    WidgetId::InventoryRow4 => 4,
                    WidgetId::InventoryRow5 => 5,
                    WidgetId::InventoryRow6 => 6,
                    WidgetId::InventoryRow7 => 7,
                    _ => 0,
                };
                if idx < self.state.inventory_nodes.len() {
                    self.state.inventory_selected = Some(idx);
                    self.state.scene_selected = None;
                    self.rebuild();
                }
                UiAction::None
            }
            WidgetId::SceneMeshRow0
            | WidgetId::SceneMeshRow1
            | WidgetId::SceneMeshRow2
            | WidgetId::SceneMeshRow3 => {
                let idx = match hit {
                    WidgetId::SceneMeshRow0 => 0,
                    WidgetId::SceneMeshRow1 => 1,
                    WidgetId::SceneMeshRow2 => 2,
                    WidgetId::SceneMeshRow3 => 3,
                    _ => 0,
                };
                if idx < self.state.scene_meshes.len() {
                    self.state.scene_selected = Some(idx);
                    self.state.inventory_selected = None;
                    self.rebuild();
                }
                UiAction::None
            }
            WidgetId::TabHome => {
                self.clear_settings_room_focus();
                if self.state.home_logged_in_as.is_some() && !self.state.home_create_open {
                    // Return to Join a host / Host a room hub when signed in.
                    self.state.shell = OnboardingShell::MainHub;
                    self.state.tab = DashTab::Home;
                } else {
                    self.state.shell = OnboardingShell::Dash;
                    self.state.tab = DashTab::Home;
                }
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorOfficial => {
                self.state.operator = OperatorKind::Official;
                self.state.custom_host_focus = false;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorFederation => {
                self.state.operator = OperatorKind::Federation;
                self.state.custom_host_focus = false;
                self.state.shell = OnboardingShell::Step1FederationStub;
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorCustom => {
                self.state.operator = OperatorKind::Custom;
                self.state.custom_host_focus = true;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorCustomHost => {
                self.state.operator = OperatorKind::Custom;
                self.state.custom_host_focus = true;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::Step1Next => {
                if self.state.operator == OperatorKind::Federation {
                    return UiAction::None;
                }
                // XR: home_session_actions_enabled=false — do not advance without applying endpoints.
                if !self.state.home_session_actions_enabled {
                    self.state.home_status =
                        "Operator select / login is Desktop-only (use Desktop client)".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    return UiAction::None;
                }
                self.state.custom_host_focus = false;
                self.state.shell = OnboardingShell::Step2Auth;
                self.state.tab = DashTab::Home;
                self.state.home_focus = Some(HomeField::Identifier);
                self.state.home_status = "Sign in to continue".into();
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::ApplyOperatorEndpoints
            }
            WidgetId::Step1Back => {
                // First-launch: no prior step. Keep Step1.
                UiAction::None
            }
            WidgetId::FederationStubBack => {
                self.state.shell = OnboardingShell::Step1Operator;
                self.state.operator = OperatorKind::Official;
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::Step2Back => {
                self.state.shell = OnboardingShell::Step1Operator;
                self.state.custom_host_focus = self.state.operator == OperatorKind::Custom;
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainJoinHost => {
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Worlds;
                self.state.home_create_open = false;
                self.state.home_status = "Join a host — pick a room, then Join as guest".into();
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainHostRoom => {
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Worlds;
                self.state.home_create_open = false;
                self.state.home_status =
                    "Host a room — Create world or Host this room on this PC".into();
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ExitClientOnly => UiAction::ExitClientOnly,
            WidgetId::ExitClientAndHost => UiAction::ExitClientAndHost,
            WidgetId::TurnModeToggle => {
                self.state.toggle_turn_mode();
                self.mark_dash_chrome_dirty();
                self.rebuild();
                log::info!("turn mode → {}", self.state.turn.mode.label());
                UiAction::None
            }
            WidgetId::HotswitchAutoToggle => {
                // PresentationGate is SoT; surface writes Gate from UiAction.
                self.state.toggle_hotswitch_auto();
                self.rebuild();
                log::info!(
                    "hotswitch auto → {}",
                    if self.state.hotswitch_auto {
                        "ON"
                    } else {
                        "OFF"
                    }
                );
                UiAction::HotswitchAutoChanged
            }
            WidgetId::HomeFieldIdentifier => {
                self.state.home_focus = Some(HomeField::Identifier);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldPassword => {
                self.state.home_focus = Some(HomeField::Password);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldUsername => {
                self.state.home_focus = Some(HomeField::Username);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldEmail => {
                self.state.home_focus = Some(HomeField::Email);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldBirthday => {
                self.state.home_focus = Some(HomeField::Birthday);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeRememberMe => {
                self.state.home_remember_me = !self.state.home_remember_me;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeTosAgreed => {
                self.state.home_tos_agreed = !self.state.home_tos_agreed;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeModeLogin => {
                self.state.home_mode = HomeMode::Login;
                self.state.home_focus = Some(HomeField::Identifier);
                // Do not carry Register checkbox across mode switches.
                self.state.home_tos_agreed = false;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeModeRegister => {
                self.state.home_mode = HomeMode::Register;
                self.state.home_focus = Some(HomeField::Username);
                self.state.home_tos_agreed = false;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeSubmit => match self.state.home_mode {
                HomeMode::Login => UiAction::AuthLogin,
                HomeMode::Register => UiAction::AuthRegister,
            },
            WidgetId::HomeLogout => {
                self.state.home_create_open = false;
                self.state.shell = OnboardingShell::Step2Auth;
                self.mark_dash_logic_dirty();
                UiAction::AuthLogout
            }
            WidgetId::HomeCreateBack => {
                self.state.home_create_open = false;
                self.state.home_focus = None;
                self.state.tab = DashTab::Worlds;
                self.state.home_status =
                    "Worlds — create cancelled; pick a room or Create world".into();
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeCreateName => {
                self.state.home_focus = Some(HomeField::SessionName);
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeTemplateGrid => UiAction::None,
            WidgetId::HomeStartSession | WidgetId::SettingsStartSession => {
                if self.state.home_session_actions_enabled {
                    UiAction::StartSession
                } else {
                    self.state.home_status = "Start session HTTP is Desktop-only this Slice".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                }
            }
            WidgetId::HomeJoinRoom | WidgetId::SettingsJoinRoom => {
                if self.state.home_session_actions_enabled {
                    UiAction::JoinRoom
                } else {
                    self.state.home_status = "Join room HTTP is Desktop-only this Slice".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                }
            }
            WidgetId::SettingsRoomId => {
                if self.state.home_logged_in_as.is_some() {
                    self.state.home_focus = Some(HomeField::RoomId);
                } else {
                    self.clear_settings_room_focus();
                }
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            // Access-level widgets handled above via `access_level_from_widget`.
            WidgetId::HomeAccessPrivate
            | WidgetId::HomeAccessLan
            | WidgetId::HomeAccessFriends
            | WidgetId::HomeAccessFriendsPlus
            | WidgetId::HomeAccessFollowers
            | WidgetId::HomeAccessFollowersPlus
            | WidgetId::HomeAccessPublic => UiAction::None,
            WidgetId::Mic
            | WidgetId::MicOff
            | WidgetId::Volume
            | WidgetId::Hearing
            | WidgetId::SyncBadge
            | WidgetId::SettingsPlaceholder => {
                log::info!("system_ui stub click");
                UiAction::None
            }
        }
    }

    fn clear_settings_room_focus(&mut self) {
        if self.state.home_focus == Some(HomeField::RoomId) {
            self.state.home_focus = None;
            self.mark_dash_form_dirty();
        }
    }

    /// Settings Advanced room_id field is editable only while that tab is active.
    fn allows_settings_room_edit(&self) -> bool {
        self.state.shell == OnboardingShell::Dash
            && self.state.tab == DashTab::Settings
            && self.state.home_focus == Some(HomeField::RoomId)
    }

    /// Desktop text into focused Home / onboarding field (ignored when dash closed).
    pub fn on_text_input(&mut self, text: &str) {
        if !self.state.open {
            return;
        }
        if self.state.shell == OnboardingShell::Step1Operator {
            self.state.append_home_text(text);
            self.mark_dash_form_dirty();
            self.rebuild();
            return;
        }
        let settings_room = self.allows_settings_room_edit();
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash
            && self.state.tab != DashTab::Home
            && !settings_room)
        {
            return;
        }
        // Logged-in hub: no editable fields. Create: session name. Settings: room_id.
        if self.state.home_logged_in_as.is_some() && !self.state.home_create_open && !settings_room
        {
            return;
        }
        if self.state.home_logged_in_as.is_some()
            && self.state.home_create_open
            && self.state.home_focus != Some(HomeField::SessionName)
        {
            return;
        }
        self.state.append_home_text(text);
        self.mark_dash_form_dirty();
        self.rebuild();
    }

    pub fn on_home_backspace(&mut self) {
        if !self.state.open {
            return;
        }
        if self.state.shell == OnboardingShell::Step1Operator {
            self.state.home_backspace();
            self.mark_dash_form_dirty();
            self.rebuild();
            return;
        }
        let settings_room = self.allows_settings_room_edit();
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash
            && self.state.tab != DashTab::Home
            && !settings_room)
        {
            return;
        }
        // RoomId must not be edited from Home (invisible) even if focus leaked.
        if self.state.home_focus == Some(HomeField::RoomId) && !settings_room {
            return;
        }
        self.state.home_backspace();
        self.mark_dash_form_dirty();
        self.rebuild();
    }

    pub fn on_home_tab_key(&mut self) {
        if !self.state.open {
            return;
        }
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash && self.state.tab != DashTab::Home)
        {
            return;
        }
        // Hub: cycle is a no-op (see DashState::cycle_home_focus).
        self.state.cycle_home_focus();
        self.mark_dash_form_dirty();
        self.rebuild();
    }

    /// Enter on Home/Step2: login/register when signed out; Start session on create panel.
    pub fn on_home_enter(&mut self) -> UiAction {
        if !self.state.open {
            return UiAction::None;
        }
        if self.state.shell == OnboardingShell::Step1Operator
            && self.state.operator != OperatorKind::Federation
        {
            if !self.state.home_session_actions_enabled {
                self.state.home_status =
                    "Operator select / login is Desktop-only (use Desktop client)".into();
                self.mark_dash_form_dirty();
                self.rebuild();
                return UiAction::None;
            }
            self.state.custom_host_focus = false;
            self.mark_dash_logic_dirty();
            self.state.shell = OnboardingShell::Step2Auth;
            self.state.tab = DashTab::Home;
            self.state.home_focus = Some(HomeField::Identifier);
            self.state.home_status = "Sign in to continue".into();
            self.rebuild();
            return UiAction::ApplyOperatorEndpoints;
        }
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash && self.state.tab != DashTab::Home)
        {
            return UiAction::None;
        }
        if self.state.home_logged_in_as.is_some() {
            if self.state.home_create_open {
                if self.state.home_session_actions_enabled {
                    return UiAction::StartSession;
                }
                self.state.home_status = "Start session HTTP is Desktop-only this Slice".into();
                self.mark_dash_form_dirty();
                self.rebuild();
            }
            return UiAction::None;
        }
        match self.state.home_mode {
            HomeMode::Login => UiAction::AuthLogin,
            HomeMode::Register => UiAction::AuthRegister,
        }
    }

    pub fn prepare_frame(&mut self, queue: &wgpu::Queue) {
        self.set_time_now();
        self.tick_fps();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.rasterize_if_dirty(queue, &self.tree, self.state.open);
        }
    }

    /// Update labels and rasterize CPU canvas (no GPU upload). Used by OpenXR.
    pub fn prepare_cpu_canvas(&mut self) {
        self.set_time_now();
        self.tick_fps();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.rasterize_cpu(&self.tree, self.state.open);
        }
    }

    pub fn canvas_rgba(&self) -> Option<(&[u8], u32, u32)> {
        self.gpu.as_ref().map(|g| g.canvas_rgba())
    }

    /// Init canvas resources without a window surface (OpenXR uses CPU pixels only).
    pub fn init_cpu_canvas(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .ok_or_else(|| anyhow::anyhow!("no wgpu adapter for system_ui CPU canvas"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("system_ui-cpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                memory_hints: Default::default(),
            },
            None,
        ))?;
        self.init_gpu(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            width,
            height,
        )?;
        self._gpu_keepalive = Some((device, queue));
        Ok(())
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if !self.state.open {
            return;
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.draw(pass);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    None,
    ExitClientOnly,
    ExitClientAndHost,
    /// Step1 NEXT: write selected operator endpoints into SharedState + prefs.
    ApplyOperatorEndpoints,
    /// Home tab: POST login (session only; no auto-join · x0037).
    AuthLogin,
    /// Home tab: POST register (session only; no auto-join · x0037).
    AuthRegister,
    /// Home tab: logout + leave room locally (x0018).
    AuthLogout,
    /// Create panel / Settings Advanced: ensure visibility + local host when Applied (x0037 / x007c).
    StartSession,
    /// Settings Advanced: guest join fixed `home_room_id` (Waiting shell · x007c Phase 4).
    JoinRoom,
    /// Settings: VR hotswitch auto toggled (surface writes PresentationGate).
    HotswitchAutoChanged,
    /// Inventory: tab opened — sync scene mesh list from SharedState (x006e).
    InventoryOpen,
    /// Inventory: reload Drive nodes from assets (x006d).
    InventoryRefresh,
    /// Inventory: take-out selected blob via host control plane (x006d).
    InventoryTakeOut,
    /// Inventory: put-back selected scene mesh as Prefab (x006e).
    InventoryPutBack,
    /// Inventory: despawn selected scene mesh via host control plane (x007b).
    InventoryDespawn,
    /// Worlds: refresh catalog (x007c).
    WorldsRefresh,
    /// Worlds: Join as guest for selected room (x007c).
    WorldsJoinGuest,
    /// Worlds: Host selected catalog room (owner detail CTA · x007c).
    WorldsHostRoom,
    /// Worlds: Host Rescue while Waiting — always the joined room, ignore catalog selection.
    WorldsHostRescue,
    /// Worlds: Leave room bus back to catalog (x007c).
    WorldsLeave,
}

fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local offset is not applied (UTC) — good enough for Slice E stub clock.
    let s = secs % 86400;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    format!("{h:02}:{m:02}:{sec:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_pointer_sets_hover_on_open_dash() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Exit;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let (id, rect) = ui
            .tree
            .hits
            .iter()
            .find(|(w, _)| *w == WidgetId::ExitClientAndHost)
            .copied()
            .expect("exit hit");
        ui.on_pointer(rect.x + 1.0, rect.y + 1.0, false);
        assert_eq!(ui.state.hover, Some(id));
        ui.on_pointer(rect.x + 1.0, rect.y + 1.0, true);
        assert_eq!(ui.state.pressed, Some(id));
        ui.clear_pointer();
        assert_eq!(ui.state.hover, None);
        assert_eq!(ui.state.pressed, None);
    }

    #[test]
    fn home_tab_has_submit_and_bottom_tab() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(ui.tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeSubmit));
        ui.state.append_home_text("alice");
        assert_eq!(ui.state.home_identifier, "alice");
    }

    #[test]
    fn step1_shows_operator_cards_and_next() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Step1Operator;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::OperatorOfficial));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::Step1Next));
        assert!(!ui.tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
    }

    #[test]
    fn step1_next_official_goes_to_step2() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Step1Operator;
        ui.state.operator = crate::operator_prefs::OperatorKind::Official;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::Step1Next)
            .map(|(_, r)| *r)
            .expect("next");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::ApplyOperatorEndpoints);
        assert_eq!(ui.state.shell, OnboardingShell::Step2Auth);
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeSubmit));
    }

    #[test]
    fn main_hub_has_join_and_host() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainJoinHost));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainHostRoom));
    }

    #[test]
    fn chrome_only_push_does_not_clobber_peer_form() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.home_identifier = "peer-typed".into();
            store.revision = 3;
            store.chrome_revision = 1;
            store.logic.open = false;
        }

        let mut ui = SystemUi::new();
        ui.dash_logic_rev = 3;
        ui.dash_chrome_rev = 1;
        ui.state.open = false;
        ui.state.home_identifier.clear();
        ui.toggle(); // chrome-only dirty
        ui.push_dash_store(&dash);

        let store = dash.lock().unwrap();
        assert!(store.logic.open);
        assert_eq!(store.chrome_revision, 2);
        assert_eq!(store.revision, 3, "form revision must stay");
        assert_eq!(store.logic.home_identifier, "peer-typed");
    }

    #[test]
    fn form_dirty_still_pulls_peer_chrome() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = true;
            store.logic.tab = DashTab::Settings;
            store.logic.home_identifier = "seed".into();
            store.revision = 1;
            store.chrome_revision = 2;
        }

        let mut ui = SystemUi::new();
        ui.dash_logic_rev = 1;
        ui.dash_chrome_rev = 1;
        ui.state.open = false;
        ui.state.tab = DashTab::Home;
        ui.state.home_identifier = "typing…".into();
        ui.mark_dash_form_dirty();

        assert!(ui.pull_dash_store(&dash));
        assert!(ui.state.open);
        assert_eq!(ui.state.tab, DashTab::Settings);
        assert_eq!(ui.state.home_identifier, "typing…");
        assert!(ui.dash_form_dirty);
    }

    #[test]
    fn stale_chrome_push_does_not_clobber_newer_peer_chrome() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = false;
            store.chrome_revision = 1;
            store.revision = 1;
        }

        let mut a = SystemUi::new();
        a.dash_chrome_rev = 1;
        a.dash_logic_rev = 1;
        a.state.open = false;
        a.toggle(); // local open=true, chrome_dirty, still thinks rev=1

        // Peer B publishes open=false→ already false, but advances chrome via tab.
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = false;
            store.logic.tab = DashTab::Exit;
            store.chrome_revision = 2;
        }

        a.push_dash_store(&dash);
        let store = dash.lock().unwrap();
        assert_eq!(store.chrome_revision, 2);
        assert!(!store.logic.open);
        assert_eq!(store.logic.tab, DashTab::Exit);
        assert!(!a.state.open);
        assert_eq!(a.state.tab, DashTab::Exit);
        assert!(!a.dash_chrome_dirty);
    }

    #[test]
    fn form_only_push_does_not_rewrite_chrome() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = true;
            store.logic.tab = DashTab::Settings;
            store.chrome_revision = 5;
            store.revision = 2;
        }

        let mut ui = SystemUi::new();
        ui.dash_chrome_rev = 5;
        ui.dash_logic_rev = 2;
        ui.state.open = false; // stale local chrome — must not be written
        ui.state.tab = DashTab::Home;
        ui.state.home_identifier = "alice".into();
        ui.mark_dash_form_dirty();
        ui.push_dash_store(&dash);

        let store = dash.lock().unwrap();
        assert!(store.logic.open);
        assert_eq!(store.logic.tab, DashTab::Settings);
        assert_eq!(store.chrome_revision, 5);
        assert_eq!(store.revision, 3);
        assert_eq!(store.logic.home_identifier, "alice");
    }

    #[test]
    fn noop_form_dirty_does_not_bump_revision() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.home_status = "same".into();
            store.revision = 4;
        }

        let mut ui = SystemUi::new();
        ui.dash_logic_rev = 4;
        ui.state.home_status = "same".into();
        ui.mark_dash_form_dirty();
        ui.push_dash_store(&dash);

        let store = dash.lock().unwrap();
        assert_eq!(store.revision, 4);
        assert!(!ui.dash_form_dirty);
    }
}
