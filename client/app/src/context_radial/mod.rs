//! Context radial menu (x00ce).
//!
//! Guarantees: Arc button click on primary **release** (falling edge); Desktop MMB/T toggle;
//! leave-pad exit (Desktop ring / XR board); pointer glyph stamped outside the layout tree;
//! XR world-anchored board (pose frozen at open/re-anchor); Save enabled when grab+blake3 ready.
//! Non-guarantees: flick-through; submenu hierarchy; shared multi-user visibility;
//! simple_controller Context; hit AABB matching the drawn Save rect (polar Arc band).

use crate::system_ui::{Color, DrawCmd, LayoutTree, Rect};

/// Meters along aim forward from laser origin (XR).
pub const CONTEXT_FORWARD_M: f32 = 0.30;
/// Board height in meters; width follows canvas aspect so the ring stays circular on-board.
pub const CONTEXT_BOARD_H: f32 = 0.42;

/// World board size matching `canvas_w`×`canvas_h` aspect (avoids oval hit/draw on XR).
pub fn context_board_size(canvas_w: f32, canvas_h: f32) -> (f32, f32) {
    let h = CONTEXT_BOARD_H;
    let w = h * (canvas_w / canvas_h.max(1.0));
    (w, h)
}

/// Logical ring diameter in canvas pixels (centered in whatever canvas we rasterize).
pub const RING_DIAMETER: f32 = 420.0;

const INNER_RATIO: f32 = 0.28;
const OUTER_RATIO: f32 = 0.92;
/// Soft leave radius vs outer ring (Desktop) / board half-extents (XR input pad).
pub const MENU_LEAVE_PAD: f32 = 1.25;
/// Consecutive outside frames before leave-close (Desktop + XR).
pub const LEAVE_MISS_FRAMES: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextItemId {
    Save,
}

