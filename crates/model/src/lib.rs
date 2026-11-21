//! Plain data types shared by every kettle crate.
//!
//! Architecture invariant: this crate does no I/O and depends only on serde.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A Steam application id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppId(pub u32);

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A 64-bit SteamID of a user account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SteamId(pub u64);

impl fmt::Display for SteamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A logged-in (or remembered) account. Holds no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub account_name: String,
    pub steam_id: SteamId,
}

/// Where a game stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InstallState {
    #[default]
    NotInstalled,
    Installed {
        size_bytes: u64,
        path: String,
    },
    UpdateRequired {
        size_bytes: u64,
        path: String,
    },
    Downloading {
        done_bytes: u64,
        total_bytes: u64,
    },
}

impl InstallState {
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed { .. } | Self::UpdateRequired { .. })
    }
}

/// One game in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    pub app_id: AppId,
    pub name: String,
    /// Lower-cased name without punctuation, for sorting and search.
    pub sort_key: String,
    pub playtime_minutes: u32,
    /// Unix seconds; 0 = never.
    pub last_played: u64,
    /// Hash of the 32x32 list icon (empty if unknown).
    #[serde(default)]
    pub icon_hash: String,
    #[serde(skip)]
    pub install: InstallState,
}

impl Game {
    pub fn new(app_id: AppId, name: String) -> Self {
        let sort_key = sort_key(&name);
        Self {
            app_id,
            name,
            sort_key,
            playtime_minutes: 0,
            last_played: 0,
            icon_hash: String::new(),
            install: InstallState::NotInstalled,
        }
    }
}

/// Sort/search key: lower-case, alphanumerics and spaces only, leading
/// "the " dropped, like Steam's library sort.
pub fn sort_key(name: &str) -> String {
    let s: String =
        name.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).flat_map(char::to_lowercase).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.strip_prefix("the ").map(str::to_string).unwrap_or(s)
}

/// Kinds of cover art Steam provides per app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtKind {
    /// 600x900 portrait capsule (library grid).
    Capsule,
    /// 3840x1240 hero banner (game page background).
    Hero,
    /// Transparent game logo drawn over the hero.
    Logo,
    /// 460x215 store header (fallback when there's no capsule).
    Header,
    /// 32x32 list icon; its file name is a per-app hash.
    Icon,
}

impl ArtKind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Capsule => "library_600x900.jpg",
            Self::Hero => "library_hero.jpg",
            Self::Logo => "logo.png",
            Self::Header => "header.jpg",
            Self::Icon => "icon.jpg",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_key_normalises() {
        assert_eq!(sort_key("The Witcher® 3: Wild Hunt"), "witcher 3 wild hunt");
        assert_eq!(sort_key("  Half-Life  2 "), "halflife 2");
    }

    #[test]
    fn install_state() {
        assert!(!InstallState::NotInstalled.is_installed());
        assert!(InstallState::Installed { size_bytes: 1, path: "/x".into() }.is_installed());
    }
}
