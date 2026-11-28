//! Desktop winit + wgpu surface (ESC dash + Noclip + mouse look + Grab).
//!
//! Guarantees: canvas-pixel pointer → system_ui `on_pointer` (hover/press) + click;
//! dash-closed LMB → cursor grab (`Confined` then `Locked`); mouse Δ → client-local
//! `CameraLook`; WASD projected from current yaw; look-capture + aim-down then
//! LMB/RMB/E hold → Grab via look-forward ray (x0013). Floor boxes need pitch down.
//! Look is never on Zenoh.
//! Auth login/register/logout and room start/join HTTP run on a worker thread; the UI
//! thread only polls the outcome (x002e / P3-9 / x0037).
//! x0053 Companion: F8 toggles VR↔Desktop present; Intent/Grab only while
//! `DesktopRole::owns_input`; present handoff zeros shared `MoveIntent`;
//! exit mirrors XR `presentation.exit_requested`. Zenoh Session ownership stays on
//! Desktop (XR uses presentation flags); bind / leave / mTLS reopen / operator
//! endpoint retarget run in `session_flow` (x00d0). Companion still redraws while
//! `vr_active` (best-effort mirror — extra GPU cost, not a hotswitch guarantee).
//!
//! Session-enter overlay + deferred `bind_room_after_join` (Binding progress) are
//! **Desktop-only**; XR companion does not mirror that wait UX (non-guarantee).
//!
//! Layout (x00d0): this file keeps winit, input / look / Grab, render, Spatial Lens and the
//! `handle_ui_action` dispatch. `support` = leaf helpers (no flow deps), `job` = worker
//! epoch plumbing, `session_flow` = auth / session / Worlds / host, `inventory_flow` =
//! Inventory / Room Save, `hotswitch_flow` = x0053 Dash mirror + XR leave / exit.
//!
//! Linux/WSL: prefer Confined + `CursorMoved` (recenter). `Locked` often succeeds but
//! yields no `DeviceEvent::MouseMotion`, so look appears dead.

mod hotswitch_flow;
mod inventory_flow;
mod job;
mod session_flow;
mod support;

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use alchemy_protocol::input_buttons;
use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Cursor, CursorGrabMode, CustomCursor, Window, WindowId};

use crate::context_radial::{self, ContextEvent, ContextItemId, ContextRadialState};
use crate::host_supervisor;
use crate::mesh::CpuMesh;
use crate::operator_prefs;
use crate::pending_local_ops::PendingPose;
use crate::scene::{
    create_depth_view, create_wgpu_for_window, CameraLook, EntityPose, MoveIntent, SceneGpu,
    SharedState,
};
use crate::spatial_lens::extract::{
    entity_id_for_row, inspector_pos_axis, inspector_secondary_axis, world_pos_xyz, InspectorScrub,
    INSPECTOR_SCRUB_M_PER_PX, INSPECTOR_SCRUB_SECONDARY_DEADZONE_PX,
};
use crate::spatial_lens::inspector_edit::{expected_hash_for_axis, publish_inspector_pos_commit};
use crate::spatial_lens::{
    decode_hierarchy_binding, decode_inspector_pos_binding, pack_personal_lens,
    place_in_front_of_eye, row_is_live, scrub_selected_row, HandLaserState, InteractionLaserConfig,
    SpatialLensGpu, SpatialLensState,
};
use crate::surface::hotswitch::DesktopRole;
use crate::system_ui::{DashTab, LayoutTree, OnboardingShell, SystemUi, UiAction};
use crate::world_hit::{desktop_look_ray, pick_grabbable_desktop, Ray};

use job::AuthJobOutcome;
use session_flow::{PendingRoomBind, RoomBindDefer, WaitingSessionSnapshot};

/// Radians per pixel (fixed; Settings Facet deferred).
const LOOK_SENSITIVITY: f32 = 0.0025;
/// Match `SceneGpu::desktop_view_proj` pitch clamp (~±88°).
const PITCH_LIMIT: f32 = 1.54;

struct DesktopGpu {
    surface: wgpu::Surface<'static>,
    scene: SceneGpu,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    /// Spatial Lens Uber-SDF pipeline (x00ba); personal panels only.
    spatial_lens: SpatialLensGpu,
}

/// Context radial overlay for one Desktop frame (layout + optional aim pointer).
struct ContextOverlayFrame<'a> {
    tree: &'a LayoutTree,
    layout_dirty: bool,
    pointer: Option<(f32, f32)>,
}

impl DesktopGpu {
    async fn new(window: Arc<Window>, mesh: &CpuMesh, shared: Arc<SharedState>) -> Result<Self> {
        let (_instance, surface, _adapter, device, queue, config, format) =
            create_wgpu_for_window(Arc::clone(&window)).await?;
        let depth_view = create_depth_view(&device, config.width, config.height);
        let scene = SceneGpu::from_device(device, queue, format, mesh, shared)?;
        let spatial_lens = SpatialLensGpu::new(&scene.device, &scene.queue, format)?;
        Ok(Self {
            surface,
            scene,
            config,
            depth_view,
            spatial_lens,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.scene.device, &self.config);
        self.depth_view = create_depth_view(&self.scene.device, width, height);
    }

    fn render(
        &mut self,
        ui: &mut SystemUi,
        highlight_entity: Option<u32>,
        lens: &SpatialLensState,
        lens_headers: &[crate::spatial_lens::GpuPanelHeader],
        lens_prims: &[crate::spatial_lens::GpuUiPrim],
        context_overlay: Option<ContextOverlayFrame<'_>>,
    ) -> Result<()> {
        self.scene.maybe_log_pose();
        let eye = self.scene.user_root_pose();
        let look = self.scene.look();
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let (view, proj) = SceneGpu::desktop_view_proj(eye, look, aspect);

        let draw_context = context_overlay.is_some();
        if ui.state.open {
            ui.prepare_frame(&self.scene.queue);
        } else if let Some(overlay) = context_overlay {
            if let Some(gpu) = ui.gpu.as_mut() {
                gpu.present_context_overlay(
                    &self.scene.queue,
                    overlay.tree,
                    overlay.layout_dirty,
                    overlay.pointer,
                );
            }
        }

        let frame = self
            .surface
            .get_current_texture()
            .context("get_current_texture")?;
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.scene
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("desktop-encoder"),
                });

        // Once per frame (not inside draw_world — XR shares that path per-eye).
        self.scene.sync_epoch_gpu(&mut encoder);
        self.scene.draw_world(
            &mut encoder,
            &frame_view,
            &self.depth_view,
            view,
            proj,
            wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.05,
                g: 0.06,
                b: 0.09,
                a: 1.0,
            }),
            highlight_entity,
        );

        // Personal Lens only on Desktop (x00f6: Dash plate is XR — Desktop keeps fullscreen overlay).
        if lens.visible {
            self.spatial_lens.upload(
                &self.scene.device,
                &self.scene.queue,
                lens_headers,
                lens_prims,
            );
            self.spatial_lens.draw(
                &mut encoder,
                &self.scene.queue,
                &frame_view,
                &self.depth_view,
                view,
                proj,
            );
        }

        let draw_ui = ui.state.open || draw_context;
        if draw_ui {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui-overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if ui.state.open {
                ui.draw(&mut pass);
            } else {
                ui.draw_canvas(&mut pass);
            }
        }

        self.scene.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

