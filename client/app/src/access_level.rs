//! Room access level ↔ world `visibility` wire values (x0034–x0037).
//!
//! Guarantees: closed allowlist of 7 levels and stable wire strings.
//! Non-guarantees: HTTP / UI layout (callers own those).
//!
//! Keep wire strings in sync with world `World.Visibility` and host
//! `ROOM_VISIBILITY_ALLOWED` (`host-server` `room_token.rs`).

/// Access level ↔ world `visibility` wire value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessLevel {
    Private,
    Lan,
    Friends,
    FriendsPlus,
    Followers,
    FollowersPlus,
    Public,
}

impl AccessLevel {
    pub const ALL: [AccessLevel; 7] = [
        Self::Private,
        Self::Lan,
        Self::Friends,
        Self::FriendsPlus,
        Self::Followers,
        Self::FollowersPlus,
        Self::Public,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Lan => "lan",
            Self::Friends => "friends",
            Self::FriendsPlus => "friends_plus",
            Self::Followers => "followers",
            Self::FollowersPlus => "followers_plus",
            Self::Public => "public",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Private => "Private (invite only)",
            Self::Lan => "LAN only",
            Self::Friends => "Friends",
            Self::FriendsPlus => "Friends+",
            Self::Followers => "Followers",
            Self::FollowersPlus => "Followers+",
            Self::Public => "Anyone (signed in)",
        }
    }

    /// Parse wire / env value; unknown → `None`.
    #[allow(dead_code)] // used by tests; host has a parallel allowlist for env
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "private" => Some(Self::Private),
            "lan" => Some(Self::Lan),
            "friends" => Some(Self::Friends),
            "friends_plus" => Some(Self::FriendsPlus),
            "followers" => Some(Self::Followers),
            "followers_plus" => Some(Self::FollowersPlus),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical wire order shared with world `Visibility` / host `ROOM_VISIBILITY_ALLOWED`.
    const WIRE_ORDER: &[&str] = &[
        "private",
        "lan",
        "friends",
        "friends_plus",
        "followers",
        "followers_plus",
        "public",
    ];

    #[test]
    fn roundtrip_all_levels() {
        for level in AccessLevel::ALL {
            assert_eq!(AccessLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(AccessLevel::parse("nope"), None);
        assert_eq!(AccessLevel::parse("PUBLIC"), None);
    }

    #[test]
    fn all_wire_strings_match_canonical_order() {
        let got: Vec<&str> = AccessLevel::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(got, WIRE_ORDER);
    }
}
