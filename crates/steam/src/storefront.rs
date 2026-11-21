//! Public store data: featured sections, app details, search and store images.
//!
//! Only public endpoints on store.steampowered.com are used, with no cookies
//! and no tokens; the only things sent are the country (`cc=`, the account's
//! country from the login, or none so Steam picks it from the IP address) and
//! the language (`l=`). JSON responses are cached briefly in our cache dir so
//! reopening the Store tab is instant and we don't hammer Valve's servers.
//! Blocking: call from a worker thread.

use crate::store;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

const BASE: &str = "https://store.steampowered.com/api";
const SECTIONS_TTL: Duration = Duration::from_secs(10 * 60);
const DETAILS_TTL: Duration = Duration::from_secs(30 * 60);
/// A store JSON answer is at most a few hundred KB; refuse anything crazy.
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
static LOCALE: RwLock<(String, String)> = RwLock::new((String::new(), String::new()));

/// Country (two letters, "" = let Steam use the IP address) and language ("" = English) for store requests.
pub fn set_locale(country: &str, language: &str) {
    let c: String = country.chars().filter(char::is_ascii_alphabetic).take(2).collect::<String>().to_lowercase();
    let l: String = language.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect();
    *LOCALE.write().unwrap_or_else(|e| e.into_inner()) = (c, l.to_lowercase());
}

/// Set only the country (from the login); the language stays.
pub fn set_country(country: &str) {
    let lang = LOCALE.read().unwrap_or_else(|e| e.into_inner()).1.clone();
    set_locale(country, &lang);
}

/// Set only the language; the country stays.
pub fn set_language(language: &str) {
    let country = LOCALE.read().unwrap_or_else(|e| e.into_inner()).0.clone();
    set_locale(&country, language);
}

/// `(country, language)` as sent: the country may be empty, the language never is.
pub fn locale() -> (String, String) {
    let g = LOCALE.read().unwrap_or_else(|e| e.into_inner());
    (g.0.clone(), if g.1.is_empty() { "english".into() } else { g.1.clone() })
}

/// `cc=xx&l=lang` for a URL (no `cc` when the country is unknown).
pub(crate) fn locale_query() -> String {
    let (c, l) = locale();
    if c.is_empty() { format!("l={l}") } else { format!("cc={c}&l={l}") }
}

/// Suffix that keeps cached answers of different regions apart.
pub(crate) fn locale_tag() -> String {
    let (c, l) = locale();
    format!("{c}-{l}")
}

/// Image hosts we accept. Image URLs come from API answers, so they are checked.
const IMAGE_HOSTS: [&str; 3] = ["steamstatic.com", "steampowered.com", "akamaihd.net"];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Platforms {
    pub windows: bool,
    pub mac: bool,
    pub linux: bool,
}

/// Price in minor units (cents), as the store reports it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Price {
    pub currency: String,
    pub initial: u32,
    pub fin: u32,
    pub discount_percent: u32,
    /// Ready-made text ("$9.99") when the store gave one.
    pub formatted: String,
    /// Ready-made text of the price before the discount ("" when not known).
    #[serde(default)]
    pub original_formatted: String,
}

impl Price {
    pub fn is_free(&self) -> bool {
        self.fin == 0
    }

    /// "$9.99", "Free" or "12.50 EUR".
    pub fn label(&self) -> String {
        if !self.formatted.is_empty() {
            return self.formatted.clone();
        }
        money(self.fin, &self.currency)
    }

    pub fn original_label(&self) -> String {
        if !self.original_formatted.is_empty() {
            return self.original_formatted.clone();
        }
        money(self.initial, &self.currency)
    }
}

fn money(cents: u32, currency: &str) -> String {
    if cents == 0 {
        return "Free".into();
    }
    let v = format!("{}.{:02}", cents / 100, cents % 100);
    match currency {
        "USD" => format!("${v}"),
        "" => v,
        c => format!("{v} {c}"),
    }
}

