//! One back/forward history across every page (Store, Library home, game pages, collections, Downloads,
//! profile pages, store pages), like the official client's. The history only remembers *where* the user was;
//! [`crate::AppUi`] applies a place by setting the tab, the selected game, the collection and the store page.

use crate::Tab;
use sr_model::AppId;

/// Where the central area is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    pub tab: Tab,
    /// The game page (library tab only).
    pub game: Option<AppId>,
    /// The store page id (see `StoreView::page_id`; 0 is the store home).
    pub store: u64,
    /// The shown collection (library tab only; `None` = all games).
    pub collection: Option<String>,
}

const MAX_PLACES: usize = 100;

#[derive(Default)]
pub(crate) struct History {
    places: Vec<Place>,
    at: usize,
}

impl History {
    /// Call every frame with where the user is now. A new place cuts off everything ahead of the current one.
    pub fn record(&mut self, now: Place) {
        if self.places.is_empty() {
            self.places.push(now);
            return;
        }
        if self.places[self.at] == now {
            return;
        }
        self.places.truncate(self.at + 1);
        self.places.push(now);
        if self.places.len() > MAX_PLACES {
            self.places.remove(0);
        }
        self.at = self.places.len() - 1;
    }

    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.places.len()
    }

    /// Step back; returns the place to show.
    pub fn back(&mut self) -> Option<Place> {
        if !self.can_back() {
            return None;
        }
        self.at -= 1;
        Some(self.places[self.at].clone())
    }

    pub fn forward(&mut self) -> Option<Place> {
        if !self.can_forward() {
            return None;
        }
        self.at += 1;
        Some(self.places[self.at].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(tab: Tab, game: Option<u32>) -> Place {
        Place { tab, game: game.map(AppId), store: 0, collection: None }
    }

    #[test]
    fn back_forward_and_cut() {
        let mut h = History::default();
        h.record(place(Tab::Library, None));
        h.record(place(Tab::Library, Some(10)));
        h.record(place(Tab::Store, None));
        assert_eq!(h.back(), Some(place(Tab::Library, Some(10))));
        assert_eq!(h.back(), Some(place(Tab::Library, None)));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(place(Tab::Library, Some(10))));
        // Going somewhere new drops the forward entries.
        h.record(place(Tab::Downloads, None));
        assert!(!h.can_forward());
        assert!(h.can_back());
    }
}
