//! XR_EXT_render_model + XR_EXT_interaction_render_model (x000b / x000c / x000d).
//!
//! Guarantees: optional enable; enumerate after sync_actions; GLB bytes + Space per model;
//! textured rig (baseColor when present); per-frame animatable node pose/visibility.
//! Non-guarantees: PBR maps, SteamVR disk paths, every runtime, skins/morphs.
//!
//! Refresh is throttled and incremental so SteamVR change events cannot rebuild
//! controllers / GPU meshes every frame (that froze the app).

use std::collections::HashSet;
use std::ptr;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use glam::{Mat4, Quat, Vec3};
use openxr as xr;
use openxr::sys;
use openxr::sys::Handle as _;

use crate::mesh::{load_textured_rigged_from_glb, CpuTexturedMesh, RigNode};

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
    pub mesh: CpuTexturedMesh,
    /// Reused buffer for `xrGetRenderModelStateEXT` (length = animatable count).
    node_states: Vec<sys::RenderModelNodeStateEXT>,
    /// Scratch worlds/visibility filled by [`ControllerModel::compose_node_worlds`].
    worlds: Vec<Mat4>,
    visible: Vec<bool>,
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

    /// Poll animatable node poses for `display_time`. No-op when count is 0.
    pub fn poll_state(&mut self, instance: &xr::Instance, display_time: xr::Time) -> Result<()> {
        if self.node_states.is_empty() {
            return Ok(());
        }
        let Some(rm) = instance.exts().ext_render_model.as_ref() else {
            return Ok(());
        };
        let get_info = sys::RenderModelStateGetInfoEXT {
            ty: sys::RenderModelStateGetInfoEXT::TYPE,
            next: ptr::null(),
            display_time,
        };
        let mut state = sys::RenderModelStateEXT {
            ty: sys::RenderModelStateEXT::TYPE,
            next: ptr::null_mut(),
            node_state_count: self.node_states.len() as u32,
            node_states: self.node_states.as_mut_ptr(),
        };
        // Safety: handle valid; buffer sized to animatableNodeCount.
        let r = unsafe { (rm.get_render_model_state)(self.handle, &get_info, &mut state) };
        if r.into_raw() < 0 {
            bail!("xrGetRenderModelStateEXT: {r:?}");
        }
        Ok(())
    }

    /// Compose node worlds and visibility from asset locals + last polled states.
    /// Returns slices into internal scratch (valid until next compose/poll that grows).
    pub fn compose_node_worlds(&mut self) -> (&[Mat4], &[bool]) {
        let n = self.mesh.nodes.len();
        self.worlds.resize(n, Mat4::IDENTITY);
        self.visible.resize(n, true);

        for (i, node) in self.mesh.nodes.iter().enumerate() {
            let (local, self_vis) = local_and_visibility(node, &self.node_states);
            let parent_vis = node.parent.map(|p| self.visible[p]).unwrap_or(true);
            self.visible[i] = parent_vis && self_vis;
            let parent_world = node
                .parent
                .map(|p| self.worlds[p])
                .unwrap_or(Mat4::IDENTITY);
            self.worlds[i] = parent_world * local;
        }

        (&self.worlds, &self.visible)
    }
}

