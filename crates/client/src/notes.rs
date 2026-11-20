//! The overlay's notes: one short text per game, kept in a small JSON file in our data dir.

use super::Client;
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::store;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The longest note kept, in characters.
pub const MAX_NOTE_CHARS: usize = 20_000;
/// How many games can have a note.
const MAX_NOTES: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct Notes {
    /// `appid -> text`
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
}

impl Notes {
    fn path() -> PathBuf {
        store::data_dir().join("notes.json")
    }

    pub fn load() -> Notes {
        store::load_json(&Notes::path()).unwrap_or_default()
    }

    /// Set or delete a note. Returns false when the note was refused (too many games).
    pub fn set(&mut self, app: AppId, text: &str) -> bool {
        let key = app.0.to_string();
        let text: String = text.chars().take(MAX_NOTE_CHARS).collect();
        if text.trim().is_empty() {
            self.apps.remove(&key);
        } else if self.apps.contains_key(&key) || self.apps.len() < MAX_NOTES {
            self.apps.insert(key, text);
        } else {
            return false;
        }
        true
    }
}

impl Client {
    pub(crate) fn set_note(&mut self, app: AppId, text: String) {
        let mut notes = Notes { apps: std::mem::take(&mut self.state.notes) };
        let ok = notes.set(app, &text);
        self.state.notes = notes.apps;
        if !ok {
            self.state.notice = Some("Too many notes; delete one first.".into());
            return;
        }
        let saved = Notes { apps: self.state.notes.clone() };
        if let Err(e) = store::save_json(&Notes::path(), &saved) {
            self.state.notice = Some(format!("Couldn't save the note: {e:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_set_trim_and_delete() {
        let mut n = Notes::default();
        assert!(n.set(AppId(7), "boss at the lake"));
        assert_eq!(n.apps.get("7").map(String::as_str), Some("boss at the lake"));
        assert!(n.set(AppId(7), "  \n"));
        assert!(n.apps.is_empty());
        let long = "x".repeat(MAX_NOTE_CHARS + 50);
        n.set(AppId(1), &long);
        assert_eq!(n.apps["1"].chars().count(), MAX_NOTE_CHARS);
    }

    #[test]
    fn notes_are_bounded() {
        let mut n = Notes::default();
        for i in 0..MAX_NOTES as u32 {
            assert!(n.set(AppId(i), "a"));
        }
        assert!(!n.set(AppId(100_000), "b"));
        assert!(n.set(AppId(3), "changed"));
    }
}
