//! Dash Facet catalog (x000f + x0018 Home).
//!
//! Guarantees: `DashRoot` is personal / not on Room `frame`; Exit Facet is pinned;
//! official default Facet set is code-defined.
//! Non-guarantees: user rearrange, Vault layout persist, `WorldRoot`, Grab edit.
//!
//! UI runtime (Canvas / Button hit + color) is shared in spirit with a future World UI,
//! but ownership stays on `DashRoot` here — never publish dash entities on the room.

use super::layout::WidgetId;

/// Personal dash container. Never published on Room `/frame`.
#[derive(Debug, Clone, Copy)]
pub struct DashRoot;

impl DashRoot {
    /// Official default Facet set shipped with the client (this Slice: code-defined).
    pub fn official_facets() -> &'static [FacetId] {
        OFFICIAL_DEFAULT_FACETS
    }

    pub fn is_pinned(id: FacetId) -> bool {
        id.pinned()
    }
}

/// Facet = panel unit on the dash (Resonite-like).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FacetId {
    /// Top chrome (mic stubs, sync, close session affordances).
    Chrome,
    /// Home tab: login / register / logout (x0018).
    Home,
    /// Settings tab body + turn toggle.
    Settings,
    /// Exit tab body + exit actions. **Pinned** — cannot be removed.
    Exit,
}

impl FacetId {
    pub fn pinned(self) -> bool {
        matches!(self, Self::Exit)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Home => "Home",
            Self::Settings => "Settings",
            Self::Exit => "Exit",
        }
    }
}

/// Code-defined official defaults (reorder / user Facets are later Slices).
pub const OFFICIAL_DEFAULT_FACETS: &[FacetId] = &[
    FacetId::Chrome,
    FacetId::Home,
    FacetId::Settings,
    FacetId::Exit,
];

/// Map a hit target to its owning Facet (layout vocabulary → Facet vocabulary).
pub fn facet_of(widget: WidgetId) -> FacetId {
    match widget {
        WidgetId::TabExit
        | WidgetId::CloseSession
        | WidgetId::ExitClientOnly
        | WidgetId::ExitClientAndHost => FacetId::Exit,
        WidgetId::TabSettings | WidgetId::TurnModeToggle | WidgetId::SettingsPlaceholder => {
            FacetId::Settings
        }
        WidgetId::TabHome
        | WidgetId::HomeFieldIdentifier
        | WidgetId::HomeFieldPassword
        | WidgetId::HomeFieldUsername
        | WidgetId::HomeFieldEmail
        | WidgetId::HomeFieldBirthday
        | WidgetId::HomeRememberMe
        | WidgetId::HomeModeLogin
        | WidgetId::HomeModeRegister
        | WidgetId::HomeSubmit
        | WidgetId::HomeLogout
        | WidgetId::HomeNewWorld
        | WidgetId::HomeJoinRoom
        | WidgetId::HomeCreateBack
        | WidgetId::HomeCreateName
        | WidgetId::HomeTemplateGrid
        | WidgetId::HomeAccessPrivate
        | WidgetId::HomeAccessLan
        | WidgetId::HomeAccessFriends
        | WidgetId::HomeAccessFriendsPlus
        | WidgetId::HomeAccessFollowers
        | WidgetId::HomeAccessFollowersPlus
        | WidgetId::HomeAccessPublic
        | WidgetId::HomeStartSession => FacetId::Home,
        WidgetId::Mic
        | WidgetId::MicOff
        | WidgetId::Volume
        | WidgetId::Hearing
        | WidgetId::SyncBadge => FacetId::Chrome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_facet_is_pinned() {
        assert!(FacetId::Exit.pinned());
        assert!(DashRoot::is_pinned(FacetId::Exit));
        assert!(!FacetId::Settings.pinned());
        assert!(!FacetId::Home.pinned());
        assert!(!FacetId::Chrome.pinned());
    }

    #[test]
    fn official_set_contains_home_and_pinned_exit() {
        let set = DashRoot::official_facets();
        assert!(set.contains(&FacetId::Home));
        assert!(set.contains(&FacetId::Exit));
        assert!(set.iter().any(|f| f.pinned()));
    }

    #[test]
    fn home_widgets_map_to_home_facet() {
        assert_eq!(facet_of(WidgetId::TabHome), FacetId::Home);
        assert_eq!(facet_of(WidgetId::HomeSubmit), FacetId::Home);
        assert_eq!(facet_of(WidgetId::HomeLogout), FacetId::Home);
    }
}