/// One card in a featured row or a search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreItem {
    pub id: u32,
    pub name: String,
    pub price: Option<Price>,
    /// 460x215 header.
    pub header_image: String,
    /// 616x353 hero-sized capsule (empty for search results).
    pub large_image: String,
    pub platforms: Platforms,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub items: Vec<StoreItem>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Screenshot {
    pub thumb: String,
    pub full: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppDetails {
    pub id: u32,
    pub name: String,
    pub short_description: String,
    pub header_image: String,
    pub screenshots: Vec<Screenshot>,
    pub price: Option<Price>,
    pub is_free: bool,
    pub genres: Vec<String>,
    pub release_date: String,
    pub developers: Vec<String>,
    pub platforms: Platforms,
    /// Trailers, best first (the store's highlighted ones lead).
    pub movies: Vec<Movie>,
}

/// One trailer. `url` is an https mp4 or webm on Steam's video CDN, opened in the system player.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Movie {
    pub name: String,
    pub thumb: String,
    pub url: String,
}

// ---- endpoints -----------------------------------------------------------

/// Featured rows, in display order. The first one ("Top Sellers") also feeds
/// the hero carousel.
pub fn featured_sections() -> Result<Vec<Section>> {
    let v = get_json(
        &format!("{BASE}/featuredcategories?{}", locale_query()),
        &format!("featured-{}.json", locale_tag()),
        SECTIONS_TTL,
    )?;
    parse_sections(&v)
}

/// The rows of a `featuredcategories` answer, in display order.
pub(crate) fn parse_sections(v: &Value) -> Result<Vec<Section>> {
    let wanted = [
        ("top_sellers", "Top Sellers"),
        ("specials", "Specials"),
        ("new_releases", "New & Trending"),
        ("coming_soon", "Coming Soon"),
    ];
    let sections: Vec<Section> = wanted
        .iter()
        .filter_map(|(key, title)| {
            let mut items: Vec<StoreItem> =
                v.get(key)?.get("items")?.as_array()?.iter().filter_map(featured_item).collect();
            // The store lists some apps twice (one entry per edition); show each once.
            let mut seen = std::collections::HashSet::new();
            items.retain(|i| seen.insert(i.id));
            (!items.is_empty()).then(|| Section { id: (*key).into(), title: (*title).into(), items })
        })
        .collect();
    if sections.is_empty() {
        bail!("the store returned no sections");
    }
    Ok(sections)
}

pub fn search(term: &str) -> Result<Vec<StoreItem>> {
    let term = term.trim();
    if term.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!("{BASE}/storesearch/?term={}&{}", urlencode(term), locale_query());
    // Searches are not cached: they are typed live and rarely repeat.
    let v = fetch_json(&url)?;
    let items = v.get("items").and_then(Value::as_array).context("no items")?;
    Ok(items.iter().filter(|i| i["type"] == "app").filter_map(search_item).collect())
}

pub fn app_details(id: u32) -> Result<Option<AppDetails>> {
    let url = format!("{BASE}/appdetails?appids={id}&{}", locale_query());
    let v = get_json(&url, &format!("app-{id}-{}.json", locale_tag()), DETAILS_TTL)?;
    let entry = &v[id.to_string()];
    if entry["success"] != true {
        return Ok(None);
    }
    Ok(Some(parse_details(id, &entry["data"])))
}

/// Raw bytes of a store image. Cached on disk by URL hash.
pub fn fetch_image(url: &str) -> Result<Vec<u8>> {
    check_image_url(url)?;
    let path = cache_path(&format!("img/{:016x}", fnv1a(url)));
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(bytes);
    }
    let resp = crate::http::agent().get(url).call().context("image request failed")?;
    let bytes = resp.into_body().with_config().limit(MAX_IMAGE_BYTES).read_to_vec()?;
    let _ = store::write_private(&path, &bytes);
    Ok(bytes)
}

fn check_image_url(url: &str) -> Result<()> {
    let Some(rest) = url.strip_prefix("https://") else { bail!("image URL is not https") };
    let host = rest.split('/').next().unwrap_or_default();
    if IMAGE_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
        Ok(())
    } else {
        bail!("image host {host} is not allowed")
    }
}

