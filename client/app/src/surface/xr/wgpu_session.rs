//! OpenXR full session via wgpu: SceneGpu dual-eye + overlays (x0051 / `ALCHEMY_XR_WGPU=1`).
//!
//! Guarantees: acquire → SceneGpu×2 + overlays → release → project;
//! drop 順: `render_models.clear` → aim/view/stage Spaces → swapchain/textures →
//! frame_stream/wait → session → bridge（ash `vk_loop` と同契約; x0050 の bridge 順に加え
//! EXT handle / Space を session より先に破棄）;
//! `frame_stream.begin` 後は成功／失敗／panic とも `end` 必須; ash と同等の
//! actions / Intent / Grab / Menu / UiAction.
//! Non-guarantees: ash 削除; multiview; Bevy; CompositionLayerQuad.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use glam::{Mat4, Quat, Vec3};
use openxr as xr;

use alchemy_protocol::{input_buttons, DEFAULT_MOVE_SPEED};

use super::input::{
    dash_quad_model, laser_length, laser_length_world, laser_model, openxr_pose_to_isometry,
    pose_to_ray, ray_hit_dash_quad, Ray,
};
use super::loco::{combine_turn_axes, intent_from_sticks, stage_move_to_world, STICK_DEADZONE};
use super::math::{projection_from_fov_default, view_from_pose};
use super::render_model::{enable_if_available, RenderModelCache};
use super::wgpu_interop::{self, FrameEndGuard, SwapchainReleaseGuard, XrWgpuBridge, VIEW_COUNT};
use super::wgpu_overlays::{GpuController, XrOverlays};
use crate::host_control;
use crate::leave_room;
use crate::mesh::CpuMesh;
use crate::scene::{
    create_depth_view, EntityPose, MoveIntent, SceneGpu, SharedState, BOX_HALF_EXTENT,
};
use crate::system_ui::{SystemUi, UiAction};
use crate::turn::{update_turn_yaw, TURN_DEADZONE};
use crate::world_hit::ray_hit_grabbable_boxes_with_extents;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
const UI_CANVAS_W: u32 = 960;
const UI_CANVAS_H: u32 = 540;
const DEFAULT_EYE_Y: f32 = 1.6;
const GRAB_SQUEEZE_THRESHOLD: f32 = 0.7;
const WORLD_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.12,
    b: 0.16,
    a: 1.0,
};

