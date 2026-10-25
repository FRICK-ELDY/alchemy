//! OpenXR + Vulkan frame loop (SteamVR). World via clear shader; UI via CPU canvas quad.
//!
//! Safety: OpenXR/Vulkan FFI; drop order ends session before destroying vk devices.

#![allow(clippy::missing_transmute_annotations)] // OpenXR vulkan_enable2 get_instance_proc_addr
#![allow(clippy::too_many_arguments)] // upload_rgba_to_image staging path

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use ash::vk::{self, Handle};
use openxr as xr;

use alchemy_protocol::DEFAULT_MOVE_SPEED;

use super::input::{
    dash_quad_model, openxr_pose_to_isometry, pose_to_ray, ray_hit_dash_quad, QUAD_HEIGHT,
    QUAD_WIDTH,
};
use super::loco::{intent_from_sticks, stage_move_to_world, STICK_DEADZONE};
use crate::host_control;
use crate::mesh::CpuMesh;
use crate::scene::{MoveIntent, SharedState, MESH_ENTITY_ID, USER_ROOT_ENTITY_ID};
use crate::system_ui::{SystemUi, UiAction};
use crate::turn::{update_turn_yaw, TURN_DEADZONE};
use glam::{Quat, Vec3};

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
const VIEW_COUNT: u32 = 2;
const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
const PIPELINE_DEPTH: usize = 2;
const UI_CANVAS_W: u32 = 960;
const UI_CANVAS_H: u32 = 540;
/// Checker floor texture (CompositionLayerQuad). Large enough to see loco slide.
const FLOOR_TEX_W: u32 = 256;
const FLOOR_TEX_H: u32 = 256;
const FLOOR_CELLS: u32 = 16;
/// World-space floor quad size (meters). Matches host Rapier floor half-extent ~20.
const FLOOR_METERS: f32 = 40.0;
const DEFAULT_EYE_Y: f32 = 1.6;

