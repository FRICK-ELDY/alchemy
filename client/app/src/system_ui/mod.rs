//! system_ui — personal DashRoot + retained Canvas (x000f + x0018 Home + x0037 create).
//!
//! Guarantees: ESC/Menu toggle (via surface), hit-tested chrome, Exit Pinned Facet,
//! onboarding shell (Step1 operator → Step2 auth → Main hub), Home account /
//! create-world form (Desktop text input), Button hover/press color, client exit /
//! auth / start-session action plumbing, Settings defaults (read-only).
//! Dash is personal — never published on Room `frame`.
//! Non-guarantees: OpenXR types inside this module, user Facet rearrange, Vault layout
//! persist, WorldRoot, XR virtual keyboard, auth/world HTTP (surface modules), final UI art,
//! Federation directory / PLAY TOGETHER session list.

mod dash;
pub mod facet;
mod font;
mod gpu;
mod icons;
mod layout;

pub use dash::{
    access_level_from_widget, DashState, DashTab, HomeField, HomeMode, OnboardingShell,
    SESSION_NAME_MAX,
};
pub use gpu::UiGpu;
pub use layout::{LayoutTree, WidgetId};

use crate::operator_prefs::OperatorKind;
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
        }
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
            self.rebuild();
            return UiAction::None;
        }
        match hit {
            WidgetId::TabExit | WidgetId::CloseSession => {
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Exit;
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabSettings => {
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Settings;
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabHome => {
                if self.state.home_logged_in_as.is_some() && !self.state.home_create_open {
                    // Return to PLAY TOGETHER / SOLO hub when signed in.
                    self.state.shell = OnboardingShell::MainHub;
                    self.state.tab = DashTab::Home;
                } else {
                    self.state.shell = OnboardingShell::Dash;
                    self.state.tab = DashTab::Home;
                }
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorOfficial => {
                self.state.operator = OperatorKind::Official;
                self.state.custom_host_focus = false;
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorFederation => {
                self.state.operator = OperatorKind::Federation;
                self.state.custom_host_focus = false;
                self.state.shell = OnboardingShell::Step1FederationStub;
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorCustom => {
                self.state.operator = OperatorKind::Custom;
                self.state.custom_host_focus = true;
                self.rebuild();
                UiAction::None
            }
            WidgetId::OperatorCustomHost => {
                self.state.operator = OperatorKind::Custom;
                self.state.custom_host_focus = true;
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
                    self.rebuild();
                    return UiAction::None;
                }
                self.state.custom_host_focus = false;
                self.state.shell = OnboardingShell::Step2Auth;
                self.state.tab = DashTab::Home;
                self.state.home_focus = Some(HomeField::Identifier);
                self.state.home_status = "Sign in to continue".into();
                self.rebuild();
                UiAction::ApplyOperatorEndpoints
            }
            WidgetId::Step1Back => {
                // First-launch: no prior step. Keep Step1.
                self.rebuild();
                UiAction::None
            }
            WidgetId::FederationStubBack => {
                self.state.shell = OnboardingShell::Step1Operator;
                self.state.operator = OperatorKind::Official;
                self.rebuild();
                UiAction::None
            }
            WidgetId::Step2Back => {
                self.state.shell = OnboardingShell::Step1Operator;
                self.state.custom_host_focus = self.state.operator == OperatorKind::Custom;
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainPlayTogether => {
                self.state.home_status =
                    "PLAY TOGETHER session list is a stub (not available yet)".into();
                self.rebuild();
                UiAction::None
            }
            WidgetId::MainSoloHost => {
                self.state.shell = OnboardingShell::Dash;
                self.state.tab = DashTab::Home;
                self.state.home_create_open = false;
                self.state.home_status =
                    "SOLO — create or join (host must use the same AUTH/WORLD URLs)".into();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ExitClientOnly => UiAction::ExitClientOnly,
            WidgetId::ExitClientAndHost => UiAction::ExitClientAndHost,
            WidgetId::TurnModeToggle => {
                self.state.toggle_turn_mode();
                self.rebuild();
                log::info!("turn mode → {}", self.state.turn.mode.label());
                UiAction::None
            }
            WidgetId::HomeFieldIdentifier => {
                self.state.home_focus = Some(HomeField::Identifier);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldPassword => {
                self.state.home_focus = Some(HomeField::Password);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldUsername => {
                self.state.home_focus = Some(HomeField::Username);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldEmail => {
                self.state.home_focus = Some(HomeField::Email);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeFieldBirthday => {
                self.state.home_focus = Some(HomeField::Birthday);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeRememberMe => {
                self.state.home_remember_me = !self.state.home_remember_me;
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeTosAgreed => {
                self.state.home_tos_agreed = !self.state.home_tos_agreed;
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeModeLogin => {
                self.state.home_mode = HomeMode::Login;
                self.state.home_focus = Some(HomeField::Identifier);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeModeRegister => {
                self.state.home_mode = HomeMode::Register;
                self.state.home_focus = Some(HomeField::Username);
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
                UiAction::AuthLogout
            }
            WidgetId::HomeNewWorld => {
                self.state.home_create_open = true;
                self.state.home_focus = Some(HomeField::SessionName);
                self.state.home_status = if self.state.home_session_actions_enabled {
                    "Pick access level, then Start session (placeholder UI)".into()
                } else {
                    "Create panel (browse only) — Start/Join HTTP is Desktop-only".into()
                };
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeCreateBack => {
                self.state.home_create_open = false;
                self.state.home_focus = None;
                self.state.home_status = "Signed in — create or join a world".into();
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeCreateName => {
                self.state.home_focus = Some(HomeField::SessionName);
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeTemplateGrid => {
                self.rebuild();
                UiAction::None
            }
            WidgetId::HomeStartSession => {
                if self.state.home_session_actions_enabled {
                    UiAction::StartSession
                } else {
                    self.state.home_status = "Start session HTTP is Desktop-only this Slice".into();
                    self.rebuild();
                    UiAction::None
                }
            }
            WidgetId::HomeJoinRoom => {
                if self.state.home_session_actions_enabled {
                    UiAction::JoinRoom
                } else {
                    self.state.home_status = "Join room HTTP is Desktop-only this Slice".into();
                    self.rebuild();
                    UiAction::None
                }
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

    /// Desktop text into focused Home / onboarding field (ignored when dash closed).
    pub fn on_text_input(&mut self, text: &str) {
        if !self.state.open {
            return;
        }
        if self.state.shell == OnboardingShell::Step1Operator {
            self.state.append_home_text(text);
            self.rebuild();
            return;
        }
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash && self.state.tab != DashTab::Home)
        {
            return;
        }
        // Logged-in hub: no editable fields. Create panel: session name only.
        if self.state.home_logged_in_as.is_some() && !self.state.home_create_open {
            return;
        }
        if self.state.home_logged_in_as.is_some()
            && self.state.home_create_open
            && self.state.home_focus != Some(HomeField::SessionName)
        {
            return;
        }
        self.state.append_home_text(text);
        self.rebuild();
    }

    pub fn on_home_backspace(&mut self) {
        if !self.state.open {
            return;
        }
        if self.state.shell == OnboardingShell::Step1Operator {
            self.state.home_backspace();
            self.rebuild();
            return;
        }
        if !matches!(
            self.state.shell,
            OnboardingShell::Step2Auth | OnboardingShell::Dash
        ) || (self.state.shell == OnboardingShell::Dash && self.state.tab != DashTab::Home)
        {
            return;
        }
        self.state.home_backspace();
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
                self.rebuild();
                return UiAction::None;
            }
            self.state.custom_host_focus = false;
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
    /// Create panel: ensure visibility + guest join (x0037).
    StartSession,
    /// Hub: guest join current `ROOM_ID` without create (x0037).
    JoinRoom,
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
            .find(|(w, _)| *w == WidgetId::ExitClientOnly)
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
    fn main_hub_has_play_and_solo() {
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
            .any(|(id, _)| *id == WidgetId::MainPlayTogether));
        assert!(ui
            .tree
            .hits
            .iter()
            .any(|(id, _)| *id == WidgetId::MainSoloHost));
    }
}
