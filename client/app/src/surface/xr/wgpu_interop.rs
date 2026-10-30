//! Adopt OpenXR-created Vulkan instance/device into wgpu (x0050).
//!
//! Guarantees: same physical device; swapchain `VkImage` → `wgpu::Texture` without
//! transferring ownership (XR owns images; `drop_callback` keeps wgpu from destroying);
//! ash device/instance destroyed on [`Drop`] (incl. `?` failure after create).
//! Non-guarantees: multiview; Bevy; ash deletion.

#![allow(clippy::missing_transmute_annotations)] // OpenXR vulkan_enable2 get_instance_proc_addr

use anyhow::{bail, Context, Result};
use ash::vk::{self, Handle};
use openxr as xr;

pub const VIEW_COUNT: u32 = 2;
const VK_TARGET_VERSION: u32 = vk::make_api_version(0, 1, 1, 0);

/// After acquire: success path uses [`Self::release`]; [`Drop`] retries if still armed
/// (panic/unwind, or fallible `release` that failed before disarm).
pub struct SwapchainReleaseGuard<'a> {
    swapchain: &'a mut xr::Swapchain<xr::Vulkan>,
    armed: bool,
}

impl<'a> SwapchainReleaseGuard<'a> {
    pub fn arm(swapchain: &'a mut xr::Swapchain<xr::Vulkan>) -> Self {
        Self {
            swapchain,
            armed: true,
        }
    }

    pub fn wait_image(&mut self, timeout: xr::Duration) -> Result<()> {
        self.swapchain
            .wait_image(timeout)
            .map_err(|e| anyhow::anyhow!("wait_image: {e}"))
    }

