//! The bigger Store tab: reviews, the wishlist, and browsing by list and tag.
//!
//! One `Command::StoreMore` and one `Event::StoreMore` carry everything here, so `lib.rs` only dispatches.
//! All data is public (see `sr_steam::store_more`); the wishlist is read-only, because adding or removing an
//! entry needs the store's web session, which Kettle doesn't have. Every request carries the store region:
//! the country the Steam login saw and the language from Settings.

use super::{Client, Command, Event, Load};
use sr_model::AppId;
use sr_steam::store_more as sm;
use sr_steam::storefront;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub use sm::{BrowseKind, Review, ReviewSummary, Reviews, Tag, WishItem, Wishlist};

/// Review sets kept in memory.
const REVIEWS_KEPT: usize = 16;
/// Browse list is cut here so an endless scroll can't grow without bound.
const BROWSE_MAX: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub enum StoreCmd {
    /// Summary and top reviews of an app (no-op while loaded or loading).
    Reviews(AppId),
    /// Load the signed-in user's wishlist; `refresh` fetches it again even when loaded.
    Wishlist { refresh: bool },
    /// Load the tag list (no-op once loaded).
    Tags,
    /// Show a browse list. `more` appends the next page to the one shown; otherwise the list starts over.
    Browse { kind: BrowseKind, tag: Option<u32>, more: bool },
}

#[derive(Debug)]
pub(crate) enum StoreEvent {
    Reviews { app: u32, result: Result<Reviews, String> },
    Wishlist { generation: u64, steam_id: u64, result: Result<Wishlist, String> },
    Tags(Result<Vec<Tag>, String>),
    Browsed { token: u64, start: usize, result: Result<sm::BrowsePage, String> },
}

#[derive(Debug, Clone, Default)]
pub struct BrowseState {
    pub kind: BrowseKind,
    pub tag: Option<u32>,
    pub items: Vec<super::StoreItem>,
    pub loading: bool,
    /// A further page may exist.
    pub more: bool,
    pub error: Option<String>,
    /// Newest request; older answers are dropped.
    token: u64,
    started: bool,
}

impl BrowseState {
    /// True once a list was asked for at least once.
    pub fn started(&self) -> bool {
        self.started
    }
}

#[derive(Debug, Clone, Default)]
pub struct StoreMoreState {
    pub reviews: HashMap<u32, Load<Arc<Reviews>>>,
    reviews_order: VecDeque<u32>,
    pub wishlist: Load<Arc<Wishlist>>,
    /// SteamID the wishlist belongs to (0 = none yet).
    pub wishlist_for: u64,
    pub tags: Load<Arc<Vec<Tag>>>,
    pub browse: BrowseState,
}

/// "US / german", for the line under the store that tells which region its prices are for.
pub fn store_region() -> String {
    let (c, l) = storefront::locale();
    let c = if c.is_empty() { "chosen by Steam from your IP address".to_string() } else { c.to_uppercase() };
    format!("{c}, language {l}")
}

impl Client {
    /// Use the Settings language for store requests.
    pub(crate) fn sync_store_language(&self) {
        storefront::set_language(self.state.settings.downloads.language_or_default());
    }

    pub(crate) fn store_more_cmd(&mut self, cmd: StoreCmd) {
        self.sync_store_language();
        match cmd {
            StoreCmd::Reviews(app) => self.load_reviews(app.0),
            StoreCmd::Wishlist { refresh } => self.load_wishlist(refresh),
            StoreCmd::Tags => self.load_tags(),
            StoreCmd::Browse { kind, tag, more } => self.load_browse(kind, tag, more),
        }
    }

    fn load_reviews(&mut self, app: u32) {
        let m = &mut self.state.store.more;
        if matches!(m.reviews.get(&app), Some(Load::Loading | Load::Ready(_))) {
            return;
        }
        m.reviews.insert(app, Load::Loading);
        m.reviews_order.retain(|a| *a != app);
        m.reviews_order.push_back(app);
        while m.reviews_order.len() > REVIEWS_KEPT {
            if let Some(old) = m.reviews_order.pop_front() {
                m.reviews.remove(&old);
            }
        }
        self.spawn_store(move || sm::reviews(app), move |result| Event::StoreMore(StoreEvent::Reviews { app, result }));
    }

