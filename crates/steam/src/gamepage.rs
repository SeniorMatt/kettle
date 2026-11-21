//! Data for the lower half of a game's page: the game's news (public Steam news API, no key, no login) and
//! the achievement summary (read-only from the official client's `appcache/stats` files).
//! Blocking: call from a worker thread.

use crate::vdf::Node;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// News is fetched again when the cached answer is older than this.
pub const NEWS_TTL: Duration = Duration::from_secs(60 * 60);
/// News items kept per game.
pub const NEWS_COUNT: usize = 5;
const MAX_JSON_BYTES: u64 = 1024 * 1024;
/// Largest stats file we read.
const MAX_STATS_BYTES: u64 = 8 * 1024 * 1024;
/// Most achievements listed for one game.
const MAX_ACHIEVEMENTS: usize = 1000;

// ---- news ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct NewsItem {
    /// Digits only (checked); the page address is built from it.
    pub gid: String,
    pub title: String,
    /// Unix seconds.
    pub date: u64,
    /// Plain text, shortened by the server and cleaned of markup.
    pub text: String,
    /// "Community Announcements", "Patch Notes", ...
    pub label: String,
}

impl NewsItem {
    /// The item on Steam's store (always an address the browser allow-list accepts).
    pub fn url(&self, app: u32) -> String {
        format!("https://store.steampowered.com/news/app/{app}/view/{}", self.gid)
    }
}

/// `https://store.steampowered.com/news/app/<id>`: all news of the game.
pub fn news_page(app: u32) -> String {
    format!("https://store.steampowered.com/news/app/{app}")
}

fn news_url(app: u32) -> String {
    format!("https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid={app}&count={NEWS_COUNT}&maxlength=300")
}

/// The game's latest news, from the one-hour cache when it is fresh.
pub fn fetch_news(app: u32) -> Result<Vec<NewsItem>> {
    let path = crate::store::cache_dir().join("news").join(format!("{app}.json"));
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < NEWS_TTL);
    if fresh
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(items) = parse_news(&bytes)
    {
        return Ok(items);
    }
    let resp = crate::http::agent().get(&news_url(app)).call().context("news request failed")?;
    let bytes = resp.into_body().with_config().limit(MAX_JSON_BYTES).read_to_vec()?;
    let items = parse_news(&bytes)?;
    let _ = crate::store::write_private(&path, &bytes);
    Ok(items)
}

/// Parse a `GetNewsForApp` answer. Newest first, at most [`NEWS_COUNT`].
pub fn parse_news(bytes: &[u8]) -> Result<Vec<NewsItem>> {
    let v: Value = serde_json::from_slice(bytes).context("the news answer is not JSON")?;
    let list = v["appnews"]["newsitems"].as_array().context("the news answer has no items")?;
    let mut out: Vec<NewsItem> = list
        .iter()
        .filter_map(|n| {
            let gid = n["gid"].as_str()?.to_string();
            if gid.is_empty() || !gid.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let title = clean_text(n["title"].as_str()?);
            if title.is_empty() {
                return None;
            }
            Some(NewsItem {
                gid,
                title,
                date: n["date"].as_u64().unwrap_or(0),
                text: clean_text(n["contents"].as_str().unwrap_or_default()),
                label: n["feedlabel"].as_str().unwrap_or_default().trim().to_string(),
            })
        })
        .collect();
    out.sort_by_key(|n| std::cmp::Reverse(n.date));
    out.truncate(NEWS_COUNT);
    Ok(out)
}