// ---- fetching and caching --------------------------------------------------

fn cache_path(name: &str) -> PathBuf {
    store::cache_dir().join("store").join(name)
}

/// Cached JSON if it is younger than `ttl`, otherwise a fresh request.
pub(crate) fn get_json(url: &str, cache_name: &str, ttl: Duration) -> Result<Value> {
    let path = cache_path(cache_name);
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < ttl);
    if fresh
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(v) = serde_json::from_slice(&bytes)
    {
        return Ok(v);
    }
    let bytes = fetch_bytes(url)?;
    let v: Value = serde_json::from_slice(&bytes).context("the store sent invalid JSON")?;
    let _ = store::write_private(&path, &bytes);
    Ok(v)
}

pub(crate) fn fetch_json(url: &str) -> Result<Value> {
    serde_json::from_slice(&fetch_bytes(url)?).context("the store sent invalid JSON")
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = crate::http::agent().get(url).call().context("store request failed")?;
    Ok(resp.into_body().with_config().limit(MAX_JSON_BYTES).read_to_vec()?)
}

// ---- parsing ---------------------------------------------------------------

pub(crate) fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

pub(crate) fn num(v: &Value, key: &str) -> u32 {
    v[key].as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32
}

fn featured_item(v: &Value) -> Option<StoreItem> {
    let price = Price {
        currency: text(v, "currency"),
        initial: num(v, "original_price"),
        fin: num(v, "final_price"),
        discount_percent: num(v, "discount_percent"),
        formatted: String::new(),
        original_formatted: String::new(),
    };
    Some(StoreItem {
        id: u32::try_from(v["id"].as_u64()?).ok()?,
        name: clean_text(v["name"].as_str()?),
        price: Some(price),
        header_image: text(v, "header_image"),
        large_image: text(v, "large_capsule_image"),
        platforms: Platforms {
            windows: v["windows_available"] == true,
            mac: v["mac_available"] == true,
            linux: v["linux_available"] == true,
        },
    })
}

fn search_item(v: &Value) -> Option<StoreItem> {
    let id = u32::try_from(v["id"].as_u64()?).ok()?;
    let p = &v["price"];
    let price = p.is_object().then(|| Price {
        currency: text(p, "currency"),
        initial: num(p, "initial"),
        fin: num(p, "final"),
        discount_percent: match num(p, "initial") {
            0 => 0,
            i => 100 * i.saturating_sub(num(p, "final")) / i,
        },
        formatted: String::new(),
        original_formatted: String::new(),
    });
    let pf = &v["platforms"];
    Some(StoreItem {
        id,
        name: clean_text(v["name"].as_str()?),
        price,
        header_image: text(v, "tiny_image"),
        large_image: String::new(),
        platforms: Platforms { windows: pf["windows"] == true, mac: pf["mac"] == true, linux: pf["linux"] == true },
    })
}

fn parse_details(id: u32, d: &Value) -> AppDetails {
    let strings = |key: &str, field: &str| -> Vec<String> {
        d[key]
            .as_array()
            .map(|a| a.iter().filter_map(|g| g[field].as_str()).map(clean_text).collect())
            .unwrap_or_default()
    };
    let devs = d["developers"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(clean_text).collect());
    let price = d["price_overview"].is_object().then(|| {
        let p = &d["price_overview"];
        Price {
            currency: text(p, "currency"),
            initial: num(p, "initial"),
            fin: num(p, "final"),
            discount_percent: num(p, "discount_percent"),
            formatted: text(p, "final_formatted"),
            original_formatted: text(p, "initial_formatted"),
        }
    });
    let pf = &d["platforms"];
    AppDetails {
        id,
        name: clean_text(&text(d, "name")),
        short_description: clean_text(&text(d, "short_description")),
        header_image: text(d, "header_image"),
        screenshots: d["screenshots"]
            .as_array()
            .map(|a| {
                a.iter().map(|s| Screenshot { thumb: text(s, "path_thumbnail"), full: text(s, "path_full") }).collect()
            })
            .unwrap_or_default(),
        price,
        is_free: d["is_free"] == true,
        genres: strings("genres", "description"),
        release_date: clean_text(d["release_date"]["date"].as_str().unwrap_or_default()),
        developers: devs.unwrap_or_default(),
        platforms: Platforms { windows: pf["windows"] == true, mac: pf["mac"] == true, linux: pf["linux"] == true },
        movies: parse_movies(&d["movies"]),
    }
}

