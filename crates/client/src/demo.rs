//! Demo library for screenshots and performance tests without a login:
//! every app id the official client has cached art for, plus installed games,
//! repeated until there are `target` entries. Enabled by `STEAM_RUST_DEMO=<n>`.

use sr_model::{AppId, Game};
use sr_steam::local;

pub fn games(target: usize) -> Vec<Game> {
    let installed = local::installed_apps();
    let mut ids: Vec<u32> = Vec::new();
    for root in local::steam_roots() {
        if let Ok(rd) = std::fs::read_dir(root.join("appcache/librarycache")) {
            ids.extend(rd.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()));
        }
    }
    ids.extend(installed.keys().copied());
    ids.retain(|id| !installed.get(id).is_some_and(|a| local::is_tool_app(a.appid, &a.name)));
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(target);
    for i in 0..target.max(ids.len()) {
        let id = ids[i % ids.len()];
        let lap = i / ids.len();
        let base = installed.get(&id).map(|a| a.name.clone()).unwrap_or_else(|| format!("App {id}"));
        let name = if lap == 0 { base } else { format!("{base} ({})", lap + 1) };
        let mut g = Game::new(AppId(id), name);
        g.playtime_minutes = (id % 5000) * (lap as u32 + 1);
        g.last_played = 1_700_000_000 + u64::from(id % 30_000_000);
        out.push(g);
    }
    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    out
}