/// News text arrives with BBCode, HTML and Steam's `{STEAM_CLAN_IMAGE}` placeholders; show plain words only.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth_square = 0u32;
    let mut in_angle = false;
    for c in s.chars() {
        match c {
            '[' if !in_angle => depth_square += 1,
            ']' if depth_square > 0 => {
                depth_square -= 1;
                out.push(' ');
            }
            '<' if depth_square == 0 => in_angle = true,
            '>' if in_angle => {
                in_angle = false;
                out.push(' ');
            }
            _ if depth_square > 0 || in_angle => {}
            c => out.push(c),
        }
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ");
    out.split_whitespace()
        .filter(|w| !w.starts_with("{STEAM_CLAN_IMAGE}") && !w.starts_with("http://") && !w.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- binary KeyValues --------------------------------------------------------

/// Read Steam's binary KeyValues format into the same tree the text parser makes (numbers become decimal
/// strings). `None` when the data is cut off or malformed. Read-only: nothing here writes the format.
pub fn parse_binary_kv(data: &[u8]) -> Option<Node> {
    let mut r = Reader { d: data, i: 0 };
    let kids = r.object(0)?;
    Some(Node::Obj(kids))
}

struct Reader<'a> {
    d: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.d.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }

    fn cstr(&mut self) -> Option<String> {
        let rest = self.d.get(self.i..)?;
        let n = rest.iter().position(|b| *b == 0)?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.i += n + 1;
        Some(s)
    }

    fn wstr(&mut self) -> Option<String> {
        let mut units = Vec::new();
        loop {
            let b = self.take(2)?;
            let u = u16::from_le_bytes([b[0], b[1]]);
            if u == 0 {
                break;
            }
            units.push(u);
        }
        Some(String::from_utf16_lossy(&units))
    }

    fn object(&mut self, depth: u32) -> Option<Vec<(String, Node)>> {
        if depth > 16 {
            return None;
        }
        let mut kids = Vec::new();
        loop {
            // The end of the data closes the top level, as Steam's own reader does.
            let Some(&t) = self.d.get(self.i) else {
                return (depth == 0).then_some(kids);
            };
            self.i += 1;
            if t == 8 {
                return Some(kids);
            }
            let key = self.cstr()?;
            let val = match t {
                0 => Node::Obj(self.object(depth + 1)?),
                1 => Node::Str(self.cstr()?),
                2 | 4 | 6 => {
                    let b = self.take(4)?;
                    Node::Str(i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string())
                }
                3 => {
                    let b = self.take(4)?;
                    Node::Str(f32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string())
                }
                5 => Node::Str(self.wstr()?),
                7 => {
                    let b = self.take(8)?;
                    Node::Str(u64::from_le_bytes(b.try_into().ok()?).to_string())
                }
                10 => {
                    let b = self.take(8)?;
                    Node::Str(i64::from_le_bytes(b.try_into().ok()?).to_string())
                }
                _ => return None,
            };
            kids.push((key, val));
            if kids.len() > 50_000 {
                return None;
            }
        }
    }
}

// ---- achievements ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Achievement {
    pub name: String,
    pub display: String,
    pub desc: String,
    /// Icon file name on Steam's community CDN (empty when the schema has none).
    pub icon: String,
    pub unlocked: bool,
    /// Unix seconds, 0 when unknown.
    pub time: u64,
}

