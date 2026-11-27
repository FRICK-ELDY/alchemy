//! SoA secondary-bone buffer + spatial anchor slot refs (x00e8).

use crate::ids::LocalSlotId;

/// Parent index sentinel: bone is pinned to a spatial [`AvatarAnchorKind`].
pub const BONE_PINNED: u32 = u32::MAX;

/// Max extra full-body tracked points beyond Root/Head/Hands (few only).
pub const MAX_EXTRA_FULLBODY_ANCHORS: usize = 8;

/// Discriminant band: `0..4` primary, `4..4+MAX_EXTRA` extras.
const EXTRA_DISC_BASE: u8 = 4;

/// Spatial anchors that **may** occupy Scene Store / Sim Arena slots.
///
/// Tip / sway bones stay in [`AvatarRigBuffer`] and must not be registered as
/// slots (§4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarAnchorKind {
    Root,
    Head,
    LeftHand,
    RightHand,
    /// Full-body extra; index into [`AvatarSpatialAnchors::extra`].
    ///
    /// Only `0..MAX_EXTRA_FULLBODY_ANCHORS` is valid ([`Self::try_discriminant`]).
    Extra(u8),
}

impl AvatarAnchorKind {
    /// Encode for SoA storage. `None` if `Extra(i)` is out of range (no wrap).
    #[inline]
    pub fn try_discriminant(self) -> Option<u8> {
        match self {
            Self::Root => Some(0),
            Self::Head => Some(1),
            Self::LeftHand => Some(2),
            Self::RightHand => Some(3),
            Self::Extra(i) => {
                if (i as usize) < MAX_EXTRA_FULLBODY_ANCHORS {
                    // i < 8 ⇒ 4+i fits in u8 with no wrap.
                    Some(EXTRA_DISC_BASE + i)
                } else {
                    None
                }
            }
        }
    }

    #[inline]
    pub fn from_discriminant(d: u8) -> Option<Self> {
        match d {
            0 => Some(Self::Root),
            1 => Some(Self::Head),
            2 => Some(Self::LeftHand),
            3 => Some(Self::RightHand),
            x if x >= EXTRA_DISC_BASE
                && (x as usize) < EXTRA_DISC_BASE as usize + MAX_EXTRA_FULLBODY_ANCHORS =>
            {
                Some(Self::Extra(x - EXTRA_DISC_BASE))
            }
            _ => None,
        }
    }
}

/// Slot refs for the few spatially synced avatar points (not secondary bones).
///
/// Full-body extras are private so callers cannot bypass [`MAX_EXTRA_FULLBODY_ANCHORS`].
#[derive(Clone, Debug, Default)]
pub struct AvatarSpatialAnchors {
    pub root: Option<LocalSlotId>,
    pub head: Option<LocalSlotId>,
    pub left_hand: Option<LocalSlotId>,
    pub right_hand: Option<LocalSlotId>,
    extra: [LocalSlotId; MAX_EXTRA_FULLBODY_ANCHORS],
    extra_len: u8,
}

impl AvatarSpatialAnchors {
    /// Set Root / Head / Hands. Returns `false` for [`AvatarAnchorKind::Extra`]
    /// (use [`Self::push_extra`]).
    pub fn set_primary(&mut self, kind: AvatarAnchorKind, slot: Option<LocalSlotId>) -> bool {
        match kind {
            AvatarAnchorKind::Root => self.root = slot,
            AvatarAnchorKind::Head => self.head = slot,
            AvatarAnchorKind::LeftHand => self.left_hand = slot,
            AvatarAnchorKind::RightHand => self.right_hand = slot,
            AvatarAnchorKind::Extra(_) => return false,
        }
        true
    }

    /// Push an extra full-body slot; returns `false` if at capacity.
    pub fn push_extra(&mut self, slot: LocalSlotId) -> bool {
        let len = self.extra_len as usize;
        if len >= MAX_EXTRA_FULLBODY_ANCHORS {
            return false;
        }
        self.extra[len] = slot;
        self.extra_len = (len + 1) as u8;
        true
    }

    /// Occupied full-body extras (dense prefix; all entries are live slots).
    pub fn extra(&self) -> &[LocalSlotId] {
        &self.extra[..self.extra_len as usize]
    }

