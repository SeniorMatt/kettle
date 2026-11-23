//! The post-game summary of a game's page: the last session (start and end) and the screenshots Kettle's
//! overlay took during it (`~/Pictures/Kettle/<game>/`), plus the small flags of the recommendation box.

use crate::Client;
use sr_model::AppId;
use sr_steam::store;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Most screenshots shown for one session.
const MAX_SHOTS: usize = 24;
/// A screenshot counts for a session when it was saved between its start and this much after its end.
const SLACK_SECS: u64 = 5;
/// "Maybe Later" hides the recommendation box for this long.
const LATER_SECS: u64 = 30 * 86_400;

/// The last session of a game and its screenshots, oldest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PostGame {
    pub start: u64,
    pub end: u64,
    pub shots: Vec<PathBuf>,
}

/// A game's name as one folder name (the overlay uses the same rule for the folder it saves into).
pub fn folder_name(game: &str) -> String {
    let s: String = game.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').trim();
    if s.is_empty() { "Games".into() } else { s.chars().take(80).collect() }
}

/// `~/Pictures/Kettle/<game>`.
pub fn shots_dir(game: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Pictures/Kettle").join(folder_name(game)))
}

/// The PNG files of `dir` saved from `start` to a few seconds after `end`, oldest first.
pub fn shots_between(dir: &Path, start: u64, end: u64) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(u64, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            (t + SLACK_SECS >= start && t <= end + SLACK_SECS).then_some((t, e.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).take(MAX_SHOTS).collect()
}

/// A few flat made-up pictures for the demo library.
fn demo_shots() -> Vec<PathBuf> {
    let dir = store::data_dir().join("demo-shots");
    let _ = std::fs::create_dir_all(&dir);
    let scenes: [([u8; 3], [u8; 3]); 4] = [
        ([0x9a, 0x86, 0x66], [0xc9, 0xb2, 0x8a]),
        ([0x6f, 0x90, 0xb0], [0x8a, 0x5a, 0x48]),
        ([0x55, 0x66, 0x48], [0x3a, 0x44, 0x34]),
        ([0xa0, 0x70, 0x50], [0x50, 0x38, 0x30]),
    ];
    let mut out = Vec::new();
    for (i, (sky, ground)) in scenes.iter().enumerate() {
        let path = dir.join(format!("demo-{i}.png"));
        if !path.exists() {
            let img = image::RgbImage::from_fn(448, 252, |x, y| {
                let block = (x / 56 + y / 42 + i as u32) % 3 == 0;
                let dark = [ground[0] / 2 + 40, ground[1] / 2 + 40, ground[2] / 2 + 40];
                image::Rgb(if y < 120 + 20 * (x / 150) {
                    *sky
                } else if block {
                    *ground
                } else {
                    dark
                })
            });
            if img.save(&path).is_err() {
                continue;
            }
        }
        out.push(path);
    }
    out
}

impl Client {
    /// Look up the game's last session and its screenshots again (page opened, game exited).
    pub(crate) fn refresh_post_game(&mut self, app: AppId) {
        let now = crate::downloads::now_unix();
        if self.play_stats.recommend_hidden(app, now) {
            self.state.gamepage.recommend_hidden.insert(app.0);
        } else {
            self.state.gamepage.recommend_hidden.remove(&app.0);
        }
        let demo = self.state.gamepage.demo;
        let session = if demo {
            (!self.play_stats.post_closed.contains_key(&app.0.to_string()))
                .then(|| (now.saturating_sub(3600), now.saturating_sub(300)))
        } else {
            self.play_stats.open_session(app)
        };
        let shots = match session {
            Some(_) if demo => demo_shots(),
            Some((start, end)) => self
                .state
                .game(app)
                .and_then(|g| shots_dir(&g.name))
                .map(|d| shots_between(&d, start, end))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        match session {
            Some((start, end)) if !shots.is_empty() => {
                self.state.gamepage.post_game.insert(app.0, PostGame { start, end, shots });
            }
            _ => {
                self.state.gamepage.post_game.remove(&app.0);
            }
        }
    }

    pub(crate) fn close_post_game(&mut self, app: AppId) {
        let end = self.state.gamepage.post_game.get(&app.0).map_or(u64::MAX - 1, |p| p.end);
        self.play_stats.post_closed.insert(app.0.to_string(), end);
        let _ = store::save_json(&crate::play::PlayStats::path(), &self.play_stats);
        self.state.gamepage.post_game.remove(&app.0);
    }

    pub(crate) fn hide_recommend(&mut self, app: AppId, forever: bool) {
        let until = if forever { u64::MAX } else { crate::downloads::now_unix() + LATER_SECS };
        self.play_stats.recommend_until.insert(app.0.to_string(), until);
        let _ = store::save_json(&crate::play::PlayStats::path(), &self.play_stats);
        self.state.gamepage.recommend_hidden.insert(app.0);
    }

    pub(crate) fn open_screenshots(&mut self, app: AppId) {
        let Some(dir) = self.state.game(app).and_then(|g| shots_dir(&g.name)) else { return };
        if dir.is_dir() {
            let r = crate::open::folder(&dir, &[dir.as_path()]);
            self.notice_on_err(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_match_the_overlay() {
        assert_eq!(folder_name("Half-Life: Alyx"), "Half-Life_ Alyx");
        assert_eq!(folder_name("  .hidden "), "hidden");
        assert_eq!(folder_name(""), "Games");
    }

    #[test]
    fn only_shots_of_the_session_are_listed() {
        let dir = std::env::temp_dir().join(format!("kettle-postgame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let now = crate::downloads::now_unix();
        assert_eq!(shots_between(&dir, now - 600, now + 1).len(), 1);
        // A session that ended long ago does not own a picture saved just now.
        assert!(shots_between(&dir, now - 7200, now - 3600).is_empty());
        assert!(shots_between(&dir.join("missing"), 0, u64::MAX / 2).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
