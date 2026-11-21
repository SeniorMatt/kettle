//! Developer CLI for the core crate (no GUI).
//!   sr-cli local            installed games found in existing Steam installs
//!   sr-cli login            QR login; writes the QR to ~/.cache/kettle/qr.svg
//!   sr-cli owned <account>  owned games via Web API (token from keyring)
//!   sr-cli cm <account>     CM logon + license count
//!   sr-cli logout <account> revoke and remove the stored token
//!   sr-cli download <appid> [--dir D] [--anonymous|--account A] [--os linux|windows] [--verify]
//!   sr-cli proton list|available|install|remove, sr-cli appinfo <appid>, sr-cli launch <appid> --dry-run

use anyhow::{Context, Result, bail};
use sr_steam::{auth, cm, local, secrets, store};
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let account = || args.get(1).cloned().context("missing <account>");
    match args.first().map(String::as_str) {
        Some("local") => cmd_local(),
        Some("login") => cmd_login(),
        Some("owned") => cmd_owned(&account()?),
        Some("cm") => cmd_cm(&account()?),
        Some("logout") => cmd_logout(&account()?),
        Some("download") => sr_steam::content::cli::run(&args[1..]),
        Some("proton" | "appinfo" | "launch") => sr_steam::compat_cli::run(&args),
        _ => bail!(
            "usage: sr-cli local | login | owned <account> | cm <account> | logout <account> | download <appid> ..."
        ),
    }
}

fn cmd_logout(account: &str) -> Result<()> {
    secrets::delete_refresh_token(account)?;
    println!("Signed out {account}");
    Ok(())
}

fn cmd_local() -> Result<()> {
    let t = Instant::now();
    let roots = local::steam_roots();
    let apps = local::installed_apps();
    let games = apps.values().filter(|a| !local::is_tool_app(a.appid, &a.name)).count();
    println!("roots: {roots:?}");
    println!("{} manifests ({games} games) in {:.1} ms", apps.len(), t.elapsed().as_secs_f64() * 1e3);
    Ok(())
}

fn cmd_login() -> Result<()> {
    let mut s = auth::begin_qr(auth::DEVICE_NAME)?;
    let svg_path = store::cache_dir().join("qr.svg");
    let write_qr = |url: &str| -> Result<()> {
        store::write_private(&svg_path, auth::qr_svg(url)?.as_bytes())?;
        println!("Scan the QR in {} with the Steam mobile app", svg_path.display());
        Ok(())
    };
    write_qr(&s.challenge_url)?;
    // Delete the QR file however this function ends.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(svg_path.clone());
    let deadline = Instant::now() + std::time::Duration::from_secs(600);
    while Instant::now() < deadline {
        std::thread::sleep(s.interval);
        match s.poll()? {
            auth::Poll::Waiting => {}
            auth::Poll::Scanned => println!("Scanned, approve on the phone..."),
            auth::Poll::NewChallenge(url) => write_qr(&url)?,
            auth::Poll::Done(t) => {
                secrets::save_refresh_token(&t.account_name, &t.refresh_token)?;
                println!("Logged in as {} ({}); token saved to the keyring", t.account_name, t.steamid);
                return Ok(());
            }
        }
    }
    bail!("QR login timed out")
}

fn token(account: &str) -> Result<String> {
    secrets::load_refresh_token(account)?.context("no token stored; run `sr-cli login` first")
}

fn cmd_owned(account: &str) -> Result<()> {
    let refresh = token(account)?;
    let steamid = auth::jwt_steamid(&refresh).context("bad token")?;
    let t = Instant::now();
    let rt = tokio::runtime::Runtime::new()?;
    let games = rt.block_on(async {
        let c = cm::connect(account, &refresh).await?;
        cm::owned_games(&c.conn, steamid).await
    })?;
    println!("{} owned games in {:.0} ms", games.len(), t.elapsed().as_secs_f64() * 1e3);
    Ok(())
}

fn cmd_cm(account: &str) -> Result<()> {
    let refresh = token(account)?;
    let rt = tokio::runtime::Runtime::new()?;
    let t = Instant::now();
    let c = rt.block_on(cm::connect(account, &refresh))?;
    println!("CM logon ok in {:.0} ms; {} licenses", t.elapsed().as_secs_f64() * 1e3, c.licenses.len());
    Ok(())
}
