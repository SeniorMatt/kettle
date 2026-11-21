//! More public store data: reviews, a user's wishlist, browsing by tag, and the tag list.
//!
//! Like `storefront`, only public endpoints are used, with no cookies and no tokens. What leaves the machine:
//! an app id, a tag id, a page offset, the country (when known) and the language. The wishlist is read through
//! the public `IWishlistService/GetWishlist` call with the account's own public SteamID (no login is sent), so it
//! only works while "Game details" in the Steam privacy settings is public. Adding or removing wishlist entries
//! needs the store session cookies, which Kettle does not hold, so that is left to the store page in the browser.
//! Blocking: call from a worker thread.

use crate::storefront::{
    self, Platforms, Price, StoreItem, clean_text, fetch_json, get_json, locale, num, text, urlencode,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const API: &str = "https://api.steampowered.com";
const STORE: &str = "https://store.steampowered.com";
const TAGS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Cards per browse page.
pub const PAGE_SIZE: usize = 48;
/// Wishlist entries shown at most (each one costs a little of a GetItems answer).
const WISHLIST_MAX: usize = 300;
const REVIEW_MAX_CHARS: usize = 900;

// ---- reviews ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// "Very Positive", "Mixed", ... (empty when the app has too few reviews).
    pub label: String,
    pub positive: u32,
    pub negative: u32,
    pub total: u32,
}

impl ReviewSummary {
    /// "Very Positive (12,345)"; "No user reviews" when there are none.
    pub fn text(&self) -> String {
        if self.total == 0 {
            return "No user reviews".into();
        }
        let label = if self.label.is_empty() { "Reviews" } else { &self.label };
        format!("{label} ({})", thousands(self.total))
    }