pub fn run_openxr(
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    {
        let r = Arc::clone(&running);
        let _ = ctrlc_set(r);
    }

    // Safety: dynamic OpenXR loader (Steam off-C: discovery in loader.rs).
    let entry = unsafe { super::loader::load_entry() }.context("OpenXR Entry::load")?;
    let available = entry.enumerate_extensions()?;
    if !available.khr_vulkan_enable2 {
        bail!("XR_KHR_vulkan_enable2 required (SteamVR OpenXR)");
    }
    let mut enabled = xr::ExtensionSet::default();
    enabled.khr_vulkan_enable2 = true;
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
    )?;
    let props = xr_instance.properties()?;
    log::info!(
        "OpenXR runtime: {} {}",
        props.runtime_name,
        props.runtime_version
    );

    let system = xr_instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
    let environment_blend_mode =
        xr_instance.enumerate_environment_blend_modes(system, VIEW_TYPE)?[0];

    let vk_target_version = vk::make_api_version(0, 1, 1, 0);
    let vk_target_version_xr = xr::Version::new(1, 1, 0);
    let reqs = xr_instance.graphics_requirements::<xr::Vulkan>(system)?;
    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        bail!(
            "Vulkan version mismatch for OpenXR (min {})",
            reqs.min_api_version_supported
        );
    }

    // Safety: Vulkan + OpenXR device lifetime tied to this function scope.
    unsafe {
        let vk_entry = ash::Entry::load().context("ash::Entry::load")?;
        let vk_app_info = vk::ApplicationInfo::default()
            .application_version(0)
            .engine_version(0)
            .api_version(vk_target_version);

        let vk_instance = {
            let raw = xr_instance
                .create_vulkan_instance(
                    system,
                    std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                    &vk::InstanceCreateInfo::default().application_info(&vk_app_info) as *const _
                        as *const _,
                )
                .map_err(|e| anyhow::anyhow!("xr create_vulkan_instance: {e}"))?
                .map_err(vk::Result::from_raw)
                .context("vkCreateInstance via XR")?;
            ash::Instance::load(vk_entry.static_fn(), vk::Instance::from_raw(raw as _))
        };

        let vk_physical_device = vk::PhysicalDevice::from_raw(
            xr_instance.vulkan_graphics_device(system, vk_instance.handle().as_raw() as _)? as _,
        );

        let queue_family_index = vk_instance
            .get_physical_device_queue_family_properties(vk_physical_device)
            .into_iter()
            .enumerate()
            .find_map(|(i, info)| {
                info.queue_flags
                    .contains(vk::QueueFlags::GRAPHICS)
                    .then_some(i as u32)
            })
            .context("no graphics queue")?;

        let vk_device = {
            let raw = xr_instance
                .create_vulkan_device(
                    system,
                    std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                    vk_physical_device.as_raw() as _,
                    &vk::DeviceCreateInfo::default()
                        .queue_create_infos(&[vk::DeviceQueueCreateInfo::default()
                            .queue_family_index(queue_family_index)
                            .queue_priorities(&[1.0])])
                        .push_next(&mut vk::PhysicalDeviceMultiviewFeatures {
                            multiview: vk::TRUE,
                            ..Default::default()
                        }) as *const _ as *const _,
                )
                .map_err(|e| anyhow::anyhow!("xr create_vulkan_device: {e}"))?
                .map_err(vk::Result::from_raw)
                .context("vkCreateDevice via XR")?;
            ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(raw as _))
        };
        let queue = vk_device.get_device_queue(queue_family_index, 0);

        let (session, mut frame_wait, mut frame_stream) = xr_instance
            .create_session::<xr::Vulkan>(
                system,
                &xr::vulkan::SessionCreateInfo {
                    instance: vk_instance.handle().as_raw() as _,
                    physical_device: vk_physical_device.as_raw() as _,
                    device: vk_device.handle().as_raw() as _,
                    queue_family_index,
                    queue_index: 0,
                },
            )?;

        let action_set = xr_instance.create_action_set("alchemy_input", "Alchemy input", 0)?;
        let hands = [
            xr_instance.string_to_path("/user/hand/left")?,
            xr_instance.string_to_path("/user/hand/right")?,
        ];
        let menu_action = action_set.create_action::<bool>("menu", "Menu toggle", &[])?;
        let select_action = action_set.create_action::<bool>("select", "Select / click", &[])?;
        let aim_action = action_set.create_action::<xr::Posef>("aim", "Aim pose", &hands)?;
        let thumbstick_action =
            action_set.create_action::<xr::Vector2f>("thumbstick", "Locomotion stick", &hands)?;

        // simple_controller: menu / select / aim (no thumbstick).
        xr_instance.suggest_interaction_profile_bindings(
            xr_instance.string_to_path("/interaction_profiles/khr/simple_controller")?,
            &[
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/left/input/menu/click")?,
                ),
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/right/input/menu/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/left/input/select/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/right/input/select/click")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
                ),
            ],
        )?;

        // Oculus Touch / Quest-style (SteamVR OpenXR).
        let _ = xr_instance.suggest_interaction_profile_bindings(
            xr_instance.string_to_path("/interaction_profiles/oculus/touch_controller")?,
            &[
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/left/input/menu/click")?,
                ),
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/left/input/y/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/left/input/x/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/right/input/a/click")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &thumbstick_action,
                    xr_instance.string_to_path("/user/hand/left/input/thumbstick")?,
                ),
                xr::Binding::new(
                    &thumbstick_action,
                    xr_instance.string_to_path("/user/hand/right/input/thumbstick")?,
                ),
            ],
        );

        // Valve Index.
        let _ = xr_instance.suggest_interaction_profile_bindings(
            xr_instance.string_to_path("/interaction_profiles/valve/index_controller")?,
            &[
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/left/input/b/click")?,
                ),
                xr::Binding::new(
                    &menu_action,
                    xr_instance.string_to_path("/user/hand/right/input/b/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/left/input/trigger/click")?,
                ),
                xr::Binding::new(
                    &select_action,
                    xr_instance.string_to_path("/user/hand/right/input/trigger/click")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/left/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &aim_action,
                    xr_instance.string_to_path("/user/hand/right/input/aim/pose")?,
                ),
                xr::Binding::new(
                    &thumbstick_action,
                    xr_instance.string_to_path("/user/hand/left/input/thumbstick")?,
                ),
                xr::Binding::new(
                    &thumbstick_action,
                    xr_instance.string_to_path("/user/hand/right/input/thumbstick")?,
                ),
            ],
        );

        session.attach_action_sets(&[&action_set])?;

        let aim_left = aim_action.create_space(session.clone(), hands[0], xr::Posef::IDENTITY)?;
        let aim_right = aim_action.create_space(session.clone(), hands[1], xr::Posef::IDENTITY)?;
        let stage =
            session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;
        let view_ref =
            session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;

        let view_mask = !(!0 << VIEW_COUNT);
        let render_pass = vk_device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&[vk::AttachmentDescription {
                    format: COLOR_FORMAT,
                    samples: vk::SampleCountFlags::TYPE_1,
                    load_op: vk::AttachmentLoadOp::CLEAR,
                    store_op: vk::AttachmentStoreOp::STORE,
                    initial_layout: vk::ImageLayout::UNDEFINED,
                    final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    ..Default::default()
                }])
                .subpasses(&[vk::SubpassDescription::default()
                    .color_attachments(&[vk::AttachmentReference {
                        attachment: 0,
                        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    }])
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)])
                .dependencies(&[vk::SubpassDependency {
                    src_subpass: vk::SUBPASS_EXTERNAL,
                    dst_subpass: 0,
                    src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                    ..Default::default()
                }])
                .push_next(
                    &mut vk::RenderPassMultiviewCreateInfo::default()
                        .view_masks(&[view_mask])
                        .correlation_masks(&[view_mask]),
                ),
            None,
        )?;

        let spv = compile_wgsl_spv(CLEAR_WGSL).context("compile clear shader")?;
        let module = vk_device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spv), None)?;
        let pipeline_layout =
            vk_device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)?;
        let pipeline = create_graphics_pipeline(&vk_device, render_pass, pipeline_layout, module)?;
        vk_device.destroy_shader_module(module, None);

        let cmd_pool = vk_device.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family_index)
                .flags(
                    vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER
                        | vk::CommandPoolCreateFlags::TRANSIENT,
                ),
            None,
        )?;
        let cmds = vk_device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(cmd_pool)
                .command_buffer_count(PIPELINE_DEPTH as u32),
        )?;
        let fences: Vec<_> = (0..PIPELINE_DEPTH)
            .map(|_| {
                vk_device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            })
            .collect::<std::result::Result<_, _>>()?;

        let mut ui = SystemUi::new();
        ui.init_cpu_canvas(UI_CANVAS_W, UI_CANVAS_H)?;
        ui.rebuild();
        log::info!(
            "OpenXR ready — Menu toggles dash, Select clicks, thumbstick Noclip, checker floor (x000a)"
        );

        let mut swapchain: Option<Swapchain> = None;
        let mut ui_swapchain: Option<UiSwapchain> = None;
        let staging_bytes = (UI_CANVAS_W * UI_CANVAS_H * 4).max(FLOOR_TEX_W * FLOOR_TEX_H * 4);
        let staging = create_staging_buffer(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            staging_bytes as vk::DeviceSize,
        )?;

        // World floor: checkerboard quad in stage space, offset by −user_root (Desktop metaphor).
        let mut floor_swapchain = create_sized_swapchain(&session, FLOOR_TEX_W, FLOOR_TEX_H)?;
        {
            let pixels = make_checkerboard_rgba(FLOOR_TEX_W, FLOOR_TEX_H, FLOOR_CELLS);
            let n = floor_swapchain.images.len();
            for _ in 0..n {
                let index = floor_swapchain.handle.acquire_image()?;
                floor_swapchain.handle.wait_image(xr::Duration::INFINITE)?;
                upload_rgba_to_image(
                    &vk_device,
                    queue,
                    cmd_pool,
                    &staging,
                    floor_swapchain.images[index as usize],
                    &pixels,
                    FLOOR_TEX_W,
                    FLOOR_TEX_H,
                )?;
                floor_swapchain.handle.release_image()?;
            }
        }
        log::info!("XR checker floor ready ({FLOOR_METERS}m, offset by user_root)");

        let mut event_storage = xr::EventDataBuffer::new();
        let mut session_running = false;
        let mut frame_i = 0usize;
        let mut menu_was_down = false;
        let mut select_was_down = false;
        let mut exit_requested = false;
        // Comfort yaw rotates viewed world (floor) only — move stays controller-aim relative.
        let mut view_yaw = 0.0_f32;
        let mut snap_cooldown = 0.0_f32;

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
                            log::info!("OpenXR session READY");
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

            if !xr_frame_state.should_render {
                frame_stream.end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[],
                )?;
                continue;
            }

            let sc = swapchain.get_or_insert_with(|| {
                let views = xr_instance
                    .enumerate_view_configuration_views(system, VIEW_TYPE)
                    .expect("view config");
                assert_eq!(views.len(), VIEW_COUNT as usize);
                let resolution = vk::Extent2D {
                    width: views[0].recommended_image_rect_width,
                    height: views[0].recommended_image_rect_height,
                };
                let handle = session
                    .create_swapchain(&xr::SwapchainCreateInfo {
                        create_flags: xr::SwapchainCreateFlags::EMPTY,
                        usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                            | xr::SwapchainUsageFlags::SAMPLED,
                        format: COLOR_FORMAT.as_raw() as _,
                        sample_count: 1,
                        width: resolution.width,
                        height: resolution.height,
                        face_count: 1,
                        array_size: VIEW_COUNT,
                        mip_count: 1,
                    })
                    .expect("create swapchain");
                let images = handle.enumerate_images().expect("swapchain images");
                let buffers = images
                    .into_iter()
                    .map(|color_image| {
                        let color_image = vk::Image::from_raw(color_image);
                        let color = vk_device
                            .create_image_view(
                                &vk::ImageViewCreateInfo::default()
                                    .image(color_image)
                                    .view_type(vk::ImageViewType::TYPE_2D_ARRAY)
                                    .format(COLOR_FORMAT)
                                    .subresource_range(vk::ImageSubresourceRange {
                                        aspect_mask: vk::ImageAspectFlags::COLOR,
                                        base_mip_level: 0,
                                        level_count: 1,
                                        base_array_layer: 0,
                                        layer_count: VIEW_COUNT,
                                    }),
                                None,
                            )
                            .expect("image view");
                        let framebuffer = vk_device
                            .create_framebuffer(
                                &vk::FramebufferCreateInfo::default()
                                    .render_pass(render_pass)
                                    .width(resolution.width)
                                    .height(resolution.height)
                                    .attachments(&[color])
                                    .layers(1),
                                None,
                            )
                            .expect("framebuffer");
                        Framebuffer { framebuffer, color }
                    })
                    .collect();
                Swapchain {
                    handle,
                    resolution,
                    buffers,
                }
            });

            let image_index = sc.handle.acquire_image()?;
            // Wait until the image is ready to render into.
            sc.handle.wait_image(xr::Duration::INFINITE)?;

            vk_device.wait_for_fences(&[fences[frame_i]], true, u64::MAX)?;
            vk_device.reset_fences(&[fences[frame_i]])?;

            let cmd = cmds[frame_i];
            vk_device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            vk_device.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(render_pass)
                    .framebuffer(sc.buffers[image_index as usize].framebuffer)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent: sc.resolution,
                    })
                    .clear_values(&[vk::ClearValue {
                        color: vk::ClearColorValue {
                            // Distinct teal so HMD clearly shows Alchemy XR path (Coin pose logged).
                            float32: [0.05, 0.12, 0.16, 1.0],
                        },
                    }]),
                vk::SubpassContents::INLINE,
            );
            let viewports = [vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: sc.resolution.width as f32,
                height: sc.resolution.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }];
            let scissors = [vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.resolution,
            }];
            vk_device.cmd_set_viewport(cmd, 0, &viewports);
            vk_device.cmd_set_scissor(cmd, 0, &scissors);
            vk_device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
            // Fullscreen triangle — world backdrop (mesh/floor follow-up; pose still tracked).
            vk_device.cmd_draw(cmd, 3, 1, 0, 0);
            vk_device.cmd_end_render_pass(cmd);
            vk_device.end_command_buffer(cmd)?;
            vk_device.queue_submit(
                queue,
                &[vk::SubmitInfo::default().command_buffers(&[cmd])],
                fences[frame_i],
            )?;
            sc.handle.release_image()?;

            let (_, views) =
                session.locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)?;

            // ---- input / UI (CPU) ----
            session.sync_actions(&[(&action_set).into()])?;
            let menu_down = menu_action.state(&session, xr::Path::NULL)?.current_state;
            if menu_down && !menu_was_down {
                ui.toggle();
                log::info!("XR menu → system_ui open={}", ui.state.open);
            }
            menu_was_down = menu_down;

            let head_located = view_ref.locate(&stage, xr_frame_state.predicted_display_time)?;
            let (head_pos, head_orient) = openxr_pose_to_isometry(head_located.pose);

            let select_down = select_action.state(&session, xr::Path::NULL)?.current_state;
            if select_down && !select_was_down && ui.state.open {
                let aim_for_ui = aim_right
                    .locate(&stage, xr_frame_state.predicted_display_time)
                    .ok()
                    .filter(|l| {
                        l.location_flags
                            .contains(xr::SpaceLocationFlags::POSITION_VALID)
                    })
                    .or_else(|| {
                        aim_left
                            .locate(&stage, xr_frame_state.predicted_display_time)
                            .ok()
                            .filter(|l| {
                                l.location_flags
                                    .contains(xr::SpaceLocationFlags::POSITION_VALID)
                            })
                    });
                if let Some(aim_loc) = aim_for_ui {
                    let (aim_pos, aim_orient) = openxr_pose_to_isometry(aim_loc.pose);
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
                                    host_control::request_host_shutdown(z);
                                }
                                exit_requested = true;
                            }
                            UiAction::None => {}
                        }
                    }
                }
            }
            select_was_down = select_down;

            // ---- VR thumbstick Noclip Intent (x000a) ----
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
                if !dash_open {
                    view_yaw = update_turn_yaw(
                        view_yaw,
                        right_stick.x,
                        turn_settings,
                        dt,
                        &mut snap_cooldown,
                        TURN_DEADZONE,
                    );
                }

                let mut intent = MoveIntent {
                    suppress: dash_open,
                    ..Default::default()
                };
                if !dash_open {
                    // Stick × controller aim in stage, then R(-view_yaw) → world Intent
                    // (floor is drawn with R(+view_yaw); signs must stay inverse).
                    let stage = intent_from_sticks(
                        left_ori,
                        (left_stick.x, left_stick.y),
                        right_ori,
                        (right_stick.x, right_stick.y),
                        max_speed,
                        DEFAULT_MOVE_SPEED,
                        STICK_DEADZONE,
                        TURN_DEADZONE,
                    );
                    let v = stage_move_to_world(stage, view_yaw);
                    intent.move_x = v.x;
                    intent.move_y = v.y;
                    intent.move_z = v.z;
                }
                if let Ok(mut slot) = shared.intent.lock() {
                    *slot = intent;
                }
            }

            ui.prepare_cpu_canvas();

            let _ = (&mesh, dash_quad_model(head_pos, head_orient));
            let user_root = shared
                .entities
                .lock()
                .ok()
                .and_then(|m| m.get(&USER_ROOT_ENTITY_ID).copied())
                .unwrap_or(crate::scene::EntityPose {
                    x: 0.0,
                    y: DEFAULT_EYE_Y,
                    z: 6.0,
                });
            if frame_i == 0 {
                if let Ok(map) = shared.entities.lock() {
                    if let Some(p) = map.get(&MESH_ENTITY_ID) {
                        log::debug!("XR coin pose {:.2},{:.2},{:.2}", p.x, p.y, p.z);
                    }
                }
                log::debug!(
                    "XR user_root {:.2},{:.2},{:.2}",
                    user_root.x,
                    user_root.y,
                    user_root.z
                );
            }

            let ui_sc = ui_swapchain.get_or_insert_with(|| {
                create_ui_swapchain(&session, &vk_device).expect("ui swapchain")
            });

            let rect = rect_to_xr(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.resolution,
            });
            let proj_views = [
                xr::CompositionLayerProjectionView::new()
                    .pose(views[0].pose)
                    .fov(views[0].fov)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&sc.handle)
                            .image_array_index(0)
                            .image_rect(rect),
                    ),
                xr::CompositionLayerProjectionView::new()
                    .pose(views[1].pose)
                    .fov(views[1].fov)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&sc.handle)
                            .image_array_index(1)
                            .image_rect(rect),
                    ),
            ];
            let proj = xr::CompositionLayerProjection::new()
                .space(&stage)
                .views(&proj_views);

            // Horizontal floor: rotate −90° about X so quad XY → world XZ; offset −user_root.
            // Acquire each frame (content already uploaded); same release-before-end pattern as UI.
            let _floor_index = floor_swapchain.handle.acquire_image()?;
            floor_swapchain.handle.wait_image(xr::Duration::INFINITE)?;
            floor_swapchain.handle.release_image()?;
            // Floor: yaw (comfort turn) * pitch −90°; position = yaw * (−user_root).
            let yaw_q = Quat::from_rotation_y(view_yaw);
            let floor_ori = yaw_q * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            let floor_pos = yaw_q * Vec3::new(-user_root.x, -user_root.y, -user_root.z);
            let floor_rect = xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: FLOOR_TEX_W as i32,
                    height: FLOOR_TEX_H as i32,
                },
            };
            let floor_quad = xr::CompositionLayerQuad::new()
                .space(&stage)
                .eye_visibility(xr::EyeVisibility::BOTH)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&floor_swapchain.handle)
                        .image_array_index(0)
                        .image_rect(floor_rect),
                )
                .pose(xr::Posef {
                    orientation: xr::Quaternionf {
                        x: floor_ori.x,
                        y: floor_ori.y,
                        z: floor_ori.z,
                        w: floor_ori.w,
                    },
                    position: xr::Vector3f {
                        x: floor_pos.x,
                        y: floor_pos.y,
                        z: floor_pos.z,
                    },
                })
                .size(xr::Extent2Df {
                    width: FLOOR_METERS,
                    height: FLOOR_METERS,
                });

            if ui.state.open {
                if let Some((pixels, cw, ch)) = ui.canvas_rgba() {
                    let ui_index = ui_sc.handle.acquire_image()?;
                    ui_sc.handle.wait_image(xr::Duration::INFINITE)?;
                    upload_rgba_to_image(
                        &vk_device,
                        queue,
                        cmd_pool,
                        &staging,
                        ui_sc.images[ui_index as usize],
                        pixels,
                        cw,
                        ch,
                    )?;
                    ui_sc.handle.release_image()?;
                    let ui_rect = xr::Rect2Di {
                        offset: xr::Offset2Di { x: 0, y: 0 },
                        extent: xr::Extent2Di {
                            width: UI_CANVAS_W as i32,
                            height: UI_CANVAS_H as i32,
                        },
                    };
                    let quad = xr::CompositionLayerQuad::new()
                        .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
                        .space(&view_ref)
                        .eye_visibility(xr::EyeVisibility::BOTH)
                        .sub_image(
                            xr::SwapchainSubImage::new()
                                .swapchain(&ui_sc.handle)
                                .image_array_index(0)
                                .image_rect(ui_rect),
                        )
                        .pose(xr::Posef {
                            orientation: xr::Quaternionf {
                                x: 0.0,
                                y: 0.0,
                                z: 0.0,
                                w: 1.0,
                            },
                            position: xr::Vector3f {
                                x: 0.0,
                                y: 0.0,
                                z: -super::input::QUAD_DISTANCE,
                            },
                        })
                        .size(xr::Extent2Df {
                            width: QUAD_WIDTH,
                            height: QUAD_HEIGHT,
                        });
                    frame_stream.end(
                        xr_frame_state.predicted_display_time,
                        environment_blend_mode,
                        &[&proj, &floor_quad, &quad],
                    )?;
                } else {
                    frame_stream.end(
                        xr_frame_state.predicted_display_time,
                        environment_blend_mode,
                        &[&proj, &floor_quad],
                    )?;
                }
            } else {
                frame_stream.end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[&proj, &floor_quad],
                )?;
            }

            frame_i = (frame_i + 1) % PIPELINE_DEPTH;
        }

        // Drop XR session before destroying Vulkan objects.
        drop(aim_left);
        drop(aim_right);
        drop(view_ref);
        drop(stage);
        drop(ui_swapchain);
        drop(floor_swapchain);
        drop(session);
        if let Some(sc) = swapchain.take() {
            for b in sc.buffers {
                vk_device.destroy_framebuffer(b.framebuffer, None);
                vk_device.destroy_image_view(b.color, None);
            }
        }
        destroy_staging(&vk_device, &staging);
        for f in fences {
            vk_device.destroy_fence(f, None);
        }
        vk_device.destroy_command_pool(cmd_pool, None);
        vk_device.destroy_pipeline(pipeline, None);
        vk_device.destroy_pipeline_layout(pipeline_layout, None);
        vk_device.destroy_render_pass(render_pass, None);
        vk_device.destroy_device(None);
        vk_instance.destroy_instance(None);
    }

    Ok(())
}

