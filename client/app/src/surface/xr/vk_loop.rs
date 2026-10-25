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
use bytemuck::{Pod, Zeroable};
use openxr as xr;

use alchemy_protocol::DEFAULT_MOVE_SPEED;

use super::input::{
    dash_quad_model, laser_length, laser_model, openxr_pose_to_isometry, pose_to_ray,
    ray_hit_dash_quad,
};
use super::loco::{combine_turn_axes, intent_from_sticks, stage_move_to_world, STICK_DEADZONE};
use super::render_model::{enable_if_available, RenderModelCache};
use crate::host_control;
use crate::mesh::{CpuMesh, CpuSubmesh};
use crate::scene::{MoveIntent, SharedState, MESH_ENTITY_ID, USER_ROOT_ENTITY_ID};
use crate::system_ui::{SystemUi, UiAction};
use crate::turn::{update_turn_yaw, TURN_DEADZONE};
use glam::{Mat4, Quat, Vec3};

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
const VIEW_COUNT: u32 = 2;
const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;
const PIPELINE_DEPTH: usize = 2;
const NEAR_Z: f32 = 0.05;
const FAR_Z: f32 = 100.0;
const UI_CANVAS_W: u32 = 960;
const UI_CANVAS_H: u32 = 540;
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

        let aim_left = aim_action.create_space(&session, hands[0], xr::Posef::IDENTITY)?;
        let aim_right = aim_action.create_space(&session, hands[1], xr::Posef::IDENTITY)?;
        let stage =
            session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;
        let view_ref =
            session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;

        let mut render_models = RenderModelCache::new(rm_support);

        let view_mask = !(!0 << VIEW_COUNT);
        let render_pass = vk_device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&[
                    vk::AttachmentDescription {
                        format: COLOR_FORMAT,
                        samples: vk::SampleCountFlags::TYPE_1,
                        load_op: vk::AttachmentLoadOp::CLEAR,
                        store_op: vk::AttachmentStoreOp::STORE,
                        initial_layout: vk::ImageLayout::UNDEFINED,
                        final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                        ..Default::default()
                    },
                    vk::AttachmentDescription {
                        format: DEPTH_FORMAT,
                        samples: vk::SampleCountFlags::TYPE_1,
                        load_op: vk::AttachmentLoadOp::CLEAR,
                        store_op: vk::AttachmentStoreOp::DONT_CARE,
                        stencil_load_op: vk::AttachmentLoadOp::DONT_CARE,
                        stencil_store_op: vk::AttachmentStoreOp::DONT_CARE,
                        initial_layout: vk::ImageLayout::UNDEFINED,
                        final_layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                        ..Default::default()
                    },
                ])
                .subpasses(&[vk::SubpassDescription::default()
                    .color_attachments(&[vk::AttachmentReference {
                        attachment: 0,
                        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    }])
                    .depth_stencil_attachment(&vk::AttachmentReference {
                        attachment: 1,
                        layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                    })
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)])
                .dependencies(&[vk::SubpassDependency {
                    src_subpass: vk::SUBPASS_EXTERNAL,
                    dst_subpass: 0,
                    src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                    dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                    dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
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
        let pipeline = create_clear_pipeline(&vk_device, render_pass, pipeline_layout, module)?;
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
        let mesh_pipe = create_mesh_pipeline(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue,
            cmd_pool,
            render_pass,
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
            "OpenXR ready — Menu toggles dash, Select+aim laser clicks Exit, thumbstick Noclip, checker floor, render models (x000e laser)"
        );

        let mut swapchain: Option<Swapchain> = None;
        let mut depth_res: Option<DepthResources> = None;
        let mut controller_gpu: Vec<GpuController> = Vec::new();
        let mut controller_gpu_gen: u64 = 0;
        let staging_bytes = UI_CANVAS_W * UI_CANVAS_H * 4;
        let staging = create_staging_buffer(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            staging_bytes as vk::DeviceSize,
        )?;

        // Floor mesh in projection (depth-tested with controllers). Not a CompositionLayerQuad.
        let floor_gpu = upload_gpu_submesh(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue,
            cmd_pool,
            &mesh_pipe,
            &floor_plane_submesh(),
        )?;
        log::info!("XR checker floor mesh ready ({FLOOR_METERS}m, depth-tested)");

        // Dash UI lives in the same projection as the laser (x000e) so draw order / depth match.
        let ui_plane_gpu = upload_gpu_submesh(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue,
            cmd_pool,
            &mesh_pipe,
            &ui_plane_submesh(),
        )?;
        let blank_ui = vec![0u8; staging_bytes as usize];
        let ui_canvas_tex = upload_rgba_texture(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue,
            cmd_pool,
            &mesh_pipe.tex_set_layout,
            &blank_ui,
            UI_CANVAS_W,
            UI_CANVAS_H,
        )?;
        log::info!("XR dash UI plane ready (projection, alpha-blended)");

        // Unit laser box along local -Z (z=0..-1); scaled per-frame by laser_model.
        let laser_gpu = upload_gpu_submesh(
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue,
            cmd_pool,
            &mesh_pipe,
            &laser_beam_submesh(),
        )?;
        log::info!("XR UI laser mesh ready (dash-open aim beam)");

        let mut event_storage = xr::EventDataBuffer::new();
        let mut session_running = false;
        let mut frame_i = 0usize;
        let mut menu_was_down = false;
        let mut select_was_down = false;
        let mut exit_requested = false;
        // Comfort yaw rotates viewed world (floor) only — move stays controller-aim relative.
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
                            log::info!("OpenXR session READY");
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

            if !xr_frame_state.should_render {
                frame_stream.end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[],
                )?;
                continue;
            }

            // Actions first so render-model enumeration (SteamVR) and input share one sync.
            session.sync_actions(&[(&action_set).into()])?;
            if let Err(err) = render_models.refresh_if_dirty(&session) {
                log::warn!("XR render model refresh: {err:#}");
            }
            if controller_gpu_gen != render_models.generation {
                for g in controller_gpu.drain(..) {
                    destroy_gpu_controller(&vk_device, g);
                }
                for m in render_models.models() {
                    match upload_gpu_controller(
                        &vk_instance,
                        vk_physical_device,
                        &vk_device,
                        queue,
                        cmd_pool,
                        &mesh_pipe,
                        &m.mesh.submeshes,
                    ) {
                        Ok(g) => controller_gpu.push(g),
                        Err(err) => log::warn!("controller GPU upload: {err:#}"),
                    }
                }
                controller_gpu_gen = render_models.generation;
            }

            let (_, views) =
                session.locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)?;

            if swapchain.is_none() {
                let cfg = xr_instance
                    .enumerate_view_configuration_views(system, VIEW_TYPE)
                    .expect("view config");
                assert_eq!(cfg.len(), VIEW_COUNT as usize);
                let resolution = vk::Extent2D {
                    width: cfg[0].recommended_image_rect_width,
                    height: cfg[0].recommended_image_rect_height,
                };
                let depth = create_depth_resources(
                    &vk_instance,
                    vk_physical_device,
                    &vk_device,
                    resolution,
                )
                .expect("depth resources");
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
                                    .attachments(&[color, depth.view])
                                    .layers(1),
                                None,
                            )
                            .expect("framebuffer");
                        Framebuffer { framebuffer, color }
                    })
                    .collect();
                depth_res = Some(depth);
                swapchain = Some(Swapchain {
                    handle,
                    resolution,
                    buffers,
                });
            }
            let sc = swapchain.as_mut().unwrap();

            let image_index = sc.handle.acquire_image()?;
            sc.handle.wait_image(xr::Duration::INFINITE)?;

            vk_device.wait_for_fences(&[fences[frame_i]], true, u64::MAX)?;
            vk_device.reset_fences(&[fences[frame_i]])?;

            // Comfort turn + user_root before projection so floor depth-tests with controllers.
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
                .entities
                .lock()
                .ok()
                .and_then(|m| m.get(&USER_ROOT_ENTITY_ID).copied())
                .unwrap_or(crate::scene::EntityPose {
                    x: 0.0,
                    y: DEFAULT_EYE_Y,
                    z: 6.0,
                });
            let yaw_q = Quat::from_rotation_y(view_yaw);
            let floor_model = Mat4::from_rotation_translation(
                yaw_q,
                yaw_q * Vec3::new(-user_root.x, -user_root.y, -user_root.z),
            );

            // Menu before canvas upload so open/close applies this frame.
            let menu_down = menu_action.state(&session, xr::Path::NULL)?.current_state;
            if menu_down && !menu_was_down {
                ui.toggle();
                log::info!("XR menu → system_ui open={}", ui.state.open);
            }
            menu_was_down = menu_down;

            // Head + UI aim before projection so laser draws with the same pose as Select.
            let head_located = view_ref.locate(&stage, xr_frame_state.predicted_display_time)?;
            let (head_pos, head_orient) = openxr_pose_to_isometry(head_located.pose);
            let aim_for_ui = if ui.state.open {
                locate_ui_aim(
                    &aim_right,
                    &aim_left,
                    &stage,
                    xr_frame_state.predicted_display_time,
                )
            } else {
                None
            };
            let laser_draw = aim_for_ui.map(|(aim_pos, aim_orient)| {
                let ray = pose_to_ray(aim_pos, aim_orient);
                let len = laser_length(
                    head_pos,
                    head_orient,
                    ray,
                    UI_CANVAS_W as f32,
                    UI_CANVAS_H as f32,
                );
                laser_model(aim_pos, aim_orient, len)
            });

            // Hover/press before rasterize so Button colors land on this frame's canvas.
            let select_down_for_ui = select_action.state(&session, xr::Path::NULL)?.current_state;
            if ui.state.open {
                if let Some((aim_pos, aim_orient)) = aim_for_ui {
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
            }

            let dash_ui_model = if ui.state.open {
                ui.prepare_cpu_canvas();
                if let Some((pixels, cw, ch)) = ui.canvas_rgba() {
                    upload_rgba_to_image(
                        &vk_device,
                        queue,
                        cmd_pool,
                        &staging,
                        ui_canvas_tex.image,
                        pixels,
                        cw,
                        ch,
                    )?;
                }
                Some(dash_quad_model(head_pos, head_orient))
            } else {
                None
            };

            update_view_proj_ubo(&vk_device, &mesh_pipe, &views)?;

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
                    .clear_values(&[
                        vk::ClearValue {
                            color: vk::ClearColorValue {
                                float32: [0.05, 0.12, 0.16, 1.0],
                            },
                        },
                        vk::ClearValue {
                            depth_stencil: vk::ClearDepthStencilValue {
                                depth: 1.0,
                                stencil: 0,
                            },
                        },
                    ]),
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
            vk_device.cmd_draw(cmd, 3, 1, 0, 0);

            // Floor then controllers in one projection (depth-tested).
            vk_device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, mesh_pipe.pipeline);
            vk_device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                mesh_pipe.layout,
                0,
                &[mesh_pipe.views_set],
                &[],
            );
            bind_mesh_texture(&vk_device, cmd, &mesh_pipe, mesh_pipe.white_set);
            push_mesh_consts(
                &vk_device,
                cmd,
                mesh_pipe.layout,
                floor_model,
                MESH_MODE_FLOOR,
                [1.0, 1.0, 1.0, 1.0],
            );
            vk_device.cmd_bind_vertex_buffers(cmd, 0, &[floor_gpu.vbo], &[0]);
            vk_device.cmd_bind_index_buffer(cmd, floor_gpu.ibo, 0, vk::IndexType::UINT32);
            vk_device.cmd_draw_indexed(cmd, floor_gpu.index_count, 1, 0, 0, 0);

            let xr_inst = session.instance();
            for (i, gpu) in controller_gpu.iter().enumerate() {
                let Some(model) = render_models.models_mut().get_mut(i) else {
                    break;
                };
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
                let root = Mat4::from_rotation_translation(ori, pos);
                let (worlds, visible) = model.compose_node_worlds();
                for sub in &gpu.submeshes {
                    if !visible.get(sub.node_index).copied().unwrap_or(false) {
                        continue;
                    }
                    let node_world = worlds
                        .get(sub.node_index)
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    let model_mat = root * node_world;
                    let (mode, tex_set) = if let Some(tex) = sub.texture.as_ref() {
                        (MESH_MODE_TEXTURED, tex.set)
                    } else {
                        (MESH_MODE_CONTROLLER, mesh_pipe.white_set)
                    };
                    bind_mesh_texture(&vk_device, cmd, &mesh_pipe, tex_set);
                    push_mesh_consts(
                        &vk_device,
                        cmd,
                        mesh_pipe.layout,
                        model_mat,
                        mode,
                        sub.factor,
                    );
                    vk_device.cmd_bind_vertex_buffers(cmd, 0, &[sub.vbo], &[0]);
                    vk_device.cmd_bind_index_buffer(cmd, sub.ibo, 0, vk::IndexType::UINT32);
                    vk_device.cmd_draw_indexed(cmd, sub.index_count, 1, 0, 0, 0);
                }
            }

            // Dash UI then laser (same projection): tip stops short of the plane (LASER_TIP_GAP).
            if let Some(ui_model) = dash_ui_model {
                bind_mesh_texture(&vk_device, cmd, &mesh_pipe, ui_canvas_tex.set);
                push_mesh_consts(
                    &vk_device,
                    cmd,
                    mesh_pipe.layout,
                    ui_model,
                    MESH_MODE_TEXTURED,
                    [1.0, 1.0, 1.0, 1.0],
                );
                vk_device.cmd_bind_vertex_buffers(cmd, 0, &[ui_plane_gpu.vbo], &[0]);
                vk_device.cmd_bind_index_buffer(cmd, ui_plane_gpu.ibo, 0, vk::IndexType::UINT32);
                vk_device.cmd_draw_indexed(cmd, ui_plane_gpu.index_count, 1, 0, 0, 0);
            }

            if let Some(laser_mat) = laser_draw {
                bind_mesh_texture(&vk_device, cmd, &mesh_pipe, mesh_pipe.white_set);
                push_mesh_consts(
                    &vk_device,
                    cmd,
                    mesh_pipe.layout,
                    laser_mat,
                    MESH_MODE_FLAT,
                    LASER_COLOR,
                );
                vk_device.cmd_bind_vertex_buffers(cmd, 0, &[laser_gpu.vbo], &[0]);
                vk_device.cmd_bind_index_buffer(cmd, laser_gpu.ibo, 0, vk::IndexType::UINT32);
                vk_device.cmd_draw_indexed(cmd, laser_gpu.index_count, 1, 0, 0, 0);
            }

            vk_device.cmd_end_render_pass(cmd);
            vk_device.end_command_buffer(cmd)?;
            vk_device.queue_submit(
                queue,
                &[vk::SubmitInfo::default().command_buffers(&[cmd])],
                fences[frame_i],
            )?;
            sc.handle.release_image()?;

            // ---- select UI (CPU); menu + canvas already applied above ----
            if select_down_for_ui && !select_was_down {
                if let Some((aim_pos, aim_orient)) = aim_for_ui {
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
            select_was_down = select_down_for_ui;

            let _ = &mesh;
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

            // Dash UI + laser are inside projection (depth-tested). No CompositionLayerQuad.
            frame_stream.end(
                xr_frame_state.predicted_display_time,
                environment_blend_mode,
                &[&proj],
            )?;

            frame_i = (frame_i + 1) % PIPELINE_DEPTH;
        }

        // Drop XR session before destroying Vulkan objects.
        render_models.clear(&xr_instance);
        for g in controller_gpu.drain(..) {
            destroy_gpu_controller(&vk_device, g);
        }
        drop(aim_left);
        drop(aim_right);
        drop(view_ref);
        drop(stage);
        drop(session);
        destroy_gpu_submesh(&vk_device, laser_gpu);
        destroy_gpu_texture(&vk_device, ui_canvas_tex);
        destroy_gpu_submesh(&vk_device, ui_plane_gpu);
        destroy_gpu_submesh(&vk_device, floor_gpu);
        if let Some(sc) = swapchain.take() {
            for b in sc.buffers {
                vk_device.destroy_framebuffer(b.framebuffer, None);
                vk_device.destroy_image_view(b.color, None);
            }
        }
        if let Some(d) = depth_res.take() {
            destroy_depth_resources(&vk_device, d);
        }
        destroy_mesh_pipeline(&vk_device, mesh_pipe);
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

struct DepthResources {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

struct GpuTexture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    sampler: vk::Sampler,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
}

struct GpuSubmesh {
    vbo: vk::Buffer,
    vmem: vk::DeviceMemory,
    ibo: vk::Buffer,
    imem: vk::DeviceMemory,
    index_count: u32,
    texture: Option<GpuTexture>,
    factor: [f32; 4],
    /// Index into the controller's rig nodes (x000d).
    node_index: usize,
}

struct GpuController {
    submeshes: Vec<GpuSubmesh>,
}

struct MeshPipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    views_set_layout: vk::DescriptorSetLayout,
    tex_set_layout: vk::DescriptorSetLayout,
    views_pool: vk::DescriptorPool,
    views_set: vk::DescriptorSet,
    ubo: vk::Buffer,
    ubo_mem: vk::DeviceMemory,
    white_image: vk::Image,
    white_memory: vk::DeviceMemory,
    white_view: vk::ImageView,
    white_sampler: vk::Sampler,
    white_pool: vk::DescriptorPool,
    white_set: vk::DescriptorSet,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewProjUbo {
    /// Two view-projection matrices (left, right), column-major.
    mats: [[f32; 16]; 2],
}

struct StagingBuffer {
    memory: vk::DeviceMemory,
    buffer: vk::Buffer,
    size: vk::DeviceSize,
}

fn floor_plane_submesh() -> CpuSubmesh {
    let h = FLOOR_METERS * 0.5;
    CpuSubmesh {
        positions: vec![[-h, 0.0, -h], [h, 0.0, -h], [h, 0.0, h], [-h, 0.0, h]],
        uvs: vec![[0.0, 0.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        base_color: None,
        base_color_factor: [1.0, 1.0, 1.0, 1.0],
        node_index: 0,
    }
}

/// Unit quad in local XY (scaled by [`dash_quad_model`]). UV: top-left origin (canvas).
fn ui_plane_submesh() -> CpuSubmesh {
    CpuSubmesh {
        positions: vec![
            [-0.5, 0.5, 0.0],
            [0.5, 0.5, 0.0],
            [0.5, -0.5, 0.0],
            [-0.5, -0.5, 0.0],
        ],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        base_color: None,
        base_color_factor: [1.0, 1.0, 1.0, 1.0],
        node_index: 0,
    }
}

/// Unit box along local -Z from z=0 to z=-1; X/Y half-extent 0.5 (scaled by laser_model).
fn laser_beam_submesh() -> CpuSubmesh {
    // 8 corners: near (z=0) then far (z=-1).
    let positions = vec![
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
        [-0.5, -0.5, -1.0],
        [0.5, -0.5, -1.0],
        [0.5, 0.5, -1.0],
        [-0.5, 0.5, -1.0],
    ];
    let indices = vec![
        // near
        0, 1, 2, 0, 2, 3, // far
        4, 6, 5, 4, 7, 6, // +X
        1, 5, 6, 1, 6, 2, // -X
        0, 3, 7, 0, 7, 4, // +Y
        3, 2, 6, 3, 6, 7, // -Y
        0, 4, 5, 0, 5, 1,
    ];
    CpuSubmesh {
        positions,
        uvs: vec![[0.0, 0.0]; 8],
        indices,
        base_color: None,
        base_color_factor: LASER_COLOR,
        node_index: 0,
    }
}

/// Prefer right aim, then left (same priority as Select click).
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
    aim_right
        .locate(stage, time)
        .ok()
        .filter(valid)
        .or_else(|| aim_left.locate(stage, time).ok().filter(valid))
        .map(|l| openxr_pose_to_isometry(l.pose))
}

fn push_mesh_consts(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    layout: vk::PipelineLayout,
    model: Mat4,
    mode: u32,
    factor: [f32; 4],
) {
    let mut bytes = [0u8; MESH_PUSH_BYTES as usize];
    let cols = model.to_cols_array();
    bytes[..64].copy_from_slice(bytemuck::bytes_of(&cols));
    bytes[64..68].copy_from_slice(&mode.to_le_bytes());
    bytes[80..96].copy_from_slice(bytemuck::bytes_of(&factor));
    unsafe {
        device.cmd_push_constants(
            cmd,
            layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            &bytes,
        );
    }
}

fn bind_mesh_texture(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    mesh_pipe: &MeshPipeline,
    tex_set: vk::DescriptorSet,
) {
    unsafe {
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            mesh_pipe.layout,
            1,
            &[tex_set],
            &[],
        );
    }
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

fn create_clear_pipeline(
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
                        .depth_test_enable(false)
                        .depth_write_enable(false)
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

/// Push: model (64) + mode u32 + pad (12) + baseColorFactor (16) → 96 bytes.
const MESH_PUSH_BYTES: u32 = 96;
const MESH_MODE_CONTROLLER: u32 = 0;
const MESH_MODE_FLOOR: u32 = 1;
const MESH_MODE_TEXTURED: u32 = 2;
const MESH_MODE_FLAT: u32 = 3;
/// Cyan UI laser (opaque).
const LASER_COLOR: [f32; 4] = [0.15, 0.95, 1.0, 1.0];

const MESH_WGSL: &str = r#"
struct Views {
    view_proj: array<mat4x4<f32>, 2>,
}
@group(0) @binding(0) var<uniform> views: Views;
@group(1) @binding(0) var base_tex: texture_2d<f32>;
@group(1) @binding(1) var base_samp: sampler;

struct Push {
    model: mat4x4<f32>,
    mode: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
    factor: vec4<f32>,
}
var<push_constant> push: Push;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Local XZ so floor checker is painted on the mesh (moves with loco, no UV scroll).
    @location(0) local_xz: vec2<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @builtin(view_index) view_index: i32,
) -> VsOut {
    var o: VsOut;
    o.local_xz = position.xz;
    o.uv = uv;
    let world = push.model * vec4<f32>(position, 1.0);
    let vp = views.view_proj[u32(view_index)];
    o.pos = vp * world;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (push.mode == 1u) {
        // 2.5 m cells ≈ FLOOR_METERS/16.
        let cell = 2.5;
        let c = floor(in.local_xz / cell);
        let light = ((i32(c.x) + i32(c.y)) & 1) == 0;
        if (light) {
            return vec4<f32>(0.82, 0.84, 0.86, 1.0);
        }
        return vec4<f32>(0.18, 0.20, 0.23, 1.0);
    }
    if (push.mode == 2u) {
        return textureSample(base_tex, base_samp, in.uv) * push.factor;
    }
    if (push.mode == 3u) {
        // UI laser / flat tint from push.factor (x000e).
        return push.factor;
    }
    // Flat controller color when GLB has no usable baseColor (x000b fallback).
    return vec4<f32>(0.75, 0.78, 0.82, 1.0);
}
"#;

fn create_mesh_pipeline(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    render_pass: vk::RenderPass,
) -> Result<MeshPipeline> {
    let spv = compile_wgsl_spv_multiview(MESH_WGSL).context("compile mesh shader")?;
    let module = unsafe {
        device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spv), None)?
    };

    let views_bindings = [vk::DescriptorSetLayoutBinding {
        binding: 0,
        descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
        descriptor_count: 1,
        stage_flags: vk::ShaderStageFlags::VERTEX,
        ..Default::default()
    }];
    let views_set_layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&views_bindings),
            None,
        )?
    };
    let tex_bindings = [
        vk::DescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
            descriptor_count: 1,
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            ..Default::default()
        },
        vk::DescriptorSetLayoutBinding {
            binding: 1,
            descriptor_type: vk::DescriptorType::SAMPLER,
            descriptor_count: 1,
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            ..Default::default()
        },
    ];
    let tex_set_layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&tex_bindings),
            None,
        )?
    };
    let push_ranges = [vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: MESH_PUSH_BYTES,
    }];
    let layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&[views_set_layout, tex_set_layout])
                .push_constant_ranges(&push_ranges),
            None,
        )?
    };

    let ubo_size = std::mem::size_of::<ViewProjUbo>() as vk::DeviceSize;
    let (ubo, ubo_mem) = create_buffer(
        instance,
        physical,
        device,
        ubo_size,
        vk::BufferUsageFlags::UNIFORM_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let views_pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&[vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::UNIFORM_BUFFER,
                    descriptor_count: 1,
                }]),
            None,
        )?
    };
    let views_set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(views_pool)
                .set_layouts(&[views_set_layout]),
        )?[0]
    };
    let ubo_info = [vk::DescriptorBufferInfo {
        buffer: ubo,
        offset: 0,
        range: ubo_size,
    }];
    unsafe {
        device.update_descriptor_sets(
            &[vk::WriteDescriptorSet {
                dst_set: views_set,
                dst_binding: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::UNIFORM_BUFFER,
                p_buffer_info: ubo_info.as_ptr(),
                ..Default::default()
            }],
            &[],
        );
    }

    let white = create_solid_texture(
        instance,
        physical,
        device,
        queue,
        cmd_pool,
        &tex_set_layout,
        [255, 255, 255, 255],
    )?;

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
    let binding_desc = [vk::VertexInputBindingDescription {
        binding: 0,
        stride: std::mem::size_of::<MeshVertex>() as u32,
        input_rate: vk::VertexInputRate::VERTEX,
    }];
    let attr_desc = [
        vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        },
        vk::VertexInputAttributeDescription {
            location: 1,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: 12,
        },
    ];
    let noop = vk::StencilOpState::default();
    let pipes = unsafe {
        device.create_graphics_pipelines(
            vk::PipelineCache::null(),
            &[vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(
                    &vk::PipelineVertexInputStateCreateInfo::default()
                        .vertex_binding_descriptions(&binding_desc)
                        .vertex_attribute_descriptions(&attr_desc),
                )
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
                        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                        .line_width(1.0),
                )
                .multisample_state(
                    &vk::PipelineMultisampleStateCreateInfo::default()
                        .rasterization_samples(vk::SampleCountFlags::TYPE_1),
                )
                .depth_stencil_state(
                    &vk::PipelineDepthStencilStateCreateInfo::default()
                        .depth_test_enable(true)
                        .depth_write_enable(true)
                        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
                        .front(noop)
                        .back(noop),
                )
                .color_blend_state(
                    &vk::PipelineColorBlendStateCreateInfo::default().attachments(&[
                        vk::PipelineColorBlendAttachmentState {
                            blend_enable: vk::TRUE,
                            src_color_blend_factor: vk::BlendFactor::SRC_ALPHA,
                            dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
                            color_blend_op: vk::BlendOp::ADD,
                            src_alpha_blend_factor: vk::BlendFactor::ONE,
                            dst_alpha_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
                            alpha_blend_op: vk::BlendOp::ADD,
                            color_write_mask: vk::ColorComponentFlags::RGBA,
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
    unsafe { device.destroy_shader_module(module, None) };

    Ok(MeshPipeline {
        pipeline: pipes[0],
        layout,
        views_set_layout,
        tex_set_layout,
        views_pool,
        views_set,
        ubo,
        ubo_mem,
        white_image: white.image,
        white_memory: white.memory,
        white_view: white.view,
        white_sampler: white.sampler,
        white_pool: white.pool,
        white_set: white.set,
    })
}

fn destroy_mesh_pipeline(device: &ash::Device, p: MeshPipeline) {
    unsafe {
        device.destroy_pipeline(p.pipeline, None);
        device.destroy_pipeline_layout(p.layout, None);
        device.destroy_descriptor_pool(p.views_pool, None);
        device.destroy_descriptor_pool(p.white_pool, None);
        device.destroy_descriptor_set_layout(p.views_set_layout, None);
        device.destroy_descriptor_set_layout(p.tex_set_layout, None);
        device.destroy_sampler(p.white_sampler, None);
        device.destroy_image_view(p.white_view, None);
        device.destroy_image(p.white_image, None);
        device.free_memory(p.white_memory, None);
        device.destroy_buffer(p.ubo, None);
        device.free_memory(p.ubo_mem, None);
    }
}

fn update_view_proj_ubo(
    device: &ash::Device,
    mesh_pipe: &MeshPipeline,
    views: &[xr::View],
) -> Result<()> {
    if views.len() < 2 {
        bail!("expected stereo views");
    }
    let mut ubo = ViewProjUbo {
        mats: [[0.0; 16]; 2],
    };
    for (i, view) in views.iter().take(2).enumerate() {
        let (pos, ori) = openxr_pose_to_isometry(view.pose);
        let view_mat = Mat4::from_rotation_translation(ori, pos).inverse();
        let proj = projection_from_fov(view.fov, NEAR_Z, FAR_Z);
        ubo.mats[i] = (proj * view_mat).to_cols_array();
    }
    unsafe {
        let ptr = device.map_memory(
            mesh_pipe.ubo_mem,
            0,
            std::mem::size_of::<ViewProjUbo>() as u64,
            vk::MemoryMapFlags::empty(),
        )?;
        std::ptr::copy_nonoverlapping(
            &ubo as *const ViewProjUbo as *const u8,
            ptr as *mut u8,
            std::mem::size_of::<ViewProjUbo>(),
        );
        device.unmap_memory(mesh_pipe.ubo_mem);
    }
    Ok(())
}

fn projection_from_fov(fov: xr::Fovf, near: f32, far: f32) -> Mat4 {
    let tan_left = fov.angle_left.tan();
    let tan_right = fov.angle_right.tan();
    let tan_up = fov.angle_up.tan();
    let tan_down = fov.angle_down.tan();
    let tan_width = tan_right - tan_left;
    // Vulkan NDC Y grows downward; OpenXR view Y grows upward → use (down − up).
    let tan_height = tan_down - tan_up;
    Mat4::from_cols(
        glam::Vec4::new(2.0 / tan_width, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 / tan_height, 0.0, 0.0),
        glam::Vec4::new(
            (tan_right + tan_left) / tan_width,
            (tan_up + tan_down) / tan_height,
            -far / (far - near),
            -1.0,
        ),
        glam::Vec4::new(0.0, 0.0, -(far * near) / (far - near), 0.0),
    )
}

fn create_depth_resources(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    extent: vk::Extent2D,
) -> Result<DepthResources> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(DEPTH_FORMAT)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(VIEW_COUNT)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let image = unsafe { device.create_image(&image_info, None)? };
    let reqs = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = find_memory_type(
        instance,
        physical,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type),
            None,
        )?
    };
    unsafe { device.bind_image_memory(image, memory, 0)? };
    let view = unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D_ARRAY)
                .format(DEPTH_FORMAT)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: VIEW_COUNT,
                }),
            None,
        )?
    };
    Ok(DepthResources {
        image,
        memory,
        view,
    })
}

