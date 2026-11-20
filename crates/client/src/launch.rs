//! The fallback launch path: hand the game to the official client through a `steam://` URI.
//! Our own launcher lives in `play.rs`; this stays as the "Launch via Steam client" menu item.

use anyhow::{Context, Result};
use sr_model::AppId;

pub fn play(app: AppId) -> Result<()> {
    let uri = format!("steam://rungameid/{app}");
    std::process::Command::new("xdg-open").arg(&uri).spawn().with_context(|| format!("couldn't open {uri}"))?;
    Ok(())
}
