//! GPU textures for cover art, bounded per kind (least recently used is
//! evicted). The client loads and decodes; this only uploads and draws.

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
}

#[derive(Default)]
pub struct Textures {
    map: HashMap<(AppId, ArtKind), Entry>,
    frame: u64,
}

pub enum Art<'a> {
    Loading,
    Missing,
    Ready(&'a egui::TextureHandle),
}

impl Textures {
    /// Once per frame: upload finished images and evict over-budget ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context, client: &Client) {
        self.frame += 1;
        for img in client.take_art() {
            let slot = match img.rgba {
                Some(rgba) => {
                    let ci = egui::ColorImage::from_rgba_unmultiplied(img.size, &rgba);
                    let name = format!("art-{}-{:?}", img.app, img.kind);
                    Slot::Ready(ctx.load_texture(name, ci, egui::TextureOptions::LINEAR))
                }
                None => Slot::Missing,
            };
            self.map.insert((img.app, img.kind), Entry { slot, last_used: self.frame });
        }
        self.evict(client);
    }

    /// Look up art, asking the client to load it when it's not known yet.
    pub fn get(&mut self, client: &Client, app: AppId, kind: ArtKind) -> Art<'_> {
        let frame = self.frame;
        let e = self.map.entry((app, kind)).or_insert_with(|| {
            client.request_art(app, kind);
            Entry { slot: Slot::Loading, last_used: frame }
        });
        e.last_used = frame;
        match &e.slot {
            Slot::Loading => Art::Loading,
            Slot::Missing => Art::Missing,
            Slot::Ready(t) => Art::Ready(t),
        }
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
