//! Three SoA faces and end-of-tick role rotation (x00da).

use super::{LocalSlotId, STATE_TELEPORT_THIS_TICK};

/// Which logical face a buffer currently plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceRole {
    Prev,
    Published,
    Work,
}

/// One SoA face: cell-relative transforms + per-slot state bits.
///
/// Guarantees: all column lengths stay equal ([`Self::slot_count`]) via private
/// `Vec`s and [`Self::resize`]. Non-guarantees: Carry-Over, dirty_epoch, parent
/// hierarchy.
///
/// Rotation columns (`rot_*`) are **allocated only** in x00da; getters/setters
/// arrive with transform API expansion (x00db+). Position + teleport bits are
/// the public mutate surface for scaffold tests.
///
/// # Panics
///
/// Slot accessors (`set_pos`, `pos`, `set_teleport`, `is_teleport`) panic if
/// `slot as usize >= slot_count()` (debug builds also `debug_assert`). Callers
/// must use in-range [`LocalSlotId`]s; fallible getters can be added when this
/// becomes a wire-facing API.
#[derive(Clone, Debug, Default)]
pub struct SimFace {
    pos_x: Vec<f32>,
    pos_y: Vec<f32>,
    pos_z: Vec<f32>,
    /// Reserved SoA columns (API later; lengths kept in sync by [`Self::resize`]).
    rot_x: Vec<f32>,
    rot_y: Vec<f32>,
    rot_z: Vec<f32>,
    rot_w: Vec<f32>,
    state_bits: Vec<u64>,
}

impl SimFace {
    pub fn with_slots(slot_count: usize) -> Self {
        let mut face = Self::default();
        face.resize(slot_count);
        face
    }

    /// Resize every SoA column to the same logical slot count.
    pub fn resize(&mut self, slot_count: usize) {
        self.pos_x.resize(slot_count, 0.0);
        self.pos_y.resize(slot_count, 0.0);
        self.pos_z.resize(slot_count, 0.0);
        self.rot_x.resize(slot_count, 0.0);
        self.rot_y.resize(slot_count, 0.0);
        self.rot_z.resize(slot_count, 0.0);
        self.rot_w.resize(slot_count, 1.0);
        self.state_bits.resize(slot_count, 0);
    }

    /// Number of logical slots (equal column lengths). Not `Vec::capacity`.
    #[inline]
    pub fn slot_count(&self) -> usize {
        self.pos_x.len()
    }

    #[inline]
    pub fn pos_x(&self) -> &[f32] {
        &self.pos_x
    }

    #[inline]
    pub fn pos_y(&self) -> &[f32] {
        &self.pos_y
    }

    #[inline]
    pub fn pos_z(&self) -> &[f32] {
        &self.pos_z
    }

    #[inline]
    pub fn state_bits(&self) -> &[u64] {
        &self.state_bits
    }

