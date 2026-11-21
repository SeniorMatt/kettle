//! HTTP client for public downloads (GitHub API and release assets).
//!
//! Unlike [`crate::http`], this one follows redirects, because release assets are served from a CDN behind a
//! redirect. It never carries credentials, stays HTTPS-only (also across redirects) and sends a generic
//! User-Agent.

use anyhow::{Context, Result};
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(5)
            .user_agent("Kettle")
            // No global timeout: a download of hundreds of MB is legitimately slow. Connecting and the
            // response headers must still arrive promptly.
            .timeout_global(None)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

/// GET a small text file, refusing bodies over `limit` bytes.
pub fn get_text(url: &str, limit: u64) -> Result<String> {
    let mut resp = agent().get(url).call().with_context(|| format!("fetching {url}"))?;
    let mut text = String::new();
    resp.body_mut().as_reader().take(limit + 1).read_to_string(&mut text)?;
    anyhow::ensure!(text.len() as u64 <= limit, "{url} is larger than expected");
    Ok(text)
}

/// Like [`get_text`], but a 404 answer is `Ok(None)` (the site has nothing for this address).
pub fn get_text_or_404(url: &str, limit: u64) -> Result<Option<String>> {
    let mut resp = match agent().get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context(format!("fetching {url}"))),
    };
    let mut text = String::new();
    resp.body_mut().as_reader().take(limit + 1).read_to_string(&mut text)?;
    anyhow::ensure!(text.len() as u64 <= limit, "{url} is larger than expected");
    Ok(Some(text))
}
