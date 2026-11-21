//! Cover art lookup: the official client's local cache first, then our own
//! cache, then Steam's CDN (downloaded into our cache). Blocking; call from a
//! worker thread.

use crate::{local, store};
use anyhow::{Result, bail};
use sr_model::{AppId, ArtKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const CDN: &str = "https://shared.steamstatic.com/store_item_assets/steam/apps";
const ICON_CDN: &str = "https://media.steampowered.com/steamcommunity/public/images/apps";

const STORE_ASSETS: &str = "https://shared.steamstatic.com/store_item_assets/";
const GET_ITEMS: &str = "https://api.steampowered.com/IStoreBrowseService/GetItems/v1";
/// A "Steam has no such image" marker is trusted this long, then we ask again.
const MISSING_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Where one app's store art really lives (newer apps use hashed paths).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreAssets {
    pub capsule: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    pub header: Option<String>,
    /// Hash of the 32x32 community icon (empty if none).
    pub icon_hash: String,
}

impl StoreAssets {
    fn url_for(&self, kind: ArtKind) -> Option<&str> {
        match kind {
            ArtKind::Capsule => self.capsule.as_deref(),
            ArtKind::Hero => self.hero.as_deref(),
            ArtKind::Logo => self.logo.as_deref(),
            ArtKind::Header => self.header.as_deref(),
            ArtKind::Icon => None,
        }
    }
}

/// `https://api.steampowered.com/IStoreBrowseService/GetItems/v1` request URL for some apps.
pub fn get_items_url(apps: &[AppId]) -> String {
    let ids: Vec<String> = apps.iter().map(|a| format!("{{\"appid\":{a}}}")).collect();
    let json = format!(
        "{{\"ids\":[{}],\"context\":{{\"language\":\"english\",\"country_code\":\"US\"}},\"data_request\":{{\"include_assets\":true}}}}",
        ids.join(",")
    );
    // The JSON only has digits, letters and `{}[]":,`; encode the ones a URL can't carry raw.
    let mut enc = String::new();
    for c in json.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => enc.push(c),
            c => enc.push_str(&format!("%{:02X}", c as u32)),
        }
    }
    format!("{GET_ITEMS}?input_json={enc}")
}

/// Full image URL from `asset_url_format` (`steam/apps/N/${FILENAME}?t=..`) and an asset value
/// (`<hash>/library_capsule.jpg` or just `library_600x900.jpg`).
pub fn asset_url(format: &str, value: &str) -> String {
    format!("{STORE_ASSETS}{}", format.replace("${FILENAME}", value))
}

/// Parse a GetItems answer into per-app asset locations. Apps without assets are skipped.
pub fn parse_get_items(json: &str) -> Vec<(AppId, StoreAssets)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let Some(items) = v.pointer("/response/store_items").and_then(|i| i.as_array()) else { return vec![] };
    items
        .iter()
        .filter_map(|it| {
            let app = u32::try_from(it.get("appid")?.as_u64()?).ok()?;
            let a = it.get("assets")?;
            let fmt = a.get("asset_url_format")?.as_str()?;
            let get = |k: &str| a.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty());
            let url = |k: &str| get(k).map(|v| asset_url(fmt, v));
            Some((
                AppId(app),
                StoreAssets {
                    capsule: url("library_capsule"),
                    hero: url("library_hero"),
                    logo: url("library_logo"),
                    header: url("header"),
                    icon_hash: get("community_icon").unwrap_or_default().to_string(),
                },
            ))
        })
        .collect()
}

const ICON_CDN_NEW: &str = "https://shared.steamstatic.com/community_assets/images/apps";

/// Max size we accept from the CDN for one image (a hero is ~1-3 MB).
const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ArtSource {
    steam_roots: Vec<PathBuf>,
    cache: PathBuf,
    /// Store asset lookups done this session (`None`: the store has nothing for the app).
    lookups: Mutex<HashMap<AppId, Option<Arc<StoreAssets>>>>,
}

impl ArtSource {
    pub fn new(steam_roots: Vec<PathBuf>) -> Self {
        Self { steam_roots, cache: store::cache_dir().join("art"), lookups: Mutex::default() }
    }

    /// Raw encoded bytes (JPEG/PNG) of the art, or `Ok(None)` if Steam has none.
    /// `icon_hash` is used for [`ArtKind::Icon`] (empty: ask the store for it) and ignored otherwise.
    pub fn fetch(&self, app: AppId, kind: ArtKind, icon_hash: &str) -> Result<Option<Vec<u8>>> {
        let mut hash = icon_hash.to_string();
        if kind == ArtKind::Icon && hash.is_empty() {
            // The official client keeps the icon as `<hash>.jpg` next to the covers: use it before asking the
            // store, so a big library doesn't turn into one store request per sidebar row.
            for root in &self.steam_roots {
                if let Some(p) = local::cached_icon(root, app.0) {
                    return Ok(Some(std::fs::read(p)?));
                }
            }
            hash = self.lookup(app)?.map(|a| a.icon_hash.clone()).unwrap_or_default();
            if hash.is_empty() {
                return Ok(None);
            }
        }
        let (file, url) = match kind {
            ArtKind::Icon => (format!("{hash}.jpg"), format!("{ICON_CDN}/{app}/{hash}.jpg")),
            k => (k.file_name().to_string(), format!("{CDN}/{app}/{}", k.file_name())),
        };
        let file = file.as_str();
        for root in &self.steam_roots {
            if let Some(p) = local::cached_art(root, app.0, file) {
                return Ok(Some(std::fs::read(p)?));
            }
        }
        let ours = self.cache.join(app.to_string()).join(file);
        if ours.is_file() {
            return Ok(Some(std::fs::read(ours)?));
        }
        // `.nf` replaces the alpha-2 `.missing` marker (which never expired and may be wrong).
        let missing = ours.with_extension("nf");
        if is_fresh(&missing) {
            return Ok(None);
        }
        let mut got = download(&url)?;
        if got.is_none() && kind == ArtKind::Icon {
            got = download(&format!("{ICON_CDN_NEW}/{app}/{hash}.jpg"))?;
        }
        if got.is_none()
            && kind != ArtKind::Icon
            && let Some(real) = self.lookup(app)?.and_then(|a| a.url_for(kind).map(str::to_string))
        {
            got = download(&real)?;
        }
        match got {
            Some(bytes) => {
                write_atomic(&ours, &bytes)?;
                Ok(Some(bytes))
            }
            None => {
                write_atomic(&missing, b"")?;
                Ok(None)
            }
        }
    }