    /// Share of positive reviews, 0 to 100.
    pub fn percent_positive(&self) -> u32 {
        let t = self.positive + self.negative;
        if t == 0 { 0 } else { (100 * u64::from(self.positive) / u64::from(t)) as u32 }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Review {
    pub recommended: bool,
    pub text: String,
    pub votes_up: u32,
    /// Hours the author played in total.
    pub hours: f32,
    /// Unix seconds.
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Reviews {
    pub summary: ReviewSummary,
    /// The most helpful ones first.
    pub items: Vec<Review>,
}

/// The summary and the 10 most helpful reviews of an app, in the interface language.
pub fn reviews(app: u32) -> Result<Reviews> {
    let (_, lang) = locale();
    let url = format!(
        "{STORE}/appreviews/{app}?json=1&language={}&filter=all&num_per_page=10&purchase_type=all",
        urlencode(&lang)
    );
    parse_reviews(&fetch_json(&url)?)
}

pub(crate) fn parse_reviews(v: &Value) -> Result<Reviews> {
    if v["success"].as_i64().unwrap_or(0) != 1 {
        bail!("the store has no reviews for this app");
    }
    let s = &v["query_summary"];
    let summary = ReviewSummary {
        label: text(s, "review_score_desc"),
        positive: num(s, "total_positive"),
        negative: num(s, "total_negative"),
        total: num(s, "total_reviews"),
    };
    let items = v["reviews"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let body = strip_markup(r["review"].as_str()?);
                    if body.is_empty() {
                        return None;
                    }
                    Some(Review {
                        recommended: r["voted_up"] == true,
                        text: body,
                        votes_up: num(r, "votes_up"),
                        hours: r["author"]["playtime_forever"].as_f64().unwrap_or(0.0) as f32 / 60.0,
                        created: r["timestamp_created"].as_u64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Reviews { summary, items })
}

/// Review text carries `[h1]`-style markup and line breaks; show it as plain text, cut to a sane length.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(REVIEW_MAX_CHARS + 8));
    let mut in_tag = false;
    let mut last_space = true;
    for c in s.chars() {
        match c {
            '[' => in_tag = true,
            ']' if in_tag => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                }
                last_space = true;
            }
            c => {
                out.push(c);
                last_space = false;
            }
        }
        if out.chars().count() >= REVIEW_MAX_CHARS {
            out.push_str("...");
            break;
        }
    }
    clean_text(&out)
}

/// 12345 -> "12,345".
pub fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---- store items (prices for a list of app ids) ------------------------------------------------------------

fn header_url(app: u32) -> String {
    format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{app}/header.jpg")
}

/// `IStoreBrowseService/GetItems` request URL: names, platforms and the best price for these apps.
pub(crate) fn items_url(apps: &[u32]) -> String {
    let (cc, lang) = locale();
    let country = if cc.is_empty() { "US".to_string() } else { cc.to_uppercase() };
    let json = serde_json::json!({
        "ids": apps.iter().map(|a| serde_json::json!({ "appid": a })).collect::<Vec<_>>(),
        "context": { "language": lang, "country_code": country },
        "data_request": { "include_basic_info": true, "include_platforms": true },
    });
    format!("{API}/IStoreBrowseService/GetItems/v1?input_json={}", urlencode(&json.to_string()))
}

fn cents(v: &Value, key: &str) -> u32 {
    match &v[key] {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32,
        _ => 0,
    }
}

/// Cards for a `GetItems` answer, in the order of `order`. Apps the store does not know are left out.
pub(crate) fn parse_store_items(v: &Value, order: &[u32]) -> Vec<StoreItem> {
    let mut by_id: HashMap<u32, StoreItem> = HashMap::new();
    for it in v["response"]["store_items"].as_array().into_iter().flatten() {
        let Some(id) = it["appid"].as_u64().or_else(|| it["id"].as_u64()).and_then(|i| u32::try_from(i).ok()) else {
            continue;
        };
        if it["success"].as_i64().unwrap_or(1) != 1 || it["name"].as_str().is_none() {
            continue;
        }
        let b = &it["best_purchase_option"];
        let price = if b.is_object() {
            let fin = cents(b, "final_price_in_cents");
            let initial = match cents(b, "original_price_in_cents") {
                0 => fin,
                o => o,
            };
            Some(Price {
                currency: String::new(),
                initial,
                fin,
                discount_percent: num(b, "discount_pct"),
                formatted: text(b, "formatted_final_price"),
                original_formatted: text(b, "formatted_original_price"),
            })
        } else if it["is_free"] == true {
            Some(Price { formatted: "Free".into(), ..Price::default() })
        } else {
            None
        };
        let pf = &it["platforms"];
        by_id.insert(
            id,
            StoreItem {
                id,
                name: clean_text(it["name"].as_str().unwrap_or_default()),
                price,
                header_image: header_url(id),
                large_image: String::new(),
                platforms: Platforms {
                    windows: pf["windows"] == true,
                    mac: pf["mac"] == true,
                    linux: pf["steamos_linux"] == true || pf["linux"] == true,
                },
            },
        );
    }
    order.iter().filter_map(|id| by_id.remove(id)).collect()
}

/// Names, prices and platforms for apps, 50 per request.
fn hydrate(apps: &[u32]) -> Result<Vec<StoreItem>> {
    let mut out = Vec::with_capacity(apps.len());
    for chunk in apps.chunks(50) {
        out.extend(parse_store_items(&fetch_json(&items_url(chunk))?, chunk));
    }
    Ok(out)
}

// ---- wishlist --------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WishItem {
    pub item: StoreItem,
    /// The user's own ranking (1 = top); 0 when never ranked.
    pub priority: u32,
    /// Unix seconds.
    pub added: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Wishlist {
    pub items: Vec<WishItem>,
    /// Entries on the list; more than `items.len()` when the list was cut.
    pub total: usize,
}

/// `(appid, priority, date_added)` from `IWishlistService/GetWishlist`, ranked first (priority 1 on top, unranked
/// at the end by newest).
pub(crate) fn parse_wishlist(v: &Value) -> Vec<(u32, u32, u64)> {
    let mut list: Vec<(u32, u32, u64)> = v["response"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            Some((u32::try_from(i["appid"].as_u64()?).ok()?, num(i, "priority"), i["date_added"].as_u64().unwrap_or(0)))
        })
        .collect();
    list.sort_by_key(|&(app, prio, added)| (prio == 0, prio, std::cmp::Reverse(added), app));
    list
}

/// The wishlist of a public profile, ranked, with price and discount.
pub fn wishlist(steam_id: u64) -> Result<Wishlist> {
    let url = format!("{API}/IWishlistService/GetWishlist/v1/?steamid={steam_id}");
    let list = parse_wishlist(&fetch_json(&url)?);
    let total = list.len();
    let shown: Vec<_> = list.into_iter().take(WISHLIST_MAX).collect();
    let ids: Vec<u32> = shown.iter().map(|w| w.0).collect();
    let cards = hydrate(&ids)?;
    let meta: HashMap<u32, (u32, u64)> = shown.iter().map(|&(a, p, d)| (a, (p, d))).collect();
    let items = cards
        .into_iter()
        .map(|item| {
            let (priority, added) = meta.get(&item.id).copied().unwrap_or((0, 0));
            WishItem { item, priority, added }
        })
        .collect();
    Ok(Wishlist { items, total })
}

// ---- tags and browsing ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub id: u32,
    pub name: String,
}

