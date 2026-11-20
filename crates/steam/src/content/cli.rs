//! `sr-cli download` — a command-line front end to the downloader.

use super::{Auth, Download, Options, Os, Phase, default_library};
use crate::secrets;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

const USAGE: &str = "usage: sr-cli download <appid> [--dir <library>] [--account <name> | --anonymous] \
[--os linux|windows|macos] [--language english] [--branch public] [--verify] [--workers 8]";

/// Run `sr-cli download ...`; `args` are the words after `download`.
pub fn run(args: &[String]) -> Result<()> {
    let mut it = args.iter();
    let app: u32 = it.next().context(USAGE)?.parse().context("<appid> must be a number")?;
    let mut opts = Options::new(app, default_library());
    let mut account: Option<String> = None;
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--dir" => opts.library = PathBuf::from(value()?),
            "--account" => account = Some(value()?),
            "--anonymous" => account = None,
            "--os" => opts.os = Some(value()?.parse::<Os>().map_err(anyhow::Error::msg)?),
            "--language" => opts.language = value()?,
            "--branch" => opts.branch = value()?,
            "--workers" => opts.workers = value()?.parse().context("--workers must be a number")?,
            "--verify" => opts.verify = true,
            _ => bail!("{USAGE}"),
        }
    }
    let auth = match account {
        None => Auth::Anonymous,
        Some(name) => {
            let refresh_token = secrets::load_refresh_token(&name)?.context("no token stored; run `sr-cli login`")?;
            Auth::Account { name, refresh_token }
        }
    };
    let dl = Download::start(opts, auth);
    print_progress(&dl);
    let s = dl.join()?;
    for w in &s.warnings {
        eprintln!("warning: {w}");
    }
    let mb = s.stats.bytes_downloaded as f64 / 1e6;
    if s.up_to_date {
        println!("{} ({}) is up to date in {:.1}s", s.app.name, s.app.appid, s.seconds);
        return Ok(());
    }
    println!(
        "{} ({}) for {}: {} files, {} bytes installed, {} bytes downloaded ({} chunks, {} reused), {} files removed, \
         {:.1}s, {:.2} MB/s -> {}",
        s.app.name,
        s.app.appid,
        s.os.as_str(),
        s.stats.files,
        s.stats.bytes,
        s.stats.bytes_downloaded,
        s.stats.chunks_downloaded,
        s.stats.chunks_reused,
        s.stats.files_removed,
        s.seconds,
        mb / s.seconds.max(0.001),
        s.install_dir.display()
    );
    Ok(())
}

/// One updating line until the download thread ends.
fn print_progress(dl: &Download) {
    let mut last = None;
    while !dl.is_finished() {
        // Keep only the newest update; an empty channel just means "wait a little".
        match dl.progress.recv_timeout(Duration::from_millis(200)) {
            Ok(p) => last = Some(p),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(p) = dl.progress.try_recv() {
            last = Some(p);
        }
        if let Some(p) = &last {
            let name = match p.phase {
                Phase::Preparing => "preparing",
                Phase::Downloading => "downloading",
                Phase::Verifying => "verifying",
                Phase::Finished => "finished",
                Phase::Failed => "failed",
            };
            let pct = if p.total_bytes == 0 { 0.0 } else { p.done_bytes as f64 * 100.0 / p.total_bytes as f64 };
            print!(
                "\r{name:<12} {:>9.1} / {:.1} MB {pct:>5.1}%  {:.2} MB/s   ",
                p.done_bytes as f64 / 1e6,
                p.total_bytes as f64 / 1e6,
                p.bytes_per_sec / 1e6
            );
            let _ = std::io::stdout().flush();
        }
    }
    println!();
}
