//! Talking to Steam's content system: depot keys, manifest request codes, the
//! CDN server list, and plain HTTPS downloads from those servers with
//! rotation when one misbehaves.

use super::CmLink;
use super::crypto::DepotKey;
use anyhow::{Context, Result, bail};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;
use steam_vent_proto_steam::steammessages_clientserver_2::{
    CMsgClientGetDepotDecryptionKey, CMsgClientGetDepotDecryptionKeyResponse,
};
use steam_vent_proto_steam::steammessages_contentsystem_steamclient::{
    CContentServerDirectory_GetManifestRequestCode_Request, CContentServerDirectory_GetServersForSteamPipe_Request,
};

const EUNLIMITED_OK: i32 = 1;

/// The depot key, or `None` when this account may not download the depot.
pub async fn depot_key<C: CmLink>(conn: &C, app: u32, depot: u32) -> Result<Option<DepotKey>> {
    let resp: CMsgClientGetDepotDecryptionKeyResponse = conn
        .job(CMsgClientGetDepotDecryptionKey { depot_id: Some(depot), app_id: Some(app), ..Default::default() })
        .await?;
    if resp.eresult() != EUNLIMITED_OK {
        return Ok(None);
    }
    let key = resp.depot_encryption_key().try_into().map_err(|_| anyhow::anyhow!("depot key is not 32 bytes"))?;
    Ok(Some(key))
}

/// The one-time code the CDN wants in a manifest URL (0 when Steam gives none).
pub async fn manifest_request_code<C: CmLink>(conn: &C, app: u32, depot: u32, gid: u64, branch: &str) -> Result<u64> {
    let req = CContentServerDirectory_GetManifestRequestCode_Request {
        app_id: Some(app),
        depot_id: Some(depot),
        manifest_id: Some(gid),
        app_branch: (branch != "public").then(|| branch.to_string()),
        ..Default::default()
    };
    Ok(conn.service_method(req).await?.manifest_request_code())
}

#[derive(Debug)]
pub struct Server {
    pub host: String,
    failures: AtomicU32,
}

/// Download pool over a list of CDN servers.
pub struct Cdn {
    servers: Vec<Server>,
    cursor: AtomicUsize,
    agent: ureq::Agent,
}

/// Servers Steam suggests for this connection's cell, best first: Steam's own
/// caches before third-party CDNs, then by load; HTTPS-capable only.
pub async fn servers<C: CmLink>(conn: &C) -> Result<Cdn> {
    let req = CContentServerDirectory_GetServersForSteamPipe_Request {
        cell_id: Some(conn.cell_id()),
        max_servers: Some(20),
        ..Default::default()
    };
    let mut list: Vec<_> = conn
        .service_method(req)
        .await
        .context("CDN server list")?
        .servers
        .into_iter()
        .filter(|s| matches!(s.type_(), "SteamCache" | "CDN") && s.https_support() != "none" && !s.host().is_empty())
        .filter(|s| !s.steam_china_only())
        .collect();
    list.sort_by(|a, b| {
        let rank = |t: &str| u8::from(t != "SteamCache");
        (rank(a.type_()), a.weighted_load())
            .partial_cmp(&(rank(b.type_()), b.weighted_load()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if list.is_empty() {
        bail!("Steam returned no usable content servers");
    }
    Ok(Cdn::new(list.iter().map(|s| s.host().to_string()).collect()))
}

impl Cdn {
    pub fn new(hosts: Vec<String>) -> Self {
        // HTTPS only and no redirects, like `http::agent`, but with room for big chunks and slow links.
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(true)
            .build()
            .into();
        let servers = hosts.into_iter().map(|host| Server { host, failures: AtomicU32::new(0) }).collect();
        Cdn { servers, cursor: AtomicUsize::new(0), agent }
    }

    pub fn hosts(&self) -> Vec<&str> {
        self.servers.iter().map(|s| s.host.as_str()).collect()
    }

    /// GET `path` (starting with `/`), trying servers in turn. Servers that
    /// failed repeatedly are tried last, so one dead host doesn't slow every request.
    pub fn get(&self, path: &str, max_size: u64) -> Result<Vec<u8>> {
        let n = self.servers.len();
        let start = self.cursor.load(Ordering::Relaxed);
        let mut order: Vec<usize> = (0..n).map(|i| (start + i) % n).collect();
        order.sort_by_key(|&i| self.servers[i].failures.load(Ordering::Relaxed).min(3));
        let mut last = None;
        for i in order.into_iter().take(3) {
            let s = &self.servers[i];
            match self.fetch(&s.host, path, max_size) {
                Ok(body) => {
                    s.failures.store(0, Ordering::Relaxed);
                    return Ok(body);
                }
                Err(e) => {
                    s.failures.fetch_add(1, Ordering::Relaxed);
                    self.cursor.store((i + 1) % n, Ordering::Relaxed);
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("no servers")))
    }

    fn fetch(&self, host: &str, path: &str, max_size: u64) -> Result<Vec<u8>> {
        let url = format!("https://{host}{path}");
        let mut resp = self.agent.get(&url).call().with_context(|| format!("GET {host}{path}"))?;
        Ok(resp.body_mut().with_config().limit(max_size).read_to_vec()?)
    }
}