#[derive(Clone, Debug)]
pub struct ContextItem {
    pub id: ContextItemId,
    pub label: &'static str,
    pub enabled: bool,
    pub close_on_press: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextEvent {
    None,
    Fired(ContextItemId),
}

#[derive(Clone, Debug)]
pub struct ContextRadialState {
    pub open: bool,
    pub owner: Option<HandSide>,
    /// Pointer in canvas pixels (origin top-left).
    pub pointer: Option<(f32, f32)>,
    pub primary_down: bool,
    /// Previous frame `primary_down` (falling-edge / release click on Arc).
    was_primary_down: bool,
    pub items: Vec<ContextItem>,
    pub hover: Option<usize>,
    pub status: String,
    pub dirty: bool,
    /// Raster / hit canvas size (Desktop window or XR UI canvas).
    pub canvas_w: f32,
    pub canvas_h: f32,
}

impl Default for ContextRadialState {
    fn default() -> Self {
        Self {
            open: false,
            owner: None,
            pointer: None,
            primary_down: false,
            was_primary_down: false,
            items: Vec::new(),
            hover: None,
            status: String::new(),
            dirty: false,
            canvas_w: 1280.0,
            canvas_h: 720.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Inner,
    Arc(usize),
    Outside,
}

impl ContextRadialState {
    pub fn close(&mut self) {
        self.open = false;
        self.owner = None;
        self.pointer = None;
        self.primary_down = false;
        self.was_primary_down = false;
        self.hover = None;
        self.status.clear();
        self.dirty = true;
    }

    /// Toggle from Desktop (no hand) or open/close for same hand.
    pub fn toggle_desktop(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            self.owner = None;
            self.status.clear();
            self.dirty = true;
        }
    }

    /// XR: Context edge on `hand`. Opposite hand while open → re-anchor (stay open).
    pub fn toggle_or_reanchor(&mut self, hand: HandSide) -> bool {
        if !self.open {
            self.open = true;
            self.owner = Some(hand);
            self.status.clear();
            self.dirty = true;
            return true;
        }
        if self.owner == Some(hand) {
            self.close();
            return false;
        }
        self.owner = Some(hand);
        self.pointer = None;
        self.status.clear();
        self.dirty = true;
        true
    }

    pub fn set_items(&mut self, items: Vec<ContextItem>) {
        if self.items.len() != items.len()
            || self
                .items
                .iter()
                .zip(items.iter())
                .any(|(a, b)| a.id != b.id || a.enabled != b.enabled || a.label != b.label)
        {
            self.items = items;
            self.dirty = true;
        }
    }

    pub fn set_pointer(&mut self, px: f32, py: f32) {
        // Pointer glyph is stamped after raster (not in the layout tree) so aim
        // motion does not force a full ring rebuild every frame.
        self.pointer = Some((px, py));
    }

    /// Clear laser/cursor hit (tests / partial miss). Leave-to-close uses [`Self::close`].
    #[allow(dead_code)]
    pub fn clear_pointer(&mut self) {
        if self.pointer.take().is_some() {
            self.dirty = true;
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if self.status != msg {
            self.status = msg;
            self.dirty = true;
        }
    }

    /// Arc button click on primary **release** (mouseup / trigger up).
    ///
    /// Release-to-click so Grab held with LMB/trigger can aim Save then let go (x00ce).
    pub fn tick_interaction(&mut self) -> ContextEvent {
        if !self.open {
            self.was_primary_down = self.primary_down;
            return ContextEvent::None;
        }
        let falling = !self.primary_down && self.was_primary_down;
        self.was_primary_down = self.primary_down;

        let Some((px, py)) = self.pointer else {
            return ContextEvent::None;
        };
        let zone = hit_zone(px, py, self.canvas_w, self.canvas_h, self.items.len());
        let new_hover = match zone {
            Zone::Arc(i) if self.items.get(i).is_some_and(|it| it.enabled) => Some(i),
            _ => None,
        };
        if new_hover != self.hover {
            self.hover = new_hover;
            self.dirty = true;
        }

        if falling {
            if let Zone::Arc(i) = zone {
                if let Some(ev) = self.fire_item(i) {
                    return ev;
                }
                // Disabled Arc: still tell the user why Save did nothing.
                if self
                    .items
                    .get(i)
                    .is_some_and(|it| matches!(it.id, ContextItemId::Save) && !it.enabled)
                {
                    self.set_status("Save needs Live + held mesh (blake3)");
                }
            }
        }
        ContextEvent::None
    }

    fn fire_item(&mut self, i: usize) -> Option<ContextEvent> {
        let item = self.items.get(i)?;
        if !item.enabled {
            return None;
        }
        let id = item.id;
        let close = item.close_on_press;
        self.primary_down = false;
        if close {
            self.close();
        } else {
            self.dirty = true;
        }
        Some(ContextEvent::Fired(id))
    }
}

/// True when pointer is beyond the soft leave radius around the ring (Desktop).
pub fn pointer_left_menu(px: f32, py: f32, canvas_w: f32, canvas_h: f32) -> bool {
    let cx = canvas_w * 0.5;
    let cy = canvas_h * 0.5;
    let dx = px - cx;
    let dy = py - cy;
    let r = (dx * dx + dy * dy).sqrt();
    let outer = RING_DIAMETER * 0.5 * OUTER_RATIO;
    r > outer * MENU_LEAVE_PAD
}

/// Build Save item when grab target is Drive-saveable.
///
/// `close_on_press: false` — caller closes on HTTP success so failures stay visible on the ring.
pub fn build_items(save_enabled: bool) -> Vec<ContextItem> {
    vec![ContextItem {
        id: ContextItemId::Save,
        label: "Save",
        enabled: save_enabled,
        close_on_press: false,
    }]
}

/// Polar Arc-band hit (inner..outer radius × equal sectors). Intentional vs the
/// drawn label rect: hover/fire uses the sector wedge so multi-item rings stay
/// usable; with one item that means the whole annulus maps to Save (not just the
/// 110×44 button AABB in [`rebuild_layout`]).
fn hit_zone(px: f32, py: f32, w: f32, h: f32, n_items: usize) -> Zone {
    let cx = w * 0.5;
    let cy = h * 0.5;
    let dx = px - cx;
    let dy = py - cy;
    let r = (dx * dx + dy * dy).sqrt();
    let max_r = RING_DIAMETER * 0.5;
    let inner = max_r * INNER_RATIO;
    let outer = max_r * OUTER_RATIO;
    if r < inner {
        return Zone::Inner;
    }
    if r > outer || n_items == 0 {
        return Zone::Outside;
    }
    // Screen Y down: angle from +X, clockwise-ish via atan2(dy, dx).
    let mut theta = dy.atan2(dx);
    if theta < 0.0 {
        theta += std::f32::consts::TAU;
    }
    let sector = std::f32::consts::TAU / n_items as f32;
    // Start Save at top (−Y): shift by −π/2.
    let adjusted = (theta + std::f32::consts::FRAC_PI_2) % std::f32::consts::TAU;
    let idx = ((adjusted / sector) as usize).min(n_items - 1);
    Zone::Arc(idx)
}

/// Fill a LayoutTree with radial visuals centered in `state.canvas_*`.
pub fn rebuild_layout(tree: &mut LayoutTree, state: &ContextRadialState) {
    tree.clear();
    let w = state.canvas_w;
    let h = state.canvas_h;
    let cx = w * 0.5;
    let cy = h * 0.5;
    let max_r = RING_DIAMETER * 0.5;
    let inner = max_r * INNER_RATIO;
    let outer = max_r * OUTER_RATIO;

    // Dim backdrop disk (approx square).
    tree.draws.push(DrawCmd::Rect {
        rect: Rect::new(cx - outer, cy - outer, outer * 2.0, outer * 2.0),
        color: Color::rgba(0.08, 0.09, 0.12, 0.85),
    });

    let n = state.items.len().max(1);
    let sector = std::f32::consts::TAU / n as f32;
    for (i, item) in state.items.iter().enumerate() {
        let mid = -std::f32::consts::FRAC_PI_2 + (i as f32 + 0.5) * sector;
        let rr = (inner + outer) * 0.5;
        let bx = cx + mid.cos() * rr;
        let by = cy + mid.sin() * rr;
        let bw = 110.0;
        let bh = 44.0;
        let hovered = state.hover == Some(i);
        let color = if !item.enabled {
            Color::rgba(0.25, 0.25, 0.28, 0.9)
        } else if hovered {
            Color::rgba(0.25, 0.55, 0.85, 0.95)
        } else {
            Color::rgba(0.18, 0.32, 0.48, 0.95)
        };
        let rect = Rect::new(bx - bw * 0.5, by - bh * 0.5, bw, bh);
        tree.draws.push(DrawCmd::Rect { rect, color });
        tree.draws.push(DrawCmd::Label {
            rect: Rect::new(rect.x + 8.0, rect.y + 10.0, rect.w - 16.0, rect.h - 12.0),
            text: item.label.to_string(),
            color: Color::rgba(0.95, 0.95, 0.98, 1.0),
            scale: 2.0,
            clip: None,
        });
    }

    tree.draws.push(DrawCmd::Rect {
        rect: Rect::new(cx - inner, cy - inner, inner * 2.0, inner * 2.0),
        color: Color::rgba(0.35, 0.38, 0.45, 0.95),
    });

    if !state.status.is_empty() {
        tree.draws.push(DrawCmd::Label {
            rect: Rect::new(24.0, h - 48.0, w - 48.0, 32.0),
            text: state.status.clone(),
            color: Color::rgba(0.9, 0.85, 0.6, 1.0),
            scale: 2.0,
            clip: None,
        });
    }
}

/// Stamp Resonite-style cursor onto an RGBA8 canvas (look-capture has no OS cursor).
pub fn stamp_pointer_rgba(buf: &mut [u8], width: u32, height: u32, px: f32, py: f32) {
    let w = width as i32;
    let h = height as i32;
    if w <= 0 || h <= 0 {
        return;
    }
    let stride = w as usize;
    let fill = |buf: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, rgba: [u8; 4]| {
        let x0 = x0.clamp(0, w);
        let y0 = y0.clamp(0, h);
        let x1 = x1.clamp(0, w);
        let y1 = y1.clamp(0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y as usize * stride + x as usize) * 4;
                if i + 3 < buf.len() {
                    buf[i..i + 4].copy_from_slice(&rgba);
                }
            }
        }
    };
    let cx = px.round() as i32;
    let cy = py.round() as i32;
    let arm = 10;
    let thick = 3;
    let tip = 6;
    let white = [255, 255, 255, 242];
    let tip_c = [89, 217, 255, 255];
    fill(
        buf,
        cx - arm,
        cy - thick / 2,
        cx + arm,
        cy - thick / 2 + thick,
        white,
    );
    fill(
        buf,
        cx - thick / 2,
        cy - arm,
        cx - thick / 2 + thick,
        cy + arm,
        white,
    );
    fill(
        buf,
        cx - tip / 2,
        cy - tip / 2,
        cx - tip / 2 + tip,
        cy - tip / 2 + tip,
        tip_c,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_zone_at_center() {
        assert_eq!(hit_zone(640.0, 360.0, 1280.0, 720.0, 1), Zone::Inner);
    }

    #[test]
    fn save_arc_near_top() {
        let z = hit_zone(640.0, 360.0 - 120.0, 1280.0, 720.0, 1);
        assert_eq!(z, Zone::Arc(0));
    }

    #[test]
    fn click_fires_on_arc_release() {
        let mut s = ContextRadialState::default();
        s.canvas_w = 1280.0;
        s.canvas_h = 720.0;
        s.open = true;
        s.set_items(build_items(true));
        s.set_pointer(640.0, 360.0 - 120.0);
        s.primary_down = true;
        assert_eq!(s.tick_interaction(), ContextEvent::None);
        s.primary_down = false;
        assert_eq!(
            s.tick_interaction(),
            ContextEvent::Fired(ContextItemId::Save)
        );
        assert!(s.open);
    }

    #[test]
    fn click_disabled_sets_status() {
        let mut s = ContextRadialState::default();
        s.canvas_w = 1280.0;
        s.canvas_h = 720.0;
        s.open = true;
        s.set_items(build_items(false));
        s.set_pointer(640.0, 360.0 - 120.0);
        s.primary_down = true;
        assert_eq!(s.tick_interaction(), ContextEvent::None);
        s.primary_down = false;
        assert_eq!(s.tick_interaction(), ContextEvent::None);
        assert!(s.status.contains("blake3"));
        assert!(s.open);
    }

    #[test]
    fn grab_hold_then_release_on_save_fires() {
        // LMB already down (Grab) when menu opens — only release over Save fires.
        let mut s = ContextRadialState::default();
        s.canvas_w = 1280.0;
        s.canvas_h = 720.0;
        s.open = true;
        s.set_items(build_items(true));
        s.primary_down = true;
        s.was_primary_down = true;
        s.set_pointer(640.0, 360.0);
        assert_eq!(s.tick_interaction(), ContextEvent::None);
        s.set_pointer(640.0, 360.0 - 120.0);
        assert_eq!(s.tick_interaction(), ContextEvent::None);
        s.primary_down = false;
        assert_eq!(
            s.tick_interaction(),
            ContextEvent::Fired(ContextItemId::Save)
        );
    }

    #[test]
    fn pointer_left_menu_beyond_leave_pad() {
        assert!(!pointer_left_menu(640.0, 360.0, 1280.0, 720.0));
        assert!(!pointer_left_menu(640.0, 360.0 - 180.0, 1280.0, 720.0));
        assert!(pointer_left_menu(640.0, 360.0 - 280.0, 1280.0, 720.0));
    }

    #[test]
    fn hand_reanchor_keeps_open() {
        let mut s = ContextRadialState::default();
        assert!(s.toggle_or_reanchor(HandSide::Right));
        assert_eq!(s.owner, Some(HandSide::Right));
        assert!(s.toggle_or_reanchor(HandSide::Left));
        assert!(s.open);
        assert_eq!(s.owner, Some(HandSide::Left));
        assert!(!s.toggle_or_reanchor(HandSide::Left));
        assert!(!s.open);
    }

    #[test]
    fn hand_reanchor_clears_status() {
        let mut s = ContextRadialState::default();
        assert!(s.toggle_or_reanchor(HandSide::Right));
        s.set_status("put_back failed");
        assert!(s.toggle_or_reanchor(HandSide::Left));
        assert!(s.status.is_empty());
        assert!(s.open);
    }

    #[test]
    fn desktop_toggle() {
        let mut s = ContextRadialState::default();
        s.toggle_desktop();
        assert!(s.open);
        s.toggle_desktop();
        assert!(!s.open);
    }

    #[test]
    fn close_clears_status() {
        let mut s = ContextRadialState::default();
        s.toggle_desktop();
        s.set_status("put-back ok");
        s.close();
        assert!(s.status.is_empty());
    }
}