    /// Fallible release for the normal path. Disarm only after success so a failed
    /// `release_image` still leaves [`Drop`] armed to retry before empty `frame end`.
    pub fn release(mut self) -> Result<()> {
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
pub struct FrameEndGuard<'s> {
    stream: &'s mut xr::FrameStream<xr::Vulkan>,
    display_time: xr::Time,
    blend: xr::EnvironmentBlendMode,
    ended: bool,
}

impl<'s> FrameEndGuard<'s> {
    pub fn new(
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

    pub fn end_empty(mut self) -> Result<()> {
        self.ended = true;
        self.stream
            .end(self.display_time, self.blend, &[])
            .context("frame_stream.end (empty)")
    }

    pub fn end_projection(
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

/// `ALCHEMY_XR_WGPU=1` enables wgpu SceneGpu XR session (x0051).
pub fn xr_wgpu_enabled() -> bool {
    xr_wgpu_flag_value(std::env::var("ALCHEMY_XR_WGPU").ok().as_deref())
}

/// Pure flag parse (unit-testable without mutating process env).
pub fn xr_wgpu_flag_value(val: Option<&str>) -> bool {
    matches!(val, Some("1"))
}

pub fn map_swapchain_format(vk_fmt: u32) -> Option<(vk::Format, wgpu::TextureFormat)> {
    let fmt = vk::Format::from_raw(vk_fmt as i32);
    let wgpu_fmt = match fmt {
        vk::Format::R8G8B8A8_SRGB => wgpu::TextureFormat::Rgba8UnormSrgb,
        vk::Format::B8G8R8A8_SRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        vk::Format::R8G8B8A8_UNORM => wgpu::TextureFormat::Rgba8Unorm,
        vk::Format::B8G8R8A8_UNORM => wgpu::TextureFormat::Bgra8Unorm,
        _ => return None,
    };
    Some((fmt, wgpu_fmt))
}

/// Destroys XR-created VkDevice/VkInstance if still owned (create mid-fail).
/// Transfer to [`XrWgpuBridge`] via [`Self::into_parts`] (clears Options so Drop is a no-op).
struct PendingAshVulkan {
    instance: Option<ash::Instance>,
    device: Option<ash::Device>,
}

impl PendingAshVulkan {
    fn new(instance: ash::Instance) -> Self {
        Self {
            instance: Some(instance),
            device: None,
        }
    }

    fn set_device(&mut self, device: ash::Device) {
        self.device = Some(device);
    }

    /// Disarm Drop and hand handles to the bridge.
    fn into_parts(mut self) -> (ash::Instance, ash::Device) {
        let device = self
            .device
            .take()
            .expect("vulkan device set before into_parts");
        let instance = self.instance.take().expect("vulkan instance");
        (instance, device)
    }
}

impl Drop for PendingAshVulkan {
    fn drop(&mut self) {
        // Safety: wgpu has not adopted these handles (create failed before into_parts).
        unsafe {
            if let Some(device) = self.device.take() {
                device.destroy_device(None);
            }
            if let Some(instance) = self.instance.take() {
                instance.destroy_instance(None);
            }
        }
    }
}

/// Owns ash + wgpu. [`Drop`] drops wgpu first, then destroys VkDevice/VkInstance.
///
/// Caller must drop textures → swapchain → `frame_stream` / `frame_wait` / `session`
/// **before** this value (declare bridge first so it drops last on unwind).
pub struct XrWgpuBridge {
    // Options so Drop::drop can release wgpu before vkDestroy* (Drop runs before field drops).
    instance: Option<ash::Instance>,
    device: Option<ash::Device>,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    wgpu_queue: Option<wgpu::Queue>,
    wgpu_device: Option<wgpu::Device>,
    wgpu_adapter: Option<wgpu::Adapter>,
    wgpu_instance: Option<wgpu::Instance>,
}

impl Drop for XrWgpuBridge {
    fn drop(&mut self) {
        // wgpu first (may still touch VkDevice), then ash destroy.
        self.wgpu_queue.take();
        self.wgpu_device.take();
        self.wgpu_adapter.take();
        self.wgpu_instance.take();
        let device = self.device.take();
        let instance = self.instance.take();
        // Safety: XR session / swapchain / wgpu texture wrappers already dropped by caller;
        // wgpu fields taken above so no remaining users of these Vk handles.
        unsafe {
            if let Some(device) = device {
                device.destroy_device(None);
            }
            if let Some(instance) = instance {
                instance.destroy_instance(None);
            }
        }
    }
}

impl XrWgpuBridge {
    /// Build Vulkan via OpenXR `vulkan_enable2` with wgpu-hal required extensions, then adopt.
    ///
    /// # Safety (caller)
    /// `xr_instance` / `system` must support `XR_KHR_vulkan_enable2`. Before dropping this
    /// value: drop frame_wait / frame_stream / session / swapchain / wgpu textures.
    pub unsafe fn create(xr_instance: &xr::Instance, system: xr::SystemId) -> Result<Self> {
        let entry = ash::Entry::load().context("ash::Entry::load")?;
        let flags = wgpu::InstanceFlags::from_build_config().with_env();
        let hal_exts =
            wgpu::hal::vulkan::Instance::desired_extensions(&entry, VK_TARGET_VERSION, flags)
                .map_err(|e| anyhow::anyhow!("wgpu-hal desired_extensions: {e}"))?;

        let ext_ptrs: Vec<*const std::ffi::c_char> = hal_exts.iter().map(|e| e.as_ptr()).collect();
        let app_info = vk::ApplicationInfo::default()
            .application_version(0)
            .engine_version(0)
            .api_version(VK_TARGET_VERSION);
        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&ext_ptrs);

        // Safety: get_instance_proc_addr matches ash Entry; XR creates the VkInstance.
        let get_proc = std::mem::transmute(entry.static_fn().get_instance_proc_addr);
        let vk_instance_raw = xr_instance
            .create_vulkan_instance(system, get_proc, &create_info as *const _ as *const _)
            .map_err(|e| anyhow::anyhow!("xr create_vulkan_instance: {e}"))?
            .map_err(vk::Result::from_raw)
            .context("vkCreateInstance via XR")?;
        let vk_instance = vk::Instance::from_raw(vk_instance_raw as u64);
        let ash_instance = ash::Instance::load(entry.static_fn(), vk_instance);
        let mut pending = PendingAshVulkan::new(ash_instance.clone());

        let vk_phd_raw = xr_instance.vulkan_graphics_device(system, vk_instance_raw)? as u64;
        let vk_phd = vk::PhysicalDevice::from_raw(vk_phd_raw);

        // Safety: instance created with `hal_exts` / VK 1.1; callback skips wgpu destroy.
        let hal_instance = wgpu::hal::vulkan::Instance::from_raw(
            entry.clone(),
            ash_instance.clone(),
            VK_TARGET_VERSION,
            0,
            None,
            hal_exts,
            flags,
            false,
            Some(Box::new(|| {})),
        )
        .map_err(|e| anyhow::anyhow!("hal Instance::from_raw: {e}"))?;

        let exposed = hal_instance
            .expose_adapter(vk_phd)
            .context("expose_adapter (XR physical device)")?;

        let features = wgpu::Features::empty();
        let device_exts = exposed.adapter.required_device_extensions(features);
        let mut phd_features = exposed
            .adapter
            .physical_device_features(&device_exts, features);
        let queue_family_index = ash_instance
            .get_physical_device_queue_family_properties(vk_phd)
            .into_iter()
            .enumerate()
            .find_map(|(i, info)| {
                info.queue_flags
                    .contains(vk::QueueFlags::GRAPHICS)
                    .then_some(i as u32)
            })
            .context("no graphics queue")?;

        let priorities = [1.0f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let ext_ptrs: Vec<*const std::ffi::c_char> =
            device_exts.iter().map(|e| e.as_ptr()).collect();
        let pre_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&ext_ptrs);
        let info = phd_features.add_to_device_create(pre_info);

        let vk_device_raw = xr_instance
            .create_vulkan_device(
                system,
                get_proc,
                vk_phd.as_raw() as _,
                &info as *const _ as *const _,
            )
            .map_err(|e| anyhow::anyhow!("xr create_vulkan_device: {e}"))?
            .map_err(vk::Result::from_raw)
            .context("vkCreateDevice via XR")?;
        let ash_device = ash::Device::load(
            ash_instance.fp_v1_0(),
            vk::Device::from_raw(vk_device_raw as u64),
        );
        pending.set_device(ash_device.clone());

        // Safety: device opened with required_device_extensions + features; callback non-destroying.
        let open_device = exposed
            .adapter
            .device_from_raw(
                ash_device.clone(),
                Some(Box::new(|| {})),
                &device_exts,
                features,
                &wgpu::MemoryHints::Performance,
                queue_family_index,
                0,
            )
            .map_err(|e| anyhow::anyhow!("device_from_raw: {e}"))?;

        // Safety: hal_instance is the one that exposed this adapter.
        let wgpu_instance = wgpu::Instance::from_hal::<wgpu::hal::api::Vulkan>(hal_instance);
        // Safety: exposed adapter came from that instance.
        let wgpu_adapter = wgpu_instance.create_adapter_from_hal::<wgpu::hal::api::Vulkan>(exposed);
        // Safety: open_device was built from that adapter.
        let (wgpu_device, wgpu_queue) = wgpu_adapter
            .create_device_from_hal::<wgpu::hal::api::Vulkan>(
                open_device,
                &wgpu::DeviceDescriptor {
                    label: Some("alchemy-xr-wgpu"),
                    required_features: features,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .context("create_device_from_hal")?;

        log::info!(
            "XR wgpu adopted: {} (queue family {queue_family_index})",
            wgpu_adapter.get_info().name
        );

        // Move ash into Self; into_parts disarms PendingAshVulkan Drop (no mem::forget).
        let (instance, device) = pending.into_parts();

        Ok(Self {
            instance: Some(instance),
            device: Some(device),
            physical_device: vk_phd,
            queue_family_index,
            wgpu_queue: Some(wgpu_queue),
            wgpu_device: Some(wgpu_device),
            wgpu_adapter: Some(wgpu_adapter),
            wgpu_instance: Some(wgpu_instance),
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        self.wgpu_device.as_ref().expect("bridge alive")
    }

    pub fn queue(&self) -> &wgpu::Queue {
        self.wgpu_queue.as_ref().expect("bridge alive")
    }

    pub fn session_create_info(&self) -> xr::vulkan::SessionCreateInfo {
        let instance = self.instance.as_ref().expect("bridge alive");
        let device = self.device.as_ref().expect("bridge alive");
        xr::vulkan::SessionCreateInfo {
            instance: instance.handle().as_raw() as _,
            physical_device: self.physical_device.as_raw() as _,
            device: device.handle().as_raw() as _,
            queue_family_index: self.queue_family_index,
            queue_index: 0,
        }
    }

    /// Wrap OpenXR swapchain images as externally owned wgpu textures.
    ///
    /// # Safety
    /// Each `raw` must be a live `VkImage` from `enumerate_images` on a swapchain created
    /// on this device with `width`×`height`×`VIEW_COUNT` and `wgpu_format`. Textures must
    /// be dropped before the swapchain (and this bridge).
    pub unsafe fn wrap_swapchain_images(
        &self,
        raw_images: &[u64],
        width: u32,
        height: u32,
        wgpu_format: wgpu::TextureFormat,
    ) -> Result<Vec<wgpu::Texture>> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: VIEW_COUNT,
        };
        let mut out = Vec::with_capacity(raw_images.len());
        for &raw in raw_images {
            let vk_image = vk::Image::from_raw(raw);
            let hal_desc = wgpu::hal::TextureDescriptor {
                label: Some("xr-swapchain"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_format,
                usage: wgpu::hal::TextureUses::COLOR_TARGET,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: vec![],
            };
            // Safety: image matches desc; Some(callback) → destroy_texture skips vkDestroyImage.
            let hal_tex = wgpu::hal::vulkan::Device::texture_from_raw(
                vk_image,
                &hal_desc,
                Some(Box::new(|| {})),
            );
            // Safety: hal_tex wraps an image on this Vulkan device.
            let tex = self
                .device()
                .create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                    hal_tex,
                    &wgpu::TextureDescriptor {
                        label: Some("xr-swapchain"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu_format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                );
            out.push(tex);
        }
        Ok(out)
    }

    pub fn make_eye_views(
        textures: &[wgpu::Texture],
        wgpu_format: wgpu::TextureFormat,
    ) -> Vec<[wgpu::TextureView; VIEW_COUNT as usize]> {
        textures
            .iter()
            .map(|tex| {
                std::array::from_fn(|eye| {
                    tex.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("xr-eye"),
                        format: Some(wgpu_format),
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT),
                        aspect: wgpu::TextureAspect::All,
                        base_mip_level: 0,
                        mip_level_count: Some(1),
                        base_array_layer: eye as u32,
                        array_layer_count: Some(1),
                    })
                })
            })
            .collect()
    }
}

pub fn pick_swapchain_format(formats: &[u32]) -> Result<(u32, wgpu::TextureFormat)> {
    for &raw in formats {
        if let Some((vk_fmt, wgpu_fmt)) = map_swapchain_format(raw) {
            return Ok((vk_fmt.as_raw() as u32, wgpu_fmt));
        }
    }
    bail!("no wgpu-mappable swapchain format in {formats:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xr_wgpu_flag_only_one() {
        assert!(!xr_wgpu_flag_value(None));
        assert!(!xr_wgpu_flag_value(Some("0")));
        assert!(!xr_wgpu_flag_value(Some("true")));
        assert!(xr_wgpu_flag_value(Some("1")));
    }

    #[test]
    fn maps_common_swapchain_formats() {
        let (vk_fmt, wgpu_fmt) =
            map_swapchain_format(vk::Format::R8G8B8A8_SRGB.as_raw() as u32).expect("srgb");
        assert_eq!(vk_fmt, vk::Format::R8G8B8A8_SRGB);
        assert_eq!(wgpu_fmt, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert!(map_swapchain_format(0).is_none());
    }

    #[test]
    fn pick_prefers_first_mappable() {
        let formats = [
            vk::Format::R8G8B8A8_SRGB.as_raw() as u32,
            vk::Format::B8G8R8A8_SRGB.as_raw() as u32,
        ];
        let (raw, wgpu_fmt) = pick_swapchain_format(&formats).expect("pick");
        assert_eq!(raw, vk::Format::R8G8B8A8_SRGB.as_raw() as u32);
        assert_eq!(wgpu_fmt, wgpu::TextureFormat::Rgba8UnormSrgb);
    }
}
