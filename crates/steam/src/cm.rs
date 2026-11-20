//! Connection to Steam's CM servers (the same servers the official client
//! uses), authenticated with the refresh token from the QR login.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::time::Duration;
use steam_vent::{Connection, ConnectionTrait, NetMessage, ServerList};
use steam_vent_proto_steam::steammessages_clientserver::CMsgClientLicenseList;

pub struct Cm {
    pub conn: Connection,
    /// Package ids + PICS access tokens of every license on the account.
    pub licenses: Vec<(u32, u64)>,
}

pub async fn connect(account: &str, refresh_token: &str) -> Result<Cm> {
    let servers = ServerList::discover().await.context("CM server discovery")?;
    let conn = Connection::access(&servers, account, refresh_token)
        .await
        .context("CM logon")
        .map_err(|e| crate::redact::safe_error(&e, &[refresh_token]))?;
    // The license list is pushed right after logon; it may already be buffered.
    let mut lic = conn
        .take_unprocessed()
        .into_iter()
        .find(|m| m.kind == CMsgClientLicenseList::KIND)
        .and_then(|m| m.into_message::<CMsgClientLicenseList>().ok());
    if lic.is_none() {
        let mut s = Box::pin(conn.on::<CMsgClientLicenseList>());
        lic = tokio::time::timeout(Duration::from_secs(15), s.next()).await.ok().flatten().and_then(|r| r.ok());
    }
    let licenses = lic
        .map(|l| l.licenses.iter().filter_map(|x| Some((x.package_id?, x.access_token.unwrap_or(0)))).collect())
        .unwrap_or_default();
    Ok(Cm { conn, licenses })
}
