//! Cover art lookup: the official client's local cache first, then our own
//! cache, then Steam's CDN (downloaded into our cache). Blocking; call from a
//! worker thread.

use crate::{local, store};
use anyhow::{Result, bail};
use sr_model::{AppId, ArtKind};
use std::path::{Path, PathBuf};

const CDN: &str = "https://shared.steamstatic.com/store_item_assets/steam/apps";
const ICON_CDN: &str = "https://media.steampowered.com/steamcommunity/public/images/apps";

/// Max size we accept from the CDN for one image (a hero is ~1-3 MB).
const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;

pub struct ArtSource {
    steam_roots: Vec<PathBuf>,
    cache: PathBuf,
}

impl ArtSource {
    pub fn new(steam_roots: Vec<PathBuf>) -> Self {
        Self { steam_roots, cache: store::cache_dir().join("art") }
    }

    /// Raw encoded bytes (JPEG/PNG) of the art, or `Ok(None)` if Steam has none.
    /// `icon_hash` is required for [`ArtKind::Icon`] and ignored otherwise.
    pub fn fetch(&self, app: AppId, kind: ArtKind, icon_hash: &str) -> Result<Option<Vec<u8>>> {
        let (file, url) = match kind {
            ArtKind::Icon if icon_hash.is_empty() => return Ok(None),
            ArtKind::Icon => (format!("{icon_hash}.jpg"), format!("{ICON_CDN}/{app}/{icon_hash}.jpg")),
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
        let missing = ours.with_extension("missing");
        if missing.exists() {
            return Ok(None);
        }
        match download(&url)? {
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
}

fn download(url: &str) -> Result<Option<Vec<u8>>> {
    let resp = ureq::get(url).config().http_status_as_error(false).build().call()?;
    match resp.status().as_u16() {
        200 => Ok(Some(resp.into_body().with_config().limit(MAX_DOWNLOAD_BYTES).read_to_vec()?)),
        403 | 404 => Ok(None),
        s => bail!("HTTP {s} for {url}"),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    store::write_private(path, bytes)
}
