//! ProtonDB rating for games without a native Linux build.
//!
//! One public request per game: `https://www.protondb.com/api/v1/reports/summaries/<appid>.json`. It carries
//! only the app id (no account, no cookies). The answer is cached in our cache folder for a day, also when the
//! site has no reports for the game.

use super::{Client, Event, Load};
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::store;
use std::path::PathBuf;

/// How long a cached answer is used.
pub const CACHE_SECS: u64 = 24 * 3600;
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Platinum,
    Gold,
    Silver,
    Bronze,
    Borked,
    /// Too few reports for a rating.
    Pending,
}

impl Tier {
    pub fn parse(s: &str) -> Option<Tier> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "platinum" => Tier::Platinum,
            "gold" => Tier::Gold,
            "silver" => Tier::Silver,
            "bronze" => Tier::Bronze,
            "borked" => Tier::Borked,
            "pending" => Tier::Pending,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Platinum => "Platinum",
            Tier::Gold => "Gold",
            Tier::Silver => "Silver",
            Tier::Bronze => "Bronze",
            Tier::Borked => "Borked",
            Tier::Pending => "Pending",
        }
    }

    /// One line for a tooltip.
    pub fn meaning(self) -> &'static str {
        match self {
            Tier::Platinum => "Runs perfectly out of the box",
            Tier::Gold => "Runs perfectly after some tweaks",
            Tier::Silver => "Runs with minor issues",
            Tier::Bronze => "Runs, but often crashes or has major issues",
            Tier::Borked => "Won't run",
            Tier::Pending => "Not enough reports yet",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtonDbRating {
    pub tier: Tier,
    /// "strong", "good", "moderate", "low", "inadequate" (as ProtonDB writes it; may be empty).
    pub confidence: String,
    /// Number of reports.
    pub total: u32,
}

impl ProtonDbRating {
    /// "Gold, 1234 reports".
    pub fn summary(&self) -> String {
        match self.total {
            0 => self.tier.label().to_string(),
            1 => format!("{}, 1 report", self.tier.label()),
            n => format!("{}, {n} reports", self.tier.label()),
        }
    }
}

/// What the cache file holds: when it was fetched and the answer (`None` = ProtonDB has no data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Cached {
    fetched: u64,
    rating: Option<ProtonDbRating>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    tier: String,
    #[serde(default)]
    confidence: String,
    #[serde(default)]
    total: u64,
}

/// Parse the summary JSON. An unknown tier name gives `None` (no rating shown).
pub fn parse_summary(json: &str) -> Option<ProtonDbRating> {
    let raw: Raw = serde_json::from_str(json).ok()?;
    let tier = Tier::parse(&raw.tier)?;
    let confidence: String = raw.confidence.chars().filter(char::is_ascii_alphabetic).take(16).collect();
    Some(ProtonDbRating { tier, confidence, total: raw.total.min(u64::from(u32::MAX)) as u32 })
}

pub fn url(app: u32) -> String {
    format!("https://www.protondb.com/api/v1/reports/summaries/{app}.json")
}

/// The ProtonDB page of a game (for a "view on ProtonDB" link).
pub fn page_url(app: u32) -> String {
    format!("https://www.protondb.com/app/{app}")
}

fn cache_path(app: u32) -> PathBuf {
    store::cache_dir().join("protondb").join(format!("{app}.json"))
}

fn fresh(c: &Cached, now: u64) -> bool {
    now >= c.fetched && now - c.fetched < CACHE_SECS
}

/// Worker: from the cache when it is under a day old, else from ProtonDB. `Ok(None)` = no reports.
fn load(app: u32, now: u64) -> Result<Option<ProtonDbRating>, String> {
    let path = cache_path(app);
    if let Some(c) = store::load_json::<Cached>(&path).filter(|c| fresh(c, now)) {
        return Ok(c.rating);
    }
    let rating = match sr_steam::http_dl::get_text_or_404(&url(app), MAX_BYTES) {
        Ok(Some(text)) => parse_summary(&text),
        // ProtonDB answers 404 for games nobody reported on.
        Ok(None) => None,
        Err(e) => {
            // An old cached answer is better than nothing when the site is down.
            return match store::load_json::<Cached>(&path) {
                Some(c) => Ok(c.rating),
                None => Err(format!("ProtonDB: {e:#}")),
            };
        }
    };
    let _ = store::save_json(&path, &Cached { fetched: now, rating: rating.clone() });
    Ok(rating)
}

impl Client {
    /// Fetch the rating of a game (no request for games with a native Linux build, or when already known).
    pub(crate) fn protondb_load(&mut self, app: AppId, native_linux: bool) {
        if native_linux || self.state.non_steam.get(app).is_some() || app.0 >= 0x8000_0000 {
            return;
        }
        if matches!(self.state.protondb.get(&app.0), Some(Load::Loading | Load::Ready(_))) {
            return;
        }
        self.state.protondb.insert(app.0, Load::Loading);
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let result = load(app.0, super::downloads::now_unix());
            let _ = tx.blocking_send(Event::ProtonDb { app, result });
            wake();
        });
    }

    pub(crate) fn apply_protondb(&mut self, app: AppId, result: Result<Option<ProtonDbRating>, String>) {
        let v = match result {
            Ok(r) => Load::Ready(r),
            Err(e) => Load::Failed(e),
        };
        // Bounded: a library has a few thousand games at most, but never keep more than this.
        if self.state.protondb.len() > 8192 {
            self.state.protondb.clear();
        }
        self.state.protondb.insert(app.0, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_summary() {
        let json = r#"{"bestReportedTier":"platinum","confidence":"strong","score":0.72,"tier":"gold",
                       "total":1523,"trendingTier":"platinum"}"#;
        let r = parse_summary(json).unwrap();
        assert_eq!((r.tier, r.confidence.as_str(), r.total), (Tier::Gold, "strong", 1523));
        assert_eq!(r.summary(), "Gold, 1523 reports");
        assert_eq!(ProtonDbRating { total: 1, ..r.clone() }.summary(), "Gold, 1 report");
    }

    #[test]
    fn bad_answers_give_no_rating() {
        assert!(parse_summary("not json").is_none());
        assert!(parse_summary(r#"{"tier":"shiny","total":3}"#).is_none());
        assert!(parse_summary("{}").is_none());
        assert_eq!(Tier::parse(" Borked "), Some(Tier::Borked));
    }

    #[test]
    fn cache_is_fresh_for_a_day() {
        let c = Cached { fetched: 1000, rating: None };
        assert!(fresh(&c, 1000 + CACHE_SECS - 1));
        assert!(!fresh(&c, 1000 + CACHE_SECS));
        assert!(!fresh(&c, 999), "a clock that went back");
    }

    #[test]
    fn the_request_names_only_the_app() {
        assert_eq!(url(620), "https://www.protondb.com/api/v1/reports/summaries/620.json");
        assert_eq!(page_url(620), "https://www.protondb.com/app/620");
        let c = Cached {
            fetched: 5,
            rating: Some(ProtonDbRating { tier: Tier::Silver, confidence: "good".into(), total: 9 }),
        };
        let back: Cached = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
