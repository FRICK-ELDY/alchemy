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
//! Federation directory / friends active-session list (future).

mod dash;
pub mod facet;
mod font;
mod gpu;
mod icons;
mod inventory_peer;
mod layout;

#[allow(unused_imports)] // public re-export; store.logic type for callers
pub use dash::SharedDashLogic;
pub use dash::{
    access_level_from_widget, fresh_create_room_id, DashState, DashTab, HomeField, HomeMode,
    MainHubTab, OnboardingShell, SharedDashStore, SESSION_NAME_MAX,
};
pub use gpu::UiGpu;
pub use inventory_peer::{
    rematch_inventory_selection, rematch_scene_selection, InventoryMirror, InventoryPeerRequest,
};
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
    /// Last applied [`crate::scene::SharedState::inventory_mirror`] revision (XR).
    inventory_mirror_rev: u64,
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
            inventory_mirror_rev: 0,
        }
    }

    /// XR: adopt Desktop Drive list + inventory status when mirror advances.
    pub fn pull_inventory_mirror(&mut self, shared: &SharedState) -> bool {
        let Ok(m) = shared.inventory_mirror.lock() else {
            return false;
        };
        if m.revision == self.inventory_mirror_rev {
            return false;
        }
        let prev_node_id = self.state.inventory_selected.and_then(|i| {
            self.state
                .inventory_nodes
                .get(i)
                .map(|(id, _, _)| id.clone())
        });
        self.state.inventory_nodes = m.nodes.clone();
        self.state.inventory_selected =
            rematch_inventory_selection(&self.state.inventory_nodes, prev_node_id.as_deref());
        self.state.inventory_status = m.status.clone();
        self.inventory_mirror_rev = m.revision;
        true
    }

    /// XR: keep scene mesh rows in sync with Manifest-derived SharedState.
    pub fn sync_inventory_scene_meshes(&mut self, shared: &SharedState) -> bool {
        let meshes = shared.scene_meshes();
        if meshes == self.state.scene_meshes {
            return false;
        }
        let prev: Vec<u32> = self
            .state
            .scene_selected
            .iter()
            .filter_map(|&i| self.state.scene_meshes.get(i).map(|(eid, _)| *eid))
            .collect();
        self.state.scene_meshes = meshes;
        self.state.scene_selected = rematch_scene_selection(&self.state.scene_meshes, &prev);
        true
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

    /// Block MainHub / Inventory while in-session (Desktop `worlds_*` or XR `in_session_peer`).
    /// Local session: keep Dash shell so Session UI stays reachable.
    fn block_nav_while_session(&mut self, msg: &str) -> bool {
        if !self.state.inventory_blocked() {
            return false;
        }
        if self.state.session_shell_active() {
            self.state.shell = OnboardingShell::Dash;
        }
        self.state.worlds_status = msg.into();
        self.state.home_status = msg.into();
        self.mark_dash_form_dirty();
        self.rebuild();
        true
    }

    /// Official / Custom card: apply endpoints and open Step2 auth.
    fn advance_step1_operator(&mut self) -> UiAction {
        if self.state.operator == OperatorKind::Federation {
            return UiAction::None;
        }
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
            let peer_inventory = store.logic.tab == DashTab::Inventory;
            changed |= store.logic.apply_chrome_to(&mut self.state);
            self.dash_chrome_rev = store.chrome_revision;
            // Reject Companion Inventory while Desktop session is live — push Home back.
            if peer_inventory
                && self.state.inventory_blocked()
                && self.state.tab != DashTab::Inventory
            {
                self.mark_dash_chrome_dirty();
            }
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
                // Peer chrome advanced: adopt Esc/Menu/settings; session landing is Desktop-owned.
                log::info!(
                    "dash chrome CAS miss: local_rev={} store_rev={} — adopting peer chrome",
                    self.dash_chrome_rev,
                    store.chrome_revision
                );
                let peer_in_session = store.logic.in_session;
                let local_in_session = self.state.in_session_for_store();
                let saved_shell = self.state.shell;
                let saved_tab = self.state.tab;
                let saved_open = self.state.open;
                rebuild_after = store.logic.apply_chrome_to(&mut self.state);
                self.dash_chrome_rev = store.chrome_revision;
                self.dash_chrome_dirty = false;
                if self.state.home_session_actions_enabled && local_in_session != peer_in_session {
                    // Leave (local false / peer true) or Join (local true / peer false):
                    // keep Desktop shell landing and retry push so store.in_session matches.
                    self.state.shell = saved_shell;
                    self.state.tab = saved_tab;
                    self.state.open = saved_open;
                    self.dash_chrome_dirty = true;
                    rebuild_after = true;
                }
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
        self.state.clamp_settings_scroll();
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

    /// Mouse wheel / trackpad scroll (canvas pixels).
    /// Settings: continuous pixel scroll. MainHub Participation catalog: discrete row steps.
    /// Positive `delta_y` scrolls content up (reveal lower widgets / later rooms).
    pub fn on_scroll(&mut self, px: f32, py: f32, delta_y: f32) {
        if !self.state.open {
            return;
        }
        if self.state.tab == DashTab::Settings {
            if !self.state.settings_panel_rect().contains(px, py) {
                return;
            }
            if self.state.settings_max_scroll() <= 0.0 {
                return;
            }
            self.state.scroll_settings_by(delta_y);
            self.rebuild();
            return;
        }
        if self.state.shell == OnboardingShell::MainHub
            && self.state.main_hub_tab == MainHubTab::Participation
            && self.state.home_logged_in_as.is_some()
            && !self.state.session_shell_active()
            && self.state.worlds_catalog_list_rect().contains(px, py)
            && self
                .state
                .worlds_catalog_max_scroll(self.state.worlds_catalog_list_h())
                > 0
            && self.state.scroll_worlds_catalog_pixels(delta_y)
        {
            self.rebuild();
        }
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
                // Allowed during session so Server Operator can be enabled for Host Rescue.
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Settings;
                self.state.settings_scroll = 0.0;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabWorlds => {
                // Worlds bottom tab removed — route to MainHub catalog (not during session).
                if self.block_nav_while_session("Leave session before opening Main hub") {
                    return UiAction::None;
                }
                self.clear_settings_room_focus();
                self.state.home_create_open = false;
                self.state.shell = OnboardingShell::MainHub;
                self.state.tab = DashTab::Home;
                self.state.main_hub_tab = MainHubTab::Participation;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                UiAction::WorldsRefresh
            }
            WidgetId::TabInventory => {
                if self.block_nav_while_session("Inventory is available when Live (x007f)") {
                    return UiAction::None;
                }
                self.clear_settings_room_focus();
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Inventory;
                self.mark_dash_chrome_dirty();
                self.rebuild();
                // Desktop syncs Manifest scene mesh list without Drive HTTP (x006e).
                UiAction::InventoryOpen
            }
            WidgetId::WorldsRefresh => UiAction::WorldsRefresh,
            // Catalog UI removed; filter/row ids kept for WidgetId exhaustiveness only.
            WidgetId::WorldsFilterAll | WidgetId::WorldsFilterMine => UiAction::None,
            WidgetId::WorldsCreate | WidgetId::HomeNewWorld | WidgetId::MainHubCreateWorld => {
                if !self.state.server_operator {
                    self.state.home_status =
                        "Enable Server Operator in Settings to create or host".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    return UiAction::None;
                }
                // Mint a fresh room_id so Start creates DemoScene — do not reuse `local`
                // (existing rooms keep take-out/despawn history in world DB).
                self.state.home_room_id = dash::fresh_create_room_id();
                self.state.home_create_open = true;
                self.state.tab = DashTab::Home;
                self.state.shell = OnboardingShell::Dash;
                self.state.home_focus = Some(crate::system_ui::HomeField::SessionName);
                self.state.home_status = if self.state.home_session_actions_enabled {
                    format!(
                        "New room_id={} — pick access, then Start (DemoScene)",
                        self.state.home_room_id
                    )
                } else {
                    "Create panel (browse only) — Start HTTP is Desktop-only".into()
                };
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeOpenWorlds => {
                if self.block_nav_while_session("Leave session before opening Main hub") {
                    return UiAction::None;
                }
                self.clear_settings_room_focus();
                self.state.home_create_open = false;
                self.state.tab = DashTab::Home;
                self.state.shell = OnboardingShell::MainHub;
                self.state.main_hub_tab = MainHubTab::Participation;
                self.state.home_status = "Main hub — join a session or organize a world".into();
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::WorldsRefresh
            }
            WidgetId::WorldsJoinGuest => UiAction::WorldsJoinGuest,
            WidgetId::WorldsHostRoom => {
                if !self.state.server_operator {
                    self.state.home_status = "Enable Server Operator in Settings to host".into();
                    self.state.worlds_status = self.state.home_status.clone();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else {
                    UiAction::WorldsHostRoom
                }
            }
            WidgetId::WorldsHostRescue => {
                if !self.state.server_operator {
                    self.state.worlds_status =
                        "Enable Server Operator in Settings to host this room".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else {
                    UiAction::WorldsHostRescue
                }
            }
            WidgetId::WorldsTakeover => {
                if !self.state.server_operator {
                    self.state.worlds_status =
                        "Enable Server Operator in Settings to take over host".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else {
                    UiAction::WorldsTakeover
                }
            }
            WidgetId::WorldsLeave | WidgetId::SessionEnterCancel => UiAction::WorldsLeave,
            WidgetId::SessionEnterBlocker => UiAction::None,
            WidgetId::WorldsRejoin => UiAction::WorldsRejoin,
            // Catalog UI removed; row ids kept for WidgetId exhaustiveness only.
            WidgetId::WorldsRow0
            | WidgetId::WorldsRow1
            | WidgetId::WorldsRow2
            | WidgetId::WorldsRow3
            | WidgetId::WorldsRow4
            | WidgetId::WorldsRow5
            | WidgetId::WorldsRow6
            | WidgetId::WorldsRow7 => UiAction::None,
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
                    self.state.scene_selected.clear();
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
                    // Toggle multi-select for put-back (x007f).
                    if let Some(pos) = self.state.scene_selected.iter().position(|&i| i == idx) {
                        self.state.scene_selected.remove(pos);
                    } else {
                        self.state.scene_selected.push(idx);
                        self.state.scene_selected.sort_unstable();
                    }
                    self.state.inventory_selected = None;
                    self.rebuild();
                }
                UiAction::None
            }
            WidgetId::TabHome => {
                self.clear_settings_room_focus();
                // In-room: Home returns to Session shell (not MainHub catalog).
                if self.state.session_shell_active() {
                    self.state.shell = OnboardingShell::Dash;
                    self.state.tab = DashTab::Home;
                    self.mark_dash_chrome_dirty();
                    self.rebuild();
                    return UiAction::None;
                }
                // XR: Desktop still in session — do not push MainHub into shared chrome.
                if self.block_nav_while_session("Leave session before opening Main hub") {
                    return UiAction::None;
                }
                if self.state.home_logged_in_as.is_some() && !self.state.home_create_open {
                    let already_hub = self.state.shell == OnboardingShell::MainHub;
                    self.state.shell = OnboardingShell::MainHub;
                    self.state.tab = DashTab::Home;
                    self.mark_dash_chrome_dirty();
                    self.rebuild();
                    // Refresh catalog only when entering hub, not on every Home re-click.
                    if already_hub {
                        UiAction::None
                    } else {
                        UiAction::WorldsRefresh
                    }
                } else {
                    self.state.shell = OnboardingShell::Dash;
                    self.state.tab = DashTab::Home;
                    self.mark_dash_chrome_dirty();
                    self.rebuild();
                    UiAction::None
                }
            }
            WidgetId::OperatorOfficial => {
                self.state.operator = OperatorKind::Official;
                self.advance_step1_operator()
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
                self.advance_step1_operator()
            }
            WidgetId::OperatorCustomHost => {
                // Edit host without leaving Step1; Enter advances (on_home_enter).
                self.state.operator = OperatorKind::Custom;
                self.state.custom_host_focus = true;
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            // NEXT removed — cards advance immediately. Id kept for exhaustiveness.
            WidgetId::Step1Next | WidgetId::Step1Back => UiAction::None,
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
            WidgetId::MainTabParticipation => {
                let switched = self.state.main_hub_tab != MainHubTab::Participation;
                self.state.main_hub_tab = MainHubTab::Participation;
                self.state.home_status = "Participation — join a public session".into();
                self.mark_dash_form_dirty();
                self.rebuild();
                // Catalog refresh only when entering Participation (Refresh button otherwise).
                if switched {
                    UiAction::WorldsRefresh
                } else {
                    UiAction::None
                }
            }
            WidgetId::MainTabOrganizing => {
                if !self.state.server_operator {
                    self.state.home_status =
                        "Enable Server Operator in Settings to organize worlds".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    return UiAction::None;
                }
                self.state.main_hub_tab = MainHubTab::Organizing;
                self.state.home_status = "Organizing — create or host a world".into();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainHubRefresh => UiAction::WorldsRefresh,
            WidgetId::MainHubJoin => UiAction::WorldsJoinGuest,
            WidgetId::MainHubHostWorld => {
                // My worlds rows are stubbed; Host CTA reserved for owner catalog.
                self.state.home_status = "My worlds hosting is not connected yet".into();
                self.mark_dash_form_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainPublicRow0
            | WidgetId::MainPublicRow1
            | WidgetId::MainPublicRow2
            | WidgetId::MainPublicRow3
            | WidgetId::MainPublicRow4
            | WidgetId::MainPublicRow5
            | WidgetId::MainPublicRow6
            | WidgetId::MainPublicRow7 => {
                let vis = match hit {
                    WidgetId::MainPublicRow0 => 0,
                    WidgetId::MainPublicRow1 => 1,
                    WidgetId::MainPublicRow2 => 2,
                    WidgetId::MainPublicRow3 => 3,
                    WidgetId::MainPublicRow4 => 4,
                    WidgetId::MainPublicRow5 => 5,
                    WidgetId::MainPublicRow6 => 6,
                    WidgetId::MainPublicRow7 => 7,
                    _ => 0,
                };
                let list_h = self.state.worlds_catalog_list_h();
                let visible = DashState::worlds_catalog_visible_rows(list_h);
                let max_scroll = self.state.worlds_rooms.len().saturating_sub(visible);
                let start = self.state.worlds_catalog_scroll.min(max_scroll);
                let idx = start + vis;
                if let Some((room_id, ..)) = self.state.worlds_rooms.get(idx) {
                    self.state.worlds_selected = Some(room_id.clone());
                    self.rebuild();
                }
                UiAction::None
            }
            WidgetId::MainMyWorldRow0
            | WidgetId::MainMyWorldRow1
            | WidgetId::MainMyWorldRow2
            | WidgetId::MainMyWorldRow3
            | WidgetId::MainMyWorldRow4
            | WidgetId::MainMyWorldRow5
            | WidgetId::MainMyWorldRow6
            | WidgetId::MainMyWorldRow7 => {
                // Stub rows are not registered as hits yet.
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
            WidgetId::SettingsScrollUp => {
                self.state.scroll_settings_by(-48.0);
                self.rebuild();
                UiAction::None
            }
            WidgetId::SettingsScrollDown => {
                self.state.scroll_settings_by(48.0);
                self.rebuild();
                UiAction::None
            }
            WidgetId::ServerOperatorToggle => {
                self.state.server_operator = !self.state.server_operator;
                if !self.state.server_operator && self.state.main_hub_tab == MainHubTab::Organizing
                {
                    self.state.main_hub_tab = MainHubTab::Participation;
                }
                let msg = if self.state.server_operator {
                    if self.state.session_shell_active() {
                        "Server Operator ON — you can Host this room now"
                    } else {
                        "Server Operator ON — Organizing unlocked (loads this PC)"
                    }
                } else {
                    "Server Operator OFF — join-only"
                };
                self.state.home_status = msg.into();
                self.state.worlds_status = msg.into();
                self.mark_dash_chrome_dirty();
                self.mark_dash_form_dirty();
                self.rebuild();
                log::info!(
                    "server operator → {}",
                    if self.state.server_operator {
                        "ON"
                    } else {
                        "OFF"
                    }
                );
                UiAction::ServerOperatorChanged
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
            WidgetId::ChromeLoginRegister => {
                self.clear_settings_room_focus();
                self.state.home_create_open = false;
                self.state.home_mode = HomeMode::Login;
                self.state.tab = DashTab::Home;
                self.state.home_focus = Some(HomeField::Identifier);
                if self.state.shell == OnboardingShell::Step1Operator
                    || self.state.shell == OnboardingShell::Step1FederationStub
                {
                    // Reach auth after operator pick; apply Official/Custom endpoints on Desktop.
                    if self.state.operator == OperatorKind::Federation {
                        self.state.home_status =
                            "Federation is not available yet — pick Official or Custom".into();
                        self.mark_dash_form_dirty();
                        self.rebuild();
                        return UiAction::None;
                    }
                    if !self.state.home_session_actions_enabled {
                        self.state.home_status =
                            "Operator select / login is Desktop-only (use Desktop client)".into();
                        self.mark_dash_form_dirty();
                        self.rebuild();
                        return UiAction::None;
                    }
                    self.state.custom_host_focus = false;
                    self.state.shell = OnboardingShell::Step2Auth;
                    self.state.home_status = "Sign in to continue".into();
                    self.mark_dash_logic_dirty();
                    self.rebuild();
                    return UiAction::ApplyOperatorEndpoints;
                }
                self.state.shell = OnboardingShell::Step2Auth;
                self.state.home_status = "Sign in or create an account".into();
                self.mark_dash_logic_dirty();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeLogout | WidgetId::ChromeLogout => {
                self.state.home_create_open = false;
                self.state.shell = OnboardingShell::Step1Operator;
                self.mark_dash_logic_dirty();
                UiAction::AuthLogout
            }
            WidgetId::HomeCreateBack => {
                self.state.home_create_open = false;
                self.state.home_focus = None;
                self.state.tab = DashTab::Home;
                self.state.shell = OnboardingShell::MainHub;
                self.state.main_hub_tab = if self.state.server_operator {
                    MainHubTab::Organizing
                } else {
                    MainHubTab::Participation
                };
                self.state.home_status = if self.state.server_operator {
                    "Create cancelled — organize a world or go back to Participation".into()
                } else {
                    "Create cancelled — enable Server Operator in Settings to organize".into()
                };
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
                if self.state.session_shell_active() {
                    self.state.home_status =
                        "Leave session from Home before Advanced Start / Join".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else if !self.state.server_operator {
                    self.state.home_status =
                        "Enable Server Operator in Settings to start a host".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else if self.state.home_session_actions_enabled {
                    UiAction::StartSession
                } else {
                    self.state.home_status = "Start session HTTP is Desktop-only this Slice".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                }
            }
            WidgetId::HomeJoinRoom | WidgetId::SettingsJoinRoom => {
                if self.state.session_shell_active() {
                    self.state.home_status =
                        "Leave session from Home before Advanced Start / Join".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    UiAction::None
                } else if self.state.home_session_actions_enabled {
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
                if !self.state.server_operator {
                    self.state.home_status =
                        "Enable Server Operator in Settings to start a host".into();
                    self.mark_dash_form_dirty();
                    self.rebuild();
                    return UiAction::None;
                }
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
    /// Settings: Server Operator toggled (persist to onboarding prefs).
    ServerOperatorChanged,
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
    /// Worlds: Manual Takeover while Waiting and session dormant (x0080).
    WorldsTakeover,
    /// Worlds: Leave room bus back to catalog (x007c).
    WorldsLeave,
    /// Worlds: KeyStale Rejoin (Leave → Join same room · x007e).
    WorldsRejoin,
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
    fn step1_shows_operator_cards_without_next() {
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
            .any(|(id, _)| *id == WidgetId::OperatorCustom));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::Step1Next));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::Step1Back));
        assert!(!ui.tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
        assert!(!ui.tree.draws.iter().any(|d| {
            matches!(
                d,
                crate::system_ui::layout::DrawCmd::Label { text, .. } if text.contains("STEP 1")
            )
        }));
    }

    #[test]
    fn step1_official_card_goes_to_step2() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Step1Operator;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::OperatorOfficial)
            .map(|(_, r)| *r)
            .expect("official");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::ApplyOperatorEndpoints);
        assert_eq!(ui.state.shell, OnboardingShell::Step2Auth);
        assert_eq!(
            ui.state.operator,
            crate::operator_prefs::OperatorKind::Official
        );
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::HomeSubmit));
    }

    #[test]
    fn chrome_shows_login_when_signed_out_and_logout_when_signed_in() {
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
            .any(|(id, _)| *id == WidgetId::ChromeLoginRegister));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ChromeLogout));

        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.shell = OnboardingShell::MainHub;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ChromeLogout));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ChromeLoginRegister));
    }

    #[test]
    fn chrome_logout_returns_auth_logout_action() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::ChromeLogout)
            .map(|(_, r)| *r)
            .expect("chrome logout");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::AuthLogout);
        assert_eq!(ui.state.shell, OnboardingShell::Step1Operator);
    }

    #[test]
    fn bottom_bar_omits_worlds_tab() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::TabWorlds));
        assert!(ui.tree.hits.iter().any(|(id, _)| *id == WidgetId::TabHome));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::TabInventory));
    }

    #[test]
    fn session_shell_shows_leave_while_waiting() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.worlds_in_room = true;
        ui.state.worlds_waiting_host = true;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::WorldsLeave));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::TabWorlds));
    }

    #[test]
    fn session_enter_overlay_shows_progress_cancel_and_logs() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.home_room_id = "demo".into();
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.state.begin_session_enter("Starting local host…");
        ui.state.set_session_enter_progress(0.40);
        ui.state
            .push_session_enter_log("Spawning host-server (cargo run can take a while)");
        ui.rebuild();
        assert!(ui.state.session_shell_active());
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SessionEnterCancel));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SessionEnterBlocker));
        assert!(ui.tree.draws.iter().any(|d| {
            matches!(
                d,
                crate::system_ui::layout::DrawCmd::Label { text, .. }
                    if text.contains("Starting local host")
            )
        }));
        assert!(ui.tree.draws.iter().any(|d| {
            matches!(
                d,
                crate::system_ui::layout::DrawCmd::Label { text, .. }
                    if text.contains("Spawning host-server")
            )
        }));
        assert!(ui.tree.draws.iter().any(|d| {
            matches!(
                d,
                crate::system_ui::layout::DrawCmd::Label { text, .. } if text.contains("40%")
            )
        }));
        let cancel = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::SessionEnterCancel)
            .map(|(_, r)| *r)
            .expect("cancel");
        assert_eq!(
            ui.on_click(cancel.x + 1.0, cancel.y + 1.0),
            UiAction::WorldsLeave
        );
    }

    #[test]
    fn session_enter_bus_bound_marks_wait_and_logs_phases() {
        let mut ui = SystemUi::new();
        ui.state.begin_session_enter("Joining…");
        ui.state.mark_session_enter_bus_bound();
        assert!(ui.state.session_enter_wait_since.is_some());
        assert!(!ui.state.session_enter_seen_manifest);
        assert!(ui
            .state
            .session_enter_logs
            .iter()
            .any(|l| l.contains("listening for Manifest")));
        assert!(ui
            .state
            .session_enter_title
            .contains("Waiting for host publish"));
        ui.state.apply_session_enter_progress(
            0.96,
            "Manifest OK — waiting for Frame…",
            "Manifest accepted gen=1 rev=1 — waiting for first Frame",
        );
        assert!(ui
            .state
            .session_enter_logs
            .iter()
            .any(|l| l.contains("Manifest accepted")));
    }

    #[test]
    fn session_shell_home_tab_does_not_escape_to_main_hub() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.worlds_in_room = true;
        ui.state.worlds_waiting_host = true;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::TabHome)
            .map(|(_, r)| *r)
            .expect("home tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::None);
        assert_eq!(ui.state.shell, OnboardingShell::Dash);
        assert_eq!(ui.state.tab, DashTab::Home);
        assert!(ui.state.session_shell_active());
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::WorldsLeave));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainHubRefresh));
    }

    #[test]
    fn session_allows_settings_and_host_rescue_enable() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.worlds_in_room = true;
        ui.state.worlds_waiting_host = true;
        ui.state.worlds_can_host_rescue = true;
        ui.state.server_operator = false;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ServerOperatorToggle));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::WorldsHostRescue));
        let settings = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::TabSettings)
            .map(|(_, r)| *r)
            .expect("settings tab");
        assert_eq!(
            ui.on_click(settings.x + 1.0, settings.y + 1.0),
            UiAction::None
        );
        assert_eq!(ui.state.tab, DashTab::Settings);
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsJoinRoom));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::SettingsStartSession));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::ServerOperatorToggle));
        let enable = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::ServerOperatorToggle)
            .map(|(_, r)| *r)
            .expect("server operator toggle");
        assert_eq!(
            ui.on_click(enable.x + 1.0, enable.y + 1.0),
            UiAction::ServerOperatorChanged
        );
        assert!(ui.state.server_operator);
        assert!(ui.tree.draws.iter().any(|d| {
            matches!(
                d,
                crate::system_ui::layout::DrawCmd::Label { text, .. }
                    if text.contains("Host this room now")
            )
        }));
        ui.state.tab = DashTab::Home;
        ui.rebuild();
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::WorldsHostRescue));
    }

    #[test]
    fn session_shell_inventory_tab_stays_on_session() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.worlds_in_room = true;
        ui.state.worlds_waiting_host = true;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::TabInventory)
            .map(|(_, r)| *r)
            .expect("inventory tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::None);
        assert_eq!(ui.state.tab, DashTab::Home);
        assert!(ui.state.session_shell_active());
        assert!(ui
            .state
            .worlds_status
            .contains("Inventory is available when Live"));
    }

    #[test]
    fn live_session_allows_inventory_tab() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.home_session_actions_enabled = true;
        ui.state.worlds_in_room = true;
        ui.state.worlds_waiting_host = false;
        ui.state.worlds_key_stale = false;
        ui.state.session_enter_active = false;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::TabInventory)
            .map(|(_, r)| *r)
            .expect("inventory tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::InventoryOpen);
        assert_eq!(ui.state.tab, DashTab::Inventory);
        ui.rebuild();
        assert!(
            ui.tree
                .hits
                .iter()
                .any(|(id, _)| *id == WidgetId::InventoryRefresh),
            "Live Inventory tab must show inventory controls, not session shell"
        );
    }

    #[test]
    fn main_hub_home_reclick_skips_worlds_refresh() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.tab = DashTab::Home;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::TabHome)
            .map(|(_, r)| *r)
            .expect("home tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::None);
        assert_eq!(ui.state.shell, OnboardingShell::MainHub);
    }

    #[test]
    fn main_hub_defaults_to_participation_tabs() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        assert_eq!(ui.state.main_hub_tab, MainHubTab::Participation);
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainTabParticipation));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainTabOrganizing));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainHubRefresh));
        assert!(!ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainHubCreateWorld));
    }

    #[test]
    fn main_hub_organizing_requires_server_operator() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.server_operator = false;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::MainTabOrganizing)
            .map(|(_, r)| *r)
            .expect("organizing tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::None);
        assert_eq!(ui.state.main_hub_tab, MainHubTab::Participation);
        assert!(ui.state.home_status.contains("Enable Server Operator"));
    }

    #[test]
    fn main_hub_organizing_tab_keeps_shell_and_shows_create() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.server_operator = true;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::MainTabOrganizing)
            .map(|(_, r)| *r)
            .expect("organizing tab");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::None);
        assert_eq!(ui.state.shell, OnboardingShell::MainHub);
        assert_eq!(ui.state.main_hub_tab, MainHubTab::Organizing);
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainHubCreateWorld));
    }

    #[test]
    fn settings_server_operator_toggle_persists_action() {
        let mut ui = SystemUi::new();
        ui.state.open = true;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Settings;
        ui.state.home_logged_in_as = Some("alice".into());
        ui.state.server_operator = false;
        ui.state.canvas_w = 960.0;
        ui.state.canvas_h = 540.0;
        ui.rebuild();
        let rect = ui
            .tree
            .hits
            .iter()
            .find(|(id, _)| *id == WidgetId::ServerOperatorToggle)
            .map(|(_, r)| *r)
            .expect("server operator toggle");
        let action = ui.on_click(rect.x + 1.0, rect.y + 1.0);
        assert_eq!(action, UiAction::ServerOperatorChanged);
        assert!(ui.state.server_operator);
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
    fn desktop_apply_chrome_does_not_latch_peer_in_session() {
        let logic = SharedDashLogic {
            in_session: true,
            tab: DashTab::Home,
            shell: OnboardingShell::Dash,
            open: true,
            ..Default::default()
        };
        let mut desktop = DashState {
            home_session_actions_enabled: true,
            in_session_peer: false,
            worlds_in_room: false,
            ..Default::default()
        };
        logic.apply_chrome_to(&mut desktop);
        assert!(
            !desktop.in_session_peer,
            "Desktop must not latch peer in_session into in_session_peer"
        );
        assert!(!desktop.inventory_blocked());
    }

    #[test]
    fn xr_apply_chrome_echoes_in_session_peer() {
        let logic = SharedDashLogic {
            in_session: true,
            inventory_live: false,
            tab: DashTab::Home,
            shell: OnboardingShell::Dash,
            open: true,
            ..Default::default()
        };
        let mut xr = DashState {
            home_session_actions_enabled: false,
            in_session_peer: false,
            inventory_live_peer: false,
            ..Default::default()
        };
        logic.apply_chrome_to(&mut xr);
        assert!(xr.in_session_peer);
        assert!(!xr.inventory_live_peer);
        assert!(xr.inventory_blocked());
    }

    #[test]
    fn inventory_unblocked_when_live() {
        let desktop = DashState {
            home_session_actions_enabled: true,
            worlds_in_room: true,
            worlds_waiting_host: false,
            worlds_key_stale: false,
            session_enter_active: false,
            ..Default::default()
        };
        assert!(desktop.inventory_live_allowed());
        assert!(!desktop.inventory_blocked());

        let waiting = DashState {
            home_session_actions_enabled: true,
            worlds_in_room: true,
            worlds_waiting_host: true,
            ..Default::default()
        };
        assert!(waiting.inventory_blocked());

        let xr_live = DashState {
            home_session_actions_enabled: false,
            in_session_peer: true,
            inventory_live_peer: true,
            ..Default::default()
        };
        assert!(!xr_live.inventory_blocked());
    }

    #[test]
    fn leave_cas_miss_keeps_main_hub_and_retries_in_session_false() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = true;
            store.logic.shell = OnboardingShell::Dash;
            store.logic.tab = DashTab::Home;
            store.logic.in_session = true; // XR still echoing pre-Leave
            store.chrome_revision = 2;
        }

        let mut desktop = SystemUi::new();
        desktop.state.home_session_actions_enabled = true;
        desktop.state.open = true;
        desktop.state.shell = OnboardingShell::MainHub;
        desktop.state.tab = DashTab::Home;
        desktop.state.worlds_in_room = false;
        desktop.state.worlds_waiting_host = false;
        desktop.state.worlds_key_stale = false;
        desktop.state.in_session_peer = false;
        desktop.dash_chrome_rev = 1; // stale vs store rev 2
        desktop.dash_chrome_dirty = true;

        desktop.push_dash_store(&dash);

        assert_eq!(desktop.state.shell, OnboardingShell::MainHub);
        assert!(!desktop.state.in_session_for_store());
        assert!(
            desktop.dash_chrome_dirty,
            "must retry push so store.in_session becomes false"
        );

        // Second push (rev now matches after adopt) writes false.
        desktop.dash_chrome_rev = dash.lock().unwrap().chrome_revision;
        desktop.push_dash_store(&dash);
        let store = dash.lock().unwrap();
        assert!(!store.logic.in_session);
        assert_eq!(store.logic.shell, OnboardingShell::MainHub);
        assert!(!desktop.dash_chrome_dirty);
    }

    #[test]
    fn join_cas_miss_retries_in_session_true() {
        let dash = Mutex::new(SharedDashStore::default());
        {
            let mut store = dash.lock().unwrap();
            store.logic.open = true;
            store.logic.shell = OnboardingShell::MainHub;
            store.logic.tab = DashTab::Home;
            store.logic.in_session = false; // peer lagging behind Join
            store.chrome_revision = 2;
        }

        let mut desktop = SystemUi::new();
        desktop.state.home_session_actions_enabled = true;
        desktop.state.open = true;
        desktop.state.shell = OnboardingShell::Dash;
        desktop.state.tab = DashTab::Home;
        desktop.state.worlds_in_room = true;
        desktop.state.worlds_waiting_host = true;
        desktop.dash_chrome_rev = 1;
        desktop.dash_chrome_dirty = true;

        desktop.push_dash_store(&dash);
        assert!(desktop.state.session_shell_active());
        assert_eq!(desktop.state.shell, OnboardingShell::Dash);
        assert!(desktop.dash_chrome_dirty);

        desktop.dash_chrome_rev = dash.lock().unwrap().chrome_revision;
        desktop.push_dash_store(&dash);
        let store = dash.lock().unwrap();
        assert!(store.logic.in_session);
        assert_eq!(store.logic.shell, OnboardingShell::Dash);
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
