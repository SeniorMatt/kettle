//! Settings about downloads, updates and notifications. Kept apart from `settings.rs`; each is one field of
//! [`crate::Settings`], and every field here has a default so an old `settings.json` still loads.

use serde::{Deserialize, Serialize};
use sr_model::AppId;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// When a game gets its updates (per game, like Steam's "Automatic Updates" property).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateBehavior {
    /// Always keep this game up to date (download as soon as an update is found).
    #[default]
    Always,
    /// Only update when I launch it.
    OnLaunch,
    /// Like `Always`, and its updates go to the front of the queue.
    HighPriority,
}

impl UpdateBehavior {
    pub const ALL: [UpdateBehavior; 3] =
        [UpdateBehavior::Always, UpdateBehavior::OnLaunch, UpdateBehavior::HighPriority];

    pub fn label(self) -> &'static str {
        match self {
            UpdateBehavior::Always => "Always keep this game up to date",
            UpdateBehavior::OnLaunch => "Only update this game when I launch it",
            UpdateBehavior::HighPriority => "High priority: always update this game first",
        }
    }
}

/// May Kettle write updates into library folders the official Steam client uses?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SteamUpdatePolicy {
    /// Ask the first time; until answered, such games are not touched.
    #[default]
    Ask,
    Never,
    Yes,
}

impl SteamUpdatePolicy {
    pub const ALL: [SteamUpdatePolicy; 3] = [SteamUpdatePolicy::Ask, SteamUpdatePolicy::Never, SteamUpdatePolicy::Yes];

    pub fn label(self) -> &'static str {
        match self {
            SteamUpdatePolicy::Ask => "Ask me",
            SteamUpdatePolicy::Never => "Never",
            SteamUpdatePolicy::Yes => "Yes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    /// Speed limit in KB/s; 0 = no limit.
    pub limit_kbps: u32,
    /// Only start automatic updates between `schedule_from` and `schedule_to` (local hours, 0..=23).
    pub schedule_enabled: bool,
    pub schedule_from: u8,
    pub schedule_to: u8,
    /// Keep downloading while a game started by Kettle runs. Off: downloads wait until it exits.
    pub allow_during_gameplay: bool,
    /// Show rates as bits per second (Mbps) instead of bytes.
    pub bits_per_second: bool,
    /// Look for updates and download them automatically.
    pub auto_update: bool,
    /// Updates of games in the official client's library folders.
    pub steam_policy: SteamUpdatePolicy,
    /// Per game (by app id); a missing entry means `Always`.
    pub behavior: BTreeMap<String, UpdateBehavior>,
    /// Language of new installs ("" = English).
    pub language: String,
    /// Library folders the user added (ours).
    pub libraries: Vec<PathBuf>,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        DownloadSettings {
            limit_kbps: 0,
            schedule_enabled: false,
            schedule_from: 2,
            schedule_to: 6,
            allow_during_gameplay: false,
            bits_per_second: false,
            auto_update: true,
            steam_policy: SteamUpdatePolicy::Ask,
            behavior: BTreeMap::new(),
            language: String::new(),
            libraries: Vec::new(),
        }
    }
}

impl DownloadSettings {
    pub fn limit_bytes_per_sec(&self) -> u64 {
        u64::from(self.limit_kbps) * 1024
    }

    pub fn behavior_of(&self, app: AppId) -> UpdateBehavior {
        self.behavior.get(&app.0.to_string()).copied().unwrap_or_default()
    }

    pub fn language_or_default(&self) -> &str {
        if self.language.is_empty() { "english" } else { &self.language }
    }

    /// May an automatic update start at local hour `hour`?
    pub fn may_auto_update_at(&self, hour: u8) -> bool {
        self.auto_update && (!self.schedule_enabled || hour_in_window(hour, self.schedule_from, self.schedule_to))
    }
}

/// Is `hour` (0..=23) inside `from..to`? The window may wrap past midnight (22 to 6); equal ends mean all day.
pub fn hour_in_window(hour: u8, from: u8, to: u8) -> bool {
    let (hour, from, to) = (hour % 24, from % 24, to % 24);
    match from.cmp(&to) {
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Less => hour >= from && hour < to,
        std::cmp::Ordering::Greater => hour >= from || hour < to,
    }
}