    fn load_wishlist(&mut self, refresh: bool) {
        let Some(id) = self.state.account.as_ref().map(|a| a.steam_id.0) else {
            self.state.store.more.wishlist = Load::Failed("Sign in to see your wishlist.".into());
            return;
        };
        let m = &mut self.state.store.more;
        if m.wishlist_for != id {
            m.wishlist = Load::Idle;
            m.wishlist_for = id;
        }
        match m.wishlist {
            Load::Loading => return,
            Load::Ready(_) if !refresh => return,
            Load::Ready(_) => {}
            _ => m.wishlist = Load::Loading,
        }
        let generation = self.generation.load(Ordering::Relaxed);
        self.spawn_store(
            move || sm::wishlist(id),
            move |result| Event::StoreMore(StoreEvent::Wishlist { generation, steam_id: id, result }),
        );
    }

    fn load_tags(&mut self) {
        let m = &mut self.state.store.more;
        if matches!(m.tags, Load::Loading | Load::Ready(_)) {
            return;
        }
        m.tags = Load::Loading;
        self.spawn_store(sm::tags, |result| Event::StoreMore(StoreEvent::Tags(result)));
    }

    fn load_browse(&mut self, kind: BrowseKind, tag: Option<u32>, more: bool) {
        let b = &mut self.state.store.more.browse;
        let start = if more {
            if b.loading || !b.more || b.items.len() >= BROWSE_MAX || b.kind != kind || b.tag != tag {
                return;
            }
            b.items.len()
        } else {
            if b.started && !b.loading && b.kind == kind && b.tag == tag && b.error.is_none() {
                return;
            }
            b.items.clear();
            b.more = false;
            0
        };
        b.token += 1;
        let token = b.token;
        (b.kind, b.tag, b.loading, b.error, b.started) = (kind, tag, true, None, true);
        self.spawn_store(
            move || sm::browse(kind, tag, start),
            move |result| Event::StoreMore(StoreEvent::Browsed { token, start, result }),
        );
    }

    pub(crate) fn apply_store_more(&mut self, ev: StoreEvent) {
        let current = self.generation.load(Ordering::Relaxed);
        let m = &mut self.state.store.more;
        match ev {
            StoreEvent::Reviews { app, result } => {
                if let Some(slot) = m.reviews.get_mut(&app) {
                    *slot = match result {
                        Ok(r) => Load::Ready(Arc::new(r)),
                        Err(e) => Load::Failed(e),
                    };
                }
            }
            StoreEvent::Wishlist { generation, steam_id, result } => {
                if generation != current || steam_id != m.wishlist_for {
                    return;
                }
                m.wishlist = match result {
                    Ok(w) => Load::Ready(Arc::new(w)),
                    Err(e) => Load::Failed(e),
                };
            }
            StoreEvent::Tags(result) => {
                m.tags = match result {
                    Ok(t) => Load::Ready(Arc::new(t)),
                    Err(e) => Load::Failed(e),
                };
            }
            StoreEvent::Browsed { token, start, result } => {
                let b = &mut m.browse;
                if token != b.token {
                    return;
                }
                b.loading = false;
                match result {
                    Ok(page) => {
                        let have: std::collections::HashSet<u32> = b.items.iter().map(|i| i.id).collect();
                        b.items.extend(page.items.into_iter().filter(|i| !have.contains(&i.id)));
                        b.more = page.more;
                        let _ = start;
                    }
                    Err(e) => {
                        b.error = Some(e);
                        b.more = false;
                    }
                }
            }
        }
    }
}

impl From<StoreCmd> for Command {
    fn from(c: StoreCmd) -> Self {
        Command::StoreMore(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_state_starts_empty() {
        let b = BrowseState::default();
        assert!(!b.started() && b.items.is_empty() && !b.loading && !b.more);
        assert_eq!(b.kind, BrowseKind::TopSellers);
    }

    #[test]
    fn commands_compare() {
        assert_eq!(StoreCmd::Tags, StoreCmd::Tags);
        assert_ne!(StoreCmd::Wishlist { refresh: true }, StoreCmd::Wishlist { refresh: false });
    }
}