struct App {
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    window: Option<Arc<Window>>,
    gpu: Option<DesktopGpu>,
    ui: SystemUi,
    zenoh: Option<zenoh::Session>,
    role: DesktopRole,
    cursor: (f64, f64),
    primary_down: bool,
    keys_down: HashSet<KeyCode>,
    /// True while cursor grab is active.
    look_captured: bool,
    /// Once raw device motion arrives, skip CursorMoved look (avoid double sensitivity).
    device_look_active: bool,
    /// Ignore one CursorMoved after warping to window center.
    skip_cursor_warp: bool,
    last_cursor: Option<(f64, f64)>,
    /// 1×1 transparent cursor (WSL often ignores `set_cursor_visible(false)` alone).
    blank_cursor: Option<CustomCursor>,
    /// Entity held by Desktop Grab (cleared on release or dash open).
    held_grab: Option<u32>,
    /// Aim-relative attach offset + rotation captured at Grab begin (host hold model).
    /// Aim is eye **position** (matches host Intent aim); mouse look does not move the
    /// predicted attach point until the eye translates.
    grab_attach_offset: Option<[f32; 3]>,
    grab_attach_rot: Option<[f32; 4]>,
    /// After host reject: block new Grab until the grab button is fully released.
    grab_needs_release: bool,
    /// RMB held.
    grab_down: bool,
    /// After look capture, require one LMB release before LMB can Grab (avoid capture-click grab).
    lmb_grab_armed: bool,
    /// Resonite-style Context radial (x00ce); independent of Dash.
    context_radial: ContextRadialState,
    context_tree: LayoutTree,
    /// Virtual pointer on radial canvas while look-captured (Δ from center).
    context_pointer: (f32, f32),
    /// Consecutive frames outside the radial leave pad (x00ce Desktop leave-close).
    context_leave_miss: u32,
    /// Layout changed → re-raster Context overlay once.
    context_gpu_dirty: bool,
    /// Rate-limit "grab miss" logs.
    last_grab_miss_log: Option<std::time::Instant>,
    /// In-flight Auth HTTP (+ join) worker: `(auth_work_epoch, rx)` (x002e / x007c).
    auth_job: Option<(u64, Receiver<AuthJobOutcome>)>,
    /// Waiting-shell host-session poll (x0080); off UI thread so ureq never blocks paint.
    waiting_session_job: Option<Receiver<WaitingSessionSnapshot>>,
    /// Last observed Desktop input ownership (READY / vr_active edges).
    last_owns_input: bool,
    /// Last applied [`SharedState::pending_mesh_epoch`] (x007d remount).
    mesh_epoch: u64,
    /// After Join/Host ticket OK: show "Binding room bus…" for one paint, then bind.
    pending_room_bind: Option<PendingRoomBind>,
    /// Set after "Binding…" has been drawn once; [`Self::poll_pending_room_bind`] then runs bind.
    room_bind_progress_painted: bool,
    /// Personal Spatial Lens (x00ba–x00bd). Toggle with `I`; activate row with `F`.
    spatial_lens: SpatialLensState,
    spatial_laser: HandLaserState,
    spatial_laser_cfg: InteractionLaserConfig,
    spatial_headers: Vec<crate::spatial_lens::GpuPanelHeader>,
    spatial_prims: Vec<crate::spatial_lens::GpuUiPrim>,
    /// Look ray currently intersects the Lens board (blocks world Grab start).
    spatial_lens_hit_panel: bool,
    spatial_lens_last_tick: Option<std::time::Instant>,
    /// `I` open already packed this frame — skip the next tick pack (avoid double walk).
    spatial_lens_fresh_pack: bool,
    /// Prior-frame `primary_down` for Inspector scrub rising-edge begin (x00f2).
    inspector_scrub_primary_was_down: bool,
}

impl App {
    fn owns_input(&self) -> bool {
        self.role.owns_input(&self.shared.presentation)
    }