pub fn run_session(
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    mut zenoh: Option<zenoh::Session>,
) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let _ = ctrlc_set(Arc::clone(&running));

    // Safety: process-global OpenXR loader (same as ash path).
    let entry = unsafe { super::loader::load_entry() }.context("OpenXR Entry::load")?;
    let available = entry.enumerate_extensions()?;
    if !available.khr_vulkan_enable2 {
        bail!("XR_KHR_vulkan_enable2 required (SteamVR OpenXR)");
    }
    let mut enabled = xr::ExtensionSet::default();
    enabled.khr_vulkan_enable2 = true;
    let rm_support = enable_if_available(&available, &mut enabled);
    let xr_instance = entry.create_instance(
        &xr::ApplicationInfo {
            application_name: "Alchemy",
            application_version: 1,
            engine_name: "alchemy",
            engine_version: 1,
            api_version: xr::Version::new(1, 0, 0),
        },
        &enabled,
        &[],
        &(),
    )?;
    let props = xr_instance.properties()?;
    log::info!(
        "OpenXR runtime (wgpu session): {} {}",
        props.runtime_name,
        props.runtime_version
    );

    let system = xr_instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
    let blend_modes = xr_instance.enumerate_environment_blend_modes(system, VIEW_TYPE)?;
    let environment_blend_mode = *blend_modes
        .first()
        .context("enumerate_environment_blend_modes returned empty")?;
    let reqs = xr_instance.graphics_requirements::<xr::Vulkan>(system)?;
    let vk_target_xr = xr::Version::new(1, 1, 0);
    if vk_target_xr < reqs.min_api_version_supported
        || vk_target_xr.major() > reqs.max_api_version_supported.major()
    {
        bail!(
            "Vulkan version mismatch for OpenXR (min {})",
            reqs.min_api_version_supported
        );
    }

    // Declared first → dropped last.
    // Safety: see XrWgpuBridge::create — Drop destroys ash after wgpu.
    let bridge = unsafe { XrWgpuBridge::create(&xr_instance, system) }?;
    // Safety: SessionCreateInfo handles match the live Vulkan objects in `bridge`.
    let (session, mut frame_wait, mut frame_stream) =
        unsafe { xr_instance.create_session::<xr::Vulkan>(system, &bridge.session_create_info())? };

    let action_set = xr_instance.create_action_set("alchemy_input", "Alchemy input", 0)?;
    let hands = [
        xr_instance.string_to_path("/user/hand/left")?,
        xr_instance.string_to_path("/user/hand/right")?,
    ];
    let menu_action = action_set.create_action::<bool>("menu", "Menu toggle", &[])?;
    let select_action = action_set.create_action::<bool>("select", "Select / click", &[])?;
    let grab_action = action_set.create_action::<f32>("grab", "Grab / squeeze", &hands)?;
    let aim_action = action_set.create_action::<xr::Posef>("aim", "Aim pose", &hands)?;
    let thumbstick_action =
        action_set.create_action::<xr::Vector2f>("thumbstick", "Locomotion stick", &hands)?;

    suggest_bindings(
        &xr_instance,
        &menu_action,
        &select_action,
        &grab_action,
        &aim_action,
        &thumbstick_action,
    )?;
    session.attach_action_sets(&[&action_set])?;

    let aim_left = aim_action.create_space(&session, hands[0], xr::Posef::IDENTITY)?;
    let aim_right = aim_action.create_space(&session, hands[1], xr::Posef::IDENTITY)?;
    let stage =
        session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;
    let view_ref =
        session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;

    let mut render_models = RenderModelCache::new(rm_support);

    let cfg = xr_instance.enumerate_view_configuration_views(system, VIEW_TYPE)?;
    ensure!(
        cfg.len() == VIEW_COUNT as usize,
        "expected {VIEW_COUNT} stereo views, got {}",
        cfg.len()
    );
    let width = cfg[0].recommended_image_rect_width;
    let height = cfg[0].recommended_image_rect_height;

    let formats = session.enumerate_swapchain_formats()?;
    let (vk_format, wgpu_format) = wgpu_interop::pick_swapchain_format(&formats)?;

    let mut swapchain = session.create_swapchain(&xr::SwapchainCreateInfo {
        create_flags: xr::SwapchainCreateFlags::EMPTY,
        usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::SAMPLED,
        format: vk_format,
        sample_count: 1,
        width,
        height,
        face_count: 1,
        array_size: VIEW_COUNT,
        mip_count: 1,
    })?;

    let raw_images = swapchain.enumerate_images()?;
    // Safety: images from this swapchain; textures/views dropped before swapchain.
    let textures =
        unsafe { bridge.wrap_swapchain_images(&raw_images, width, height, wgpu_format) }?;
    let eye_views = XrWgpuBridge::make_eye_views(&textures, wgpu_format);

    let depth_left = create_depth_view(bridge.device(), width, height);
    let depth_right = create_depth_view(bridge.device(), width, height);
    let depths = [&depth_left, &depth_right];

    let scene = SceneGpu::from_device(
        bridge.device().clone(),
        bridge.queue().clone(),
        wgpu_format,
        &mesh,
        Arc::clone(&shared),
    )?;
    let overlays = XrOverlays::new(
        bridge.device(),
        bridge.queue(),
        wgpu_format,
        UI_CANVAS_W,
        UI_CANVAS_H,
    )?;

    let mut ui = SystemUi::new();
    ui.state.home_session_actions_enabled = false;
    ui.state.home_room_id = shared.room_id.to_string();
    if shared.overlay_asset_home_status(&mut ui.state.home_status) {
        ui.state.open = true;
        ui.state.tab = crate::system_ui::DashTab::Home;
    }
    ui.init_cpu_canvas(UI_CANVAS_W, UI_CANVAS_H)?;
    ui.rebuild();

    let mut controller_gpu: Vec<GpuController> = Vec::new();
    let mut controller_gpu_gen: u64 = 0;
    // Reused each frame: (gpu index, stage root). Worlds/visible stay in ControllerModel scratch.
    let mut ctrl_roots: Vec<(usize, Mat4)> = Vec::new();

    log::info!(
        "ALCHEMY_XR_WGPU=1 session — SceneGpu dual-eye + overlays ({width}x{height}×{VIEW_COUNT}); Menu/Select/Grab as ash path"
    );

    let mut event_storage = xr::EventDataBuffer::new();
    let mut session_running = false;
    let mut menu_was_down = false;
    let mut select_was_down = false;
    let mut held_grab: Option<u32> = None;
    let mut exit_requested = false;
    let mut logged_box_count: Option<usize> = None;
    let mut view_yaw = 0.0_f32;
    let mut snap_armed = true;

    'main_loop: loop {
        if !running.load(Ordering::Relaxed) || exit_requested {
            match session.request_exit() {
                Ok(()) => {}
                Err(xr::sys::Result::ERROR_SESSION_NOT_RUNNING) => break,
                Err(e) => log::warn!("request_exit: {e}"),
            }
        }

        while let Some(event) = xr_instance.poll_event(&mut event_storage)? {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => match e.state() {
                    xr::SessionState::READY => {
                        session.begin(VIEW_TYPE)?;
                        session_running = true;
                        log::info!("OpenXR session READY (wgpu)");
                    }
                    xr::SessionState::FOCUSED => {
                        render_models.mark_dirty();
                    }
                    xr::SessionState::STOPPING => {
                        session.end()?;
                        session_running = false;
                    }
                    xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                        break 'main_loop;
                    }
                    _ => {}
                },
                InteractionRenderModelsChangedEXT => {
                    log::info!("XR InteractionRenderModelsChanged");
                    render_models.mark_dirty();
                }
                InstanceLossPending(_) => break 'main_loop,
                _ => {}
            }
        }

        if !session_running {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        let xr_frame_state = frame_wait.wait()?;
        frame_stream.begin()?;
        let frame_end = FrameEndGuard::new(
            &mut frame_stream,
            xr_frame_state.predicted_display_time,
            environment_blend_mode,
        );

        if !xr_frame_state.should_render {
            frame_end.end_empty()?;
            continue;
        }

        session.sync_actions(&[(&action_set).into()])?;
        if let Err(err) = render_models.refresh_if_dirty(&session) {
            log::warn!("XR render model refresh: {err:#}");
        }
        if controller_gpu_gen != render_models.generation {
            controller_gpu.clear();
            for m in render_models.models() {
                match overlays.upload_controller(&m.mesh) {
                    Ok(g) => controller_gpu.push(g),
                    Err(err) => log::warn!("controller GPU upload: {err:#}"),
                }
            }
            controller_gpu_gen = render_models.generation;
        }

        // Comfort turn + Intent (same as ash).
        {
            let left_stick = thumbstick_action.state(&session, hands[0])?.current_state;
            let right_stick = thumbstick_action.state(&session, hands[1])?.current_state;
            let left_ori = aim_left
                .locate(&stage, xr_frame_state.predicted_display_time)
                .ok()
                .filter(|l| {
                    l.location_flags
                        .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
                })
                .map(|l| openxr_pose_to_isometry(l.pose).1)
                .unwrap_or(Quat::IDENTITY);
            let right_ori = aim_right
                .locate(&stage, xr_frame_state.predicted_display_time)
                .ok()
                .filter(|l| {
                    l.location_flags
                        .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
                })
                .map(|l| openxr_pose_to_isometry(l.pose).1)
                .unwrap_or(Quat::IDENTITY);
            let dash_open = ui.state.open;
            let max_speed = ui.state.max_speed;
            let turn_settings = ui.state.turn;
            let dt = {
                let ns = xr_frame_state.predicted_display_period.as_nanos();
                ((ns as f64) / 1_000_000_000.0).clamp(0.001, 0.1) as f32
            };
            let mut intent = MoveIntent {
                suppress: dash_open,
                ..Default::default()
            };
            if !dash_open {
                let turn_axis = combine_turn_axes(left_stick.x, right_stick.x);
                view_yaw = update_turn_yaw(
                    view_yaw,
                    turn_axis,
                    turn_settings,
                    dt,
                    &mut snap_armed,
                    TURN_DEADZONE,
                );
                let stage_move = intent_from_sticks(
                    left_ori,
                    (left_stick.x, left_stick.y),
                    right_ori,
                    (right_stick.x, right_stick.y),
                    max_speed,
                    DEFAULT_MOVE_SPEED,
                    STICK_DEADZONE,
                );
                let v = stage_move_to_world(stage_move, view_yaw);
                intent.move_x = v.x;
                intent.move_y = v.y;
                intent.move_z = v.z;
            }
            if let Ok(mut slot) = shared.intent.lock() {
                *slot = intent;
            }
        }

        let user_root = shared
            .world
            .lock()
            .ok()
            .and_then(|w| w.entities.get(&shared.local_user_root()).copied())
            .unwrap_or(EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
                ..Default::default()
            });
        let yaw_q = Quat::from_rotation_y(view_yaw);
        let floor_model = Mat4::from_rotation_translation(
            yaw_q,
            yaw_q * Vec3::new(-user_root.x, -user_root.y, -user_root.z),
        );

        let box_defs = shared.box_defs_snapshot();
        type PoseList = Vec<(u32, EntityPose)>;
        let (draw_boxes, grabbable_boxes) = shared
            .world
            .lock()
            .ok()
            .map(|w| {
                let mut draw = PoseList::new();
                let mut grab = PoseList::new();
                for (id, p) in w.entities.iter() {
                    if box_defs.half_extents.contains_key(id) {
                        draw.push((*id, *p));
                    }
                    if box_defs.grabbable.contains(id) {
                        grab.push((*id, *p));
                    }
                }
                (draw, grab)
            })
            .unwrap_or_else(|| (PoseList::new(), PoseList::new()));
        if logged_box_count != Some(draw_boxes.len()) {
            if draw_boxes.is_empty() {
                log::warn!("XR: no Manifest boxes in entity map yet (waiting for host frame)");
            } else {
                let p0 = draw_boxes[0].1;
                log::info!(
                    "XR boxes ready: draw={} grab={} first=({:.2},{:.2},{:.2})",
                    draw_boxes.len(),
                    grabbable_boxes.len(),
                    p0.x,
                    p0.y,
                    p0.z
                );
            }
            logged_box_count = Some(draw_boxes.len());
        }

        let menu_down = menu_action.state(&session, xr::Path::NULL)?.current_state;
        if menu_down && !menu_was_down {
            ui.toggle();
            log::info!("XR menu → system_ui open={}", ui.state.open);
        }
        menu_was_down = menu_down;

        let head_located = view_ref.locate(&stage, xr_frame_state.predicted_display_time)?;
        let (head_pos, head_orient) = openxr_pose_to_isometry(head_located.pose);
        let aim_pose = locate_ui_aim(
            &aim_right,
            &aim_left,
            &stage,
            xr_frame_state.predicted_display_time,
        );
        let world_hit_id = if !ui.state.open {
            aim_pose.and_then(|(aim_pos, aim_orient)| {
                let stage_ray = pose_to_ray(aim_pos, aim_orient);
                let world_ray = ray_to_world(stage_ray, floor_model);
                ray_hit_grabbable_boxes_with_extents(world_ray, &grabbable_boxes, |id| {
                    box_defs
                        .half_extents
                        .get(&id)
                        .copied()
                        .unwrap_or(BOX_HALF_EXTENT)
                })
                .map(|h| h.entity_id)
            })
        } else {
            None
        };

        let select_down_for_ui = select_action.state(&session, xr::Path::NULL)?.current_state;
        let squeeze_l = grab_action.state(&session, hands[0])?.current_state;
        let squeeze_r = grab_action.state(&session, hands[1])?.current_state;
        let grip_down = squeeze_l >= GRAB_SQUEEZE_THRESHOLD || squeeze_r >= GRAB_SQUEEZE_THRESHOLD;
        if shared.overlay_asset_home_status(&mut ui.state.home_status) {
            ui.state.open = true;
            ui.state.tab = crate::system_ui::DashTab::Home;
        }
        if ui.state.open {
            held_grab = None;
        } else {
            let grab_down = grip_down || select_down_for_ui;
            if grab_down {
                if held_grab.is_none() {
                    held_grab = world_hit_id;
                    if let Some(id) = held_grab {
                        log::info!("XR Grab hold begin entity={id}");
                    }
                }
            } else if held_grab.take().is_some() {
                log::info!("XR Grab hold release");
            }
        }
        if let Ok(mut slot) = shared.intent.lock() {
            if let (Some(id), Some((aim_pos, aim_orient))) = (held_grab, aim_pose) {
                let world_origin =
                    ray_to_world(pose_to_ray(aim_pos, aim_orient), floor_model).origin;
                slot.buttons |= input_buttons::GRAB;
                slot.grab_entity_id = id;
                slot.aim_x = world_origin.x;
                slot.aim_y = world_origin.y;
                slot.aim_z = world_origin.z;
            } else {
                slot.buttons &= !input_buttons::GRAB;
                slot.grab_entity_id = 0;
                slot.aim_x = 0.0;
                slot.aim_y = 0.0;
                slot.aim_z = 0.0;
            }
        }

        let laser_draw = aim_pose.map(|(aim_pos, aim_orient)| {
            let stage_ray = pose_to_ray(aim_pos, aim_orient);
            let len = if ui.state.open {
                laser_length(
                    head_pos,
                    head_orient,
                    stage_ray,
                    UI_CANVAS_W as f32,
                    UI_CANVAS_H as f32,
                )
            } else {
                let world_ray = ray_to_world(stage_ray, floor_model);
                laser_length_world(world_ray, &grabbable_boxes)
            };
            laser_model(aim_pos, aim_orient, len)
        });

        if ui.state.open {
            if let Some((aim_pos, aim_orient)) = aim_pose {
                let ray = pose_to_ray(aim_pos, aim_orient);
                if let Some((px, py)) = ray_hit_dash_quad(
                    head_pos,
                    head_orient,
                    ray,
                    UI_CANVAS_W as f32,
                    UI_CANVAS_H as f32,
                ) {
                    ui.on_pointer(px, py, select_down_for_ui);
                } else {
                    ui.clear_pointer();
                }
            } else {
                ui.clear_pointer();
            }
        } else {
            ui.clear_pointer();
        }

        let dash_ui_model = if ui.state.open {
            ui.prepare_cpu_canvas();
            if let Some((pixels, cw, ch)) = ui.canvas_rgba() {
                overlays.upload_ui_canvas(pixels, cw, ch);
            }
            Some(dash_quad_model(head_pos, head_orient))
        } else {
            None
        };

        let highlight = held_grab.or(world_hit_id);

        let (_, views) =
            session.locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)?;
        ensure!(
            views.len() >= VIEW_COUNT as usize,
            "locate_views returned {} views, need {VIEW_COUNT}",
            views.len()
        );

        let image_index = swapchain.acquire_image()? as usize;
        let mut release = SwapchainReleaseGuard::arm(&mut swapchain);
        release.wait_image(xr::Duration::INFINITE)?;

        let eyes = eye_views.get(image_index).with_context(|| {
            format!(
                "swapchain image index {image_index} out of range ({})",
                eye_views.len()
            )
        })?;

        // Poll + locate roots once; compose stays in ControllerModel scratch (no per-frame Vec).
        let xr_inst = session.instance();
        ctrl_roots.clear();
        for (i, model) in render_models.models_mut().iter_mut().enumerate() {
            if i >= controller_gpu.len() {
                break;
            }
            if let Err(err) = model.poll_state(xr_inst, xr_frame_state.predicted_display_time) {
                log::warn!("XR render model state: {err:#}");
            }
            let Ok(loc) = model
                .space
                .locate(&stage, xr_frame_state.predicted_display_time)
            else {
                continue;
            };
            if !loc
                .location_flags
                .contains(xr::SpaceLocationFlags::POSITION_VALID)
                || !loc
                    .location_flags
                    .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
            {
                continue;
            }
            let (pos, ori) = openxr_pose_to_isometry(loc.pose);
            ctrl_roots.push((i, Mat4::from_rotation_translation(ori, pos)));
        }
        // Compose once; both eyes read immutable scratch via `composed_node_worlds`.
        for &(gi, _) in &ctrl_roots {
            if let Some(model) = render_models.models_mut().get_mut(gi) {
                let _ = model.compose_node_worlds();
            }
        }

        // Per-eye submit so SceneGpu / overlay UBOs are not overwritten mid-encoder.
        for eye in 0..VIEW_COUNT as usize {
            let view_stage = view_from_pose(views[eye].pose);
            let proj = projection_from_fov_default(views[eye].fov);
            let view_world = view_stage * floor_model;
            let view_proj_stage = proj * view_stage;

            let mut encoder =
                bridge
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("xr-wgpu-eye"),
                    });
            scene.draw_world(
                &mut encoder,
                &eyes[eye],
                depths[eye],
                view_world,
                proj,
                wgpu::LoadOp::Clear(WORLD_CLEAR),
                highlight,
            );

            overlays.draw_controllers(
                &mut encoder,
                &eyes[eye],
                depths[eye],
                view_proj_stage,
                |draw| {
                    for &(gi, root) in &ctrl_roots {
                        let Some(gpu) = controller_gpu.get(gi) else {
                            continue;
                        };
                        let Some(model) = render_models.models().get(gi) else {
                            continue;
                        };
                        let (worlds, visible) = model.composed_node_worlds();
                        draw(gpu, root, worlds, visible);
                    }
                },
            );
            if let Some(model) = dash_ui_model {
                overlays.draw_dash(
                    &mut encoder,
                    &eyes[eye],
                    depths[eye],
                    view_proj_stage,
                    model,
                );
            }
            if let Some(model) = laser_draw {
                overlays.draw_laser(
                    &mut encoder,
                    &eyes[eye],
                    depths[eye],
                    view_proj_stage,
                    model,
                );
            }
            bridge.queue().submit(Some(encoder.finish()));
        }

        let _ = bridge.device().poll(wgpu::Maintain::Wait);
        release.release()?;

        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: width as i32,
                height: height as i32,
            },
        };
        let proj_views = [
            xr::CompositionLayerProjectionView::new()
                .pose(views[0].pose)
                .fov(views[0].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&swapchain)
                        .image_array_index(0)
                        .image_rect(rect),
                ),
            xr::CompositionLayerProjectionView::new()
                .pose(views[1].pose)
                .fov(views[1].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&swapchain)
                        .image_array_index(1)
                        .image_rect(rect),
                ),
        ];
        let proj = xr::CompositionLayerProjection::new()
            .space(&stage)
            .views(&proj_views);
        frame_end.end_projection(&proj)?;

        // Select click after present path (CPU UI).
        if select_down_for_ui && !select_was_down {
            if let Some((aim_pos, aim_orient)) = aim_pose {
                let ray = pose_to_ray(aim_pos, aim_orient);
                if let Some((px, py)) = ray_hit_dash_quad(
                    head_pos,
                    head_orient,
                    ray,
                    UI_CANVAS_W as f32,
                    UI_CANVAS_H as f32,
                ) {
                    match ui.on_click(px, py) {
                        UiAction::ExitClientOnly => {
                            log::info!("XR UI: exit client only");
                            exit_requested = true;
                        }
                        UiAction::ExitClientAndHost => {
                            log::info!("XR UI: exit client + host stub");
                            if let Some(ref z) = zenoh {
                                let cap = shared.wire_cap();
                                host_control::request_host_shutdown(
                                    z,
                                    shared.room_id.as_ref(),
                                    cap.as_deref(),
                                );
                            }
                            exit_requested = true;
                        }
                        UiAction::AuthLogin | UiAction::AuthRegister => {
                            ui.state.home_status =
                                "Login/register text entry is Desktop-only (x0018)".into();
                            ui.rebuild();
                        }
                        UiAction::AuthLogout => {
                            leave_room(&shared, &mut zenoh);
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.clear_session();
                            }
                            ui.state.home_logged_in_as = None;
                            ui.state.home_create_open = false;
                            ui.state.home_status =
                                "Signed out — Leave sent (host reclaim immediate)".into();
                            ui.rebuild();
                        }
                        UiAction::StartSession | UiAction::JoinRoom => {
                            ui.state.home_status =
                                "Start/Join session HTTP is Desktop-only this Slice (use silent login or Desktop)".into();
                            ui.rebuild();
                        }
                        UiAction::None => {}
                    }
                }
            }
        }
        select_was_down = select_down_for_ui;
    }

    // OpenXR lifetime: destroy EXT handles + Spaces before session (ash vk_loop parity).
    // Explicit success-path reorder was a porting bug — Spaces must not outlive session.
    controller_gpu.clear();
    render_models.clear(&xr_instance);
    drop(aim_left);
    drop(aim_right);
    drop(view_ref);
    drop(stage);
    drop(overlays);
    drop(scene);
    drop(depth_right);
    drop(depth_left);
    drop(eye_views);
    drop(textures);
    drop(swapchain);
    drop(frame_stream);
    drop(frame_wait);
    drop(session);
    drop(bridge);
    let _ = entry;
    Ok(())
}

