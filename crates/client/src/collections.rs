//! Favourites, hidden games and collections.
//!
//! Two sources are merged: the official client's (read-only, see `sr_steam::collections`) and our own
//! (`collections.json` in our data dir). We never write Steam's file.
//!
//! The rule for favourites and hidden games: a game is a favourite (hidden) if Steam's list or ours says so,
//! unless we explicitly un-favourited (un-hid) it; that choice is stored in our file as an override and is
//! dropped again when the game is added back. Collections made here are ours and editable; Steam's are shown
//! as they are, read-only. A dynamic Steam collection lists only the games added to it by hand, because its
//! filter is not evaluated.

use super::{Client, Command};
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::{collections as steam, store};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

/// Collections we keep, and the longest name.
const MAX_COLLECTIONS: usize = 200;
const MAX_NAME_CHARS: usize = 60;

/// Special ids of the two built-in lists (the same words Steam uses).
pub const FAVORITE: &str = "favorite";
pub const HIDDEN: &str = "hidden";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct OurCollection {
    id: String,
    name: String,
    apps: BTreeSet<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Ours {
    #[serde(default)]
    favorites: BTreeSet<u32>,
    /// Favourites in Steam's list that we turned off.
    #[serde(default)]
    unfavorited: BTreeSet<u32>,
    #[serde(default)]
    hidden: BTreeSet<u32>,
    /// Games hidden in Steam's list that we showed again.
    #[serde(default)]
    unhidden: BTreeSet<u32>,
    #[serde(default)]
    collections: Vec<OurCollection>,
    /// Counter for new collection ids.
    #[serde(default)]
    next_id: u32,
}

/// What the UI shows as one collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: String,
    pub name: String,
    /// A Steam dynamic collection: only its hand-added games are listed.
    pub dynamic: bool,
    /// Ours (can be changed); Steam's are read-only.
    pub ours: bool,
    pub apps: HashSet<u32>,
}

/// Merged favourites, hidden games and collections.
#[derive(Debug, Clone, Default)]
pub struct Collections {
    /// Bumped on every change, so views can cache lists built from this.
    pub rev: u64,
    pub favorites: HashSet<u32>,
    pub hidden: HashSet<u32>,
    /// Steam's collections, then ours, by name. Not the favourites or hidden lists.
    pub sections: Vec<Section>,
    /// What the library page shows: `FAVORITE`, `HIDDEN` or a section id (`None` = all games).
    pub selected: Option<String>,
    ours: Ours,
    steam: steam::SteamCollections,
}

impl Collections {
    fn path() -> PathBuf {
        store::data_dir().join("collections.json")
    }

    pub(crate) fn load() -> Self {
        let mut c = Self { ours: store::load_json(&Self::path()).unwrap_or_default(), ..Self::default() };
        c.rebuild();
        c
    }

    pub fn is_favorite(&self, app: AppId) -> bool {
        self.favorites.contains(&app.0)
    }

    pub fn is_hidden(&self, app: AppId) -> bool {
        self.hidden.contains(&app.0)
    }

    /// Our collections a game is in or could go to: `(id, name, contains)`.
    pub fn ours_for(&self, app: AppId) -> Vec<(&str, &str, bool)> {
        self.ours.collections.iter().map(|c| (c.id.as_str(), c.name.as_str(), c.apps.contains(&app.0))).collect()
    }

    /// The games of the list `id` (`FAVORITE`, `HIDDEN` or a section id).
    pub fn members(&self, id: &str) -> Option<&HashSet<u32>> {
        match id {
            FAVORITE => Some(&self.favorites),
            HIDDEN => Some(&self.hidden),
            _ => self.sections.iter().find(|s| s.id == id).map(|s| &s.apps),
        }
    }

    /// Display name of the list `id`.
    pub fn title(&self, id: &str) -> Option<&str> {
        match id {
            FAVORITE => Some("Favorites"),
            HIDDEN => Some("Hidden"),
            _ => self.sections.iter().find(|s| s.id == id).map(|s| s.name.as_str()),
        }
    }

