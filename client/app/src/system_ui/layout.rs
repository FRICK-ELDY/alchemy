//! Retained layout primitives + hit testing for system_ui Canvas.
//!
//! Guarantees: axis-aligned rect hit test in canvas pixel space.
//! Non-guarantees: flexbox, wrapping, VR pointer rays.

#![allow(dead_code)]

/// Stable id for hit targets and dirty tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetId {
    Mic,
    MicOff,
    Volume,
    Hearing,
    SyncBadge,
    CloseSession,
    TabExit,
    TabSettings,
    ExitClientOnly,
    ExitClientAndHost,
    SettingsPlaceholder,
    /// Toggle Snap ↔ Smooth comfort turn.
    TurnModeToggle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Clone, Debug)]
pub enum DrawCmd {
    Rect {
        rect: Rect,
        color: Color,
    },
    Icon {
        rect: Rect,
        name: &'static str,
    },
    Label {
        rect: Rect,
        text: String,
        color: Color,
        scale: f32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct LayoutTree {
    pub draws: Vec<DrawCmd>,
    /// Later entries are front-most for hit testing.
    pub hits: Vec<(WidgetId, Rect)>,
}

impl LayoutTree {
    pub fn clear(&mut self) {
        self.draws.clear();
        self.hits.clear();
    }

    pub fn push_rect(&mut self, rect: Rect, color: Color) {
        self.draws.push(DrawCmd::Rect { rect, color });
    }

    pub fn push_icon(&mut self, rect: Rect, name: &'static str) {
        self.draws.push(DrawCmd::Icon { rect, name });
    }

    pub fn push_label(&mut self, rect: Rect, text: impl Into<String>, color: Color, scale: f32) {
        self.draws.push(DrawCmd::Label {
            rect,
            text: text.into(),
            color,
            scale,
        });
    }

    pub fn register_hit(&mut self, id: WidgetId, rect: Rect) {
        self.hits.push((id, rect));
    }

    /// Front-most (last registered overlapping) wins.
    pub fn hit_test(&self, px: f32, py: f32) -> Option<WidgetId> {
        self.hits
            .iter()
            .rev()
            .find(|(_, r)| r.contains(px, py))
            .map(|(id, _)| *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_edges() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(39.9, 59.9));
        assert!(!r.contains(40.0, 20.0));
        assert!(!r.contains(10.0, 60.0));
    }

    #[test]
    fn hit_test_prefers_later_registration_order_via_rev() {
        let mut tree = LayoutTree::default();
        tree.register_hit(WidgetId::TabExit, Rect::new(0.0, 0.0, 100.0, 100.0));
        tree.register_hit(WidgetId::ExitClientOnly, Rect::new(10.0, 10.0, 20.0, 20.0));
        assert_eq!(tree.hit_test(15.0, 15.0), Some(WidgetId::ExitClientOnly));
        assert_eq!(tree.hit_test(50.0, 50.0), Some(WidgetId::TabExit));
        assert_eq!(tree.hit_test(200.0, 200.0), None);
    }
}
