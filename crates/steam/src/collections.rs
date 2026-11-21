//! The official client's collections, read-only.
//!
//! Steam keeps them in `<root>/userdata/<accountid>/config/cloudstorage/cloud-storage-namespace-1.json`: a JSON
//! array of `[key, {"key", "timestamp", "value": "<json string>", "is_deleted"?}]`. Keys `user-collections.<id>`
//! hold `{"id","name","added":[appids],"removed":[appids]}`; dynamic collections also carry a `filterSpec`.
//! The ids `favorite` and `hidden` are the special lists. We never write this file.

use crate::local;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// SteamID64 of account number 0 of the public universe; subtract it to get the folder name in `userdata`.
const ID64_BASE: u64 = 76_561_197_960_265_728;
/// More than any real library has; keeps a damaged file from eating memory.
const MAX_COLLECTIONS: usize = 2_000;
const MAX_APPS: usize = 200_000;

/// One of Steam's own collections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamCollection {
    pub id: String,
    pub name: String,
    /// Games added by hand (for a dynamic collection these come on top of its filter).
    pub apps: HashSet<u32>,
    /// It has a `filterSpec`, which we do not evaluate: only the hand-added games are listed.
    pub dynamic: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamCollections {
    pub favorites: HashSet<u32>,
    pub hidden: HashSet<u32>,
    pub collections: Vec<SteamCollection>,
}

/// The folder name of an account inside `userdata`.
pub fn account_id(steam_id64: u64) -> Option<u32> {
    steam_id64.checked_sub(ID64_BASE).filter(|&a| a > 0).and_then(|a| u32::try_from(a).ok())
}

/// The collections file of `steam_id64` in the first Steam root that has one.
pub fn find_file(steam_id64: u64) -> Option<PathBuf> {
    let id = account_id(steam_id64)?;
    local::steam_roots().iter().map(|r| file_in(r, id)).find(|p| p.is_file())
}

fn file_in(root: &Path, account: u32) -> PathBuf {
    root.join("userdata").join(account.to_string()).join("config/cloudstorage/cloud-storage-namespace-1.json")
}

/// Load the collections of the signed-in account (empty when Steam has none).
pub fn load(steam_id64: u64) -> SteamCollections {
    find_file(steam_id64).and_then(|p| std::fs::read_to_string(p).ok()).map(|s| parse(&s)).unwrap_or_default()
}

fn app_ids(v: Option<&Value>) -> HashSet<u32> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|n| u32::try_from(n).ok()).take(MAX_APPS).collect())
        .unwrap_or_default()
}

/// Parse the file's text. Defensive: anything unexpected is skipped, never an error.
pub fn parse(src: &str) -> SteamCollections {
    let mut out = SteamCollections::default();
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(src) else { return out };
    for entry in entries {
        let Some(pair) = entry.as_array() else { continue };
        let (Some(key), Some(body)) = (pair.first().and_then(Value::as_str), pair.get(1)) else { continue };
        let Some(id) = key.strip_prefix("user-collections.") else { continue };
        if body.get("is_deleted").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        // `value` is a JSON document inside a string; tolerate a plain object too.
        let value = match body.get("value") {
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok(),
            Some(v @ Value::Object(_)) => Some(v.clone()),
            _ => None,
        };
        let Some(value) = value else { continue };
        let removed = app_ids(value.get("removed"));
        let mut apps = app_ids(value.get("added"));
        apps.retain(|a| !removed.contains(a));
        match id {
            "favorite" => out.favorites = apps,
            "hidden" => out.hidden = apps,
            _ if out.collections.len() < MAX_COLLECTIONS => {
                let name = value.get("name").and_then(Value::as_str).unwrap_or("").trim();
                if name.is_empty() {
                    continue;
                }
                let dynamic = value.get("filterSpec").is_some_and(|f| !f.is_null());
                out.collections.push(SteamCollection { id: id.to_string(), name: name.to_string(), apps, dynamic });
            }
            _ => {}
        }
    }
    out.collections.sort_by_key(|c| c.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      ["user-collections.favorite", {"key":"user-collections.favorite","timestamp":1,
        "value":"{\"id\":\"favorite\",\"name\":\"Favorites\",\"added\":[10,20,30],\"removed\":[30]}"}],
      ["user-collections.hidden", {"key":"user-collections.hidden","timestamp":1,
        "value":"{\"id\":\"hidden\",\"name\":\"Hidden\",\"added\":[99],\"removed\":[]}"}],
      ["user-collections.uc-aaa", {"key":"user-collections.uc-aaa","timestamp":2,
        "value":"{\"id\":\"uc-aaa\",\"name\":\"Zombies\",\"added\":[1,2,3],\"removed\":[]}"}],
      ["user-collections.uc-bbb", {"key":"user-collections.uc-bbb","timestamp":2,
        "value":"{\"id\":\"uc-bbb\",\"name\":\"Cozy\",\"added\":[5],\"removed\":[],\"filterSpec\":{\"nFormatVersion\":2}}"}],
      ["user-collections.uc-gone", {"is_deleted":true,"value":"{\"id\":\"uc-gone\",\"name\":\"Old\",\"added\":[7]}"}],
      ["user-collections.uc-bad", {"value":"not json"}],
      ["user-collections.uc-noname", {"value":"{\"added\":[1]}"}],
      ["something.else", {"value":"{}"}],
      42,
      ["user-collections.uc-neg", {"value":"{\"name\":\"Odd\",\"added\":[-1, 4294967296, \"x\", 8]}"}]
    ]"#;

    #[test]
    fn parses_sample() {
        let c = parse(SAMPLE);
        assert_eq!(c.favorites, HashSet::from([10, 20]));
        assert_eq!(c.hidden, HashSet::from([99]));
        let names: Vec<_> = c.collections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Cozy", "Odd", "Zombies"]);
        assert!(c.collections[0].dynamic && !c.collections[2].dynamic);
        assert_eq!(c.collections[1].apps, HashSet::from([8]));
        assert_eq!(c.collections[2].apps, HashSet::from([1, 2, 3]));
    }

    #[test]
    fn garbage_gives_empty() {
        for s in ["", "{}", "null", "[1,2]", "[[\"user-collections.x\"]]", "\u{0}"] {
            assert_eq!(parse(s), SteamCollections::default());
        }
    }

    #[test]
    fn account_id_maps() {
        assert_eq!(account_id(76_561_197_960_265_728 + 12345), Some(12345));
        assert_eq!(account_id(0), None);
    }
}