struct Swapchain {
    handle: xr::Swapchain<xr::Vulkan>,
    resolution: vk::Extent2D,
    buffers: Vec<Framebuffer>,
}

struct Framebuffer {
    framebuffer: vk::Framebuffer,
    color: vk::ImageView,
}

struct UiSwapchain {
    handle: xr::Swapchain<xr::Vulkan>,
    images: Vec<vk::Image>,
}

struct StagingBuffer {
    memory: vk::DeviceMemory,
    buffer: vk::Buffer,
    size: vk::DeviceSize,
}

fn create_ui_swapchain(
    session: &xr::Session<xr::Vulkan>,
    _vk_device: &ash::Device,
) -> Result<UiSwapchain> {
    create_sized_swapchain(session, UI_CANVAS_W, UI_CANVAS_H)
}

fn create_sized_swapchain(
    session: &xr::Session<xr::Vulkan>,
    width: u32,
    height: u32,
) -> Result<UiSwapchain> {
    let handle = session.create_swapchain(&xr::SwapchainCreateInfo {
        create_flags: xr::SwapchainCreateFlags::EMPTY,
        usage_flags: xr::SwapchainUsageFlags::TRANSFER_DST | xr::SwapchainUsageFlags::SAMPLED,
        format: COLOR_FORMAT.as_raw() as _,
        sample_count: 1,
        width,
        height,
        face_count: 1,
        array_size: 1,
        mip_count: 1,
    })?;
    let images = handle
        .enumerate_images()?
        .into_iter()
        .map(vk::Image::from_raw)
        .collect();
    Ok(UiSwapchain { handle, images })
}

