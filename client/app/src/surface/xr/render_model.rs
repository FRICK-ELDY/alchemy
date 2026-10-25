//! XR_EXT_render_model + XR_EXT_interaction_render_model (x000b).
//!
//! Guarantees: optional enable; enumerate after sync_actions; GLB bytes + Space per model.
//! Non-guarantees: animatable nodes, textures, SteamVR disk paths, every runtime.
//!
//! Refresh is throttled and incremental so SteamVR change events cannot rebuild
//! controllers / GPU meshes every frame (that froze the app).

use std::collections::HashSet;
use std::ptr;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use openxr as xr;
use openxr::sys;
use openxr::sys::Handle as _;

use crate::mesh::{load_flattened_mesh_from_glb, CpuMesh};

/// Min gap between refresh attempts while assets/ids are still settling.
const RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// Whether the instance was created with both render-model extensions.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderModelExtSupport {
    pub enabled: bool,
}

/// Probe available extensions and set flags on `ExtensionSet` when both EXTs exist.
pub fn enable_if_available(
    available: &xr::ExtensionSet,
    enabled: &mut xr::ExtensionSet,
) -> RenderModelExtSupport {
    if available.ext_render_model && available.ext_interaction_render_model {
        // OpenXR 1.0 API version needs EXT_uuid for UuidEXT.
        if available.ext_uuid {
            enabled.ext_uuid = true;
        }
        enabled.ext_render_model = true;
        enabled.ext_interaction_render_model = true;
        log::info!("OpenXR: enabling XR_EXT_render_model + XR_EXT_interaction_render_model");
        RenderModelExtSupport { enabled: true }
    } else {
        log::warn!(
            "OpenXR: render model EXTs unavailable (render_model={}, interaction={}); controllers will not draw",
            available.ext_render_model,
            available.ext_interaction_render_model
        );
        RenderModelExtSupport { enabled: false }
    }
}

pub struct ControllerModel {
    pub id: sys::RenderModelIdEXT,
    handle: sys::RenderModelEXT,
    /// Destroyed with Space Drop.
    pub space: xr::Space,
    pub mesh: CpuMesh,
}

impl ControllerModel {
    fn destroy_handle(&self, instance: &xr::Instance) {
        if let Some(fp) = instance.exts().ext_render_model.as_ref() {
            // Safety: handle created on this instance; destroy once.
            unsafe {
                let _ = (fp.destroy_render_model)(self.handle);
            }
        }
    }
}

pub struct RenderModelCache {
    support: RenderModelExtSupport,
    /// Re-enumerate after FOCUSED / InteractionRenderModelsChanged / pending assets.
    dirty: bool,
    /// Bumped only when the set of GPU-visible meshes changes.
    pub generation: u64,
    models: Vec<ControllerModel>,
    last_attempt: Option<Instant>,
}

impl RenderModelCache {
    pub fn new(support: RenderModelExtSupport) -> Self {
        Self {
            support,
            dirty: support.enabled,
            generation: 0,
            models: Vec::new(),
            last_attempt: None,
        }
    }

    pub fn models(&self) -> &[ControllerModel] {
        &self.models
    }

    pub fn mark_dirty(&mut self) {
        if self.support.enabled {
            self.dirty = true;
        }
    }

