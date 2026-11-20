//! Steam Web API calls authorised with the user's access token.

use anyhow::{Context, Result};
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

#[derive(Deserialize)]
struct Wrap {
    response: OwnedResp,
}

#[derive(Deserialize)]
struct OwnedResp {
    #[serde(default)]
    games: Vec<OwnedGame>,
}

pub fn owned_games(access_token: &str, steamid: u64) -> Result<Vec<OwnedGame>> {
    let url = format!(
        "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?access_token={access_token}&steamid={steamid}\
         &include_appinfo=true&include_played_free_games=true&skip_unvetted_apps=false"
    );
    let w: Wrap = ureq::get(&url).call().context("GetOwnedGames")?.body_mut().read_json()?;
    Ok(w.response.games)
}