fn destroy_depth_resources(device: &ash::Device, d: DepthResources) {
    unsafe {
        device.destroy_image_view(d.view, None);
        device.destroy_image(d.image, None);
        device.free_memory(d.memory, None);
    }
}

fn create_buffer(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    props: vk::MemoryPropertyFlags,
) -> Result<(vk::Buffer, vk::DeviceMemory)> {
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?
    };
    let reqs = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type = find_memory_type(instance, physical, reqs.memory_type_bits, props)?;
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type),
            None,
        )?
    };
    unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
    Ok((buffer, memory))
}

fn upload_gpu_controller(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    mesh_pipe: &MeshPipeline,
    submeshes: &[CpuSubmesh],
) -> Result<GpuController> {
    let mut out = Vec::with_capacity(submeshes.len());
    for sub in submeshes {
        out.push(upload_gpu_submesh(
            instance, physical, device, queue, cmd_pool, mesh_pipe, sub,
        )?);
    }
    Ok(GpuController { submeshes: out })
}

fn upload_gpu_submesh(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    mesh_pipe: &MeshPipeline,
    sub: &CpuSubmesh,
) -> Result<GpuSubmesh> {
    if sub.positions.len() != sub.uvs.len() {
        bail!(
            "submesh pos/uv len mismatch: {} vs {}",
            sub.positions.len(),
            sub.uvs.len()
        );
    }
    let mut verts = Vec::with_capacity(sub.positions.len());
    for (pos, uv) in sub.positions.iter().zip(sub.uvs.iter()) {
        verts.push(MeshVertex { pos: *pos, uv: *uv });
    }
    let vbytes = bytemuck::cast_slice::<MeshVertex, u8>(&verts);
    let ibytes = bytemuck::cast_slice::<u32, u8>(&sub.indices);
    let (vbo, vmem) = create_buffer(
        instance,
        physical,
        device,
        vbytes.len() as u64,
        vk::BufferUsageFlags::VERTEX_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let (ibo, imem) = create_buffer(
        instance,
        physical,
        device,
        ibytes.len() as u64,
        vk::BufferUsageFlags::INDEX_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    unsafe {
        let ptr = device.map_memory(vmem, 0, vbytes.len() as u64, vk::MemoryMapFlags::empty())?;
        std::ptr::copy_nonoverlapping(vbytes.as_ptr(), ptr as *mut u8, vbytes.len());
        device.unmap_memory(vmem);
        let ptr = device.map_memory(imem, 0, ibytes.len() as u64, vk::MemoryMapFlags::empty())?;
        std::ptr::copy_nonoverlapping(ibytes.as_ptr(), ptr as *mut u8, ibytes.len());
        device.unmap_memory(imem);
    }

    let texture = match sub.base_color.as_ref() {
        Some(img) => Some(upload_rgba_texture(
            instance,
            physical,
            device,
            queue,
            cmd_pool,
            &mesh_pipe.tex_set_layout,
            &img.pixels,
            img.width,
            img.height,
        )?),
        None => None,
    };

    Ok(GpuSubmesh {
        vbo,
        vmem,
        ibo,
        imem,
        index_count: sub.indices.len() as u32,
        texture,
        factor: sub.base_color_factor,
        node_index: sub.node_index,
    })
}

fn destroy_gpu_controller(device: &ash::Device, g: GpuController) {
    for sub in g.submeshes {
        destroy_gpu_submesh(device, sub);
    }
}

fn destroy_gpu_submesh(device: &ash::Device, g: GpuSubmesh) {
    if let Some(tex) = g.texture {
        destroy_gpu_texture(device, tex);
    }
    unsafe {
        device.destroy_buffer(g.vbo, None);
        device.free_memory(g.vmem, None);
        device.destroy_buffer(g.ibo, None);
        device.free_memory(g.imem, None);
    }
}

fn destroy_gpu_texture(device: &ash::Device, t: GpuTexture) {
    unsafe {
        device.destroy_descriptor_pool(t.pool, None);
        device.destroy_sampler(t.sampler, None);
        device.destroy_image_view(t.view, None);
        device.destroy_image(t.image, None);
        device.free_memory(t.memory, None);
    }
}

fn create_solid_texture(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    tex_set_layout: &vk::DescriptorSetLayout,
    rgba: [u8; 4],
) -> Result<GpuTexture> {
    upload_rgba_texture(
        instance,
        physical,
        device,
        queue,
        cmd_pool,
        tex_set_layout,
        &rgba,
        1,
        1,
    )
}

fn upload_rgba_texture(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    queue: vk::Queue,
    cmd_pool: vk::CommandPool,
    tex_set_layout: &vk::DescriptorSetLayout,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<GpuTexture> {
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .context("texture size overflow")?;
    if pixels.len() < bytes {
        bail!("texture pixel buffer too small");
    }

    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let image = unsafe { device.create_image(&image_info, None)? };
    let reqs = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = find_memory_type(
        instance,
        physical,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type),
            None,
        )?
    };
    unsafe { device.bind_image_memory(image, memory, 0)? };

    let view = unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk::Format::R8G8B8A8_SRGB)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                }),
            None,
        )?
    };
    let sampler = unsafe {
        device.create_sampler(
            &vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .max_lod(0.0),
            None,
        )?
    };

    let staging = create_staging_buffer(instance, physical, device, bytes as vk::DeviceSize)?;
    let upload = upload_rgba_to_image(
        device,
        queue,
        cmd_pool,
        &staging,
        image,
        &pixels[..bytes],
        width,
        height,
    );
    destroy_staging(device, &staging);
    upload?;

    let pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&[
                    vk::DescriptorPoolSize {
                        ty: vk::DescriptorType::SAMPLED_IMAGE,
                        descriptor_count: 1,
                    },
                    vk::DescriptorPoolSize {
                        ty: vk::DescriptorType::SAMPLER,
                        descriptor_count: 1,
                    },
                ]),
            None,
        )?
    };
    let set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&[*tex_set_layout]),
        )?[0]
    };
    let image_infos = [vk::DescriptorImageInfo {
        sampler: vk::Sampler::null(),
        image_view: view,
        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    }];
    let sampler_infos = [vk::DescriptorImageInfo {
        sampler,
        image_view: vk::ImageView::null(),
        image_layout: vk::ImageLayout::UNDEFINED,
    }];
    unsafe {
        device.update_descriptor_sets(
            &[
                vk::WriteDescriptorSet {
                    dst_set: set,
                    dst_binding: 0,
                    descriptor_count: 1,
                    descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                    p_image_info: image_infos.as_ptr(),
                    ..Default::default()
                },
                vk::WriteDescriptorSet {
                    dst_set: set,
                    dst_binding: 1,
                    descriptor_count: 1,
                    descriptor_type: vk::DescriptorType::SAMPLER,
                    p_image_info: sampler_infos.as_ptr(),
                    ..Default::default()
                },
            ],
            &[],
        );
    }

    Ok(GpuTexture {
        image,
        memory,
        view,
        sampler,
        pool,
        set,
    })
}