    fn window_title(&self) -> &'static str {
        match self.role {
            DesktopRole::Solo => "Alchemy Client — Desktop",
            DesktopRole::Companion if !self.shared.presentation.xr_session_live() => {
                // XR init fail / exit → Solo-like.
                "Alchemy Client — Desktop"
            }
            DesktopRole::Companion if !self.shared.presentation.xr_session_started() => {
                "Alchemy Client — Desktop (XR starting…)"
            }
            DesktopRole::Companion if self.shared.presentation.vr_active() => {
                "Alchemy Client — VR active (F8 → Desktop)"
            }
            DesktopRole::Companion => "Alchemy Client — Desktop (F8 → VR)",
        }
    }

    fn refresh_window_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(self.window_title());
        }
    }

    fn sync_hotswitch_ui(&mut self) {
        let view_changed =
            hotswitch_flow::sync_hotswitch_ui_state(&mut self.ui, &self.shared, self.role);
        // Title depends on live|started|vr, not only owns_input — refresh with UI sync.
        self.refresh_window_title();
        if view_changed && self.ui.state.open {
            self.ui.rebuild();
        }
    }

    fn apply_look_delta(&mut self, dx: f64, dy: f64) {
        if !self.owns_input() || self.ui.state.open {
            return;
        }
        if self.context_radial.open {
            // Radial pointer steers with mouse; look keeps updating so WASD yaw stays live
            // while the menu is open (x00ce — radial must not suppress locomotion).
            const RADIAL_SENS: f32 = 1.6;
            let w = self.context_radial.canvas_w;
            let h = self.context_radial.canvas_h;
            self.context_pointer.0 =
                (self.context_pointer.0 + dx as f32 * RADIAL_SENS).clamp(0.0, w);
            self.context_pointer.1 =
                (self.context_pointer.1 + dy as f32 * RADIAL_SENS).clamp(0.0, h);
        }
        if !self.look_captured {
            return;
        }
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if let Ok(mut look) = self.shared.look.lock() {
            *look = apply_mouse_look(*look, dx, dy, LOOK_SENSITIVITY, PITCH_LIMIT);
        }
    }

    fn sync_context_radial_items(&mut self) {
        let save_ok = self.held_grab.is_some_and(|eid| {
            !self.ui.state.inventory_blocked()
                && self
                    .shared
                    .scene_meshes()
                    .iter()
                    .any(|(id, b)| *id == eid && b.len() == 64)
        });
        self.context_radial
            .set_items(context_radial::build_items(save_ok));
        // Surface why Save is grey while Grab is held (cuboid / not Live / no blake3).
        if self.context_radial.open && self.held_grab.is_some() && !save_ok {
            let hint = if self.ui.state.inventory_blocked() {
                "Save needs Live session"
            } else {
                "Save needs held mesh blake3 (cuboid boxes cannot Save)"
            };
            if self.context_radial.status.is_empty()
                || self.context_radial.status.starts_with("Save needs")
            {
                self.context_radial.set_status(hint);
            }
        }
    }

    fn sync_context_canvas_size(&mut self) {
        if let Some(gpu) = self.gpu.as_ref() {
            self.context_radial.canvas_w = gpu.config.width.max(1) as f32;
            self.context_radial.canvas_h = gpu.config.height.max(1) as f32;
        }
    }

    fn toggle_context_radial(&mut self) {
        if self.ui.state.open {
            return;
        }
        self.sync_context_canvas_size();
        self.context_radial.toggle_desktop();
        if self.context_radial.open {
            self.context_pointer = (
                self.context_radial.canvas_w * 0.5,
                self.context_radial.canvas_h * 0.5,
            );
            self.context_leave_miss = 0;
            self.context_radial
                .set_pointer(self.context_pointer.0, self.context_pointer.1);
            self.sync_context_radial_items();
            context_radial::rebuild_layout(&mut self.context_tree, &self.context_radial);
            self.context_radial.dirty = false;
            self.context_gpu_dirty = true;
            log::info!("context radial open");
        } else {
            self.context_leave_miss = 0;
            if let Some(gpu) = self.ui.gpu.as_mut() {
                gpu.clear_context_overlay_cache();
            }
            log::info!("context radial closed");
        }
    }

    fn close_context_radial(&mut self) {
        self.context_radial.close();
        self.context_leave_miss = 0;
        if let Some(gpu) = self.ui.gpu.as_mut() {
            gpu.clear_context_overlay_cache();
        }
    }

    fn tick_context_radial(&mut self) -> Option<UiAction> {
        if !self.context_radial.open {
            return None;
        }
        if self.ui.state.open {
            self.close_context_radial();
            return None;
        }
        self.sync_context_canvas_size();
        self.sync_context_radial_items();
        self.context_radial.primary_down = self.primary_down;
        self.context_radial
            .set_pointer(self.context_pointer.0, self.context_pointer.1);
        // Leave ring soft-pad → close (same idea as XR board leave).
        // While Grab is held, keep the menu so WASD reposition + Save stay usable.
        let grab_held = self.grab_button_held() || self.held_grab.is_some();
        if !grab_held
            && context_radial::pointer_left_menu(
                self.context_pointer.0,
                self.context_pointer.1,
                self.context_radial.canvas_w,
                self.context_radial.canvas_h,
            )
        {
            self.context_leave_miss = self.context_leave_miss.saturating_add(1);
            if self.context_leave_miss >= context_radial::LEAVE_MISS_FRAMES {
                self.close_context_radial();
                log::info!("context radial closed (pointer left menu)");
                return None;
            }
        } else {
            self.context_leave_miss = 0;
        }
        let ev = self.context_radial.tick_interaction();
        if self.context_radial.dirty {
            context_radial::rebuild_layout(&mut self.context_tree, &self.context_radial);
            self.context_radial.dirty = false;
            self.context_gpu_dirty = true;
        }
        match ev {
            ContextEvent::Fired(ContextItemId::Save) => {
                let eid = self.held_grab?;
                Some(UiAction::GrabRadialSave(eid))
            }
            ContextEvent::None => None,
        }
    }

    /// Context Save with status on the ring (fail keeps open · x00ce review).
    fn handle_context_save(&mut self, entity_id: u32) {
        if self.ui.state.inventory_blocked() {
            self.context_radial.set_status("Save needs Live session");
            context_radial::rebuild_layout(&mut self.context_tree, &self.context_radial);
            self.context_radial.dirty = false;
            self.context_gpu_dirty = true;
            return;
        }
        self.ui.state.scene_meshes = self.shared.scene_meshes();
        log::info!("context Save put_back begin entity={entity_id}");
        match inventory_flow::run_put_back_for_entities_result(
            &mut self.ui,
            &self.shared,
            &[entity_id],
        ) {
            Ok(msg) => {
                log::info!("context Save put_back ok: {msg}");
                self.context_radial.set_status(msg);
                self.close_context_radial();
            }
            Err(msg) => {
                log::warn!("context Save put_back failed: {msg}");
                self.context_radial.set_status(msg);
                context_radial::rebuild_layout(&mut self.context_tree, &self.context_radial);
                self.context_radial.dirty = false;
                self.context_gpu_dirty = true;
            }
        }
    }

    /// Fires when Desktop gains/loses Intent ownership (READY edge or vr_active).
    fn on_input_ownership_edge(&mut self) {
        let owns = self.owns_input();
        if owns == self.last_owns_input {
            return;
        }
        self.last_owns_input = owns;
        self.refresh_window_title();
        // Ownership edge: drop look / Grab and zero shared Intent (no ghost WASD/stick).
        self.set_look_capture(false);
        self.clear_grab_attach();
        self.grab_down = false;
        self.keys_down.clear();
        self.ui.clear_pointer();
        self.shared.clear_move_intent();
        if owns {
            log::info!("hotswitch: Desktop took input ownership");
        } else {
            log::info!("hotswitch: Desktop cleared Intent / yielded to VR");
        }
    }

    /// XR-requested leave / rejoin / host shutdown (Desktop is the Zenoh owner · x00d5).
    fn poll_presentation_requests(&mut self) {
        hotswitch_flow::poll_presentation_requests(
            &mut self.ui,
            &self.shared,
            &mut self.zenoh,
            &mut self.auth_job,
            session_flow::RoomBindDefer {
                pending: &mut self.pending_room_bind,
                painted: &mut self.room_bind_progress_painted,
            },
        );
    }

    fn toggle_hotswitch_manual(&mut self) {
        if hotswitch_flow::toggle_hotswitch_manual(self.role, &self.shared) {
            self.on_input_ownership_edge();
        }
    }

    /// x0068: Token refresh installed new mTLS PEMs → reopen Zenoh while joined.
    fn poll_mtls_reopen(&mut self) {
        session_flow::poll_mtls_reopen(&self.shared, &mut self.zenoh);
    }

    /// XR Inventory ops queued on SharedState (x007f). Desktop owns HTTP/Zenoh.
    fn drain_inventory_peer_actions(&mut self, event_loop: &ActiveEventLoop) {
        let pending = match self.shared.inventory_peer_actions.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => {
                log::warn!("inventory_peer_actions lock poisoned — XR queue dropped");
                Vec::new()
            }
        };
        for req in pending {
            let action = match inventory_flow::apply_inventory_peer_request(
                &mut self.ui,
                &self.shared,
                &req,
            ) {
                Some(a) => a,
                None => continue,
            };
            handle_ui_action(
                action,
                &mut self.ui,
                &self.shared,
                &mut self.zenoh,
                &mut self.auth_job,
                RoomBindDefer {
                    pending: &mut self.pending_room_bind,
                    painted: &mut self.room_bind_progress_painted,
                },
                event_loop,
            );
        }
    }

    /// Non-blocking poll of Auth worker (x002e). Safe to call every frame.
    /// Drains `EnterProgress` without clearing the job; finals clear it.
    fn poll_auth_job(&mut self) {
        session_flow::poll_auth_job(
            &mut self.ui,
            &self.shared,
            &mut self.auth_job,
            RoomBindDefer {
                pending: &mut self.pending_room_bind,
                painted: &mut self.room_bind_progress_painted,
            },
        );
    }

    fn apply_hidden_cursor(&self, window: &Window) {
        window.set_cursor_visible(false);
        if let Some(blank) = self.blank_cursor.as_ref() {
            window.set_cursor(Cursor::Custom(blank.clone()));
        }
    }

    fn apply_default_cursor(&self, window: &Window) {
        window.set_cursor(Cursor::Icon(winit::window::CursorIcon::Default));
        window.set_cursor_visible(true);
    }

    fn warp_cursor_center(&mut self, mark_skip: bool) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        if size.width < 2 || size.height < 2 {
            return;
        }
        let cx = size.width as f64 * 0.5;
        let cy = size.height as f64 * 0.5;
        if window
            .set_cursor_position(PhysicalPosition::new(cx, cy))
            .is_ok()
        {
            if mark_skip {
                self.skip_cursor_warp = true;
            }
            self.last_cursor = Some((cx, cy));
            self.cursor = (cx, cy);
        }
    }

    /// Re-assert hide + soft-confine. WSL Confined often reports Ok but still leaks.
    fn maintain_look_capture(&mut self) {
        if !self.look_captured || self.ui.state.open {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        let _ = window
            .set_cursor_grab(CursorGrabMode::Confined)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Locked));
        self.apply_hidden_cursor(&window);
        // Warping while Context is open flings the radial pointer (leave-close / aim jump).
        if self.context_radial.open {
            return;
        }
        // Pull back only if drifted; avoid perpetual skip_cursor_warp (kills look).
        let size = window.inner_size();
        let cx = size.width as f64 * 0.5;
        let cy = size.height as f64 * 0.5;
        let (x, y) = self.cursor;
        let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
        if dist_sq > 36.0 {
            self.warp_cursor_center(true);
        }
    }

    fn set_look_capture(&mut self, capture: bool) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if capture {
            // Confined first: keeps CursorMoved on Linux/WSL. Locked often swallows motion
            // without DeviceEvent::MouseMotion (look appears frozen after "capture on").
            let mode = match window.set_cursor_grab(CursorGrabMode::Confined) {
                Ok(()) => CursorGrabMode::Confined,
                Err(err) => {
                    log::warn!("cursor Confined failed ({err}); trying Locked");
                    match window.set_cursor_grab(CursorGrabMode::Locked) {
                        Ok(()) => CursorGrabMode::Locked,
                        Err(err2) => {
                            log::warn!("cursor Locked failed: {err2}");
                            self.look_captured = false;
                            self.apply_default_cursor(&window);
                            return;
                        }
                    }
                }
            };
            self.apply_hidden_cursor(&window);
            self.look_captured = true;
            self.lmb_grab_armed = false;
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.warp_cursor_center(true);
            log::info!(
                "mouse look capture on ({mode:?}); aim at a box (look down) then hold LMB/RMB/E"
            );
        } else if self.look_captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            self.apply_default_cursor(&window);
            self.look_captured = false;
            self.lmb_grab_armed = false;
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.last_cursor = None;
            log::info!("mouse look capture off");
        }
    }

    fn grab_button_held(&self) -> bool {
        self.grab_down
            || (self.primary_down && self.lmb_grab_armed)
            || self.keys_down.contains(&KeyCode::KeyE)
    }

    fn aim_highlight(&self) -> Option<u32> {
        if let Some(id) = self.held_grab {
            return Some(id);
        }
        if !self.look_captured || self.ui.state.open {
            return None;
        }
        let (eye, look) = self.eye_and_look();
        let ray = desktop_look_ray(eye, look);
        let (boxes, defs) = self.grabbable_boxes();
        pick_grabbable_desktop(ray, &boxes, &defs.half_extents).map(|h| h.entity_id)
    }

    fn on_cursor_moved_look(&mut self, x: f64, y: f64) {
        if !self.look_captured || self.ui.state.open || self.device_look_active {
            self.cursor = (x, y);
            return;
        }
        if self.skip_cursor_warp {
            self.skip_cursor_warp = false;
            self.last_cursor = Some((x, y));
            self.cursor = (x, y);
            return;
        }
        if let Some((lx, ly)) = self.last_cursor {
            self.apply_look_delta(x - lx, y - ly);
        }
        self.last_cursor = Some((x, y));
        self.cursor = (x, y);
        self.warp_cursor_center(true);
    }

    fn eye_and_look(&self) -> (EntityPose, CameraLook) {
        // Display snap (x0092): Redraw path uses begin_display_frame cache; mid-frame
        // rebuild_intent reuses that cache (or ensure freeze if cache was cleared).
        let eye = self.shared.user_root_pose();
        let look = self.shared.look.lock().ok().map(|g| *g).unwrap_or_default();
        (eye, look)
    }

    fn grabbable_boxes(
        &self,
    ) -> (
        Vec<(u32, EntityPose)>,
        std::sync::Arc<crate::scene::BoxDefs>,
    ) {
        // Display TRS (epoch α twin) so OBB matches mesh/box draw (x0091).
        self.shared.grabbable_display_poses()
    }

    fn clear_grab_attach(&mut self) {
        self.held_grab = None;
        self.grab_attach_offset = None;
        self.grab_attach_rot = None;
        // Drop prediction override immediately (do not wait for next predict_held_grab(None)).
        self.shared.clear_held_grab_prediction();
    }

    /// Pack Hierarchy/Inspector prims + Stickiness hover (x00bb–x00bd).
    /// Desktop Dash stays fullscreen CPU overlay (x00f6 plate path is XR-only until U0.1).
    fn tick_spatial_lens(&mut self) {
        if !self.spatial_lens.visible {
            if let Some(prev) = self.spatial_lens.scrub.take() {
                self.shared.pending_local.clear(prev.entity_id);
            }
            self.spatial_lens.soft_lease = None;
            self.spatial_headers.clear();
            self.spatial_prims.clear();
            self.spatial_lens.hovered_binding = 0;
            self.spatial_lens_hit_panel = false;
            self.spatial_lens_last_tick = None;
            self.spatial_lens_fresh_pack = false;
            self.inspector_scrub_primary_was_down = false;
            return;
        }
        let now = std::time::Instant::now();
        let dt = self
            .spatial_lens_last_tick
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(1.0 / 60.0)
            .clamp(1.0 / 240.0, 0.05);
        self.spatial_lens_last_tick = Some(now);

        if let Some(eid) = scrub_selected_row(&mut self.spatial_lens, &self.shared.epoch) {
            self.shared.pending_local.clear(eid);
        }
        crate::epoch_view::scrub_xray_row(&self.shared.epoch);
        // Intentional: while Lens holds a live selection, it owns Tape X-Ray
        // (Settings Hierarchy click also writes xray_row; Lens wins each tick when selected).
        if let Some(row) = self.spatial_lens.selected_row {
            self.shared.epoch.set_xray_row(Some(row));
        }
        self.refresh_soft_lease_chrome();

        // DashState.laser is SoT (Settings + chrome sync · x00ee).
        self.spatial_laser_cfg = self.ui.state.laser;
        let (eye, look) = self.eye_and_look();
        let ray = desktop_look_ray(eye, look);
        let dir = [ray.dir.x, ray.dir.y, ray.dir.z];
        let _ = self
            .spatial_laser
            .update_smoothed_direction(dir, &self.spatial_laser_cfg, dt);

        // Pack unless `I` already packed this frame. Hover fill uses prior
        // `hovered_binding` (1-frame lag by design — avoids a second hierarchy walk).
        let skip_pack = self.spatial_lens_fresh_pack;
        self.spatial_lens_fresh_pack = false;
        if !skip_pack {
            let _ = pack_personal_lens(
                &mut self.spatial_lens,
                &self.shared.epoch,
                &mut self.spatial_headers,
                &mut self.spatial_prims,
            );
        }

        let smooth_ray = Ray {
            origin: ray.origin,
            dir: glam::Vec3::from_array(self.spatial_laser.smoothed_dir).normalize_or_zero(),
        };
        let hit =
            crate::spatial_lens::pick_panel(smooth_ray, &self.spatial_headers, &self.spatial_prims);
        self.spatial_lens_hit_panel = hit.is_some();
        let (raw_uv, hovered, rect) = match hit {
            Some(h) => (h.uv_px, h.binding_id, h.rect_px),
            None => ([0.0, 0.0], 0, [0.0; 4]),
        };
        let trigger = if self.primary_down { 0.5 } else { 0.0 };
        let (_sticky_uv, binding) = self.spatial_laser.apply_stickiness_and_press_lock(
            raw_uv,
            hovered,
            rect,
            trigger,
            &self.spatial_laser_cfg,
            dt,
        );
        self.spatial_lens.hovered_binding = binding;
        // Scrub UV is independent of stickiness/press-lock: panel hit → live UV;
        // miss while primary held → last-good UV (never miss sentinel `[0,0]`).
        let panel_hit = hit.as_ref().map(|h| (h.uv_px, h.binding_id));
        self.tick_inspector_scrub(panel_hit);
    }

    /// SoftLease chrome for selected entity (x00f3). Physical Grab 非対象.
    fn refresh_soft_lease_chrome(&mut self) {
        use crate::spatial_lens::extract::SoftLeaseChrome;
        let Some(row) = self.spatial_lens.selected_row else {
            self.spatial_lens.soft_lease = None;
            return;
        };
        let Some(entity_id) = entity_id_for_row(&self.shared.epoch, row) else {
            self.spatial_lens.soft_lease = None;
            return;
        };
        let now = std::time::Instant::now();
        let holder = self
            .shared
            .soft_lease_hints
            .lock()
            .ok()
            .and_then(|mut t| t.holder_of(entity_id, now));
        let Some(holder_actor_id) = holder else {
            self.spatial_lens.soft_lease = None;
            return;
        };
        let local_actor = self
            .shared
            .auth
            .lock()
            .ok()
            .and_then(|g| g.local_sub.clone())
            .map(|sub| alchemy_shared_sim::actor_id_from_sub(&sub));
        let is_mine = local_actor == Some(holder_actor_id);
        self.spatial_lens.soft_lease = Some(SoftLeaseChrome {
            holder_actor_id,
            is_mine,
        });
    }

    /// Desktop Inspector POS scrub: drag updates draft; release → 1× proposal (x00f2 / x00f4).
    ///
    /// UV source while scrubbing: **panel hit UV** (not press-lock). On miss with
    /// primary held, reuse [`InspectorScrub::last_uv`]. Begin only on rising edge
    /// with a real panel hit on a POS field.
    ///
    /// 2D scrub (x00f4): `Δu` → primary axis; `Δv` → cyclic next axis.
    fn tick_inspector_scrub(&mut self, panel_hit: Option<([f32; 2], u32)>) {
        let rising = self.primary_down && !self.inspector_scrub_primary_was_down;
        self.inspector_scrub_primary_was_down = self.primary_down;

        if self.primary_down {
            if self.spatial_lens.scrub.is_none() && rising {
                let Some((uv, binding)) = panel_hit else {
                    return;
                };
                if decode_inspector_pos_binding(binding).is_none() {
                    return;
                }
                let Some(row) = self.spatial_lens.selected_row else {
                    return;
                };
                if inspector_pos_axis(binding).is_none() {
                    return;
                }
                let Some(entity_id) = entity_id_for_row(&self.shared.epoch, row) else {
                    return;
                };
                let Some(base_xyz) = world_pos_xyz(&self.shared.epoch, row) else {
                    return;
                };
                let mut expected_hash = [[0u8; 32]; 3];
                for (i, &v) in base_xyz.iter().enumerate() {
                    expected_hash[i] = expected_hash_for_axis(v);
                }
                self.spatial_lens.scrub = Some(InspectorScrub {
                    field_binding: binding,
                    row,
                    entity_id,
                    start_uv: uv,
                    last_uv: uv,
                    base_xyz,
                    draft_xyz: base_xyz,
                    dirty: [false; 3],
                    expected_hash,
                });
            }

            let preview = {
                let Some(scrub) = self.spatial_lens.scrub.as_mut() else {
                    return;
                };
                let Some(primary) = inspector_pos_axis(scrub.field_binding) else {
                    return;
                };
                let secondary = inspector_secondary_axis(primary);
                let uv = match panel_hit {
                    Some((uv, _)) => {
                        scrub.last_uv = uv;
                        uv
                    }
                    None => scrub.last_uv,
                };
                let du = (uv[0] - scrub.start_uv[0]) as f64;
                let dv = (uv[1] - scrub.start_uv[1]) as f64;
                // Dirty vs press-time base (Commit absolute). Preview blends Curr for
                // non-dirty axes so physics drift follows until release (x00f2 / x00f4).
                scrub.dirty = [false; 3];
                let mut primary_v = scrub.base_xyz[primary] + du * INSPECTOR_SCRUB_M_PER_PX;
                if !primary_v.is_finite() {
                    primary_v = scrub.base_xyz[primary];
                }
                if (primary_v - scrub.base_xyz[primary]).abs() > f64::EPSILON {
                    scrub.dirty[primary] = true;
                    scrub.draft_xyz[primary] = primary_v;
                } else {
                    scrub.draft_xyz[primary] = scrub.base_xyz[primary];
                }
                let secondary_v = scrub.base_xyz[secondary] + dv * INSPECTOR_SCRUB_M_PER_PX;
                if secondary_v.is_finite()
                    && dv.abs() >= INSPECTOR_SCRUB_SECONDARY_DEADZONE_PX
                    && (secondary_v - scrub.base_xyz[secondary]).abs() > f64::EPSILON
                {
                    scrub.dirty[secondary] = true;
                    scrub.draft_xyz[secondary] = secondary_v;
                } else {
                    scrub.draft_xyz[secondary] = scrub.base_xyz[secondary];
                }
                // Third axis stays at base for Commit; Curr overlay happens below.
                let third = 3 - primary - secondary;
                scrub.dirty[third] = false;
                scrub.draft_xyz[third] = scrub.base_xyz[third];
                (scrub.entity_id, scrub.row, scrub.draft_xyz, scrub.dirty)
            };
            let (entity_id, row, draft_xyz, dirty) = preview;
            // Non-finite / missing Curr → drop scrub (no stale Commit on release).
            let Some(curr) = world_pos_xyz(&self.shared.epoch, row) else {
                self.spatial_lens.scrub = None;
                self.shared.pending_local.clear(entity_id);
                return;
            };
            // Preview: Curr for clean axes, draft for dirty (f32 PendingPose; Commit f64).
            let mut pose = PendingPose::xyz(curr[0] as f32, curr[1] as f32, curr[2] as f32);
            if dirty[0] {
                pose.x = draft_xyz[0] as f32;
            }
            if dirty[1] {
                pose.y = draft_xyz[1] as f32;
            }
            if dirty[2] {
                pose.z = draft_xyz[2] as f32;
            }
            self.shared.pending_local.push_pose(entity_id, pose);
            return;
        }

        // Release: refresh expected_hash from live Epoch (physics drift OK; Inspector
        // absolute draft wins). SoftLease still serializes multi-actor structural edits;
        // physical Grab is a separate SSoT (not detected by this hash).
        let Some(mut scrub) = self.spatial_lens.scrub.take() else {
            return;
        };
        self.shared.pending_local.clear(scrub.entity_id);
        if !scrub.dirty.iter().any(|&d| d) {
            return;
        }
        if let Some(cur) = world_pos_xyz(&self.shared.epoch, scrub.row) {
            for (i, &is_dirty) in scrub.dirty.iter().enumerate() {
                if is_dirty {
                    scrub.expected_hash[i] = expected_hash_for_axis(cur[i]);
                }
            }
        }
        let Some(zenoh) = self.zenoh.as_ref() else {
            log::warn!("inspector scrub commit skipped: no zenoh");
            return;
        };
        if let Err(err) = publish_inspector_pos_commit(&self.shared, zenoh, &scrub) {
            log::warn!("inspector scrub commit failed: {err:#}");
        }
    }

    /// `F` while lens visible: select hierarchy row under sticky binding.
    fn activate_spatial_lens_binding(&mut self) {
        if let Some(eid) = scrub_selected_row(&mut self.spatial_lens, &self.shared.epoch) {
            self.shared.pending_local.clear(eid);
        }
        let binding = self.spatial_lens.hovered_binding;
        if let Some(row) = decode_hierarchy_binding(binding) {
            if row_is_live(&self.shared.epoch, row) {
                if let Some(prev) = self.spatial_lens.scrub.take() {
                    self.shared.pending_local.clear(prev.entity_id);
                }
                self.spatial_lens.selected_row = Some(row);
                self.shared.epoch.set_xray_row(Some(row));
                log::info!("spatial_lens select row={row}");
            }
        }
    }
}

