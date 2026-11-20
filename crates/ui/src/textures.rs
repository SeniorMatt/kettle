//! GPU textures for cover art, bounded per kind (least recently used is
//! evicted). The client loads and decodes; this only uploads and draws.

use crate::anim;
use sr_client::Client;
use sr_model::{AppId, ArtKind};
use std::collections::HashMap;

/// Max live textures per kind. A 300x450 capsule is ~0.5 MB of VRAM.
fn budget(kind: ArtKind) -> usize {
    match kind {
        ArtKind::Capsule => 400,
        ArtKind::Header => 200,
        ArtKind::Hero | ArtKind::Logo => 12,
        ArtKind::Icon => 600,
    }
}

enum Slot {
    Loading,
    Missing,
    Ready(egui::TextureHandle),
}

struct Entry {
    slot: Slot,
    last_used: u64,
    /// egui time when the texture first became ready, for the fade-in.
    ready_at: f64,
}

/// Length of the cover fade-in, in seconds.
const FADE: f32 = 0.2;

#[derive(Default)]
pub struct Textures {
    map: HashMap<(AppId, ArtKind), Entry>,
    web: HashMap<String, Entry>,
    frame: u64,
    now: f64,
    ctx: Option<egui::Context>,
}

pub enum Art<'a> {
    Loading,
    Missing,
    /// The texture and its fade-in opacity (0..1).
    Ready(&'a egui::TextureHandle, f32),
}

impl Textures {
    /// Once per frame: upload finished images and evict over-budget ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context, client: &Client) {
        self.frame += 1;
        self.now = ctx.input(|i| i.time);
        self.ctx = Some(ctx.clone());
        self.upload_web(ctx, client);
        for img in client.take_art() {
            let slot = match img.rgba {
                Some(rgba) => {
                    let ci = egui::ColorImage::from_rgba_unmultiplied(img.size, &rgba);
                    let name = format!("art-{}-{:?}", img.app, img.kind);
                    Slot::Ready(ctx.load_texture(name, ci, egui::TextureOptions::LINEAR))
                }
                None => Slot::Missing,
            };
            self.map.insert((img.app, img.kind), Entry { slot, last_used: self.frame, ready_at: self.now });
        }
        self.evict(client);
    }

    /// Look up art, asking the client to load it when it's not known yet.
    pub fn get(&mut self, client: &Client, app: AppId, kind: ArtKind) -> Art<'_> {
        let frame = self.frame;
        let e = self.map.entry((app, kind)).or_insert_with(|| {
            client.request_art(app, kind);
            Entry { slot: Slot::Loading, last_used: frame, ready_at: 0.0 }
        });
        e.last_used = frame;
        show(e, self.ctx.as_ref(), self.now)
    }

    /// Store image by URL (same fade-in, own bounded LRU).
    pub fn get_web(&mut self, client: &Client, url: &str) -> Art<'_> {
        if url.is_empty() {
            return Art::Missing;
        }
        let frame = self.frame;
        let e = self.web.entry(url.to_string()).or_insert_with(|| {
            client.request_web_image(url);
            Entry { slot: Slot::Loading, last_used: frame, ready_at: 0.0 }
        });
        e.last_used = frame;
        show(e, self.ctx.as_ref(), self.now)
    }

    fn upload_web(&mut self, ctx: &egui::Context, client: &Client) {
        for img in client.take_web_images() {
            let slot = match img.rgba {
                Some(rgba) => {
                    let ci = egui::ColorImage::from_rgba_unmultiplied(img.size, &rgba);
                    Slot::Ready(ctx.load_texture("store-image", ci, egui::TextureOptions::LINEAR))
                }
                None => Slot::Missing,
            };
            self.web.insert(img.url, Entry { slot, last_used: self.frame, ready_at: self.now });
        }
        let over = self.web.values().filter(|e| matches!(e.slot, Slot::Ready(_))).count().saturating_sub(WEB_BUDGET);
        if over > 0 {
            let mut keys: Vec<_> = self
                .web
                .iter()
                .filter(|(_, e)| matches!(e.slot, Slot::Ready(_)))
                .map(|(k, e)| (e.last_used, k.clone()))
                .collect();
            keys.sort_unstable();
            for (_, k) in keys.into_iter().take(over) {
                self.web.remove(&k);
                client.forget_web_image(&k);
            }
        }
        let frame = self.frame;
        self.web.retain(|k, e| {
            let stale = matches!(e.slot, Slot::Loading) && frame - e.last_used > 600;
            if stale {
                client.forget_web_image(k);
            }
            !stale
        });
    }

    fn evict(&mut self, client: &Client) {
        for kind in [ArtKind::Capsule, ArtKind::Header, ArtKind::Hero, ArtKind::Logo, ArtKind::Icon] {
            let mut keys: Vec<_> = self
                .map
                .iter()
                .filter(|(k, e)| k.1 == kind && matches!(e.slot, Slot::Ready(_)))
                .map(|(k, e)| (e.last_used, *k))
                .collect();
            let over = keys.len().saturating_sub(budget(kind));
            if over == 0 {
                continue;
            }
            keys.sort_unstable_by_key(|(used, _)| *used);
            for (_, k) in keys.into_iter().take(over) {
                self.map.remove(&k);
                client.forget_art(k.0, k.1);
            }
        }
        // Loading entries that scrolled away long ago: let them be re-requested.
        let frame = self.frame;
        self.map.retain(|k, e| {
            let stale = matches!(e.slot, Slot::Loading) && frame - e.last_used > 600;
            if stale {
                client.forget_art(k.0, k.1);
            }
            !stale
        });
    }
}

/// Max live store textures (480x215 .. 960x540 each).
const WEB_BUDGET: usize = 120;

fn show<'a>(e: &'a Entry, ctx: Option<&egui::Context>, now: f64) -> Art<'a> {
    match &e.slot {
        Slot::Loading => Art::Loading,
        Slot::Missing => Art::Missing,
        Slot::Ready(t) => {
            let alpha = match ctx {
                Some(ctx) if now - e.ready_at < f64::from(FADE) => anim::since(ctx, e.ready_at, FADE),
                _ => 1.0,
            };
            Art::Ready(t, alpha)
        }
    }
}