    /// The store's real asset paths for an app (one request per app and session).
    fn lookup(&self, app: AppId) -> Result<Option<Arc<StoreAssets>>> {
        if let Some(hit) = self.lookups.lock().expect("lookup lock").get(&app) {
            return Ok(hit.clone());
        }
        // The lock is not held during the request: the other art workers keep loading meanwhile.
        let found = self.request_assets(app);
        // A failed lookup counts as "nothing found" for this session, so it isn't repeated every frame.
        let found = found.unwrap_or(None);
        self.lookups.lock().expect("lookup lock").insert(app, found.clone());
        Ok(found)
    }

    fn request_assets(&self, app: AppId) -> Result<Option<Arc<StoreAssets>>> {
        let resp = ureq::get(&get_items_url(&[app]))
            .config()
            .timeout_global(Some(LOOKUP_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .call()?;
        if resp.status().as_u16() != 200 {
            bail!("HTTP {} from the store asset lookup", resp.status().as_u16());
        }
        let body = resp.into_body().with_config().limit(1024 * 1024).read_to_string()?;
        Ok(parse_get_items(&body).into_iter().find(|(a, _)| *a == app).map(|(_, v)| Arc::new(v)))
    }
}

fn is_fresh(marker: &Path) -> bool {
    std::fs::metadata(marker)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < MISSING_TTL))
}

fn download(url: &str) -> Result<Option<Vec<u8>>> {
    // A stalled download must not hold an art worker forever.
    let resp =
        ureq::get(url).config().timeout_global(Some(DOWNLOAD_TIMEOUT)).http_status_as_error(false).build().call()?;
    match resp.status().as_u16() {
        200 => Ok(Some(resp.into_body().with_config().limit(MAX_DOWNLOAD_BYTES).read_to_vec()?)),
        403 | 404 => Ok(None),
        s => bail!("HTTP {s} for {url}"),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    store::write_private(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"response":{"store_items":[
      {"appid":4001890,"success":1,"assets":{"asset_url_format":"steam/apps/4001890/${FILENAME}?t=1792508550",
       "header":"45c4/header.jpg","library_capsule":"2806/library_capsule.jpg","library_hero":"3f39/library_hero.jpg",
       "community_icon":"5c598fcb"}},
      {"appid":440,"assets":{"asset_url_format":"steam/apps/440/${FILENAME}?t=1","header":"header.jpg",
       "library_capsule":"library_600x900.jpg","community_icon":"f568"}},
      {"appid":7,"success":2}]}}"#;

    #[test]
    fn parses_hashed_and_plain_assets() {
        let v = parse_get_items(SAMPLE);
        assert_eq!(v.len(), 2);
        let (app, a) = &v[0];
        assert_eq!(*app, AppId(4001890));
        assert_eq!(
            a.capsule.as_deref(),
            Some(
                "https://shared.steamstatic.com/store_item_assets/steam/apps/4001890/2806/library_capsule.jpg?t=1792508550"
            )
        );
        assert_eq!(a.icon_hash, "5c598fcb");
        assert!(a.logo.is_none());
        assert_eq!(
            v[1].1.url_for(ArtKind::Header).unwrap(),
            "https://shared.steamstatic.com/store_item_assets/steam/apps/440/header.jpg?t=1"
        );
    }

    #[test]
    fn bad_json_is_empty() {
        assert!(parse_get_items("nope").is_empty());
        assert!(parse_get_items("{}").is_empty());
    }

    #[test]
    fn request_url_is_encoded() {
        let u = get_items_url(&[AppId(1), AppId(2)]);
        assert!(u.starts_with(GET_ITEMS));
        assert!(u.contains("%22appid%22%3A1"));
        assert!(!u.contains('{') && !u.contains('"'));
    }

    /// Needs the network: `cargo test -p sr-steam live_hashed -- --ignored`
    #[test]
    #[ignore]
    fn live_hashed_apps_resolve() {
        let src = ArtSource::new(vec![]);
        for app in [4_001_890, 3_892_270, 440] {
            let cover = src.fetch(AppId(app), ArtKind::Capsule, "").unwrap();
            let icon = src.fetch(AppId(app), ArtKind::Icon, "").unwrap();
            let hdr = src.fetch(AppId(app), ArtKind::Header, "").unwrap();
            eprintln!(
                "{app}: cover {:?} icon {:?} header {:?}",
                cover.map(|b| b.len()),
                icon.map(|b| b.len()),
                hdr.map(|b| b.len())
            );
        }
    }
}