    #[inline]
    fn index(&self, slot: LocalSlotId) -> usize {
        let i = slot as usize;
        debug_assert!(
            i < self.slot_count(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );
        i
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn set_pos(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32) {
        let i = self.index(slot);
        self.pos_x[i] = x;
        self.pos_y[i] = y;
        self.pos_z[i] = z;
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn pos(&self, slot: LocalSlotId) -> [f32; 3] {
        let i = self.index(slot);
        [self.pos_x[i], self.pos_y[i], self.pos_z[i]]
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    ///
    /// Does not auto-clear on rotate — see [`crate::sim_arena`] module docs.
    #[inline]
    pub fn set_teleport(&mut self, slot: LocalSlotId, on: bool) {
        let i = self.index(slot);
        if on {
            self.state_bits[i] |= STATE_TELEPORT_THIS_TICK;
        } else {
            self.state_bits[i] &= !STATE_TELEPORT_THIS_TICK;
        }
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn is_teleport(&self, slot: LocalSlotId) -> bool {
        let i = self.index(slot);
        (self.state_bits[i] & STATE_TELEPORT_THIS_TICK) != 0
    }

    #[cfg(test)]
    fn columns_aligned(&self) -> bool {
        let n = self.pos_x.len();
        self.pos_y.len() == n
            && self.pos_z.len() == n
            && self.rot_x.len() == n
            && self.rot_y.len() == n
            && self.rot_z.len() == n
            && self.rot_w.len() == n
            && self.state_bits.len() == n
    }
}

/// Triple-buffered Sim Arena (Prev / Published / Work).
///
/// Tick contract (x00da scaffold only): writers mutate [`Self::work_mut`]; at tick
/// end call [`Self::rotate_faces`] so Work→Published→Prev and the old Prev buffer
/// becomes the next Work. Carry-Over into recycled Work is x00db.
///
/// [`Self::rotate_faces`] does **not** clear [`STATE_TELEPORT_THIS_TICK`] — clear
/// ownership is documented on [`crate::sim_arena`].
#[derive(Debug)]
pub struct SimArena {
    faces: [SimFace; 3],
    /// Index of the face currently playing Prev / Published / Work.
    prev_i: u8,
    published_i: u8,
    work_i: u8,
    /// Monotonic epoch advanced only via [`Self::rotate_faces`].
    epoch: u64,
}

impl SimArena {
    pub fn with_slots(slot_count: usize) -> Self {
        Self {
            faces: [
                SimFace::with_slots(slot_count),
                SimFace::with_slots(slot_count),
                SimFace::with_slots(slot_count),
            ],
            prev_i: 0,
            published_i: 1,
            work_i: 2,
            epoch: 0,
        }
    }

    /// Logical slots per face (equal across columns). Not `Vec::capacity`.
    pub fn slot_count(&self) -> usize {
        self.faces[0].slot_count()
    }

    pub fn resize(&mut self, slot_count: usize) {
        for face in &mut self.faces {
            face.resize(slot_count);
        }
    }

    /// Monotonic face-generation counter (advanced only by [`Self::rotate_faces`]).
    #[inline]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    #[inline]
    pub fn face_index(&self, role: FaceRole) -> usize {
        match role {
            FaceRole::Prev => self.prev_i as usize,
            FaceRole::Published => self.published_i as usize,
            FaceRole::Work => self.work_i as usize,
        }
    }

    #[inline]
    pub fn prev(&self) -> &SimFace {
        &self.faces[self.prev_i as usize]
    }

    #[inline]
    pub fn published(&self) -> &SimFace {
        &self.faces[self.published_i as usize]
    }

    #[inline]
    pub fn work(&self) -> &SimFace {
        &self.faces[self.work_i as usize]
    }

    #[inline]
    pub fn work_mut(&mut self) -> &mut SimFace {
        &mut self.faces[self.work_i as usize]
    }

    /// End-of-tick role rotation: Work→Published, Published→Prev, Prev→Work.
    ///
    /// Does **not** copy rows (Carry-Over is x00db). Recycled Work may hold stale
    /// data until Carry-Over or full overwrite.
    ///
    /// Does **not** clear [`STATE_TELEPORT_THIS_TICK`] — recycle leaves the bit
    /// in place; x00db Step 0 clears it on Work after pose/rot Carry-Over.
    pub fn rotate_faces(&mut self) {
        let old_prev = self.prev_i;
        self.prev_i = self.published_i;
        self.published_i = self.work_i;
        self.work_i = old_prev;
        self.epoch = self.epoch.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_arena_has_distinct_face_roles() {
        let arena = SimArena::with_slots(4);
        assert_ne!(
            arena.face_index(FaceRole::Prev),
            arena.face_index(FaceRole::Published)
        );
        assert_ne!(
            arena.face_index(FaceRole::Published),
            arena.face_index(FaceRole::Work)
        );
        assert_ne!(
            arena.face_index(FaceRole::Prev),
            arena.face_index(FaceRole::Work)
        );
        assert_eq!(arena.epoch(), 0);
        assert_eq!(arena.slot_count(), 4);
    }

    #[test]
    fn rotate_advances_roles_and_epoch() {
        let mut arena = SimArena::with_slots(2);
        let i_prev = arena.face_index(FaceRole::Prev);
        let i_pub = arena.face_index(FaceRole::Published);
        let i_work = arena.face_index(FaceRole::Work);

        arena.rotate_faces();

        assert_eq!(arena.face_index(FaceRole::Prev), i_pub);
        assert_eq!(arena.face_index(FaceRole::Published), i_work);
        assert_eq!(arena.face_index(FaceRole::Work), i_prev);
        assert_eq!(arena.epoch(), 1);
    }

    #[test]
    fn work_write_becomes_published_after_rotate() {
        let mut arena = SimArena::with_slots(2);
        arena.work_mut().set_pos(0, 1.0, 2.0, 3.0);
        arena.work_mut().set_teleport(0, true);

        assert_eq!(arena.published().pos(0), [0.0, 0.0, 0.0]);
        assert!(!arena.published().is_teleport(0));

        arena.rotate_faces();

        assert_eq!(arena.published().pos(0), [1.0, 2.0, 3.0]);
        assert!(arena.published().is_teleport(0));
        assert_eq!(arena.prev().pos(0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn double_rotate_slides_published_to_prev() {
        let mut arena = SimArena::with_slots(1);
        arena.work_mut().set_pos(0, 10.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.work_mut().set_pos(0, 20.0, 0.0, 0.0);
        arena.rotate_faces();

        assert_eq!(arena.prev().pos(0), [10.0, 0.0, 0.0]);
        assert_eq!(arena.published().pos(0), [20.0, 0.0, 0.0]);
    }

    #[test]
    fn teleport_bit_matches_roadmap_constant() {
        assert_eq!(STATE_TELEPORT_THIS_TICK, 1 << 11);
    }

    #[test]
    fn resize_keeps_all_columns_aligned() {
        let mut arena = SimArena::with_slots(2);
        arena.resize(8);
        assert_eq!(arena.slot_count(), 8);
        assert!(arena.prev().columns_aligned());
        assert!(arena.published().columns_aligned());
        assert!(arena.work().columns_aligned());
        arena.resize(3);
        assert_eq!(arena.slot_count(), 3);
        assert!(arena.work().columns_aligned());
    }

    #[test]
    fn rotate_recycles_stale_work_without_carry_over() {
        // 3-face ring: after three rotates the physical buffer that held `1.0`
        // returns as Work still carrying that value (no Carry-Over in x00da).
        //   tick1 write 1 → rotate; tick2 write 2 → rotate; tick3 no write → rotate
        //   ⇒ Work == buffer that published `1.0` two epochs earlier.
        let mut arena = SimArena::with_slots(1);
        arena.work_mut().set_pos(0, 1.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.work_mut().set_pos(0, 2.0, 0.0, 0.0);
        arena.rotate_faces();
        // Work is a never-written-this-tick buffer; leave it untouched.
        arena.rotate_faces();

        assert_eq!(arena.prev().pos(0), [2.0, 0.0, 0.0]);
        assert_eq!(
            arena.work().pos(0),
            [1.0, 0.0, 0.0],
            "recycled Work keeps stale value from two publishes ago without Carry-Over"
        );
    }

    #[test]
    fn rotate_does_not_clear_teleport_bit() {
        let mut arena = SimArena::with_slots(1);
        arena.work_mut().set_teleport(0, true);
        arena.rotate_faces();
        assert!(
            arena.published().is_teleport(0),
            "teleport must survive into Published; clear is tick-boundary (x00db/x00dc)"
        );
    }
}
