//! Background cover-art loader: fetch (local cache / CDN) + decode + downscale
//! on worker threads. Newest requests are served first (LIFO), so the rows the
//! user is looking at now win over rows scrolled past.

use crate::Waker;
use sr_model::{AppId, ArtKind};
use sr_steam::art::ArtSource;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

const WORKERS: usize = 3;
/// Pending requests beyond this are dropped (oldest first); they'll be asked
/// for again if they scroll back into view.
const MAX_PENDING: usize = 128;

/// A decoded, downscaled RGBA image ready for upload as a texture.
pub struct ArtImage {
    pub app: AppId,
    pub kind: ArtKind,
    pub size: [usize; 2],
    /// `None` when Steam has no such image (or it failed to load).
    pub rgba: Option<Vec<u8>>,
}

/// Largest size we keep per kind (2x the usual on-screen size).
fn max_size(kind: ArtKind) -> (u32, u32) {
    match kind {
        ArtKind::Capsule => (300, 450),
        ArtKind::Hero => (1920, 620),
        ArtKind::Logo => (640, 360),
        ArtKind::Header => (460, 215),
        ArtKind::Icon => (32, 32),
    }
}

type Key = (AppId, ArtKind);

#[derive(Default)]
struct Shared {
    pending: Vec<Key>,
    /// Requested and not yet forgotten: never queued twice.
    known: HashSet<Key>,
    done: Vec<ArtImage>,
    icon_hashes: HashMap<AppId, String>,
}

pub struct ArtLoader {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl ArtLoader {
    pub fn new(steam_roots: Vec<PathBuf>, wake: Waker) -> Self {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let source = Arc::new(ArtSource::new(steam_roots));
        for i in 0..WORKERS {
            let (shared, source, wake) = (shared.clone(), source.clone(), wake.clone());
            std::thread::Builder::new()
                .name(format!("art-{i}"))
                .spawn(move || worker(&shared, &source, &wake))
                .expect("spawn art worker");
        }
        Self { shared }
    }

    pub fn request(&self, app: AppId, kind: ArtKind) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().expect("art lock");
        if s.known.insert((app, kind)) {
            s.pending.push((app, kind));
            if s.pending.len() > MAX_PENDING {
                let dropped = s.pending.remove(0);
                s.known.remove(&dropped);
            }
            cv.notify_one();
        }
    }

    pub fn forget(&self, app: AppId, kind: ArtKind) {
        self.shared.0.lock().expect("art lock").known.remove(&(app, kind));
    }

    pub fn set_icon_hashes(&self, hashes: HashMap<AppId, String>) {
        self.shared.0.lock().expect("art lock").icon_hashes = hashes;
    }

    pub fn take(&self) -> Vec<ArtImage> {
        std::mem::take(&mut self.shared.0.lock().expect("art lock").done)
    }
}

fn worker(shared: &(Mutex<Shared>, Condvar), source: &ArtSource, wake: &Waker) {
    let (lock, cv) = shared;
    loop {
        let (app, kind, hash) = {
            let mut s = lock.lock().expect("art lock");
            loop {
                if let Some((app, kind)) = s.pending.pop() {
                    let hash = if kind == ArtKind::Icon { s.icon_hashes.get(&app).cloned() } else { None };
                    break (app, kind, hash.unwrap_or_default());
                }
                s = cv.wait(s).expect("art lock");
            }
        };
        let img = load(source, app, kind, &hash);
        lock.lock().expect("art lock").done.push(img);
        wake();
    }
}

fn load(source: &ArtSource, app: AppId, kind: ArtKind, icon_hash: &str) -> ArtImage {
    let none = ArtImage { app, kind, size: [0, 0], rgba: None };
    let Ok(Some(bytes)) = source.fetch(app, kind, icon_hash) else { return none };
    // Steam sometimes stores PNGs as .jpg: detect the format from the bytes.
    let Ok(img) = image::load_from_memory(&bytes) else { return none };
    let (mw, mh) = max_size(kind);
    let img = if img.width() > mw || img.height() > mh {
        img.resize(mw, mh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.into_rgba8();
    ArtImage { app, kind, size: [rgba.width() as usize, rgba.height() as usize], rgba: Some(rgba.into_raw()) }
}