impl Achievement {
    /// Where the icon lives (a public image on Steam's CDN; the app id and the icon name are all that is sent).
    pub fn icon_url(&self, app: u32) -> Option<String> {
        let ok =
            !self.icon.is_empty() && self.icon.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
        ok.then(|| {
            let hash = self.icon.trim_end_matches(".jpg");
            format!("https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/{app}/{hash}.jpg")
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Achievements {
    pub total: u32,
    pub unlocked: u32,
    /// Unlocked ones first (newest first), then the locked ones in schema order.
    pub list: Vec<Achievement>,
}

impl Achievements {
    pub fn percent(&self) -> u32 {
        if self.total == 0 { 0 } else { (u64::from(self.unlocked) * 100 / u64::from(self.total)) as u32 }
    }
}

/// The text of a schema string that may be a plain value or a per-language object.
fn localized(n: Option<&Node>) -> String {
    match n {
        Some(Node::Str(s)) => s.clone(),
        Some(o @ Node::Obj(_)) => {
            o.get_str("english").or_else(|| o.children().first().and_then(|(_, v)| v.str())).unwrap_or("").to_string()
        }
        None => String::new(),
    }
}

/// Combine the schema (`UserGameStatsSchema_<app>.bin`) and the player's stats (`UserGameStats_<account>_<app>.bin`)
/// into the summary. `extra_unlocked` adds achievement names unlocked elsewhere (Kettle's own stats file).
/// `None` when the schema has no achievements.
pub fn build_achievements(
    schema: &Node,
    stats: Option<&Node>,
    extra_unlocked: &[(String, u64)],
) -> Option<Achievements> {
    let root = schema.children().first().map(|(_, v)| v)?;
    let cache = stats.and_then(|s| s.get("cache").or_else(|| s.children().first().and_then(|(_, v)| v.get("cache"))));
    let mut list = Vec::new();
    for (stat_id, stat) in root.get("stats")?.children() {
        let Some(bits) = stat.get("bits") else { continue };
        let mask = cache
            .and_then(|c| c.get(stat_id))
            .and_then(|s| s.get_str("data"))
            .and_then(|d| d.trim().parse::<i64>().ok())
            .unwrap_or(0) as u32;
        for (bit, ach) in bits.children() {
            let Some(name) = ach.get_str("name") else { continue };
            let Ok(index) = bit.parse::<u32>() else { continue };
            let display = localized(ach.path(&["display", "name"]));
            let mut time = cache
                .and_then(|c| c.get(stat_id))
                .and_then(|s| s.path(&["AchievementTimes", bit]))
                .and_then(Node::str)
                .and_then(|t| t.parse::<u64>().ok())
                .unwrap_or(0);
            let mut unlocked = index < 32 && mask & (1 << index) != 0;
            if let Some((_, t)) = extra_unlocked.iter().find(|(n, _)| n == name) {
                unlocked = true;
                time = time.max(*t);
            }
            list.push(Achievement {
                name: name.to_string(),
                display: if display.is_empty() { name.to_string() } else { display },
                desc: localized(ach.path(&["display", "desc"])),
                icon: ach.path(&["display"]).and_then(|d| d.get_str("icon")).unwrap_or_default().to_string(),
                unlocked,
                time,
            });
            if list.len() >= MAX_ACHIEVEMENTS {
                break;
            }
        }
    }
    if list.is_empty() {
        return None;
    }
    let unlocked = list.iter().filter(|a| a.unlocked).count() as u32;
    let total = list.len() as u32;
    // Stable: locked ones keep the schema order.
    list.sort_by(|a, b| {
        b.unlocked.cmp(&a.unlocked).then(if a.unlocked { b.time.cmp(&a.time) } else { std::cmp::Ordering::Equal })
    });
    Some(Achievements { total, unlocked, list })
}

fn read_limited(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_STATS_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

/// Read the official client's stats files for `app` and `account` (the 32-bit account number) under one Steam
/// root. Never writes. `None` when the schema is missing (the game has no achievements, or was never run).
pub fn read_local(root: &Path, app: u32, account: u32, extra_unlocked: &[(String, u64)]) -> Option<Achievements> {
    let dir: PathBuf = root.join("appcache/stats");
    let schema = parse_binary_kv(&read_limited(&dir.join(format!("UserGameStatsSchema_{app}.bin")))?)?;
    let stats = read_limited(&dir.join(format!("UserGameStats_{account}_{app}.bin"))).and_then(|b| parse_binary_kv(&b));
    build_achievements(&schema, stats.as_ref(), extra_unlocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny writer for the binary format, so fixtures are written by hand.
    fn obj(key: &str, kids: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0u8];
        v.extend(key.bytes());
        v.push(0);
        for k in kids {
            v.extend(k);
        }
        v.push(8);
        v
    }
    fn s(key: &str, val: &str) -> Vec<u8> {
        let mut v = vec![1u8];
        v.extend(key.bytes());
        v.push(0);
        v.extend(val.bytes());
        v.push(0);
        v
    }
    fn i(key: &str, val: i32) -> Vec<u8> {
        let mut v = vec![2u8];
        v.extend(key.bytes());
        v.push(0);
        v.extend(val.to_le_bytes());
        v
    }
    fn ach(bit: &str, name: &str, title: &str, icon: &str) -> Vec<u8> {
        obj(bit, &[s("name", name), obj("display", &[obj("name", &[s("english", title)]), s("icon", icon)])])
    }
    fn schema() -> Vec<u8> {
        let stats = obj(
            "stats",
            &[obj(
                "1",
                &[
                    i("type", 4),
                    obj(
                        "bits",
                        &[ach("0", "A", "First", "aa"), ach("1", "B", "Second", "bb"), ach("2", "C", "Third", "cc")],
                    ),
                ],
            )],
        );
        let mut v = obj("440", &[s("gameName", "Test"), stats]);
        // The file ends right after the root object.
        v.truncate(v.len());
        v
    }
    fn stats(mask: i32) -> Vec<u8> {
        obj("cache", &[i("crc", 5), obj("1", &[i("data", mask)])])
    }

    #[test]
    fn binary_kv_reads_all_value_types() {
        let mut d = Vec::new();
        d.extend(s("a", "x"));
        d.extend(i("b", -3));
        d.extend([7u8, b'c', 0]);
        d.extend(9_000_000_000u64.to_le_bytes());
        d.extend([3u8, b'd', 0]);
        d.extend(1.5f32.to_le_bytes());
        d.extend([5u8, b'e', 0, b'h', 0, b'i', 0, 0, 0]);
        d.extend(obj("n", &[s("k", "v")]));
        let n = parse_binary_kv(&d).unwrap();
        assert_eq!(n.get_str("a"), Some("x"));
        assert_eq!(n.get_str("b"), Some("-3"));
        assert_eq!(n.get_str("c"), Some("9000000000"));
        assert_eq!(n.get_str("d"), Some("1.5"));
        assert_eq!(n.get_str("e"), Some("hi"));
        assert_eq!(n.path(&["n", "k"]).and_then(Node::str), Some("v"));
    }

    #[test]
    fn binary_kv_rejects_cut_off_data() {
        let d = schema();
        for cut in [1, 5, d.len() / 2, d.len() - 2] {
            assert!(parse_binary_kv(&d[..cut]).is_none(), "cut at {cut}");
        }
        assert!(parse_binary_kv(&[9, b'x', 0]).is_none());
    }

    #[test]
    fn achievements_combine_schema_and_bitmask() {
        let schema = parse_binary_kv(&schema()).unwrap();
        let st = parse_binary_kv(&stats(0b101)).unwrap();
        let a = build_achievements(&schema, Some(&st), &[]).unwrap();
        assert_eq!((a.total, a.unlocked, a.percent()), (3, 2, 66));
        assert_eq!(a.list[0].display, "First");
        assert!(a.list[0].unlocked && a.list[1].unlocked && !a.list[2].unlocked);
        assert_eq!(a.list[2].name, "B");
        assert_eq!(
            a.list[0].icon_url(440).unwrap(),
            "https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/440/aa.jpg"
        );
    }

    #[test]
    fn achievements_without_a_stats_file_are_all_locked_and_extra_names_count() {
        let schema = parse_binary_kv(&schema()).unwrap();
        let a = build_achievements(&schema, None, &[]).unwrap();
        assert_eq!((a.total, a.unlocked), (3, 0));
        let b = build_achievements(&schema, None, &[("C".into(), 1_700_000_000)]).unwrap();
        assert_eq!(b.unlocked, 1);
        assert_eq!(b.list[0].name, "C");
        assert_eq!(b.list[0].time, 1_700_000_000);
    }

    #[test]
    fn a_schema_without_achievements_gives_none() {
        let d = obj("1", &[s("gameName", "x"), obj("stats", &[obj("1", &[i("type", 1)])])]);
        assert!(build_achievements(&parse_binary_kv(&d).unwrap(), None, &[]).is_none());
    }

    #[test]
    fn icon_names_with_odd_characters_are_refused() {
        let mut a = Achievement {
            name: "x".into(),
            display: "x".into(),
            desc: String::new(),
            icon: "a/../b".into(),
            unlocked: true,
            time: 0,
        };
        assert!(a.icon_url(1).is_none());
        a.icon = String::new();
        assert!(a.icon_url(1).is_none());
    }

    const NEWS: &str = r#"{"appnews":{"appid":440,"newsitems":[
        {"gid":"100","title":"Old [b]post[/b]","url":"https://evil.example/x","date":100,"contents":"Hello <b>world</b> {STEAM_CLAN_IMAGE}/1/a.png [url=https://x.y]link[/url] &amp; more","feedlabel":"Community Announcements"},
        {"gid":"200","title":"New one","date":300,"contents":"Text","feedlabel":"Patch Notes"},
        {"gid":"../x","title":"Bad id","date":400,"contents":""},
        {"gid":"300","title":"","date":500,"contents":""}],"count":4}}"#;

    #[test]
    fn news_is_parsed_cleaned_and_sorted() {
        let n = parse_news(NEWS.as_bytes()).unwrap();
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].title, "New one");
        assert_eq!(n[0].label, "Patch Notes");
        assert_eq!(n[1].title, "Old post");
        assert_eq!(n[1].text, "Hello world link & more");
        assert_eq!(n[0].url(440), "https://store.steampowered.com/news/app/440/view/200");
    }

    #[test]
    fn news_errors_are_errors() {
        assert!(parse_news(b"not json").is_err());
        assert!(parse_news(br#"{"appnews":{}}"#).is_err());
        assert!(parse_news(br#"{"appnews":{"newsitems":[]}}"#).unwrap().is_empty());
    }
}