    /// Count of configured spatial slot refs (primary + extras).
    pub fn spatial_slot_count(&self) -> usize {
        let mut n = 0;
        if self.root.is_some() {
            n += 1;
        }
        if self.head.is_some() {
            n += 1;
        }
        if self.left_hand.is_some() {
            n += 1;
        }
        if self.right_hand.is_some() {
            n += 1;
        }
        n + self.extra_len as usize
    }

    pub fn slot(&self, kind: AvatarAnchorKind) -> Option<LocalSlotId> {
        match kind {
            AvatarAnchorKind::Root => self.root,
            AvatarAnchorKind::Head => self.head,
            AvatarAnchorKind::LeftHand => self.left_hand,
            AvatarAnchorKind::RightHand => self.right_hand,
            AvatarAnchorKind::Extra(i) => {
                let i = i as usize;
                if i < self.extra_len as usize {
                    Some(self.extra[i])
                } else {
                    None
                }
            }
        }
    }
}

/// World positions for anchors feeding [`super::step`] (caller-resolved from Arena).
///
/// Prefer [`Self::set`] / [`Self::get`] for all kinds. Primary fields are `pub`
/// for ergonomic bulk fill; **full-body `extra` is private** — write only via
/// [`Self::set`] so range checks stay enforced.
#[derive(Clone, Copy, Debug)]
pub struct AvatarAnchorWorld {
    pub root: [f32; 3],
    pub head: [f32; 3],
    pub left_hand: [f32; 3],
    pub right_hand: [f32; 3],
    extra: [[f32; 3]; MAX_EXTRA_FULLBODY_ANCHORS],
}

impl Default for AvatarAnchorWorld {
    fn default() -> Self {
        Self {
            root: [0.0; 3],
            head: [0.0; 3],
            left_hand: [0.0; 3],
            right_hand: [0.0; 3],
            extra: [[0.0; 3]; MAX_EXTRA_FULLBODY_ANCHORS],
        }
    }
}

impl AvatarAnchorWorld {
    /// Store a finite world position. Returns `false` (unchanged) when `pos` is
    /// non-finite or `Extra(i)` is out of range — same gate as
    /// [`AvatarRigBuffer::push_pinned`].
    pub fn set(&mut self, kind: AvatarAnchorKind, pos: [f32; 3]) -> bool {
        if !pos.iter().all(|v| v.is_finite()) {
            return false;
        }
        match kind {
            AvatarAnchorKind::Root => self.root = pos,
            AvatarAnchorKind::Head => self.head = pos,
            AvatarAnchorKind::LeftHand => self.left_hand = pos,
            AvatarAnchorKind::RightHand => self.right_hand = pos,
            AvatarAnchorKind::Extra(i) => {
                if (i as usize) >= MAX_EXTRA_FULLBODY_ANCHORS {
                    return false;
                }
                self.extra[i as usize] = pos;
            }
        }
        true
    }

    #[inline]
    pub fn get(&self, kind: AvatarAnchorKind) -> Option<[f32; 3]> {
        match kind {
            AvatarAnchorKind::Root => Some(self.root),
            AvatarAnchorKind::Head => Some(self.head),
            AvatarAnchorKind::LeftHand => Some(self.left_hand),
            AvatarAnchorKind::RightHand => Some(self.right_hand),
            AvatarAnchorKind::Extra(i) => {
                let i = i as usize;
                if i < MAX_EXTRA_FULLBODY_ANCHORS {
                    Some(self.extra[i])
                } else {
                    None
                }
            }
        }
    }
}

/// Local continuous SoA for tip / sway bones — **not** Scene Store managed.
///
/// This type does not take [`crate::scene_store::SceneStore`] or
/// [`crate::sim_arena::SimArena`]; secondary bones cannot allocate slots via API.
#[derive(Clone, Debug, Default)]
pub struct AvatarRigBuffer {
    pub(crate) pos_x: Vec<f32>,
    pub(crate) pos_y: Vec<f32>,
    pub(crate) pos_z: Vec<f32>,
    pub(crate) prev_x: Vec<f32>,
    pub(crate) prev_y: Vec<f32>,
    pub(crate) prev_z: Vec<f32>,
    pub(crate) inv_mass: Vec<f32>,
    /// Parent bone index in this buffer, or [`BONE_PINNED`].
    pub(crate) parent: Vec<u32>,
    /// When [`BONE_PINNED`], valid [`AvatarAnchorKind::try_discriminant`] encoding.
    pub(crate) pin_anchor: Vec<u8>,
    pub(crate) rest_len: Vec<f32>,
    pub(crate) compliance: Vec<f32>,
}

