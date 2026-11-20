//! QR-code login through Steam's IAuthenticationService (HTTPS Web API).
//! The user approves in the Steam mobile app; we never see a password.
//! Result: a long-lived refresh token (stored in the system keyring) and a
//! short-lived access token (kept in memory only).

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use prost::Message;
use std::time::Duration;

const API: &str = "https://api.steampowered.com";

/// EAuthTokenPlatformType
pub const PLATFORM_STEAM_CLIENT: i32 = 1;

#[derive(Clone, PartialEq, Message)]
struct DeviceDetails {
    #[prost(string, tag = "1")]
    device_friendly_name: String,
    #[prost(int32, tag = "2")]
    platform_type: i32,
    #[prost(int32, tag = "3")]
    os_type: i32,
}

#[derive(Clone, PartialEq, Message)]
struct BeginQrReq {
    #[prost(string, tag = "1")]
    device_friendly_name: String,
    #[prost(int32, tag = "2")]
    platform_type: i32,
    #[prost(message, optional, tag = "3")]
    device_details: Option<DeviceDetails>,
    #[prost(string, tag = "4")]
    website_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct BeginQrResp {
    #[prost(uint64, tag = "1")]
    client_id: u64,
    #[prost(string, tag = "2")]
    challenge_url: String,
    #[prost(bytes = "vec", tag = "3")]
    request_id: Vec<u8>,
    #[prost(float, tag = "4")]
    interval: f32,
}

#[derive(Clone, PartialEq, Message)]
struct PollReq {
    #[prost(uint64, tag = "1")]
    client_id: u64,
    #[prost(bytes = "vec", tag = "2")]
    request_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct PollResp {
    #[prost(uint64, tag = "1")]
    new_client_id: u64,
    #[prost(string, tag = "2")]
    new_challenge_url: String,
    #[prost(string, tag = "3")]
    refresh_token: String,
    #[prost(string, tag = "4")]
    access_token: String,
    #[prost(bool, tag = "5")]
    had_remote_interaction: bool,
    #[prost(string, tag = "6")]
    account_name: String,
}

#[derive(Clone, PartialEq, Message)]
struct RenewReq {
    #[prost(string, tag = "1")]
    refresh_token: String,
    #[prost(fixed64, tag = "2")]
    steamid: u64,
}

#[derive(Clone, PartialEq, Message)]
struct RenewResp {
    #[prost(string, tag = "1")]
    access_token: String,
}

/// Call an IAuthenticationService method with a protobuf body.
fn call<Q: Message, R: Message + Default>(method: &str, req: &Q) -> Result<R> {
    let body = STANDARD.encode(req.encode_to_vec());
    let mut resp = crate::http::agent()
        .post(&format!("{API}/IAuthenticationService/{method}/v1"))
        .send_form([("input_protobuf_encoded", body.as_str())])
        .with_context(|| format!("{method}: request failed"))?;
    let eresult = resp.headers().get("x-eresult").and_then(|v| v.to_str().ok()).unwrap_or("1").to_string();
    if eresult != "1" {
        bail!("{method}: Steam returned EResult {eresult}");
    }
    let bytes = resp.body_mut().read_to_vec()?;
    Ok(R::decode(bytes.as_slice())?)
}

pub struct QrSession {
    pub challenge_url: String,
    client_id: u64,
    request_id: Vec<u8>,
    pub interval: Duration,
}

#[derive(Debug)]
pub enum Poll {
    Waiting,
    /// The phone scanned the code; waiting for "Approve".
    Scanned,
    /// The code rotated; show the new URL.
    NewChallenge(String),
    Done(Tokens),
}

pub struct Tokens {
    pub account_name: String,
    pub steamid: u64,
    pub refresh_token: String,
    pub access_token: String,
}

impl Drop for Tokens {
    /// Wipe tokens from memory once they've been moved to the keyring/session.
    fn drop(&mut self) {
        for t in [&mut self.refresh_token, &mut self.access_token] {
            // SAFETY: zero bytes are valid UTF-8.
            unsafe { t.as_bytes_mut() }.fill(0);
        }
    }
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print tokens.
        f.debug_struct("Tokens")
            .field("account_name", &self.account_name)
            .field("steamid", &self.steamid)
            .finish_non_exhaustive()
    }
}

pub fn begin_qr(device_name: &str) -> Result<QrSession> {
    let req = BeginQrReq {
        device_friendly_name: device_name.into(),
        platform_type: PLATFORM_STEAM_CLIENT,
        device_details: Some(DeviceDetails {
            device_friendly_name: device_name.into(),
            platform_type: PLATFORM_STEAM_CLIENT,
            os_type: -203, /* Linux */
        }),
        website_id: "Client".into(),
    };
    let r: BeginQrResp = call("BeginAuthSessionViaQR", &req)?;
    Ok(QrSession {
        challenge_url: r.challenge_url,
        client_id: r.client_id,
        request_id: r.request_id,
        interval: Duration::from_secs_f32(r.interval.clamp(1.0, 10.0)),
    })
}

impl QrSession {
    pub fn poll(&mut self) -> Result<Poll> {
        let r: PollResp =
            call("PollAuthSessionStatus", &PollReq { client_id: self.client_id, request_id: self.request_id.clone() })?;
        if !r.refresh_token.is_empty() {
            let steamid = jwt_steamid(&r.refresh_token).context("refresh token has no steamid")?;
            return Ok(Poll::Done(Tokens {
                account_name: r.account_name,
                steamid,
                refresh_token: r.refresh_token,
                access_token: r.access_token,
            }));
        }
        if r.new_client_id != 0 && !r.new_challenge_url.is_empty() {
            self.client_id = r.new_client_id;
            self.challenge_url = r.new_challenge_url.clone();
            return Ok(Poll::NewChallenge(r.new_challenge_url));
        }
        Ok(if r.had_remote_interaction { Poll::Scanned } else { Poll::Waiting })
    }
}

/// Get a fresh access token from a stored refresh token.
pub fn renew_access_token(refresh_token: &str) -> Result<String> {
    let steamid = jwt_steamid(refresh_token).context("bad refresh token")?;
    let r: RenewResp = call("GenerateAccessTokenForApp", &RenewReq { refresh_token: refresh_token.into(), steamid })
        .map_err(|e| crate::redact::safe_error(&e, &[refresh_token]))?;
    if r.access_token.is_empty() {
        bail!("no access token returned");
    }
    Ok(r.access_token)
}

#[derive(Clone, PartialEq, Message)]
struct RevokeReq {
    #[prost(string, tag = "1")]
    refresh_token: String,
    #[prost(fixed64, tag = "2")]
    steamid: u64,
    /// EAuthTokenRevokeAction: 1 = permanent.
    #[prost(int32, tag = "3")]
    revoke_action: i32,
}

/// Invalidate a refresh token on Steam's side (used on sign-out), so a copy
/// that leaked anywhere stops working.
pub fn revoke_refresh_token(refresh_token: &str) -> Result<()> {
    let steamid = jwt_steamid(refresh_token).context("bad refresh token")?;
    let access = renew_access_token(refresh_token)?;
    let req = RevokeReq { refresh_token: refresh_token.into(), steamid, revoke_action: 1 };
    let body = STANDARD.encode(req.encode_to_vec());
    // Token in the POST body, not the URL, so it can't end up in error text.
    let url = format!("{API}/IAuthenticationService/RevokeRefreshToken/v1");
    let r = crate::http::agent()
        .post(&url)
        .send_form([("access_token", access.as_str()), ("input_protobuf_encoded", body.as_str())]);
    let res: Result<()> = match r {
        Ok(resp) => match resp.headers().get("x-eresult").and_then(|v| v.to_str().ok()) {
            Some("1") | None => Ok(()),
            Some(code) => Err(anyhow::anyhow!("RevokeRefreshToken: EResult {code}")),
        },
        Err(e) => Err(e.into()),
    };
    res.map_err(|e| crate::redact::safe_error(&e, &[refresh_token, &access]))
}

fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()
}

pub fn jwt_steamid(token: &str) -> Option<u64> {
    jwt_claims(token)?.get("sub")?.as_str()?.parse().ok()
}

/// Expiry (unix seconds) of a JWT token.
pub fn jwt_expiry(token: &str) -> Option<u64> {
    jwt_claims(token)?.get("exp")?.as_u64()
}

/// Render a QR code as SVG (for the CLI test; the app draws it natively).
pub fn qr_svg(data: &str) -> Result<String> {
    let code = qrcode::QrCode::new(data.as_bytes()).map_err(|e| anyhow::anyhow!("QR: {e:?}"))?;
    Ok(code.render::<qrcode::render::svg::Color>().min_dimensions(320, 320).quiet_zone(true).build())
}

/// QR modules as a bool grid (true = dark), for native drawing.
pub fn qr_matrix(data: &str) -> Result<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(data.as_bytes()).map_err(|e| anyhow::anyhow!("QR: {e:?}"))?;
    let w = code.width();
    Ok((w, code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect()))
}