/// High-contrast checker so thumbstick loco is visible without mesh world draw.
fn make_checkerboard_rgba(width: u32, height: u32, cells: u32) -> Vec<u8> {
    let cells = cells.max(2);
    let mut out = vec![0u8; (width * height * 4) as usize];
    let cell_w = width as f32 / cells as f32;
    let cell_h = height as f32 / cells as f32;
    for y in 0..height {
        for x in 0..width {
            let cx = (x as f32 / cell_w) as u32;
            let cy = (y as f32 / cell_h) as u32;
            let light = (cx + cy) % 2 == 0;
            let (r, g, b) = if light {
                (210u8, 215, 220)
            } else {
                (45u8, 50, 58)
            };
            let i = ((y * width + x) * 4) as usize;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = 255;
        }
    }
    out
}

fn create_staging_buffer(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    size: vk::DeviceSize,
) -> Result<StagingBuffer> {
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?
    };
    let reqs = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type = find_memory_type(
        instance,
        physical,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type),
            None,
        )?
    };
    unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
    Ok(StagingBuffer {
        memory,
        buffer,
        size,
    })
}

fn destroy_staging(device: &ash::Device, staging: &StagingBuffer) {
    unsafe {
        device.destroy_buffer(staging.buffer, None);
        device.free_memory(staging.memory, None);
    }
}

