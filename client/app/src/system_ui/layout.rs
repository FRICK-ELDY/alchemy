//! Retained layout primitives + hit testing for system_ui Canvas (UI runtime).
//!
//! Guarantees: axis-aligned rect hit test in canvas pixel space; shared by
//! Desktop pointer and XR surface (pixels only — no OpenXR types here).
//! Non-guarantees: flexbox, wrapping, WorldRoot, Facet rearrange.

#![allow(dead_code)]

/// Stable id for hit targets and dirty tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetId {
    Mic,
    MicOff,
    Volume,
    Hearing,
    /// Top chrome: not signed in → open login/register.
    ChromeLoginRegister,
    /// Top chrome: signed in → logout.
    ChromeLogout,
    SyncBadge,
    CloseSession,
    TabHome,
    TabExit,
    TabSettings,
    /// Inventory take-out / put-back (x006d / x006e).
    TabInventory,
    /// Worlds catalog (x007c).
    TabWorlds,
    InventoryRefresh,
    InventoryTakeOut,
    InventoryPutBack,
    /// Scene mesh despawn (x007b).
    InventoryDespawn,
    InventoryRow0,
    InventoryRow1,
    InventoryRow2,
    InventoryRow3,
    InventoryRow4,
    InventoryRow5,
    InventoryRow6,
    InventoryRow7,
    SceneMeshRow0,
    SceneMeshRow1,
    SceneMeshRow2,
    SceneMeshRow3,
    ExitClientOnly,
    ExitClientAndHost,
    SettingsPlaceholder,
    /// Toggle Snap ↔ Smooth comfort turn.
    TurnModeToggle,
    /// Toggle XR↔Desktop auto hotswitch (x0053).
    HotswitchAutoToggle,
    /// Settings: enable local host / Organizing (Server Operator mode).
    ServerOperatorToggle,
    /// Settings panel scroll (overflow above bottom tabs).
    SettingsScrollUp,
    SettingsScrollDown,
    /// Home / account shell (x0018).
    HomeFieldIdentifier,
    HomeFieldPassword,
    HomeFieldUsername,
    HomeFieldEmail,
    HomeFieldBirthday,
    HomeRememberMe,
    HomeTosAgreed,
    HomeModeLogin,
    HomeModeRegister,
    HomeSubmit,
    HomeLogout,
    /// Logged-in hub (x0037 / x007c Phase 4): open Worlds catalog.
    HomeOpenWorlds,
    /// Logged-in hub / Worlds: open create-world panel.
    HomeNewWorld,
    HomeJoinRoom,
    /// Create-world panel (placeholder UI).
    HomeCreateBack,
    HomeCreateName,
    HomeTemplateGrid,
    HomeAccessPrivate,
    HomeAccessLan,
    HomeAccessFriends,
    HomeAccessFriendsPlus,
    HomeAccessFollowers,
    HomeAccessFollowersPlus,
    HomeAccessPublic,
    HomeStartSession,
    /// Settings advanced room controls (x007c Phase 4).
    SettingsRoomId,
    SettingsJoinRoom,
    SettingsStartSession,
    /// Onboarding Step1 (operator select).
    OperatorOfficial,
    OperatorFederation,
    OperatorCustom,
    OperatorCustomHost,
    Step1Next,
    Step1Back,
    FederationStubBack,
    Step2Back,
    /// MainHub tab: join sessions (default).
    MainTabParticipation,
    /// MainHub tab: create / host worlds.
    MainTabOrganizing,
    /// Participation: refresh public session catalog.
    MainHubRefresh,
    /// Participation: Join selected public session as guest.
    MainHubJoin,
    /// Organizing: open create-world flow.
    MainHubCreateWorld,
    /// Organizing: host selected owned world (stub rows until API).
    MainHubHostWorld,
    /// Participation public session rows (maps into `worlds_selected`).
    MainPublicRow0,
    MainPublicRow1,
    MainPublicRow2,
    MainPublicRow3,
    MainPublicRow4,
    MainPublicRow5,
    MainPublicRow6,
    MainPublicRow7,
    /// Organizing my-world rows (stub; reserved for owner catalog).
    MainMyWorldRow0,
    MainMyWorldRow1,
    MainMyWorldRow2,
    MainMyWorldRow3,
    MainMyWorldRow4,
    MainMyWorldRow5,
    MainMyWorldRow6,
    MainMyWorldRow7,
    /// Worlds catalog (x007c).
    WorldsRefresh,
    WorldsFilterAll,
    WorldsFilterMine,
    WorldsCreate,
    WorldsJoinGuest,
    WorldsHostRoom,
    WorldsLeave,
    /// KeyStale: Leave → Join with current room (x007e).
    WorldsRejoin,
    WorldsHostRescue,
    WorldsRow0,
    WorldsRow1,
    WorldsRow2,
    WorldsRow3,
    WorldsRow4,
    WorldsRow5,
    WorldsRow6,
    WorldsRow7,
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

    /// Axis-aligned intersection; `None` if empty.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
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
        /// Optional scissor (canvas pixels); applied at rasterize.
        clip: Option<Rect>,
    },
    Label {
        rect: Rect,
        text: String,
        color: Color,
        scale: f32,
        /// Optional scissor (canvas pixels); applied at rasterize.
        clip: Option<Rect>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct LayoutTree {
    pub draws: Vec<DrawCmd>,
    /// Later entries are front-most for hit testing.
    pub hits: Vec<(WidgetId, Rect)>,
    /// Optional scissor for subsequent draws / hits (Settings scroll panel, etc.).
    pub clip: Option<Rect>,
}

impl LayoutTree {
    pub fn clear(&mut self) {
        self.draws.clear();
        self.hits.clear();
        self.clip = None;
    }

    pub fn push_rect(&mut self, rect: Rect, color: Color) {
        let rect = match self.clip {
            Some(clip) => match rect.intersect(clip) {
                Some(r) => r,
                None => return,
            },
            None => rect,
        };
        self.draws.push(DrawCmd::Rect { rect, color });
    }

    pub fn push_icon(&mut self, rect: Rect, name: &'static str) {
        if let Some(clip) = self.clip {
            if rect.intersect(clip).is_none() {
                return;
            }
        }
        self.draws.push(DrawCmd::Icon {
            rect,
            name,
            clip: self.clip,
        });
    }

    pub fn push_label(&mut self, rect: Rect, text: impl Into<String>, color: Color, scale: f32) {
        if let Some(clip) = self.clip {
            if rect.intersect(clip).is_none() {
                return;
            }
        }
        self.draws.push(DrawCmd::Label {
            rect,
            text: text.into(),
            color,
            scale,
            clip: self.clip,
        });
    }

    pub fn register_hit(&mut self, id: WidgetId, rect: Rect) {
        let rect = match self.clip {
            Some(clip) => match rect.intersect(clip) {
                Some(r) => r,
                None => return,
            },
            None => rect,
        };
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
