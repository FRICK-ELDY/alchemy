//! OpenXR clear-only path via wgpu (x0050 / `ALCHEMY_XR_WGPU=1`).
//!
//! Guarantees: acquire → wgpu clear (distinct color) → release → project to compositor;
//! drop 順: eye_views → textures → swapchain → stage → frame_stream → frame_wait →
//! session → [`XrWgpuBridge`]. `frame_stream.begin` 後は成功／失敗／panic とも `end` 必須。
//! Non-guarantees: mesh / UI / laser / controllers (→ x0051).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use openxr as xr;

use super::wgpu_interop::{self, XrWgpuBridge, VIEW_COUNT, WGPU_CLEAR_COLOR};
use crate::mesh::CpuMesh;
use crate::scene::SharedState;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

/// After acquire: success path uses [`Self::release`]; [`Drop`] retries if still armed
/// (panic/unwind, or fallible `release` that failed before disarm).
struct SwapchainReleaseGuard<'a> {
    swapchain: &'a mut xr::Swapchain<xr::Vulkan>,
    armed: bool,
}

impl<'a> SwapchainReleaseGuard<'a> {
    fn arm(swapchain: &'a mut xr::Swapchain<xr::Vulkan>) -> Self {
        Self {
            swapchain,
            armed: true,
        }
    }

    fn wait_image(&mut self, timeout: xr::Duration) -> Result<()> {
        self.swapchain
            .wait_image(timeout)
            .map_err(|e| anyhow::anyhow!("wait_image: {e}"))
    }

    /// Fallible release for the normal path. Disarm only after success so a failed
    /// `release_image` still leaves [`Drop`] armed to retry before empty `frame end`.
    fn release(mut self) -> Result<()> {
        self.swapchain
            .release_image()
            .context("swapchain release_image")?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for SwapchainReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(err) = self.swapchain.release_image() {
                log::warn!("swapchain release_image on panic/unwind: {err}");
            }
        }
    }
}

/// After `frame_stream.begin`: success uses `end_*`; [`Drop`] empty-ends on panic/`?` skip.
struct FrameEndGuard<'s> {
    stream: &'s mut xr::FrameStream<xr::Vulkan>,
    display_time: xr::Time,
    blend: xr::EnvironmentBlendMode,
    ended: bool,
}

impl<'s> FrameEndGuard<'s> {
    fn new(
        stream: &'s mut xr::FrameStream<xr::Vulkan>,
        display_time: xr::Time,
        blend: xr::EnvironmentBlendMode,
    ) -> Self {
        Self {
            stream,
            display_time,
            blend,
            ended: false,
        }
    }

    fn end_empty(mut self) -> Result<()> {
        self.ended = true;
        self.stream
            .end(self.display_time, self.blend, &[])
            .context("frame_stream.end (empty)")
    }

    fn end_projection(
        mut self,
        proj: &xr::CompositionLayerProjection<'_, xr::Vulkan>,
    ) -> Result<()> {
        self.ended = true;
        self.stream
            .end(self.display_time, self.blend, &[proj])
            .context("frame_stream.end (projection)")
    }
}

impl Drop for FrameEndGuard<'_> {
    fn drop(&mut self) {
        if !self.ended {
            if let Err(err) = self.stream.end(self.display_time, self.blend, &[]) {
                log::warn!("frame_stream.end (empty) on panic/unwind: {err}");
            }
        }
    }
}

pub fn run_clear_only(
    _mesh: CpuMesh,
    _shared: Arc<SharedState>,
    _zenoh: Option<zenoh::Session>,
) -> Result<()> {
    // Safety: process-global OpenXR loader (same as ash path).
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
        &(),
    )?;
    let props = xr_instance.properties()?;
    log::info!(
        "OpenXR runtime (wgpu clear): {} {}",
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

    // Declared first → dropped last (after session / swapchain / textures on `?` unwind).
    // Safety: see XrWgpuBridge::create — Drop destroys ash after wgpu.
    let bridge = unsafe { XrWgpuBridge::create(&xr_instance, system) }?;
    // Safety: SessionCreateInfo handles match the live Vulkan objects in `bridge`.
    // Binding order: session, frame_wait, frame_stream → drop reverse = stream, wait, session
    // (stream/wait hold Session Arc; must die before session, before bridge).
    let (session, mut frame_wait, mut frame_stream) =
        unsafe { xr_instance.create_session::<xr::Vulkan>(system, &bridge.session_create_info())? };

    let stage =
        session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;

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
    // Safety: images from this swapchain; textures/views dropped before swapchain (decl order).
    let textures =
        unsafe { bridge.wrap_swapchain_images(&raw_images, width, height, wgpu_format) }?;
    let eye_views = XrWgpuBridge::make_eye_views(&textures, wgpu_format);

    log::info!(
        "ALCHEMY_XR_WGPU=1 clear-only — magenta clear ({width}x{height}×{VIEW_COUNT}); quit SteamVR / kill process to exit"
    );

    let mut event_storage = xr::EventDataBuffer::new();
    let mut session_running = false;

    'main_loop: loop {
        while let Some(event) = xr_instance.poll_event(&mut event_storage)? {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => match e.state() {
                    xr::SessionState::READY => {
                        session.begin(VIEW_TYPE)?;
                        session_running = true;
                        log::info!("OpenXR session READY (wgpu clear)");
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
        let frame_end = FrameEndGuard::new(
            &mut frame_stream,
            xr_frame_state.predicted_display_time,
            environment_blend_mode,
        );

        if !xr_frame_state.should_render {
            frame_end.end_empty()?;
            continue;
        }

        // On `?` before end_*: FrameEndGuard Drop empty-ends. release() disarms only on
        // success; failure keeps armed so Drop retries before that empty end.
        let (_, views) =
            session.locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)?;
        ensure!(
            views.len() >= VIEW_COUNT as usize,
            "locate_views returned {} views, need {VIEW_COUNT}",
            views.len()
        );

        let image_index = swapchain.acquire_image()? as usize;
        // Guard before wait_image so wait/clear failure still releases (Drop or release()).
        let mut release = SwapchainReleaseGuard::arm(&mut swapchain);
        release.wait_image(xr::Duration::INFINITE)?;

        let eyes = eye_views.get(image_index).with_context(|| {
            format!(
                "swapchain image index {image_index} out of range ({})",
                eye_views.len()
            )
        })?;
        let mut encoder = bridge
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xr-wgpu-clear"),
            });
        for view in eyes {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xr-wgpu-clear-eye"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(WGPU_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        bridge.queue().submit(Some(encoder.finish()));
        // Correctness: GPU done before XR release. x0051: replace with queue fence /
        // OpenXR vulkan sync to avoid per-frame CPU stall (Maintain::Wait).
        let _ = bridge.device().poll(wgpu::Maintain::Wait);
        // wgpu render-pass end leaves COLOR_ATTACHMENT_OPTIMAL-class usage; matches ash
        // path finalLayout and SteamVR OpenXR expectations (validate under VK layers if black).
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
    }

    // Explicit drop order (x0050): views → textures → swapchain → stage →
    // frame_stream → frame_wait → session → bridge (VkDevice/VkInstance).
    drop(eye_views);
    drop(textures);
    drop(swapchain);
    drop(stage);
    drop(frame_stream);
    drop(frame_wait);
    drop(session);
    drop(bridge);
    let _ = entry;
    Ok(())
}