    pub(crate) fn set_steam(&mut self, s: steam::SteamCollections) {
        if self.steam != s {
            self.steam = s;
            self.rebuild();
        }
    }

    /// Demo mode (no account): a few favourites and one collection, so the sections can be seen.
    pub(crate) fn set_demo(&mut self, ids: &[u32]) {
        let pick = |from: usize, n: usize| ids.iter().skip(from).step_by(7).take(n).copied().collect::<HashSet<u32>>();
        let demo = steam::SteamCollections {
            favorites: pick(0, 4),
            hidden: pick(3, 2),
            collections: vec![
                steam::SteamCollection {
                    id: "uc-demo".into(),
                    name: "Co-op nights".into(),
                    apps: pick(1, 12),
                    dynamic: false,
                },
                steam::SteamCollection {
                    id: "uc-dyn".into(),
                    name: "Under 2 hours".into(),
                    apps: HashSet::new(),
                    dynamic: true,
                },
            ],
        };
        self.set_steam(demo);
    }

    fn rebuild(&mut self) {
        let (s, o) = (&self.steam, &self.ours);
        self.favorites = s.favorites.union(&o.favorites.iter().copied().collect()).copied().collect();
        self.favorites.retain(|a| !o.unfavorited.contains(a));
        self.hidden = s.hidden.union(&o.hidden.iter().copied().collect()).copied().collect();
        self.hidden.retain(|a| !o.unhidden.contains(a));
        let mut sections: Vec<Section> = s
            .collections
            .iter()
            .map(|c| Section {
                id: c.id.clone(),
                name: c.name.clone(),
                dynamic: c.dynamic,
                ours: false,
                apps: c.apps.clone(),
            })
            .collect();
        sections.extend(o.collections.iter().map(|c| Section {
            id: c.id.clone(),
            name: c.name.clone(),
            dynamic: false,
            ours: true,
            apps: c.apps.iter().copied().collect(),
        }));
        sections.sort_by_key(|c| c.name.to_lowercase());
        self.sections = sections;
        if self.selected.as_deref().is_some_and(|id| self.members(id).is_none()) {
            self.selected = None;
        }
        self.rev += 1;
    }

    fn set_favorite(&mut self, app: u32, on: bool) {
        let o = &mut self.ours;
        if on {
            o.unfavorited.remove(&app);
            o.favorites.insert(app);
        } else {
            o.favorites.remove(&app);
            if self.steam.favorites.contains(&app) {
                o.unfavorited.insert(app);
            }
        }
    }

    fn set_hidden(&mut self, app: u32, on: bool) {
        let o = &mut self.ours;
        if on {
            o.unhidden.remove(&app);
            o.hidden.insert(app);
        } else {
            o.hidden.remove(&app);
            if self.steam.hidden.contains(&app) {
                o.unhidden.insert(app);
            }
        }
    }

    /// Create a collection; returns its id, or `None` when the name is empty or there are too many.
    fn new_collection(&mut self, name: &str) -> Option<String> {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        if name.is_empty() || self.ours.collections.len() >= MAX_COLLECTIONS {
            return None;
        }
        self.ours.next_id += 1;
        let id = format!("kettle-{}", self.ours.next_id);
        self.ours.collections.push(OurCollection { id: id.clone(), name, apps: BTreeSet::new() });
        Some(id)
    }

    fn set_member(&mut self, id: &str, app: u32, on: bool) {
        if let Some(c) = self.ours.collections.iter_mut().find(|c| c.id == id) {
            if on {
                c.apps.insert(app);
            } else {
                c.apps.remove(&app);
            }
        }
    }
}

/// What the UI can ask for; every change is saved to `collections.json` at once.
#[derive(Debug, Clone, PartialEq)]
pub enum CollCmd {
    SetFavorite(AppId, bool),
    SetHidden(AppId, bool),
    /// Put a game into (or take it out of) one of our collections.
    SetMember {
        collection: String,
        app: AppId,
        on: bool,
    },
    /// A new collection of ours, optionally with its first game.
    New {
        name: String,
        app: Option<AppId>,
    },
    Delete(String),
    /// What the library page shows (`None` = all games).
    Show(Option<String>),
}