    /// Incremental, throttled refresh. Does not tear down working models every frame.
    pub fn refresh_if_dirty<G>(&mut self, session: &xr::Session<G>) -> Result<()> {
        if !self.support.enabled || !self.dirty {
            return Ok(());
        }
        if let Some(t) = self.last_attempt {
            if t.elapsed() < RETRY_INTERVAL {
                return Ok(());
            }
        }
        self.last_attempt = Some(Instant::now());

        let instance = session.instance();
        let Some(rm) = instance.exts().ext_render_model.as_ref() else {
            self.dirty = false;
            return Ok(());
        };
        let Some(irm) = instance.exts().ext_interaction_render_model.as_ref() else {
            self.dirty = false;
            return Ok(());
        };

        let ids = unsafe { enumerate_ids(session, irm)? };
        if ids.is_empty() {
            log::debug!("XR render models: enumerate returned 0 ids (will retry)");
            // Stay dirty; throttle prevents a spin.
            return Ok(());
        }

        let want: HashSet<u64> = ids.iter().map(|id| id.into_raw()).collect();
        let have: HashSet<u64> = self.models.iter().map(|m| m.id.into_raw()).collect();

        if have == want {
            self.dirty = false;
            return Ok(());
        }

        // Drop models the runtime no longer lists.
        let mut kept = Vec::with_capacity(self.models.len());
        let mut removed = false;
        for m in self.models.drain(..) {
            if want.contains(&m.id.into_raw()) {
                kept.push(m);
            } else {
                m.destroy_handle(instance);
                removed = true;
            }
        }
        self.models = kept;

        let have_now: HashSet<u64> = self.models.iter().map(|m| m.id.into_raw()).collect();
        let mut added = false;
        let mut pending = false;
        for id in ids {
            if have_now.contains(&id.into_raw()) {
                continue;
            }
            match unsafe { load_one(session, rm, id) } {
                Ok(Some(model)) => {
                    log::info!(
                        "XR render model loaded ({} verts)",
                        model.mesh.positions.len()
                    );
                    self.models.push(model);
                    added = true;
                }
                Ok(None) => {
                    pending = true;
                }
                Err(err) => {
                    log::warn!("XR render model load failed: {err:#}");
                }
            }
        }

        if added || removed {
            self.generation = self.generation.wrapping_add(1);
        }

        if pending {
            // Keep retrying for unavailable assets; do not rebuild what we already have.
            self.dirty = true;
        } else {
            self.dirty = false;
            log::info!("XR render models: {} ready", self.models.len());
        }
        Ok(())
    }

    pub fn clear(&mut self, instance: &xr::Instance) {
        for m in self.models.drain(..) {
            m.destroy_handle(instance);
            drop(m.space);
        }
        self.generation = self.generation.wrapping_add(1);
    }
}

impl Drop for RenderModelCache {
    fn drop(&mut self) {
        // Callers should `clear` before destroying the session when possible.
        self.models.clear();
    }
}