fn local_and_visibility(node: &RigNode, states: &[sys::RenderModelNodeStateEXT]) -> (Mat4, bool) {
    match node.anim_index {
        Some(ai) => {
            let Some(st) = states.get(ai as usize) else {
                return (node.asset_local, true);
            };
            let pose = st.node_pose;
            let (pos, ori) = (
                Vec3::new(pose.position.x, pose.position.y, pose.position.z),
                Quat::from_xyzw(
                    pose.orientation.x,
                    pose.orientation.y,
                    pose.orientation.z,
                    pose.orientation.w,
                ),
            );
            (
                Mat4::from_rotation_translation(ori, pos),
                st.is_visible.into(),
            )
        }
        None => (node.asset_local, true),
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

    pub fn models_mut(&mut self) -> &mut [ControllerModel] {
        &mut self.models
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
                    let textured = model
                        .mesh
                        .submeshes
                        .iter()
                        .filter(|s| s.base_color.is_some())
                        .count();
                    log::info!(
                        "XR render model loaded ({} verts, {} submeshes, {} textured, {} anim nodes, {} rig nodes)",
                        model.mesh.vertex_count(),
                        model.mesh.submeshes.len(),
                        textured,
                        model.node_states.len(),
                        model.mesh.nodes.len()
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

    let anim_count = props.animatable_node_count as usize;
    let (glb, anim_names) =
        match unsafe { fetch_glb_and_names(session, rm, props.cache_id, anim_count) } {
            Ok(v) => v,
            Err(err) => {
                unsafe {
                    let _ = (rm.destroy_render_model)(handle);
                }
                return Err(err);
            }
        };

    let name_refs: Vec<&str> = anim_names.iter().map(String::as_str).collect();
    let mesh = match load_textured_rigged_from_glb(&glb, &name_refs) {
        Ok(m) => m,
        Err(err) => {
            unsafe {
                let _ = (rm.destroy_render_model)(handle);
            }
            return Err(err).context("rig controller GLB");
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

    let node_states = vec![
        sys::RenderModelNodeStateEXT {
            node_pose: sys::Posef::IDENTITY,
            is_visible: sys::TRUE,
        };
        anim_count
    ];
    let node_len = mesh.nodes.len();

    Ok(Some(ControllerModel {
        id,
        handle,
        space,
        mesh,
        node_states,
        worlds: vec![Mat4::IDENTITY; node_len],
        visible: vec![true; node_len],
    }))
}

unsafe fn fetch_glb_and_names<G>(
    session: &xr::Session<G>,
    rm: &xr::raw::RenderModelEXT,
    cache_id: sys::UuidEXT,
    anim_count: usize,
) -> Result<(Vec<u8>, Vec<String>)> {
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
    if r.into_raw() < 0 {
        unsafe {
            let _ = (rm.destroy_render_model_asset)(asset);
        }
        bail!("xrGetRenderModelAssetDataEXT: {r:?}");
    }
    buf.truncate(data.buffer_count_output as usize);

    let names = if anim_count == 0 {
        Vec::new()
    } else {
        match unsafe { read_animatable_names(rm, asset, anim_count) } {
            Ok(n) => n,
            Err(err) => {
                unsafe {
                    let _ = (rm.destroy_render_model_asset)(asset);
                }
                return Err(err);
            }
        }
    };

    unsafe {
        let _ = (rm.destroy_render_model_asset)(asset);
    }
    Ok((buf, names))
}

unsafe fn read_animatable_names(
    rm: &xr::raw::RenderModelEXT,
    asset: sys::RenderModelAssetEXT,
    anim_count: usize,
) -> Result<Vec<String>> {
    let mut node_props = vec![
        sys::RenderModelAssetNodePropertiesEXT {
            unique_name: [0; 64]
        };
        anim_count
    ];
    let props_info = sys::RenderModelAssetPropertiesGetInfoEXT {
        ty: sys::RenderModelAssetPropertiesGetInfoEXT::TYPE,
        next: ptr::null(),
    };
    let mut properties = sys::RenderModelAssetPropertiesEXT {
        ty: sys::RenderModelAssetPropertiesEXT::TYPE,
        next: ptr::null_mut(),
        node_property_count: anim_count as u32,
        node_properties: node_props.as_mut_ptr(),
    };
    // Safety: asset valid; node_property_count == animatableNodeCount.
    let r = unsafe { (rm.get_render_model_asset_properties)(asset, &props_info, &mut properties) };
    if r.into_raw() < 0 {
        bail!("xrGetRenderModelAssetPropertiesEXT: {r:?}");
    }

    let mut names = Vec::with_capacity(anim_count);
    for p in &node_props {
        let bytes: Vec<u8> = p
            .unique_name
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        names.push(String::from_utf8(bytes).context("animatable node name is not UTF-8")?);
    }
    Ok(names)
}