fn find_memory_type(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    type_bits: u32,
    props: vk::MemoryPropertyFlags,
) -> Result<u32> {
    let mem_props = unsafe { instance.get_physical_device_memory_properties(physical) };
    for i in 0..mem_props.memory_type_count {
        if type_bits & (1 << i) != 0
            && mem_props.memory_types[i as usize]
                .property_flags
                .contains(props)
        {
            return Ok(i);
        }
    }
    bail!("no suitable Vulkan memory type for staging")
}

fn upload_rgba_to_image(
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    staging: &StagingBuffer,
    image: vk::Image,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<()> {
    let bytes = (width * height * 4) as usize;
    if pixels.len() < bytes || staging.size < bytes as u64 {
        bail!("staging/pixel size mismatch");
    }
    unsafe {
        let ptr =
            device.map_memory(staging.memory, 0, staging.size, vk::MemoryMapFlags::empty())?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr as *mut u8, bytes);
        device.unmap_memory(staging.memory);

        let cmd = device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(cmd_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )?[0];
        device.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let barrier_to_dst = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_dst],
        );
        device.cmd_copy_buffer_to_image(
            cmd,
            staging.buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D::default(),
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            }],
        );
        let barrier_to_sample = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_sample],
        );
        device.end_command_buffer(cmd)?;
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
        device.queue_submit(
            queue,
            &[vk::SubmitInfo::default().command_buffers(&[cmd])],
            fence,
        )?;
        device.wait_for_fences(&[fence], true, u64::MAX)?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(cmd_pool, &[cmd]);
    }
    Ok(())
}