pub(crate) fn parse_tags(v: &Value) -> Vec<Tag> {
    let mut tags: Vec<Tag> = v["response"]["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Tag { id: u32::try_from(t["tagid"].as_u64()?).ok()?, name: clean_text(t["name"].as_str()?) })
        })
        .collect();
    tags.sort_by_key(|t| t.name.to_lowercase());
    tags
}

/// Every store tag (genres included), by name.
pub fn tags() -> Result<Vec<Tag>> {
    let (_, lang) = locale();
    let url = format!("{API}/IStoreService/GetTagList/v1/?language={}", urlencode(&lang));
    let v = get_json(&url, &format!("tags-{lang}.json"), TAGS_TTL)?;
    let tags = parse_tags(&v);
    if tags.is_empty() {
        bail!("the store sent no tags");
    }
    Ok(tags)
}

/// Which list the Browse page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum BrowseKind {
    #[default]
    TopSellers,
    NewReleases,
    Specials,
    ComingSoon,
}

impl BrowseKind {
    pub const ALL: [BrowseKind; 4] =
        [BrowseKind::TopSellers, BrowseKind::NewReleases, BrowseKind::Specials, BrowseKind::ComingSoon];

    pub fn label(self) -> &'static str {
        match self {
            BrowseKind::TopSellers => "Top Sellers",
            BrowseKind::NewReleases => "New Releases",
            BrowseKind::Specials => "Specials",
            BrowseKind::ComingSoon => "Coming Soon",
        }
    }

    fn query(self) -> &'static str {
        match self {
            BrowseKind::TopSellers => "filter=topsellers",
            BrowseKind::NewReleases => "filter=newreleases",
            BrowseKind::Specials => "specials=1",
            BrowseKind::ComingSoon => "filter=popularcomingsoon",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrowsePage {
    pub items: Vec<StoreItem>,
    /// True when a further page may exist.
    pub more: bool,
}

pub(crate) fn browse_url(kind: BrowseKind, tag: Option<u32>, start: usize) -> String {
    let tags = tag.map(|t| format!("&tags={t}")).unwrap_or_default();
    format!(
        "{STORE}/search/results/?json=1&category1=998&{}&start={start}&count={PAGE_SIZE}{tags}&{}",
        kind.query(),
        storefront::locale_query()
    )
}

/// `(appid, name)` pairs from a `search/results?json=1` answer, in order. The app id is in the capsule URL.
pub(crate) fn parse_search_results(v: &Value) -> Vec<(u32, String)> {
    v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let logo = i["logo"].as_str()?;
            let rest = logo.split("/apps/").nth(1)?;
            let id: u32 = rest.split('/').next()?.parse().ok()?;
            Some((id, clean_text(i["name"].as_str().unwrap_or_default())))
        })
        .collect()
}