/// Desktop UI latch (x00ab): after reject, block new Grab until the button is released.
///
/// Kept on the surface layer (not prediction core) so XR can diverge later.
pub(crate) fn desktop_grab_may_start(
    needs_release: &mut bool,
    button_held: bool,
    just_rejected: bool,
) -> bool {
    if just_rejected {
        *needs_release = true;
    }
    if !button_held {
        *needs_release = false;
    }
    button_held && !*needs_release
}

impl App {
    /// Update held Grab from LMB/RMB/E + look-forward pick (x0013).
    ///
    /// Spec (x00ab): after host reject, **re-Grab is forbidden until the grab button
    /// is released** (no press-and-hold retry loop / Intent spam).
    ///
    /// x00f4 / F1-3: dense grab coords stay on Intent (not `…/edit`). Release does
    /// **not** SetField-commit pose (Epoch/Frame SSoT; Grab release Commit = 後続).
    fn update_grab_hold(&mut self) {
        // Host reject Snap (x00ab heuristic until Phase 3 acked_intent_seq / holder).
        let just_rejected = self
            .shared
            .owned_prediction
            .lock()
            .ok()
            .is_some_and(|mut o| o.take_grab_rejected());
        if just_rejected {
            if self.held_grab.take().is_some() {
                log::info!("desktop grab released (host reject)");
            }
            self.grab_attach_offset = None;
            self.grab_attach_rot = None;
            self.shared.clear_held_grab_prediction();
        }
        let button = self.grab_button_held();
        let may_start = desktop_grab_may_start(&mut self.grab_needs_release, button, just_rejected);
        if self.ui.state.open || !self.look_captured {
            if self.held_grab.is_some() {
                log::info!("desktop grab released (dash/look)");
                self.clear_grab_attach();
            }
            return;
        }
        // Context radial owns LMB as Primary — freeze Grab begin/release while open (x00ce review).
        if self.context_radial.open {
            return;
        }
        if button {
            // Looking at Spatial Lens board: do not start world Grab through the panel.
            if self.held_grab.is_none() && may_start && !self.spatial_lens_hit_panel {
                let (eye, look) = self.eye_and_look();
                let ray = desktop_look_ray(eye, look);
                let (boxes, defs) = self.grabbable_boxes();
                if let Some(hit) = pick_grabbable_desktop(ray, &boxes, &defs.half_extents) {
                    // Same `boxes` as pick — missing id is a logic bug; do not invent origin attach.
                    let Some((_, display)) =
                        boxes.iter().find(|(id, _)| *id == hit.entity_id).copied()
                    else {
                        log::warn!(
                            "desktop grab: pick entity={} missing from display pose list",
                            hit.entity_id
                        );
                        return;
                    };
                    // Host hold model: offset = display entity − aim (eye **position**).
                    let offset = [display.x - eye.x, display.y - eye.y, display.z - eye.z];
                    let rot = [display.qx, display.qy, display.qz, display.qw];
                    let pred = crate::owned_prediction::PredictionPose::xyz_quat(
                        display.x, display.y, display.z, rot[0], rot[1], rot[2], rot[3],
                    );
                    // Only mirror Desktop held state when prediction begin succeeds
                    // (avoids Intent GRAB while grab_rejected made begin_held a no-op).
                    if self.shared.begin_held_grab(hit.entity_id, pred) {
                        self.grab_attach_offset = Some(offset);
                        self.grab_attach_rot = Some(rot);
                        self.held_grab = Some(hit.entity_id);
                        log::info!("desktop grab begin entity={}", hit.entity_id);
                    } else if self
                        .shared
                        .owned_prediction
                        .lock()
                        .ok()
                        .is_some_and(|mut o| o.take_grab_rejected())
                    {
                        self.grab_needs_release = true;
                        log::info!("desktop grab begin blocked (pending host reject)");
                    } else {
                        // Lock poison / transient false: leave needs_release alone; retry next tick.
                        log::warn!("desktop grab begin skipped (prediction lock/begin failed)");
                    }
                } else {
                    let now = std::time::Instant::now();
                    let should_log = self
                        .last_grab_miss_log
                        .map(|t| now.duration_since(t).as_secs_f32() > 1.0)
                        .unwrap_or(true);
                    if should_log {
                        self.last_grab_miss_log = Some(now);
                        log::info!(
                            "desktop grab miss — look down at a floor box until it brightens, then hold LMB/RMB/E"
                        );
                    }
                }
            }
        } else if self.held_grab.is_some() {
            log::info!("desktop grab released");
            self.clear_grab_attach();
        }
    }