fn ray_to_world(stage_ray: Ray, floor_model: Mat4) -> Ray {
    let inv = floor_model.inverse();
    let origin = inv.transform_point3(stage_ray.origin);
    let dir = inv.transform_vector3(stage_ray.dir).normalize_or_zero();
    Ray { origin, dir }
}

fn locate_ui_aim(
    aim_right: &xr::Space,
    aim_left: &xr::Space,
    stage: &xr::Space,
    time: xr::Time,
) -> Option<(Vec3, Quat)> {
    let valid = |l: &xr::SpaceLocation| {
        l.location_flags
            .contains(xr::SpaceLocationFlags::POSITION_VALID)
            && l.location_flags
                .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
    };
    if let Ok(l) = aim_right.locate(stage, time) {
        if valid(&l) {
            return Some(openxr_pose_to_isometry(l.pose));
        }
    }
    if let Ok(l) = aim_left.locate(stage, time) {
        if valid(&l) {
            return Some(openxr_pose_to_isometry(l.pose));
        }
    }
    None
}

fn suggest_bindings(
    xr_instance: &xr::Instance,
    menu_action: &xr::Action<bool>,
    select_action: &xr::Action<bool>,
    grab_action: &xr::Action<f32>,
    aim_action: &xr::Action<xr::Posef>,
    thumbstick_action: &xr::Action<xr::Vector2f>,
) -> Result<()> {
    xr_instance.suggest_interaction_profile_bindings(
        xr_instance.string_to_path("/interaction_profiles/khr/simple_controller")?,
        &[
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/left/input/menu/click")?,
            ),
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/right/input/menu/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/left/input/select/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/right/input/select/click")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
            ),
        ],
    )?;
    let _ = xr_instance.suggest_interaction_profile_bindings(
        xr_instance.string_to_path("/interaction_profiles/oculus/touch_controller")?,
        &[
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/left/input/menu/click")?,
            ),
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/left/input/y/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/left/input/x/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/right/input/a/click")?,
            ),
            xr::Binding::new(
                grab_action,
                xr_instance.string_to_path("/user/hand/left/input/squeeze/value")?,
            ),
            xr::Binding::new(
                grab_action,
                xr_instance.string_to_path("/user/hand/right/input/squeeze/value")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
            ),
            xr::Binding::new(
                thumbstick_action,
                xr_instance.string_to_path("/user/hand/left/input/thumbstick")?,
            ),
            xr::Binding::new(
                thumbstick_action,
                xr_instance.string_to_path("/user/hand/right/input/thumbstick")?,
            ),
        ],
    );
    let _ = xr_instance.suggest_interaction_profile_bindings(
        xr_instance.string_to_path("/interaction_profiles/valve/index_controller")?,
        &[
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/left/input/b/click")?,
            ),
            xr::Binding::new(
                menu_action,
                xr_instance.string_to_path("/user/hand/right/input/b/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/left/input/trigger/click")?,
            ),
            xr::Binding::new(
                select_action,
                xr_instance.string_to_path("/user/hand/right/input/trigger/click")?,
            ),
            xr::Binding::new(
                grab_action,
                xr_instance.string_to_path("/user/hand/left/input/squeeze/value")?,
            ),
            xr::Binding::new(
                grab_action,
                xr_instance.string_to_path("/user/hand/right/input/squeeze/value")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
            ),
            xr::Binding::new(
                aim_action,
                xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
            ),
            xr::Binding::new(
                thumbstick_action,
                xr_instance.string_to_path("/user/hand/left/input/thumbstick")?,
            ),
            xr::Binding::new(
                thumbstick_action,
                xr_instance.string_to_path("/user/hand/right/input/thumbstick")?,
            ),
        ],
    );
    Ok(())
}

fn ctrlc_set(running: Arc<AtomicBool>) -> Result<()> {
    let _ = running;
    Ok(())
}
