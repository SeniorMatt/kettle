//! Demo library for screenshots and performance tests without a login:
//! every app id the official client has cached art for, plus installed games,
//! repeated until there are `target` entries. Enabled by `KETTLE_DEMO=<n>`.

use crate::friends::{Friend, GamePlayed, PersonaState};
use sr_model::{AppId, Game};
use sr_steam::gamepage::{Achievement, Achievements, NewsItem};
use sr_steam::local;

/// Steamworks SDK Redist: free to download without a login.
const DEMO_DOWNLOAD_APP: u32 = 1007;

pub fn games(target: usize) -> Vec<Game> {
    let installed = local::installed_apps();
    let mut ids: Vec<u32> = Vec::new();
    for root in local::steam_roots() {
        if let Ok(rd) = std::fs::read_dir(root.join("appcache/librarycache")) {
            ids.extend(rd.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()));
        }
    }
    ids.extend(installed.keys().copied());
    // `KETTLE_DEMO_APPS=1,2`: extra app ids (e.g. ones with hashed store art) to try.
    if let Ok(extra) = std::env::var("KETTLE_DEMO_APPS") {
        ids.extend(extra.split(',').filter_map(|s| s.trim().parse::<u32>().ok()));
    }
    ids.retain(|id| !installed.get(id).is_some_and(|a| local::is_tool_app(a.appid, &a.name)));
    // A small app that anonymous logins may download, so the install flow can be tried without an account.
    ids.push(DEMO_DOWNLOAD_APP);
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(target);
    for i in 0..target.max(ids.len()) {
        let id = ids[i % ids.len()];
        let lap = i / ids.len();
        let base = installed.get(&id).map(|a| a.name.clone()).unwrap_or_else(|| match id {
            DEMO_DOWNLOAD_APP => "Steamworks SDK Redist".to_string(),
            _ => known_name(id).map_or_else(|| format!("App {id}"), str::to_string),
        });
        let name = if lap == 0 { base } else { format!("{base} ({})", lap + 1) };
        let mut g = Game::new(AppId(id), name);
        g.playtime_minutes = (id % 5000) * (lap as u32 + 1);
        g.last_played = 1_700_000_000 + u64::from(id % 30_000_000);
        out.push(g);
    }
    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    out
}

/// Names of well-known apps, so demo screenshots show real titles even without a local Steam.
fn known_name(id: u32) -> Option<&'static str> {
    const NAMES: &[(u32, &str)] = &[
        (220, "Half-Life 2"),
        (400, "Portal"),
        (440, "Team Fortress 2"),
        (550, "Left 4 Dead 2"),
        (570, "Dota 2"),
        (620, "Portal 2"),
        (730, "Counter-Strike 2"),
        (4000, "Garry's Mod"),
        (105600, "Terraria"),
        (211820, "Starbound"),
        (239140, "Dying Light"),
        (252490, "Rust"),
        (271590, "Grand Theft Auto V"),
        (275850, "No Man's Sky"),
        (292030, "The Witcher 3: Wild Hunt"),
        (322330, "Don't Starve Together"),
        (367520, "Hollow Knight"),
        (374320, "DARK SOULS III"),
        (413150, "Stardew Valley"),
        (489830, "The Elder Scrolls V: Skyrim Special Edition"),
        (504230, "Celeste"),
        (646570, "Slay the Spire"),
        (1086940, "Baldur's Gate 3"),
        (1091500, "Cyberpunk 2077"),
        (1145360, "Hades"),
        (1174180, "Red Dead Redemption 2"),
        (1245620, "ELDEN RING"),
    ];
    NAMES.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

// ---- game page (demo mode only) -------------------------------------------------

/// Two made-up friends playing the game.
pub fn playing(app: u32, name: &str) -> Vec<Friend> {
    let mk = |id: u64, who: &str, rich: &str| Friend {
        id: sr_model::SteamId(76_561_198_000_000_000 + id),
        name: who.into(),
        avatar_url: None,
        state: PersonaState::Online,
        game: Some(GamePlayed { app_id: app, name: name.into() }),
        last_logoff: 0,
        rich: Some(rich.into()),
    };
    vec![mk(901, "Tiha", "Casual - Upward"), mk(902, "Rowan", "Competitive - Badwater")]
}

/// Five news items, newest first (today, yesterday and older, so the date groups show).
pub fn news(app: u32) -> Vec<NewsItem> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let item = |n: u64, age_days: u64, label: &str, title: &str, text: &str| NewsItem {
        gid: format!("{app}00{n}"),
        title: title.into(),
        date: now.saturating_sub(age_days * 86_400 + 600),
        text: text.into(),
        label: label.into(),
    };
    vec![
        item(
            1,
            0,
            "Patch Notes",
            "Update Released",
            "Fixed a crash when joining a match late, adjusted several map spawn points and updated the localization files.",
        ),
        item(
            2,
            1,
            "Community Announcements",
            "Weekend event is live",
            "Collect event items until Monday and trade them with friends. The community market opens on Sunday.",
        ),
        item(
            3,
            4,
            "Patch Notes",
            "Small hotfix",
            "Fixed an exploit on one of the maps and a visual glitch in the scoreboard.",
        ),
        item(
            4,
            12,
            "Community Announcements",
            "Community map contest results",
            "Thanks to everyone who sent in a map. The winners have been added to the game.",
        ),
        item(
            5,
            30,
            "Patch Notes",
            "Summer update",
            "New cosmetic items, balance changes and many bug fixes. See the full list of changes on the update page.",
        ),
    ]
}

/// 12 of 30 achievements unlocked.
pub fn achievements(_app: u32) -> Achievements {
    let list: Vec<Achievement> = (0..30u32)
        .map(|i| Achievement {
            name: format!("ACH_{i}"),
            display: format!("Achievement {}", i + 1),
            desc: String::new(),
            icon: String::new(),
            unlocked: i < 12,
            time: 1_700_000_000 - u64::from(i) * 86_400,
        })
        .collect();
    Achievements { total: 30, unlocked: 12, list }
}

/// Three DLC: (app id, name, installed).
pub fn dlc(app: u32) -> Vec<(u32, String, bool)> {
    vec![
        (app + 1, "Soundtrack".into(), true),
        (app + 2, "Deluxe Cosmetic Pack".into(), false),
        (app + 3, "Supporter Pack".into(), false),
    ]
}
