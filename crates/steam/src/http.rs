//! The HTTP client used for every request that carries a token or login data.
//!
//! HTTPS only and no redirects, so a token can never be sent in clear text or
//! forwarded to another host; a global timeout so a stalled request can't hang
//! the login.

use std::sync::OnceLock;
use std::time::Duration;

pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}
