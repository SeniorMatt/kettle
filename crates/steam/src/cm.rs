//! Connection to Steam's CM servers (the same servers the official client
//! uses), authenticated with the refresh token from the QR login.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::time::Duration;
use steam_vent::{Connection, ConnectionTrait, NetMessage, RawNetMessage, ServerList};
use steam_vent_proto_steam::steammessages_auth_steamclient::CAuthentication_AccessToken_GenerateForApp_Request;
use steam_vent_proto_steam::steammessages_clientserver::CMsgClientLicenseList;
use steam_vent_proto_steam::steammessages_clientserver_login::{CMsgClientHeartBeat, CMsgClientLoggedOff};
use steam_vent_proto_steam::steammessages_player_steamclient::{
    CPlayer_GetOwnedGames_Request, CPlayer_GetOwnedGames_Response,
};

use crate::webapi::OwnedGame;

/// How long server discovery and the logon may each take before we give up (and retry later).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);
/// The owned-games answer of a big library is large.
const OWNED_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Clone)]
pub struct Cm {
    pub conn: Connection,
    /// Package ids + PICS access tokens of every license on the account.
    pub licenses: Vec<(u32, u64)>,
    /// Other messages that arrived right after logon and had no listener yet (the friends list, persona
    /// states). The friends session replays them.
    pub early: Vec<RawNetMessage>,
}

pub async fn connect(account: &str, refresh_token: &str) -> Result<Cm> {
    let servers = tokio::time::timeout(CONNECT_TIMEOUT, ServerList::discover())
        .await
        .map_err(|_| anyhow::anyhow!("CM server discovery timed out"))?
        .context("CM server discovery")?;
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, Connection::access(&servers, account, refresh_token))
        .await
        .map_err(|_| anyhow::anyhow!("CM logon timed out"))?
        .context("CM logon")
        .map_err(|e| crate::redact::safe_error(&e, &[refresh_token]))?;
    // The license list is pushed right after logon; it may already be buffered.
    let mut early = Vec::new();
    let mut lic = None;
    for m in conn.take_unprocessed() {
        if lic.is_none() && m.kind == CMsgClientLicenseList::KIND {
            lic = m.into_message::<CMsgClientLicenseList>().ok();
        } else {
            early.push(m);
        }
    }
    if lic.is_none() {
        let mut s = Box::pin(conn.on::<CMsgClientLicenseList>());
        lic = tokio::time::timeout(Duration::from_secs(15), s.next()).await.ok().flatten().and_then(|r| r.ok());
    }
    let licenses = lic
        .map(|l| l.licenses.iter().filter_map(|x| Some((x.package_id?, x.access_token.unwrap_or(0)))).collect())
        .unwrap_or_default();
    Ok(Cm { conn, licenses, early })
}

/// The games the account owns, asked over the logged-in CM connection (`Player.GetOwnedGames#1`).
/// Tokens of the "SteamClient" kind can't use the Web API for this, so the CM is the only way.
pub async fn owned_games(conn: &Connection, steamid: u64) -> Result<Vec<OwnedGame>> {
    let req = CPlayer_GetOwnedGames_Request {
        steamid: Some(steamid),
        include_appinfo: Some(true),
        include_played_free_games: Some(true),
        skip_unvetted_apps: Some(false),
        language: Some("english".into()),
        ..Default::default()
    };
    let mut conn = conn.clone();
    conn.set_timeout(OWNED_TIMEOUT);
    let resp = conn.service_method(req).await.context("Player.GetOwnedGames over the CM")?;
    Ok(map_owned_games(&resp))
}

/// Turn the protobuf answer into our model. Entries without an app id are dropped.
pub fn map_owned_games(resp: &CPlayer_GetOwnedGames_Response) -> Vec<OwnedGame> {
    resp.games
        .iter()
        .filter_map(|g| {
            let appid = u32::try_from(g.appid?).ok()?;
            Some(OwnedGame {
                appid,
                name: g.name.clone().unwrap_or_default(),
                playtime_forever: g.playtime_forever.and_then(|m| u32::try_from(m).ok()).unwrap_or(0),
                rtime_last_played: u64::from(g.rtime_last_played.unwrap_or(0)),
                img_icon_url: g.img_icon_url.clone().unwrap_or_default(),
            })
        })
        .collect()
}

/// A fresh access token for Web API calls. For refresh tokens of the "SteamClient" kind Steam rejects
/// the plain HTTPS `GenerateAccessTokenForApp` (EResult 15), so it is sent as the unified message
/// `Authentication.GenerateAccessTokenForApp#1` over an authenticated CM connection.
pub async fn generate_access_token(conn: &Connection, refresh_token: &str, steamid: u64) -> Result<String> {
    let req = CAuthentication_AccessToken_GenerateForApp_Request {
        refresh_token: Some(refresh_token.to_string()),
        steamid: Some(steamid),
        ..Default::default()
    };
    let resp = conn
        .service_method(req)
        .await
        .context("Authentication.GenerateAccessTokenForApp over the CM")
        .map_err(|e| crate::redact::safe_error(&e, &[refresh_token]))?;
    match resp.access_token {
        Some(t) if !t.is_empty() => Ok(t),
        _ => anyhow::bail!("Steam returned no access token"),
    }
}

/// How often a live connection is probed with a heartbeat, to notice a link that died silently.
const PROBE_EVERY: Duration = Duration::from_secs(30);

/// Returns why the connection ended: Steam logged us off, the stream closed or a heartbeat could not be sent.
pub async fn until_closed(conn: &Connection) -> String {
    let mut off = Box::pin(conn.on::<CMsgClientLoggedOff>());
    let mut probe = tokio::time::interval(PROBE_EVERY);
    probe.tick().await;
    loop {
        tokio::select! {
            m = off.next() => return match m {
                Some(Ok(m)) => logged_off_text(m.eresult()),
                _ => "The connection to Steam closed".into(),
            },
            _ = probe.tick() => {
                if conn.send(CMsgClientHeartBeat::default()).await.is_err() {
                    return "The connection to Steam was lost".into();
                }
            }
        }
    }
}

/// `EResult::LogonSessionReplaced` is 34.
fn logged_off_text(eresult: i32) -> String {
    match eresult {
        34 => "Steam ended this session because the account signed in somewhere else (EResult 34)".into(),
        n => format!("Steam logged this session off (EResult {n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_vent_proto_steam::steammessages_player_steamclient::cplayer_get_owned_games_response::Game;

    #[test]
    fn logoff_text_names_the_replaced_session() {
        assert!(logged_off_text(34).contains("somewhere else"));
        assert!(logged_off_text(5).contains("EResult 5"));
    }

    #[test]
    fn maps_owned_games() {
        let mut resp = CPlayer_GetOwnedGames_Response::default();
        resp.games.push(Game {
            appid: Some(440),
            name: Some("Team Fortress 2".into()),
            playtime_forever: Some(120),
            rtime_last_played: Some(1_700_000_000),
            img_icon_url: Some("abc".into()),
            ..Default::default()
        });
        resp.games.push(Game { appid: Some(10), ..Default::default() });
        resp.games.push(Game { appid: Some(-1), ..Default::default() });
        resp.games.push(Game::default());
        let out = map_owned_games(&resp);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].appid, out[0].name.as_str(), out[0].playtime_forever), (440, "Team Fortress 2", 120));
        assert_eq!((out[0].rtime_last_played, out[0].img_icon_url.as_str()), (1_700_000_000, "abc"));
        assert_eq!((out[1].appid, out[1].name.as_str(), out[1].playtime_forever), (10, "", 0));
    }
}