    fn rebuild_intent(&mut self) {
        if !self.owns_input() {
            // XR owns Intent while VR-present. Handoff zeros Intent in
            // `on_input_ownership_edge` / XR edge — do not write here.
            return;
        }
        self.update_grab_hold();

        // Context radial must not suppress locomotion (unlike Dash · x00ce).
        let dash_open = self.ui.state.open;
        let (eye, look) = self.eye_and_look();
        let yaw = look.yaw;
        let forward_x = yaw.sin();
        let forward_z = -yaw.cos();
        let right_x = yaw.cos();
        let right_z = yaw.sin();

        let mut intent = MoveIntent {
            suppress: dash_open,
            ..Default::default()
        };

        if !dash_open {
            let mut local_x = 0.0_f32;
            let mut local_z = 0.0_f32;
            let mut local_y = 0.0_f32;

            if self.keys_down.contains(&KeyCode::KeyW) {
                local_z += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyS) {
                local_z -= 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyD) {
                local_x += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyA) {
                local_x -= 1.0;
            }
            if self.keys_down.contains(&KeyCode::Space) {
                local_y += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyC) {
                local_y -= 1.0;
            }

            intent.move_x = local_x * right_x + local_z * forward_x;
            intent.move_z = local_x * right_z + local_z * forward_z;
            intent.move_y = local_y;

            let horiz = (intent.move_x * intent.move_x + intent.move_z * intent.move_z).sqrt();
            if horiz > 1.0 {
                intent.move_x /= horiz;
                intent.move_z /= horiz;
            }

            let mut buttons = 0u32;
            if self.keys_down.contains(&KeyCode::KeyY) {
                buttons |= input_buttons::PRIMARY;
            }
            if self.keys_down.contains(&KeyCode::KeyT) {
                buttons |= input_buttons::CONTEXT;
            }
            if self.keys_down.contains(&KeyCode::KeyR) {
                buttons |= input_buttons::SECONDARY;
            }
            if self.keys_down.contains(&KeyCode::Space) {
                buttons |= input_buttons::JUMP;
            }
            if let Some(id) = self.held_grab {
                buttons |= input_buttons::GRAB;
                intent.grab_entity_id = id;
                intent.aim_x = eye.x;
                intent.aim_y = eye.y;
                intent.aim_z = eye.z;
            }
            intent.buttons = buttons;
        }

        // x00ab: predict only (aim + attach offset). Reconcile runs on Applied Frame.
        let _ = look; // aim uses eye; look reserved for future controller aim
        let predicted_held = match (
            self.held_grab,
            self.grab_attach_offset,
            self.grab_attach_rot,
        ) {
            (Some(_), Some(off), Some(rot)) => {
                Some(crate::owned_prediction::PredictionPose::xyz_quat(
                    eye.x + off[0],
                    eye.y + off[1],
                    eye.z + off[2],
                    rot[0],
                    rot[1],
                    rot[2],
                    rot[3],
                ))
            }
            _ => None,
        };
        self.shared
            .predict_held_grab(self.held_grab, predicted_held);

        if let Ok(mut guard) = self.shared.intent.lock() {
            *guard = intent;
        }
    }
}