/// The local hour (0..=23) at `unix`, given the zone's offset in seconds east of UTC.
pub fn local_hour(unix: u64, utc_offset: i64) -> u8 {
    (((unix as i64 + utc_offset).rem_euclid(86_400)) / 3600) as u8
}

/// Format a rate the way the user chose: "12.3 MB/s" or "98 Mbps".
pub fn rate_text(bytes_per_sec: f64, bits: bool) -> String {
    if bytes_per_sec < 1.0 {
        return "-".into();
    }
    if bits {
        let mbps = bytes_per_sec * 8.0 / 1_000_000.0;
        if mbps >= 10.0 { format!("{mbps:.0} Mbps") } else { format!("{mbps:.1} Mbps") }
    } else {
        let mb = bytes_per_sec / (1u64 << 20) as f64;
        if mb >= 1.0 { format!("{mb:.1} MB/s") } else { format!("{:.0} KB/s", bytes_per_sec / 1024.0) }
    }
}

/// Which desktop notifications to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    /// Master switch.
    pub enabled: bool,
    pub download_done: bool,
    pub friend_online: bool,
    pub chat_message: bool,
    pub friend_game: bool,
    /// Ask the notification server for its theme's sound (no sound files of our own).
    pub sound: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        NotifySettings {
            enabled: true,
            download_done: true,
            friend_online: false,
            chat_message: true,
            friend_game: false,
            sound: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyKind {
    DownloadDone,
    FriendOnline,
    ChatMessage,
    FriendGame,
}

impl NotifySettings {
    pub fn wants(&self, kind: NotifyKind) -> bool {
        self.enabled
            && match kind {
                NotifyKind::DownloadDone => self.download_done,
                NotifyKind::FriendOnline => self.friend_online,
                NotifyKind::ChatMessage => self.chat_message,
                NotifyKind::FriendGame => self.friend_game,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_window_wraps_midnight() {
        assert!(hour_in_window(3, 2, 6));
        assert!(!hour_in_window(6, 2, 6));
        assert!(!hour_in_window(1, 2, 6));
        // 22:00 to 06:00
        assert!(hour_in_window(23, 22, 6));
        assert!(hour_in_window(0, 22, 6));
        assert!(hour_in_window(5, 22, 6));
        assert!(!hour_in_window(12, 22, 6));
        // Equal ends: all day.
        assert!(hour_in_window(13, 5, 5));
    }

    #[test]
    fn auto_update_respects_switch_and_schedule() {
        let mut d = DownloadSettings::default();
        assert!(d.may_auto_update_at(12));
        d.schedule_enabled = true;
        assert!(!d.may_auto_update_at(12));
        assert!(d.may_auto_update_at(3));
        d.auto_update = false;
        assert!(!d.may_auto_update_at(3));
    }

    #[test]
    fn local_hour_applies_the_offset() {
        // 1970-01-02 00:30 UTC.
        let t = 86_400 + 1800;
        assert_eq!(local_hour(t, 0), 0);
        assert_eq!(local_hour(t, 3600 * 2), 2);
        assert_eq!(local_hour(t, -3600), 23);
    }

    #[test]
    fn rates_in_bytes_and_bits() {
        assert_eq!(rate_text(0.0, false), "-");
        assert_eq!(rate_text(12.0 * 1048576.0, false), "12.0 MB/s");
        assert_eq!(rate_text(512.0 * 1024.0, false), "512 KB/s");
        assert_eq!(rate_text(1_250_000.0, true), "10 Mbps");
        assert_eq!(rate_text(250_000.0, true), "2.0 Mbps");
    }

    #[test]
    fn per_game_behavior_defaults_to_always() {
        let mut d = DownloadSettings::default();
        assert_eq!(d.behavior_of(AppId(10)), UpdateBehavior::Always);
        d.behavior.insert("10".into(), UpdateBehavior::OnLaunch);
        assert_eq!(d.behavior_of(AppId(10)), UpdateBehavior::OnLaunch);
        assert_eq!(d.behavior_of(AppId(11)), UpdateBehavior::Always);
    }

    #[test]
    fn notification_toggles() {
        let mut n = NotifySettings::default();
        assert!(n.wants(NotifyKind::DownloadDone));
        assert!(!n.wants(NotifyKind::FriendOnline));
        n.enabled = false;
        assert!(!n.wants(NotifyKind::DownloadDone));
    }
}
