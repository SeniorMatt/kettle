//! Data types of the owned-games answer. The list itself comes over the CM (see `cm::owned_games`): refresh
//! tokens of the "SteamClient" kind are refused by the plain Web API.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedGame {
    pub appid: u32,
    #[serde(default)]
    pub name: String,
    /// Minutes.
    #[serde(default)]
    pub playtime_forever: u32,
    #[serde(default)]
    pub rtime_last_played: u64,
    #[serde(default)]
    pub img_icon_url: String,
}
