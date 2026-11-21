//! `sr-cli proton ...`, `sr-cli appinfo ...` and `sr-cli launch ...`: developer commands for compatibility tools,
//! Steam's launch data and game launching.
//!
//!   sr-cli proton list [--dir D]                       installed tools (or those in one compatibilitytools.d)
//!   sr-cli proton available [source]                   downloadable releases (ge, cachyos, em, luxtorpeda, sarek)
//!   sr-cli proton install <source> <tag|latest> --dir D
//!   sr-cli proton remove <name> --dir D
//!   sr-cli appinfo <appid>                             launch entries from Steam's appinfo.vdf
//!   sr-cli launch <appid> [--dry-run] [--windows] [--tool NAME] [-- args...]

use crate::compat::{self, ToolOrigin};
use crate::proton_releases::{self as rel, Phase, Source};
use crate::run::{self, PrepareOpts};
use crate::{appinfo, launch_opts, local};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

pub fn run(args: &[String]) -> Result<()> {
    let rest = &args[1..];
    match args[0].as_str() {
        "proton" => match rest.first().map(String::as_str) {
            Some("list") => cmd_list(rest),
            Some("available") => cmd_available(rest),
            Some("install") => cmd_install(rest),
            Some("remove") => cmd_remove(rest),
            _ => bail!("usage: sr-cli proton list|available|install|remove ..."),
        },
        "appinfo" => cmd_appinfo(rest),
        "launch" => cmd_launch(rest),
        other => bail!("unknown command {other}"),
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn dir_arg(args: &[String]) -> Result<PathBuf> {
    flag_value(args, "--dir").map(PathBuf::from).context("missing --dir <compatibilitytools.d folder>")
}

fn appid_arg(args: &[String]) -> Result<u32> {
    args.first().and_then(|a| a.parse().ok()).context("missing <appid>")
}

fn cmd_list(args: &[String]) -> Result<()> {
    let tools = match flag_value(args, "--dir") {
        Some(d) => compat::scan_tools_dir(&PathBuf::from(d), ToolOrigin::Ours),
        None => compat::discover(),
    };
    for t in &tools {
        println!(
            "{:<32} {:<10} {:<12} {}",
            t.internal_name,
            format!("{:?}", t.kind),
            format!("{:?}", t.origin),
            t.dir.display()
        );
    }
    println!("{} tools", tools.len());
    if args.iter().all(|a| a != "--dir") {
        let map = compat::steam_mapping_all();
        println!("Steam's per-game choices: {} games ({} global default)", map.len(), map.get(&0).map_or("no", |n| n));
    }
    Ok(())
}

fn cmd_available(args: &[String]) -> Result<()> {
    let sources = match args.get(1) {
        Some(s) => vec![Source::from_id(s).with_context(|| format!("unknown source {s}"))?],
        None => Source::ALL.to_vec(),
    };
    println!("CPU level: {:?}", rel::cpu_level());
    for s in sources {
        println!("== {} ({}) {}", s.label(), s.id(), s.description());
        for r in rel::releases(s, false)?.iter().take(8) {
            let mb = r.archive.size as f64 / 1e6;
            let sum = if r.checksum.is_some() || r.archive.sha256.is_some() { "checksum" } else { "no checksum" };
            println!("  {:<32} {:>7.0} MB  {}  ({})", r.tag, mb, r.archive.name, sum);
        }
    }
    Ok(())
}

fn cmd_install(args: &[String]) -> Result<()> {
    let source =
        args.get(1).and_then(|s| Source::from_id(s)).context("usage: proton install <source> <tag|latest> --dir D")?;
    let tag = args.get(2).map(String::as_str).unwrap_or("latest");
    let dir = dir_arg(args)?;
    let release = rel::find_release(source, tag, false)?;
    println!(
        "installing {} {} ({:.0} MB) into {}",
        source.label(),
        release.tag,
        release.archive.size as f64 / 1e6,
        dir.display()
    );
    let t = Instant::now();
    let mut last = (Phase::Download, 0u64);
    let cancel = AtomicBool::new(false);
    let done = rel::install(&release, &dir, &cancel, &mut |p| {
        let pct = p.done * 100 / p.total.max(1);
        if p.phase != last.0 || pct >= last.1 + 10 {
            last = (p.phase, pct);
            eprintln!("  {:?} {pct}% ({:.1}s)", p.phase, t.elapsed().as_secs_f64());
        }
    })?;
    println!(
        "installed {} in {:.1}s, checksum {}",
        done.dir.display(),
        t.elapsed().as_secs_f64(),
        if done.verified { "verified" } else { "NOT available" }
    );
    Ok(())
}

fn cmd_remove(args: &[String]) -> Result<()> {
    let name = args.get(1).context("usage: proton remove <name> --dir D")?;
    anyhow::ensure!(!name.contains(['/', '\\']) && name != "." && name != "..", "bad tool name");
    let dir = dir_arg(args)?.join(name);
    rel::remove(&dir)?;
    println!("removed {}", dir.display());
    Ok(())
}

fn cmd_appinfo(args: &[String]) -> Result<()> {
    let appid = appid_arg(args)?;
    let t = Instant::now();
    for root in local::steam_roots() {
        let mut file = appinfo::AppInfoFile::open(&appinfo::AppInfoFile::path_in(&root))?;
        let opened = t.elapsed();
        println!("{}: {} apps indexed in {:.1} ms", root.display(), file.len(), opened.as_secs_f64() * 1e3);
        if let Some(info) = file.get(appid)? {
            println!("{info:#?}");
            println!("parse of app {appid}: {:.2} ms", (t.elapsed() - opened).as_secs_f64() * 1e3);
            return Ok(());
        }
    }
    bail!("app {appid} not found in appinfo.vdf")
}

fn cmd_launch(args: &[String]) -> Result<()> {
    let appid = appid_arg(args)?;
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let opts = PrepareOpts {
        force_windows: args[..split].iter().any(|a| a == "--windows"),
        tool: flag_value(&args[..split], "--tool"),
        extra_args: args.get(split + 1..).unwrap_or(&[]).to_vec(),
        // `--options "..."` tries a text; otherwise what a launch would use (ours, else the official client's).
        launch_options: flag_value(&args[..split], "--options").or_else(|| {
            let ours = launch_opts::LaunchOptions::load();
            launch_opts::effective(appid, ours.get(appid), None)
        }),
    };
    let p = run::prepare(appid, &opts)?;
    println!("{} ({appid}){}", p.name, if p.installed { "" } else { "  [not installed: path is where it would go]" });
    println!("entry:   {} {}", p.entry.executable, p.entry.arguments);
    if let Some((name, why)) = &p.tool {
        println!("tool:    {name} ({why:?})");
    }
    println!("runner:  {:?}", p.plan.runner);
    println!("program: {}", p.plan.program.display());
    println!("args:    {:?}", p.plan.args);
    println!("cwd:     {}", p.plan.cwd.display());
    if let Some(text) = &opts.launch_options {
        println!("launch options: {text}");
    }
    for (k, v) in &p.plan.env {
        println!("env:     {k}={v}");
    }
    let shown: Vec<_> = p.plan.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    println!("command: {}", launch_opts::command_line(&p.plan, &shown));
    // Only looks; the client is never started from here.
    let needs = crate::steam_client::uses_steamworks(&p.install_dir);
    let yn = |b: bool| if b { "yes" } else { "no" };
    println!("needs the Steam client: {}, running: {}", yn(needs), yn(crate::steam_client::is_running()));
    if args.iter().any(|a| a == "--dry-run") {
        return Ok(());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let pid = run::spawn(&p.plan, move |f| {
        let _ = tx.send(f);
    })?;
    println!("started pid {pid}, log {}", run::log_path(appid).display());
    let f = rx.recv()?;
    println!("exited {:?} after {:.0}s", f.code, f.played.as_secs_f64());
    Ok(())
}