fn rect_to_xr(r: vk::Rect2D) -> xr::Rect2Di {
    xr::Rect2Di {
        offset: xr::Offset2Di {
            x: r.offset.x,
            y: r.offset.y,
        },
        extent: xr::Extent2Di {
            width: r.extent.width as i32,
            height: r.extent.height as i32,
        },
    }
}

const CLEAR_WGSL: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.08, 0.18, 0.22, 1.0);
}
"#;

fn compile_wgsl_spv(src: &str) -> Result<Vec<u32>> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| anyhow::anyhow!("{e}"))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let spv = naga::back::spv::write_vec(
        &module,
        &info,
        &naga::back::spv::Options {
            lang_version: (1, 3),
            ..Default::default()
        },
        None,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(spv)
}

fn create_graphics_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    module: vk::ShaderModule,
) -> Result<vk::Pipeline> {
    let entry = c"vs_main";
    let entry_fs = c"fs_main";
    let stages = [
        vk::PipelineShaderStageCreateInfo {
            stage: vk::ShaderStageFlags::VERTEX,
            module,
            p_name: entry.as_ptr(),
            ..Default::default()
        },
        vk::PipelineShaderStageCreateInfo {
            stage: vk::ShaderStageFlags::FRAGMENT,
            module,
            p_name: entry_fs.as_ptr(),
            ..Default::default()
        },
    ];
    let noop = vk::StencilOpState::default();
    let pipes = unsafe {
        device.create_graphics_pipelines(
            vk::PipelineCache::null(),
            &[vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vk::PipelineVertexInputStateCreateInfo::default())
                .input_assembly_state(
                    &vk::PipelineInputAssemblyStateCreateInfo::default()
                        .topology(vk::PrimitiveTopology::TRIANGLE_LIST),
                )
                .viewport_state(
                    &vk::PipelineViewportStateCreateInfo::default()
                        .viewport_count(1)
                        .scissor_count(1),
                )
                .rasterization_state(
                    &vk::PipelineRasterizationStateCreateInfo::default()
                        .cull_mode(vk::CullModeFlags::NONE)
                        .line_width(1.0),
                )
                .multisample_state(
                    &vk::PipelineMultisampleStateCreateInfo::default()
                        .rasterization_samples(vk::SampleCountFlags::TYPE_1),
                )
                .depth_stencil_state(
                    &vk::PipelineDepthStencilStateCreateInfo::default()
                        .front(noop)
                        .back(noop),
                )
                .color_blend_state(
                    &vk::PipelineColorBlendStateCreateInfo::default().attachments(&[
                        vk::PipelineColorBlendAttachmentState {
                            color_write_mask: vk::ColorComponentFlags::RGBA,
                            ..Default::default()
                        },
                    ]),
                )
                .dynamic_state(
                    &vk::PipelineDynamicStateCreateInfo::default()
                        .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]),
                )
                .layout(layout)
                .render_pass(render_pass)
                .subpass(0)],
            None,
        )
    }
    .map_err(|(_, e)| e)?;
    Ok(pipes[0])
}

fn ctrlc_set(running: Arc<AtomicBool>) -> Result<()> {
    // Optional: without ctrlc crate, ignore. Use env / Exit UI instead.
    let _ = running;
    Ok(())
}