impl AvatarRigBuffer {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            pos_x: Vec::with_capacity(cap),
            pos_y: Vec::with_capacity(cap),
            pos_z: Vec::with_capacity(cap),
            prev_x: Vec::with_capacity(cap),
            prev_y: Vec::with_capacity(cap),
            prev_z: Vec::with_capacity(cap),
            inv_mass: Vec::with_capacity(cap),
            parent: Vec::with_capacity(cap),
            pin_anchor: Vec::with_capacity(cap),
            rest_len: Vec::with_capacity(cap),
            compliance: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn bone_count(&self) -> usize {
        self.pos_x.len()
    }

    pub fn clear(&mut self) {
        self.pos_x.clear();
        self.pos_y.clear();
        self.pos_z.clear();
        self.prev_x.clear();
        self.prev_y.clear();
        self.prev_z.clear();
        self.inv_mass.clear();
        self.parent.clear();
        self.pin_anchor.clear();
        self.rest_len.clear();
        self.compliance.clear();
    }

    /// Pin a bone to a spatial anchor (`inv_mass = 0`).
    ///
    /// `None` if `anchor` is an out-of-range `Extra` or `pos` is non-finite.
    pub fn push_pinned(&mut self, anchor: AvatarAnchorKind, pos: [f32; 3]) -> Option<u32> {
        let disc = anchor.try_discriminant()?;
        // Round-trip must succeed so snap_pins never silently skips.
        let _ = AvatarAnchorKind::from_discriminant(disc)?;
        if !pos.iter().all(|v| v.is_finite()) {
            return None;
        }
        self.push_row(BONE_PINNED, disc, 0.0, 0.0, 0.0, pos)
    }

    /// Append a secondary (non-slot) bone under `parent` in this buffer.
    ///
    /// `None` if `parent` is out of range / is not a live index, or values are
    /// non-finite / negative.
    pub fn push_secondary(
        &mut self,
        parent: u32,
        rest_len: f32,
        compliance: f32,
        pos: [f32; 3],
    ) -> Option<u32> {
        if parent == BONE_PINNED || (parent as usize) >= self.bone_count() {
            return None;
        }
        if !pos.iter().all(|v| v.is_finite())
            || !rest_len.is_finite()
            || !compliance.is_finite()
            || rest_len < 0.0
            || compliance < 0.0
        {
            return None;
        }
        // pin_anchor unused for non-pinned; store 0.
        self.push_row(parent, 0, 1.0, rest_len, compliance, pos)
    }

    fn push_row(
        &mut self,
        parent: u32,
        pin_disc: u8,
        inv_mass: f32,
        rest_len: f32,
        compliance: f32,
        pos: [f32; 3],
    ) -> Option<u32> {
        self.pos_x.push(pos[0]);
        self.pos_y.push(pos[1]);
        self.pos_z.push(pos[2]);
        self.prev_x.push(pos[0]);
        self.prev_y.push(pos[1]);
        self.prev_z.push(pos[2]);
        self.inv_mass.push(inv_mass);
        self.parent.push(parent);
        self.pin_anchor.push(pin_disc);
        self.rest_len.push(rest_len);
        self.compliance.push(compliance);
        Some((self.bone_count() - 1) as u32)
    }

    #[inline]
    pub fn pos_x(&self, i: usize) -> f32 {
        self.pos_x[i]
    }
    #[inline]
    pub fn pos_y(&self, i: usize) -> f32 {
        self.pos_y[i]
    }
    #[inline]
    pub fn pos_z(&self, i: usize) -> f32 {
        self.pos_z[i]
    }

    /// Pin-anchor discriminant for tests / debug (pinned bones only).
    #[cfg(test)]
    pub(crate) fn pin_discriminant(&self, i: usize) -> u8 {
        self.pin_anchor[i]
    }
}