fn compile_wgsl_spv_multiview(src: &str) -> Result<Vec<u32>> {
    let module =
        naga::front::wgsl::parse_str(src).map_err(|e| anyhow::anyhow!("wgsl parse: {e}"))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| {
        // naga::WithSpan prints inner causes when Display'd via Debug.
        anyhow::anyhow!("wgsl validate: {e:?}")
    })?;
    let spv = naga::back::spv::write_vec(
        &module,
        &info,
        &naga::back::spv::Options {
            lang_version: (1, 3),
            flags: naga::back::spv::WriterFlags::empty(),
            binding_map: Default::default(),
            capabilities: Some(
                [
                    naga::back::spv::Capability::Shader,
                    naga::back::spv::Capability::MultiView,
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        },
        None,
    )
    .map_err(|e| anyhow::anyhow!("spv write: {e}"))?;
    Ok(spv)
}

fn ctrlc_set(running: Arc<AtomicBool>) -> Result<()> {
    // Optional: without ctrlc crate, ignore. Use env / Exit UI instead.
    let _ = running;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_wgsl_compiles_with_multiview() {
        let spv = match compile_wgsl_spv_multiview(MESH_WGSL) {
            Ok(s) => s,
            Err(e) => panic!("MESH_WGSL: {e:#}"),
        };
        assert!(!spv.is_empty());
    }

    #[test]
    fn clear_wgsl_compiles() {
        let spv = compile_wgsl_spv(CLEAR_WGSL).expect("CLEAR_WGSL");
        assert!(!spv.is_empty());
    }
}
