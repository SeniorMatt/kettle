//! Background loader for store images keyed by URL: download (cached on disk by
//! `storefront`), decode and downscale on worker threads. Like the cover-art
//! loader, the newest request is served first so what is on screen now wins.

use crate::Waker;
use sr_steam::storefront;
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};

const WORKERS: usize = 3;
/// Pending requests beyond this are dropped (oldest first) and asked for again
/// when they are still on screen.
const MAX_PENDING: usize = 64;
/// Largest size kept (screenshots are 1920x1080 at the source).
const MAX_SIZE: (u32, u32) = (960, 600);

/// A decoded, downscaled RGBA image ready for upload as a texture.
pub struct WebImage {
    pub url: String,
    pub size: [usize; 2],
    /// `None` when the download or decode failed.
    pub rgba: Option<Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    pending: Vec<String>,
    known: HashSet<String>,
    done: Vec<WebImage>,
}

pub struct WebImageLoader {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl WebImageLoader {
    pub fn new(wake: Waker) -> Self {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        for i in 0..WORKERS {
            let (shared, wake) = (shared.clone(), wake.clone());
            std::thread::Builder::new()
                .name(format!("web-img-{i}"))
                .spawn(move || worker(&shared, &wake))
                .expect("spawn image worker");
        }
        Self { shared }
    }

    pub fn request(&self, url: &str) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().expect("image lock");
        if s.known.insert(url.to_string()) {
            s.pending.push(url.to_string());
            if s.pending.len() > MAX_PENDING {
                let dropped = s.pending.remove(0);
                s.known.remove(&dropped);
            }
            cv.notify_one();
        }
    }

    pub fn forget(&self, url: &str) {
        self.shared.0.lock().expect("image lock").known.remove(url);
    }

    pub fn take(&self) -> Vec<WebImage> {
        std::mem::take(&mut self.shared.0.lock().expect("image lock").done)
    }
}

fn worker(shared: &(Mutex<Shared>, Condvar), wake: &Waker) {
    let (lock, cv) = shared;
    loop {
        let url = {
            let mut s = lock.lock().expect("image lock");
            loop {
                if let Some(url) = s.pending.pop() {
                    break url;
                }
                s = cv.wait(s).expect("image lock");
            }
        };
        let img = load(url);
        lock.lock().expect("image lock").done.push(img);
        wake();
    }
}

fn load(url: String) -> WebImage {
    let decoded = storefront::fetch_image(&url).ok().and_then(|b| image::load_from_memory(&b).ok());
    let Some(img) = decoded else { return WebImage { url, size: [0, 0], rgba: None } };
    let img = if img.width() > MAX_SIZE.0 || img.height() > MAX_SIZE.1 {
        img.resize(MAX_SIZE.0, MAX_SIZE.1, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.into_rgba8();
    WebImage { url, size: [rgba.width() as usize, rgba.height() as usize], rgba: Some(rgba.into_raw()) }
}
