//! Play / install.
//!
//! Interim: until our own downloader (phase 4) and launcher (phase 5) exist,
//! these hand the action to the official client through `steam://` URIs.

use anyhow::{Context, Result};
use sr_model::AppId;

fn open_uri(uri: &str) -> Result<()> {
    std::process::Command::new("xdg-open").arg(uri).spawn().with_context(|| format!("couldn't open {uri}"))?;
    Ok(())
}

pub fn play(app: AppId) -> Result<()> {
    open_uri(&format!("steam://rungameid/{app}"))
}

pub fn install(app: AppId) -> Result<()> {
    open_uri(&format!("steam://install/{app}"))
}
