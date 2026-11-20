//! Connection to Steam's CM servers (the same servers the official client
//! uses), authenticated with the refresh token from the QR login.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::time::Duration;
use steam_vent::{Connection, ConnectionTrait, NetMessage, RawNetMessage, ServerList};
use steam_vent_proto_steam::steammessages_clientserver::CMsgClientLicenseList;

pub struct Cm {
    pub conn: Connection,
    /// Package ids + PICS access tokens of every license on the account.
    pub licenses: Vec<(u32, u64)>,
    /// Other messages that arrived right after logon and had no listener yet (the friends list, persona
    /// states). The friends session replays them.
    pub early: Vec<RawNetMessage>,
}

pub async fn connect(account: &str, refresh_token: &str) -> Result<Cm> {
    let servers = ServerList::discover().await.context("CM server discovery")?;
    let conn = Connection::access(&servers, account, refresh_token)
        .await
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