unsafe fn enumerate_ids<G>(
    session: &xr::Session<G>,
    irm: &xr::raw::InteractionRenderModelEXT,
) -> Result<Vec<sys::RenderModelIdEXT>> {
    let info = sys::InteractionRenderModelIdsEnumerateInfoEXT {
        ty: sys::InteractionRenderModelIdsEnumerateInfoEXT::TYPE,
        next: ptr::null(),
    };
    let mut count = 0u32;
    // Safety: session + extension enabled; two-call idiom.
    let r = unsafe {
        (irm.enumerate_interaction_render_model_ids)(
            session.as_raw(),
            &info,
            0,
            &mut count,
            ptr::null_mut(),
        )
    };
    if r.into_raw() < 0 {
        bail!("enumerate_interaction_render_model_ids(count): {r:?}");
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![sys::RenderModelIdEXT::from_raw(0); count as usize];
    let r = unsafe {
        (irm.enumerate_interaction_render_model_ids)(
            session.as_raw(),
            &info,
            count,
            &mut count,
            ids.as_mut_ptr(),
        )
    };
    if r.into_raw() < 0 {
        bail!("enumerate_interaction_render_model_ids: {r:?}");
    }
    ids.truncate(count as usize);
    Ok(ids)
}

unsafe fn load_one<G>(
    session: &xr::Session<G>,
    rm: &xr::raw::RenderModelEXT,
    id: sys::RenderModelIdEXT,
) -> Result<Option<ControllerModel>> {
    let create_info = sys::RenderModelCreateInfoEXT {
        ty: sys::RenderModelCreateInfoEXT::TYPE,
        next: ptr::null(),
        render_model_id: id,
        gltf_extension_count: 0,
        gltf_extensions: ptr::null(),
    };
    let mut handle = sys::RenderModelEXT::NULL;
    let r = unsafe { (rm.create_render_model)(session.as_raw(), &create_info, &mut handle) };
    if r.into_raw() < 0 {
        bail!("xrCreateRenderModelEXT: {r:?}");
    }

    let props_info = sys::RenderModelPropertiesGetInfoEXT {
        ty: sys::RenderModelPropertiesGetInfoEXT::TYPE,
        next: ptr::null(),
    };
    let mut props = sys::RenderModelPropertiesEXT {
        ty: sys::RenderModelPropertiesEXT::TYPE,
        next: ptr::null_mut(),
        cache_id: sys::UuidEXT { data: [0; 16] },
        animatable_node_count: 0,
    };
    let r = unsafe { (rm.get_render_model_properties)(handle, &props_info, &mut props) };
    if r == sys::Result::ERROR_RENDER_MODEL_ASSET_UNAVAILABLE_EXT {
        unsafe {
            let _ = (rm.destroy_render_model)(handle);
        }
        return Ok(None);
    }
    if r.into_raw() < 0 {
        unsafe {
            let _ = (rm.destroy_render_model)(handle);
        }
        bail!("xrGetRenderModelPropertiesEXT: {r:?}");
    }

    let glb = match unsafe { fetch_glb(session, rm, props.cache_id) } {
        Ok(b) => b,
        Err(err) => {
            unsafe {
                let _ = (rm.destroy_render_model)(handle);
            }
            return Err(err);
        }
    };

    let mesh = match load_flattened_mesh_from_glb(&glb) {
        Ok(m) => m,
        Err(err) => {
            unsafe {
                let _ = (rm.destroy_render_model)(handle);
            }
            return Err(err).context("flatten controller GLB");
        }
    };

    let space_info = sys::RenderModelSpaceCreateInfoEXT {
        ty: sys::RenderModelSpaceCreateInfoEXT::TYPE,
        next: ptr::null(),
        render_model: handle,
    };
    let mut space_handle = sys::Space::NULL;
    let r =
        unsafe { (rm.create_render_model_space)(session.as_raw(), &space_info, &mut space_handle) };
    if r.into_raw() < 0 {
        unsafe {
            let _ = (rm.destroy_render_model)(handle);
        }
        bail!("xrCreateRenderModelSpaceEXT: {r:?}");
    }

    // Safety: space created for this session.
    let space = unsafe { xr::Space::reference_from_raw(session.clone(), space_handle) };

    Ok(Some(ControllerModel {
        id,
        handle,
        space,
        mesh,
    }))
}

unsafe fn fetch_glb<G>(
    session: &xr::Session<G>,
    rm: &xr::raw::RenderModelEXT,
    cache_id: sys::UuidEXT,
) -> Result<Vec<u8>> {
    let create_info = sys::RenderModelAssetCreateInfoEXT {
        ty: sys::RenderModelAssetCreateInfoEXT::TYPE,
        next: ptr::null(),
        cache_id,
    };
    let mut asset = sys::RenderModelAssetEXT::NULL;
    let r = unsafe { (rm.create_render_model_asset)(session.as_raw(), &create_info, &mut asset) };
    if r == sys::Result::ERROR_RENDER_MODEL_ASSET_UNAVAILABLE_EXT {
        bail!("render model asset unavailable");
    }
    if r.into_raw() < 0 {
        bail!("xrCreateRenderModelAssetEXT: {r:?}");
    }

    let get_info = sys::RenderModelAssetDataGetInfoEXT {
        ty: sys::RenderModelAssetDataGetInfoEXT::TYPE,
        next: ptr::null(),
    };
    let mut data = sys::RenderModelAssetDataEXT {
        ty: sys::RenderModelAssetDataEXT::TYPE,
        next: ptr::null_mut(),
        buffer_capacity_input: 0,
        buffer_count_output: 0,
        buffer: ptr::null_mut(),
    };
    let r = unsafe { (rm.get_render_model_asset_data)(asset, &get_info, &mut data) };
    if r.into_raw() < 0 {
        unsafe {
            let _ = (rm.destroy_render_model_asset)(asset);
        }
        bail!("xrGetRenderModelAssetDataEXT(size): {r:?}");
    }
    let size = data.buffer_count_output as usize;
    if size == 0 {
        unsafe {
            let _ = (rm.destroy_render_model_asset)(asset);
        }
        bail!("render model GLB size is 0");
    }
    let mut buf = vec![0u8; size];
    data.buffer_capacity_input = size as u32;
    data.buffer = buf.as_mut_ptr();
    let r = unsafe { (rm.get_render_model_asset_data)(asset, &get_info, &mut data) };
    unsafe {
        let _ = (rm.destroy_render_model_asset)(asset);
    }
    if r.into_raw() < 0 {
        bail!("xrGetRenderModelAssetDataEXT: {r:?}");
    }
    buf.truncate(data.buffer_count_output as usize);
    Ok(buf)
}