/// Apply raw mouse delta to look. `dx` right / `dy` down in pixels.
/// Positive `dx` increases yaw (turn right toward +X when looking down −Z).
pub(crate) fn apply_mouse_look(
    look: CameraLook,
    dx: f64,
    dy: f64,
    sensitivity: f32,
    pitch_limit: f32,
) -> CameraLook {
    CameraLook {
        yaw: look.yaw + (dx as f32) * sensitivity,
        pitch: (look.pitch - (dy as f32) * sensitivity).clamp(-pitch_limit, pitch_limit),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if self.blank_cursor.is_none() {
            match CustomCursor::from_rgba(vec![0, 0, 0, 0], 1, 1, 0, 0) {
                Ok(source) => {
                    self.blank_cursor = Some(event_loop.create_custom_cursor(source));
                }
                Err(err) => log::warn!("blank cursor rgba failed: {err}"),
            }
        }
        let attrs = Window::default_attributes()
            .with_title(self.window_title())
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gpu = pollster::block_on(DesktopGpu::new(
            Arc::clone(&window),
            &self.mesh,
            Arc::clone(&self.shared),
        ))
        .expect("init gpu");
        self.ui
            .init_gpu(
                &gpu.scene.device,
                &gpu.scene.queue,
                gpu.scene.surface_format,
                gpu.config.width,
                gpu.config.height,
            )
            .expect("init system_ui gpu");
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.sync_hotswitch_ui();
        self.ui.rebuild();
        log::info!(
            "system_ui ready — Esc dash; MMB/T context radial; F8 hotswitch (XR companion); click to look; Grab LMB/RMB/E; WASD+Space+C"
        );
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.context_radial.open && !self.ui.state.open {
                // Look-captured: Device Δ only (set device_look_active so CursorMoved
                // does not double-add). Uncaptured: ignore Device — absolute CursorMoved only.
                if self.look_captured {
                    self.device_look_active = true;
                    self.apply_look_delta(delta.0, delta.1);
                }
            } else if self.look_captured && !self.ui.state.open {
                self.device_look_active = true;
                self.apply_look_delta(delta.0, delta.1);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                crate::laser_prefs::flush_blocking();
                self.shared.presentation.request_exit();
                event_loop.exit();
            }
            WindowEvent::Focused(false) => {
                self.set_look_capture(false);
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.ui.resize(&gpu.scene.device, size.width, size.height);
                }
                if self.context_radial.open {
                    let prev_w = self.context_radial.canvas_w.max(1.0);
                    let prev_h = self.context_radial.canvas_h.max(1.0);
                    self.sync_context_canvas_size();
                    let sx = self.context_radial.canvas_w / prev_w;
                    let sy = self.context_radial.canvas_h / prev_h;
                    self.context_pointer.0 *= sx;
                    self.context_pointer.1 *= sy;
                    self.context_radial.dirty = true;
                    self.context_gpu_dirty = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.ui.state.open {
                    self.cursor = (position.x, position.y);
                    self.ui.on_pointer(
                        self.cursor.0 as f32,
                        self.cursor.1 as f32,
                        self.primary_down,
                    );
                } else if self.context_radial.open && self.look_captured {
                    // Same dual path as look: DeviceEvent sets device_look_active; if it never
                    // arrives (Linux/WSL Confined), CursorMoved relative is the fallback.
                    self.on_cursor_moved_look(position.x, position.y);
                } else if self.context_radial.open {
                    // Uncaptured: absolute cursor → radial canvas only.
                    self.cursor = (position.x, position.y);
                    self.context_pointer = (
                        (position.x as f32).clamp(0.0, self.context_radial.canvas_w),
                        (position.y as f32).clamp(0.0, self.context_radial.canvas_h),
                    );
                } else if self.look_captured {
                    self.on_cursor_moved_look(position.x, position.y);
                } else {
                    self.cursor = (position.x, position.y);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.ui.state.open {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -y * 48.0,
                        MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                    };
                    self.ui
                        .on_scroll(self.cursor.0 as f32, self.cursor.1 as f32, dy);
                } else if self.spatial_lens.visible {
                    let steps = match delta {
                        MouseScrollDelta::LineDelta(_, y) => {
                            if y > 0.0 {
                                -1isize
                            } else if y < 0.0 {
                                1
                            } else {
                                0
                            }
                        }
                        MouseScrollDelta::PixelDelta(p) => {
                            if p.y > 0.0 {
                                -1
                            } else if p.y < 0.0 {
                                1
                            } else {
                                0
                            }
                        }
                    };
                    if steps != 0 {
                        // Clamp happens inside the next `pack_personal_lens` walk.
                        let next = self.spatial_lens.hierarchy_scroll as isize + steps;
                        self.spatial_lens.hierarchy_scroll = next.max(0) as usize;
                        // Open-frame `fresh_pack` would skip that walk — force a pack next tick.
                        self.spatial_lens_fresh_pack = false;
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                self.primary_down = pressed;
                if self.ui.state.open {
                    self.ui.on_pointer(
                        self.cursor.0 as f32,
                        self.cursor.1 as f32,
                        self.primary_down,
                    );
                    if self.primary_down {
                        let action = self.ui.on_click(self.cursor.0 as f32, self.cursor.1 as f32);
                        handle_ui_action(
                            action,
                            &mut self.ui,
                            &self.shared,
                            &mut self.zenoh,
                            &mut self.auth_job,
                            RoomBindDefer {
                                pending: &mut self.pending_room_bind,
                                painted: &mut self.room_bind_progress_painted,
                            },
                            event_loop,
                        );
                    }
                } else if self.context_radial.open {
                    self.context_radial.primary_down = pressed;
                    if let Some(UiAction::GrabRadialSave(eid)) = self.tick_context_radial() {
                        self.handle_context_save(eid);
                    }
                } else if pressed && !self.look_captured && self.owns_input() {
                    self.set_look_capture(true);
                } else if !pressed && self.look_captured {
                    // Arm Grab on LMB only after the capture click is released.
                    self.lmb_grab_armed = true;
                }
                self.rebuild_intent();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => {
                if state == ElementState::Pressed && self.owns_input() && !self.ui.state.open {
                    self.toggle_context_radial();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                if self.owns_input() {
                    self.grab_down = state == ElementState::Pressed;
                }
                self.rebuild_intent();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if pressed {
                        self.keys_down.insert(code);
                    } else {
                        self.keys_down.remove(&code);
                    }
                    if pressed && !event.repeat && code == KeyCode::F8 {
                        self.toggle_hotswitch_manual();
                    }
                    if pressed && !event.repeat && code == KeyCode::KeyI && !self.ui.state.open {
                        self.spatial_lens.visible = !self.spatial_lens.visible;
                        if self.spatial_lens.visible {
                            let (eye, look) = self.eye_and_look();
                            place_in_front_of_eye(&mut self.spatial_lens, eye, look, 1.4);
                            let ray = desktop_look_ray(eye, look);
                            self.spatial_laser
                                .snap_direction([ray.dir.x, ray.dir.y, ray.dir.z]);
                            if let Some(eid) =
                                scrub_selected_row(&mut self.spatial_lens, &self.shared.epoch)
                            {
                                self.shared.pending_local.clear(eid);
                            }
                            // SoftLease chrome before first pack (avoid 1–2 frame lag · x00f3).
                            self.refresh_soft_lease_chrome();
                            let _ = pack_personal_lens(
                                &mut self.spatial_lens,
                                &self.shared.epoch,
                                &mut self.spatial_headers,
                                &mut self.spatial_prims,
                            );
                            let smooth_ray = Ray {
                                origin: ray.origin,
                                dir: glam::Vec3::from_array(self.spatial_laser.smoothed_dir)
                                    .normalize_or_zero(),
                            };
                            // Same forward as place/snap — Grab gate from real pick, not assumed.
                            self.spatial_lens_hit_panel = crate::spatial_lens::pick_panel(
                                smooth_ray,
                                &self.spatial_headers,
                                &self.spatial_prims,
                            )
                            .is_some();
                            self.spatial_lens_fresh_pack = true;
                            self.spatial_lens_last_tick = Some(std::time::Instant::now());
                        } else {
                            if let Some(prev) = self.spatial_lens.scrub.take() {
                                self.shared.pending_local.clear(prev.entity_id);
                            }
                            self.inspector_scrub_primary_was_down = false;
                            self.spatial_headers.clear();
                            self.spatial_prims.clear();
                            self.spatial_lens_hit_panel = false;
                            self.spatial_lens_last_tick = None;
                            self.spatial_lens_fresh_pack = false;
                        }
                        log::info!("spatial_lens visible={}", self.spatial_lens.visible);
                    }
                    if pressed
                        && !event.repeat
                        && code == KeyCode::KeyF
                        && self.spatial_lens.visible
                        && !self.ui.state.open
                    {
                        self.activate_spatial_lens_binding();
                    }
                    if pressed && !event.repeat && code == KeyCode::Escape {
                        if self.context_radial.open {
                            self.close_context_radial();
                            log::info!("context radial closed (Esc)");
                        } else {
                            self.ui.toggle();
                            if self.ui.state.open {
                                self.set_look_capture(false);
                                self.clear_grab_attach();
                                self.grab_down = false;
                                self.ui.clear_pointer();
                            }
                            log::info!("system_ui open={}", self.ui.state.open);
                        }
                    }
                    if pressed
                        && !event.repeat
                        && code == KeyCode::KeyT
                        && self.owns_input()
                        && !self.ui.state.open
                    {
                        self.toggle_context_radial();
                    }
                    let dash_text = self.ui.state.open
                        && matches!(self.ui.state.tab, DashTab::Home | DashTab::Settings);
                    if pressed && dash_text {
                        match code {
                            KeyCode::Backspace => {
                                self.ui.on_home_backspace();
                            }
                            KeyCode::Tab if self.ui.state.tab == DashTab::Home => {
                                self.ui.on_home_tab_key();
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter
                                if !event.repeat && self.ui.state.tab == DashTab::Home =>
                            {
                                let action = self.ui.on_home_enter();
                                handle_ui_action(
                                    action,
                                    &mut self.ui,
                                    &self.shared,
                                    &mut self.zenoh,
                                    &mut self.auth_job,
                                    RoomBindDefer {
                                        pending: &mut self.pending_room_bind,
                                        painted: &mut self.room_bind_progress_painted,
                                    },
                                    event_loop,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                if pressed {
                    if let Some(text) = event.text.as_ref() {
                        let dash_text = self.ui.state.open
                            && matches!(self.ui.state.tab, DashTab::Home | DashTab::Settings);
                        if dash_text {
                            // Skip control-ish single chars already handled as keys.
                            if text != "\u{8}" && text != "\t" && text != "\r" && text != "\n" {
                                self.ui.on_text_input(text);
                            }
                        }
                    }
                }
                // Mid-frame Grab: `ensure_display_frame` reuses last Redraw freeze
                // (what is still on screen) until the next `begin_display_frame`.
                self.rebuild_intent();
            }
            WindowEvent::RedrawRequested => {
                self.ui.pull_dash_logic(&self.shared);
                let hier = self.ui.pull_hierarchy(&self.shared);
                let xray = self.ui.pull_epoch_xray(&self.shared, false);
                if hier || xray {
                    self.ui.rebuild();
                }
                session_flow::tick_worlds_session_ui(
                    &mut self.ui,
                    &self.shared,
                    &mut self.zenoh,
                    &mut self.auth_job,
                    &mut self.waiting_session_job,
                );
                self.on_input_ownership_edge();
                self.refresh_window_title();
                self.sync_hotswitch_ui();
                self.poll_presentation_requests();
                self.poll_auth_job();
                self.drain_inventory_peer_actions(event_loop);
                self.poll_mtls_reopen();
                // Freeze display α/PrevCurr before Grab + GPU sync (x0091 同一 α).
                self.shared.begin_display_frame();
                // Lens hit before Grab so panel look blocks world pick this frame.
                self.tick_spatial_lens();
                if self.owns_input() {
                    self.maintain_look_capture();
                    self.rebuild_intent();
                } else if self.look_captured {
                    self.set_look_capture(false);
                }
                if let Some(action) = self.tick_context_radial() {
                    match action {
                        UiAction::GrabRadialSave(eid) => self.handle_context_save(eid),
                        other => handle_ui_action(
                            other,
                            &mut self.ui,
                            &self.shared,
                            &mut self.zenoh,
                            &mut self.auth_job,
                            RoomBindDefer {
                                pending: &mut self.pending_room_bind,
                                painted: &mut self.room_bind_progress_painted,
                            },
                            event_loop,
                        ),
                    }
                }
                // Mirror while VR-active is intentional (non-guarantee GPU cost).
                let highlight = if self.owns_input() {
                    self.aim_highlight()
                } else {
                    None
                };
                let context_gpu_dirty = self.context_gpu_dirty;
                let context_overlay = if self.context_radial.open && !self.ui.state.open {
                    Some(ContextOverlayFrame {
                        tree: &self.context_tree,
                        layout_dirty: context_gpu_dirty,
                        pointer: self.context_radial.pointer,
                    })
                } else {
                    None
                };
                let rendered = if let Some(gpu) = self.gpu.as_mut() {
                    match gpu.render(
                        &mut self.ui,
                        highlight,
                        &self.spatial_lens,
                        &self.spatial_headers,
                        &self.spatial_prims,
                        context_overlay,
                    ) {
                        Ok(()) => {
                            if context_gpu_dirty {
                                self.context_gpu_dirty = false;
                            }
                            true
                        }
                        Err(err) => {
                            log::error!("render failed: {err:#}");
                            false
                        }
                    }
                } else {
                    false
                };
                // Only after a successful paint — never skip the Binding frame on gpu miss/fail.
                if rendered && self.pending_room_bind.is_some() {
                    self.room_bind_progress_painted = true;
                }
                self.ui.push_dash_logic(&self.shared);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_presentation_requests();
        if self.shared.presentation.exit_requested() {
            crate::laser_prefs::flush_blocking();
            event_loop.exit();
            return;
        }
        self.on_input_ownership_edge();
        self.poll_auth_job();
        self.drain_inventory_peer_actions(event_loop);
        // After Join/Host ticket: bind only once "Binding room bus…" has painted.
        self.poll_pending_room_bind();
        self.poll_mtls_reopen();
        self.apply_pending_world_mesh();
        // x0048: re-apply sticky asset failure after Login/Start/Join settle (skips "…").
        if self
            .shared
            .overlay_asset_home_status(&mut self.ui.state.home_status)
        {
            self.ui.state.open = true;
            self.ui.state.tab = DashTab::Home;
            self.ui.mark_dash_logic_dirty();
            // Publish before next Redraw so XR cannot interleave an older push.
            self.ui.push_dash_logic(&self.shared);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

impl App {
    /// Manifest remount / Leave empty shell (x007d).
    fn apply_pending_world_mesh(&mut self) {
        let Some(mesh) = self.shared.pending_mesh_if_newer(&mut self.mesh_epoch) else {
            return;
        };
        self.mesh = mesh;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.scene.replace_world_mesh(&self.mesh);
        }
    }

    /// Run deferred bind only after "Binding room bus…" has painted once.
    fn poll_pending_room_bind(&mut self) {
        let Some(pending) = self.pending_room_bind.take() else {
            return;
        };
        if !self.room_bind_progress_painted {
            self.pending_room_bind = Some(pending);
            return;
        }
        self.room_bind_progress_painted = false;
        session_flow::complete_pending_room_bind(
            &mut self.ui,
            &self.shared,
            &mut self.zenoh,
            pending,
        );
    }
}

fn handle_ui_action(
    action: UiAction,
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    room_bind: RoomBindDefer<'_>,
    event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::HotswitchAutoChanged => hotswitch_flow::apply_hotswitch_auto_changed(ui, shared),
        UiAction::ServerOperatorChanged => {
            let server_operator = ui.state.server_operator;
            operator_prefs::update_prefs(|prefs| {
                prefs.server_operator = server_operator;
            });
            log::info!("server operator prefs → {server_operator}");
        }
        UiAction::LaserConfigChanged => {
            crate::laser_prefs::save(&ui.state.laser);
            log::info!(
                "laser prefs → primary_min={:.2} stick={:.2} show_desktop={}",
                ui.state.laser.laser_show_primary_min,
                ui.state.laser.laser_stickiness,
                ui.state.laser.show_lasers_in_desktop_mode
            );
        }
        UiAction::ExitClientOnly | UiAction::ExitClientAndHost => {
            // Intentional thin glue (x00d0): host stop is session_flow; process teardown
            // needs `ActiveEventLoop` so it stays in the dispatcher (not a flow).
            // Exit Client only abolished: stop supervisor-owned host only.
            log::info!("system_ui: exit (handover stub + stop supervised host if any)");
            session_flow::exit_stop_supervised_host(shared);
            crate::laser_prefs::flush_blocking();
            shared.presentation.request_exit();
            event_loop.exit();
        }
        UiAction::ApplyOperatorEndpoints => {
            session_flow::apply_operator_endpoints(ui, shared, zenoh)
        }
        UiAction::AuthLogin => session_flow::auth_login(ui, shared, auth_job),
        UiAction::AuthRegister => session_flow::auth_register(ui, shared, auth_job),
        UiAction::AuthLogout => session_flow::auth_logout(ui, shared, zenoh, auth_job, room_bind),
        UiAction::StartSession => session_flow::start_session(ui, shared, zenoh, auth_job),
        UiAction::JoinRoom => session_flow::join_room(ui, shared, zenoh, auth_job),
        UiAction::WorldsRefresh => session_flow::start_worlds_catalog_refresh(ui, shared, auth_job),
        UiAction::WorldsJoinGuest => session_flow::worlds_join_guest(ui, shared, zenoh, auth_job),
        UiAction::WorldsHostRoom => session_flow::worlds_host_room(ui, shared, zenoh, auth_job),
        UiAction::WorldsHostRescue => session_flow::worlds_host_rescue(ui, shared, zenoh, auth_job),
        UiAction::WorldsTakeover => session_flow::worlds_takeover(ui, shared, zenoh, auth_job),
        UiAction::WorldsLeave => session_flow::worlds_leave(ui, shared, zenoh, auth_job, room_bind),
        UiAction::WorldsRejoin => {
            session_flow::worlds_rejoin(ui, shared, zenoh, auth_job, room_bind)
        }
        UiAction::InventoryRefresh => inventory_flow::inventory_refresh(ui, shared),
        UiAction::InventoryTakeOut => {
            inventory_flow::inventory_take_out(ui, shared, zenoh, auth_job)
        }
        UiAction::InventoryOpen => inventory_flow::inventory_open(ui, shared),
        UiAction::InventoryPutBack => inventory_flow::inventory_put_back(ui, shared),
        UiAction::GrabRadialSave(_entity_id) => {
            // Desktop handles this via `App::handle_context_save` (radial status).
        }
        UiAction::InventoryDespawn => inventory_flow::inventory_despawn(ui, shared, zenoh),
        UiAction::RoomSaveChanges => inventory_flow::room_save_changes(ui, shared, zenoh),
        UiAction::HierarchyCollapseToggle(row) => {
            ui.apply_hierarchy_collapse_toggle(shared, row);
        }
        UiAction::HierarchyRelayout => {
            ui.apply_hierarchy_relayout(shared);
        }
        UiAction::None => {}
    }
}

pub fn run(
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
    role: DesktopRole,
) -> Result<()> {
    // Dev target/: refresh sibling host-server.exe off the UI/Auth path when stale.
    host_supervisor::spawn_warm_dev_host_bin();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut ui = SystemUi::new();
    ui.state.home_room_id = shared.get_room_id().to_string();
    let prefs = operator_prefs::load();
    ui.state.operator = prefs.operator;
    ui.state.custom_host = prefs.custom_host.clone();
    ui.state.server_operator = prefs.server_operator;
    let laser_cfg = crate::laser_prefs::load();
    ui.state.laser = laser_cfg;
    ui.state.hotswitch_available = role == DesktopRole::Companion;
    ui.state.hotswitch_auto = shared
        .presentation
        .hotswitch_auto
        .load(std::sync::atomic::Ordering::SeqCst);
    ui.state.hotswitch_suppressed = shared
        .presentation
        .auto_suppressed
        .load(std::sync::atomic::Ordering::SeqCst);
    ui.state.refresh_hotswitch_label();
    let (joined, authenticated, username) = shared
        .auth
        .lock()
        .ok()
        .map(|g| (g.is_joined(), g.is_authenticated(), g.username.clone()))
        .unwrap_or((false, false, None));
    // Boot never auto-joins; `joined` is defensive only (should be false at surface start).
    if joined {
        ui.state.home_logged_in_as = username;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.worlds_in_room = true;
        ui.state.home_status = "In room — use Session for Leave / Host Rescue".into();
        ui.state.worlds_status = "In room — Leave or Host Rescue from Session".into();
    } else {
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        if authenticated {
            ui.state.home_logged_in_as = username;
            ui.state.shell = OnboardingShell::MainHub;
            ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
            ui.state.home_status = "Signed in — join a public session".into();
        } else if !prefs.operator_chosen {
            ui.state.shell = OnboardingShell::Step1Operator;
            ui.state.home_status = "Select a server operator".into();
        } else {
            ui.state.shell = OnboardingShell::Step2Auth;
            ui.state.home_status = "Sign in to continue".into();
        }
    }
    // x0048: sticky asset failure → Home status (not Coin fixture).
    if shared.overlay_asset_home_status(&mut ui.state.home_status) {
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        if ui.state.shell == OnboardingShell::MainHub {
            ui.state.shell = OnboardingShell::Dash;
        }
    }
    // Seed SharedState so XR can pull the same logical shell before first redraw.
    ui.mark_dash_logic_dirty();
    ui.push_dash_logic(&shared);
    let last_owns_input = role.owns_input(&shared.presentation);
    let boot_catalog = ui.state.shell == OnboardingShell::MainHub
        && ui.state.home_logged_in_as.is_some()
        && !joined;
    let mut app = App {
        mesh,
        shared,
        window: None,
        gpu: None,
        ui,
        zenoh,
        role,
        cursor: (0.0, 0.0),
        primary_down: false,
        keys_down: HashSet::new(),
        look_captured: false,
        device_look_active: false,
        skip_cursor_warp: false,
        last_cursor: None,
        blank_cursor: None,
        held_grab: None,
        grab_attach_offset: None,
        grab_attach_rot: None,
        grab_needs_release: false,
        grab_down: false,
        lmb_grab_armed: false,
        context_radial: ContextRadialState::default(),
        context_tree: LayoutTree::default(),
        context_pointer: (640.0, 360.0),
        context_leave_miss: 0,
        context_gpu_dirty: false,
        last_grab_miss_log: None,
        auth_job: None,
        waiting_session_job: None,
        last_owns_input,
        mesh_epoch: 0,
        pending_room_bind: None,
        room_bind_progress_painted: false,
        spatial_lens: SpatialLensState::default(),
        spatial_laser: HandLaserState::default(),
        spatial_laser_cfg: laser_cfg,
        spatial_headers: Vec::new(),
        spatial_prims: Vec::new(),
        spatial_lens_hit_panel: false,
        spatial_lens_last_tick: None,
        spatial_lens_fresh_pack: false,
        inspector_scrub_primary_was_down: false,
    };
    // silent / remember-me MainHub entry — same catalog load as LoginOk / RegisterOk.
    if boot_catalog {
        session_flow::start_worlds_catalog_refresh(&mut app.ui, &app.shared, &mut app.auth_job);
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_grab_may_start_blocks_until_release() {
        let mut needs = false;
        assert!(desktop_grab_may_start(&mut needs, true, false));
        assert!(!desktop_grab_may_start(&mut needs, true, true));
        assert!(needs);
        assert!(!desktop_grab_may_start(&mut needs, true, false));
        assert!(!desktop_grab_may_start(&mut needs, false, false));
        assert!(!needs);
        assert!(desktop_grab_may_start(&mut needs, true, false));
    }

    #[test]
    fn mouse_look_yaw_pitch_and_clamp() {
        let look = CameraLook {
            yaw: 0.0,
            pitch: 0.0,
        };
        // Mouse right → +yaw; mouse up (negative dy) → +pitch.
        let next = apply_mouse_look(look, 100.0, -50.0, 0.01, PITCH_LIMIT);
        assert!((next.yaw - 1.0).abs() < 1e-5);
        assert!((next.pitch - 0.5).abs() < 1e-5);

        let high = apply_mouse_look(
            CameraLook {
                yaw: 0.0,
                pitch: 1.5,
            },
            0.0,
            -1000.0,
            0.01,
            PITCH_LIMIT,
        );
        assert!((high.pitch - PITCH_LIMIT).abs() < 1e-5);
    }
}