/// One page of a browse list. Falls back to names and images without prices if the price lookup fails.
pub fn browse(kind: BrowseKind, tag: Option<u32>, start: usize) -> Result<BrowsePage> {
    let v = fetch_json(&browse_url(kind, tag, start)).context("browse request failed")?;
    let found = parse_search_results(&v);
    let more = v["items"].as_array().map_or(0, Vec::len) >= PAGE_SIZE / 2;
    let mut seen = std::collections::HashSet::new();
    let found: Vec<(u32, String)> = found.into_iter().filter(|(id, _)| seen.insert(*id)).collect();
    let ids: Vec<u32> = found.iter().map(|f| f.0).collect();
    let items = match hydrate(&ids) {
        Ok(items) if !items.is_empty() => items,
        _ => found
            .into_iter()
            .map(|(id, name)| StoreItem {
                id,
                name,
                price: None,
                header_image: header_url(id),
                large_image: String::new(),
                platforms: Platforms::default(),
            })
            .collect(),
    };
    Ok(BrowsePage { items, more })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn reviews_summary_and_texts() {
        let v = json(
            r#"{"success":1,"query_summary":{"num_reviews":2,"review_score":8,"review_score_desc":"Very Positive","total_positive":10000,"total_negative":2345,"total_reviews":12345},
            "reviews":[
              {"recommendationid":"1","author":{"steamid":"7656","playtime_forever":600},"review":"[h1]Great[/h1]\nLoved   it &amp; more","timestamp_created":1700000000,"voted_up":true,"votes_up":42},
              {"recommendationid":"2","author":{"playtime_forever":30},"review":"   ","voted_up":false,"votes_up":1},
              {"recommendationid":"3","author":{"playtime_forever":90},"review":"Meh","voted_up":false,"votes_up":7}]}"#,
        );
        let r = parse_reviews(&v).unwrap();
        assert_eq!(r.summary.text(), "Very Positive (12,345)");
        assert_eq!(r.summary.percent_positive(), 81);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].text, "Great Loved it & more");
        assert!(r.items[0].recommended);
        assert_eq!(r.items[0].votes_up, 42);
        assert!((r.items[0].hours - 10.0).abs() < 0.01);
        assert!(!r.items[1].recommended);
    }

    #[test]
    fn reviews_failure_and_empty_summary() {
        assert!(parse_reviews(&json(r#"{"success":0}"#)).is_err());
        let r = parse_reviews(&json(r#"{"success":1,"query_summary":{"total_reviews":0},"reviews":[]}"#)).unwrap();
        assert_eq!(r.summary.text(), "No user reviews");
        assert!(r.items.is_empty());
    }

    #[test]
    fn long_reviews_are_cut() {
        let long = "word ".repeat(1000);
        let t = strip_markup(&long);
        assert!(t.chars().count() <= REVIEW_MAX_CHARS + 3);
        assert!(t.ends_with("..."));
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn wishlist_is_sorted_by_priority() {
        let v = json(
            r#"{"response":{"items":[
              {"appid":30,"priority":0,"date_added":100},
              {"appid":20,"priority":2,"date_added":50},
              {"appid":40,"priority":0,"date_added":200},
              {"appid":10,"priority":1,"date_added":10}]}}"#,
        );
        let ids: Vec<u32> = parse_wishlist(&v).iter().map(|w| w.0).collect();
        assert_eq!(ids, vec![10, 20, 40, 30]);
        assert!(parse_wishlist(&json(r#"{"response":{}}"#)).is_empty());
    }

    #[test]
    fn store_items_with_prices_in_given_order() {
        let v = json(
            r#"{"response":{"store_items":[
              {"id":620,"appid":620,"success":1,"name":"Portal 2","platforms":{"windows":true,"mac":true,"steamos_linux":true},
               "best_purchase_option":{"final_price_in_cents":"249","original_price_in_cents":"999","discount_pct":75,"formatted_final_price":"2,49€","formatted_original_price":"9,99€"}},
              {"id":10,"appid":10,"success":1,"name":"Counter-Strike","is_free":true},
              {"id":99,"appid":99,"success":2},
              {"id":5,"appid":5,"success":1,"name":"Unreleased"}]}}"#,
        );
        let items = parse_store_items(&v, &[10, 620, 99, 5, 7]);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Counter-Strike", "Portal 2", "Unreleased"]);
        let p = items[1].price.as_ref().unwrap();
        assert_eq!((p.fin, p.initial, p.discount_percent), (249, 999, 75));
        assert_eq!(p.label(), "2,49€");
        assert_eq!(p.original_label(), "9,99€");
        assert!(items[1].platforms.linux && items[1].platforms.windows);
        assert!(items[0].price.as_ref().unwrap().is_free());
        assert!(items[2].price.is_none());
        assert!(items[1].header_image.starts_with("https://"));
    }

    #[test]
    fn search_results_give_app_ids() {
        let v = json(
            r#"{"desc":"","items":[
              {"name":"Counter-Strike 2","logo":"https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/730/abc/capsule_sm_120.jpg?t=1"},
              {"name":"A Bundle","logo":"https://shared.fastly.steamstatic.com/store_item_assets/steam/bundles/5/x.jpg"},
              {"name":"Old","logo":"https://cdn.akamai.steamstatic.com/steam/apps/220/capsule_sm_120.jpg"}]}"#,
        );
        assert_eq!(parse_search_results(&v), vec![(730, "Counter-Strike 2".to_string()), (220, "Old".to_string())]);
    }

    #[test]
    fn tags_are_sorted_by_name() {
        let v = json(
            r#"{"response":{"tags":[{"tagid":122,"name":"RPG"},{"tagid":19,"name":"Action"},{"tagid":9,"name":"strategy"}]}}"#,
        );
        let t = parse_tags(&v);
        assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), vec![19, 122, 9]);
    }

    #[test]
    fn browse_urls_carry_tag_and_offset() {
        let u = browse_url(BrowseKind::TopSellers, Some(19), 96);
        assert!(u.contains("filter=topsellers") && u.contains("tags=19") && u.contains("start=96"));
        let u = browse_url(BrowseKind::Specials, None, 0);
        assert!(u.contains("specials=1") && !u.contains("tags="));
    }

    #[test]
    fn items_url_is_encoded_json() {
        let u = items_url(&[620, 10]);
        assert!(u.starts_with("https://api.steampowered.com/IStoreBrowseService/GetItems/v1?input_json=%7B"));
        assert!(!u.contains(' ') && !u.contains('"'));
    }

    #[test]
    fn featured_categories_sections() {
        let v = json(
            r#"{"top_sellers":{"items":[
                {"id":1,"name":"One","discount_percent":0,"original_price":1000,"final_price":1000,"currency":"USD","header_image":"h","windows_available":true},
                {"id":1,"name":"One again","final_price":1000},
                {"id":2,"name":"Two","discount_percent":50,"original_price":2000,"final_price":1000,"currency":"USD","linux_available":true}]},
              "specials":{"items":[]},
              "coming_soon":{"items":[{"id":3,"name":"Soon"}]}}"#,
        );
        let s = storefront::parse_sections(&v).unwrap();
        assert_eq!(s.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["top_sellers", "coming_soon"]);
        assert_eq!(s[0].items.len(), 2);
        assert_eq!(s[0].items[1].price.as_ref().unwrap().discount_percent, 50);
        assert!(s[0].items[1].platforms.linux);
        assert!(storefront::parse_sections(&json("{}")).is_err());
    }
}
