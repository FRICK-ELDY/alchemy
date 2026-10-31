//! system_ui — personal DashRoot + retained Canvas (x000f + x0018 Home + x0037 create + x0066 Connect).
//!
//! Guarantees: ESC/Menu toggle (via surface), hit-tested chrome, Exit Pinned Facet,
//! Home / Connect form strings (Desktop text input), Button hover/press color,
//! client exit / auth / start-session / Connect Apply action plumbing, Settings defaults (read-only).
//! Dash is personal — never published on Room `frame`.
//! Non-guarantees: OpenXR types inside this module, user Facet rearrange, Vault layout
//! persist, WorldRoot, XR virtual keyboard, auth/world HTTP (surface modules), final UI art.

mod dash;
pub mod facet;
mod font;
mod gpu;
mod icons;
mod layout;

pub use dash::{
    access_level_from_widget, ConnectField, ConnectMode, DashState, DashTab, HomeField, HomeMode,
    SESSION_NAME_MAX,
};
pub use gpu::UiGpu;
pub use layout::{LayoutTree, WidgetId};

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
                self.state.tab = DashTab::Exit;
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabSettings => {
                self.state.tab = DashTab::Settings;
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabHome => {
                self.state.tab = DashTab::Home;
                self.rebuild();
                UiAction::None
            }
            WidgetId::TabConnect => {
                self.state.tab = DashTab::Connect;
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectModeSimple => {
                self.state.set_connect_simple_mode();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectModeDetail => {
                self.state.set_connect_detail_mode();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectFieldDomain => {
                self.state.connect_focus = Some(ConnectField::Domain);
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectFieldAuth => {
                self.state.connect_focus = Some(ConnectField::Auth);
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectFieldAssets => {
                self.state.connect_focus = Some(ConnectField::Assets);
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectFieldWorld => {
                self.state.connect_focus = Some(ConnectField::World);
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectRelay0
            | WidgetId::ConnectRelay1
            | WidgetId::ConnectRelay2
            | WidgetId::ConnectRelay3 => {
                let row = match hit {
                    WidgetId::ConnectRelay0 => 0,
                    WidgetId::ConnectRelay1 => 1,
                    WidgetId::ConnectRelay2 => 2,
                    WidgetId::ConnectRelay3 => 3,
                    _ => 0,
                };
                if let Some(abs) = self.state.connect_relay_abs(row) {
                    self.state.connect_focus = Some(ConnectField::Relay(abs));
                }
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectSelectRelay0
            | WidgetId::ConnectSelectRelay1
            | WidgetId::ConnectSelectRelay2
            | WidgetId::ConnectSelectRelay3 => {
                let row = match hit {
                    WidgetId::ConnectSelectRelay0 => 0,
                    WidgetId::ConnectSelectRelay1 => 1,
                    WidgetId::ConnectSelectRelay2 => 2,
                    WidgetId::ConnectSelectRelay3 => 3,
                    _ => 0,
                };
                if let Some(abs) = self.state.connect_relay_abs(row) {
                    self.state.select_connect_relay(abs);
                }
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectRelayScrollUp => {
                self.state.scroll_relays_up();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectRelayScrollDown => {
                self.state.scroll_relays_down();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectAddRelay => {
                self.state.add_connect_relay();
                self.rebuild();
                UiAction::None
            }
            WidgetId::ConnectApply => UiAction::ConnectApply,
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

    /// Desktop text into focused Home / Connect field.
    pub fn on_text_input(&mut self, text: &str) {
        if !self.state.open {
            return;
        }
        match self.state.tab {
            DashTab::Home => {
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
            DashTab::Connect => {
                self.state.append_connect_text(text);
                self.rebuild();
            }
            _ => {}
        }
    }

    pub fn on_home_backspace(&mut self) {
        if !self.state.open {
            return;
        }
        match self.state.tab {
            DashTab::Home => {
                self.state.home_backspace();
                self.rebuild();
            }
            DashTab::Connect => {
                self.state.connect_backspace();
                self.rebuild();
            }
            _ => {}
        }
    }

    pub fn on_home_tab_key(&mut self) {
        if !self.state.open {
            return;
        }
        match self.state.tab {
            DashTab::Home => {
                self.state.cycle_home_focus();
                self.rebuild();
            }
            DashTab::Connect => {
                self.state.cycle_connect_focus();
                self.rebuild();
            }
            _ => {}
        }
    }

    /// Enter on Home: login/register / Start; on Connect: Apply.
    pub fn on_home_enter(&mut self) -> UiAction {
        if !self.state.open {
            return UiAction::None;
        }
        if self.state.tab == DashTab::Connect {
            return UiAction::ConnectApply;
        }
        if self.state.tab != DashTab::Home {
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
    /// Connect tab: persist + apply home-node endpoints (x0066).
    ConnectApply,
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
}