/// Trailers from the `movies` array of appdetails. Plain `http` links are upgraded to `https`; entries without
/// an mp4 or webm are dropped.
pub(crate) fn parse_movies(v: &Value) -> Vec<Movie> {
    let mut out: Vec<(bool, Movie)> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let pick =
                        |k: &str| ["max", "480"].iter().find_map(|q| m[k][*q].as_str().filter(|s| !s.is_empty()));
                    let url = pick("mp4").or_else(|| pick("webm"))?;
                    let url = url.replacen("http://", "https://", 1);
                    let url = url.split('?').next().unwrap_or_default().to_string();
                    let thumb = text(m, "thumbnail").replacen("http://", "https://", 1);
                    let movie = Movie { name: clean_text(&text(m, "name")), thumb, url };
                    Some((m["highlight"] == true, movie))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(h, _)| !*h);
    out.into_iter().map(|(_, m)| m).collect()
}

/// Store texts carry HTML tags and entities; show plain text.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    out.trim().to_string()
}

pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Stable, dependency-free hash for cache file names.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_strips_tags_and_entities() {
        assert_eq!(clean_text("<b>Half</b> &amp; &quot;Life&quot;"), "Half & \"Life\"");
    }

    #[test]
    fn price_labels() {
        let p = Price {
            currency: "USD".into(),
            initial: 5999,
            fin: 2999,
            discount_percent: 50,
            formatted: String::new(),
            original_formatted: String::new(),
        };
        assert_eq!(p.label(), "$29.99");
        assert_eq!(p.original_label(), "$59.99");
        assert_eq!(money(0, "USD"), "Free");
    }

    #[test]
    fn only_steam_image_hosts_are_allowed() {
        assert!(check_image_url("https://shared.akamai.steamstatic.com/a.jpg").is_ok());
        assert!(check_image_url("http://shared.akamai.steamstatic.com/a.jpg").is_err());
        assert!(check_image_url("https://evil.example/steamstatic.com").is_err());
        assert!(check_image_url("https://steamstatic.com.evil.example/a.jpg").is_err());
    }

    #[test]
    fn movies_prefer_highlights_and_https() {
        let v: Value = serde_json::from_str(
            r#"[{"name":"B","thumbnail":"http://x.steamstatic.com/t.jpg","webm":{"480":"http://v.steamstatic.com/b.webm?t=1","max":"http://v.steamstatic.com/b_max.webm?t=1"},"highlight":false},
                {"name":"A","thumbnail":"","mp4":{"480":"http://v.steamstatic.com/a.mp4","max":"http://v.steamstatic.com/a_max.mp4"},"highlight":true},
                {"name":"none","webm":{}}]"#,
        )
        .unwrap();
        let m = parse_movies(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "A");
        assert_eq!(m[0].url, "https://v.steamstatic.com/a_max.mp4");
        assert_eq!(m[1].url, "https://v.steamstatic.com/b_max.webm");
        assert_eq!(m[1].thumb, "https://x.steamstatic.com/t.jpg");
    }

    #[test]
    fn locale_query_follows_settings() {
        set_locale("DE", "German");
        assert_eq!(locale_query(), "cc=de&l=german");
        set_locale("", "");
        assert_eq!(locale_query(), "l=english");
    }

    #[test]
    fn urlencode_escapes() {
        assert_eq!(urlencode("half life"), "half%20life");
    }
}