impl Client {
    pub(crate) fn collections_cmd(&mut self, cmd: CollCmd) {
        let c = &mut self.state.collections;
        match cmd {
            CollCmd::Show(id) => {
                c.selected = id.filter(|id| c.members(id).is_some());
                c.rev += 1;
                return;
            }
            CollCmd::SetFavorite(app, on) => c.set_favorite(app.0, on),
            CollCmd::SetHidden(app, on) => c.set_hidden(app.0, on),
            CollCmd::SetMember { collection, app, on } => c.set_member(&collection, app.0, on),
            CollCmd::New { name, app } => match c.new_collection(&name) {
                Some(id) => {
                    if let Some(app) = app {
                        c.set_member(&id, app.0, true);
                    }
                }
                None => {
                    self.state.notice = Some("Couldn't make that collection (empty name or too many).".into());
                    return;
                }
            },
            CollCmd::Delete(id) => c.ours.collections.retain(|x| x.id != id),
        }
        c.rebuild();
        let saved = store::save_json(&Collections::path(), &c.ours);
        if let Err(e) = saved {
            self.state.notice = Some(format!("Couldn't save collections: {e:#}"));
        }
    }

    /// Re-read Steam's collections for the signed-in account (demo mode keeps its own).
    pub(crate) fn reload_steam_collections(&mut self) {
        match &self.state.account {
            None => self.state.collections.set_steam(Default::default()),
            Some(a) if a.steam_id.0 != 0 => self.state.collections.set_steam(steam::load(a.steam_id.0)),
            Some(_) => {}
        }
    }
}

impl From<CollCmd> for Command {
    fn from(c: CollCmd) -> Self {
        Command::Collections(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_steam() -> Collections {
        let mut c = Collections::default();
        c.steam.favorites = HashSet::from([1, 2]);
        c.steam.hidden = HashSet::from([9]);
        c.rebuild();
        c
    }

    #[test]
    fn favorites_merge_and_override() {
        let mut c = with_steam();
        assert!(c.is_favorite(AppId(1)));
        c.set_favorite(3, true);
        c.set_favorite(1, false);
        c.rebuild();
        assert_eq!(c.favorites, HashSet::from([2, 3]));
        // Added back: the override goes away.
        c.set_favorite(1, true);
        c.rebuild();
        assert!(c.is_favorite(AppId(1)) && c.ours.unfavorited.is_empty());
        // Removing one only we had leaves no override.
        c.set_favorite(3, false);
        c.rebuild();
        assert!(!c.is_favorite(AppId(3)) && c.ours.unfavorited.is_empty());
    }

    #[test]
    fn hidden_override() {
        let mut c = with_steam();
        c.set_hidden(9, false);
        c.rebuild();
        assert!(!c.is_hidden(AppId(9)));
        c.set_hidden(4, true);
        c.rebuild();
        assert!(c.is_hidden(AppId(4)));
    }

    #[test]
    fn our_collections() {
        let mut c = with_steam();
        assert!(c.new_collection("   ").is_none());
        let id = c.new_collection("  Chill  ").unwrap();
        c.set_member(&id, 5, true);
        c.rebuild();
        assert_eq!(c.title(&id), Some("Chill"));
        assert_eq!(c.members(&id), Some(&HashSet::from([5])));
        c.selected = Some(id.clone());
        c.ours.collections.clear();
        c.rebuild();
        assert!(c.selected.is_none());
        let long = "x".repeat(200);
        let id = c.new_collection(&long).unwrap();
        assert_eq!(c.title(&id).map(|t| t.chars().count()), None); // not rebuilt yet
        c.rebuild();
        assert_eq!(c.title(&id).unwrap().chars().count(), MAX_NAME_CHARS);
    }
}
