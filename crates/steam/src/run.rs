//! Starting a game ourselves, without `steam://`.
//!
//! Two layers:
//! - [`plan`] is a pure function from inputs (installed app, launch entry, chosen tool, paths) to a
//!   [`LaunchPlan`] (program, arguments, working dir, environment). It does no I/O, so every case is unit tested.
//! - [`spawn`] is the thin layer that starts the plan without blocking and reaps the child on a helper thread.
//!
//! Windows games run through Proton with the same `compatdata/<appid>` prefix the official client uses, so saves
//! and settings are shared both ways. When `umu-run` is installed it is preferred, because it brings the Steam
//! Linux Runtime container along. On Windows itself there is no Proton: the exe is started directly.

use crate::appinfo::{AppInfo, AppInfoFile, LaunchEntry};
use crate::compat::{self, ChoiceSource, Choices, CompatTool, ToolKind};
use crate::launch_opts;
use crate::local::{self, InstalledApp};
use crate::store;
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

/// The OS we run on; a parameter so both worlds can be tested on one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    Windows,
}

impl HostOs {
    pub fn current() -> HostOs {
        if cfg!(windows) { HostOs::Windows } else { HostOs::Linux }
    }

    /// The name used in appinfo `oslist`.
    pub fn oslist_name(self) -> &'static str {
        match self {
            HostOs::Linux => "linux",
            HostOs::Windows => "windows",
        }
    }
}

/// How the program is started; for display and for the UI to explain what will happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    /// A Linux binary, run as is.
    Native,
    /// `<tool>/proton waitforexitandrun <exe>`, possibly inside the Steam Linux Runtime container.
    Proton,
    /// `umu-run <exe>`.
    Umu,
    /// A Windows exe on Windows.
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub appid: u32,
    pub runner: Runner,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Variables added to the inherited environment.
    pub env: BTreeMap<String, String>,
    /// Folders to create before starting (the Proton prefix parent).
    pub create_dirs: Vec<PathBuf>,
}

/// Everything [`plan`] needs.
pub struct PlanInput<'a> {
    pub appid: u32,
    pub install_dir: &'a Path,
    pub entry: &'a LaunchEntry,
    /// User launch options appended after the entry's own arguments.
    pub extra_args: &'a [String],
    pub host: HostOs,
    /// Steam root to hand to Proton as the "client install path"; our data dir when there is none.
    pub steam_root: Option<&'a Path>,
    pub tool: Option<&'a CompatTool>,
    /// Steam Linux Runtime folder the tool wants (used without umu).
    pub runtime: Option<&'a CompatTool>,
    pub umu: Option<&'a Path>,
    /// `<library>/steamapps/compatdata/<appid>`
    pub compatdata: &'a Path,
}

/// Does the entry need a compatibility layer on this host?
pub fn needs_compat(entry: &LaunchEntry, host: HostOs) -> bool {
    host == HostOs::Linux && !entry.runs_on("linux") && entry.runs_on("windows")
}

/// Build the launch plan. Pure.
pub fn plan(i: &PlanInput) -> Result<LaunchPlan> {
    let exe = resolve_inside(i.install_dir, &i.entry.executable).context("launch entry has an unsafe executable")?;
    let cwd = match i.entry.workingdir.trim() {
        "" => i.install_dir.to_path_buf(),
        wd => resolve_inside(i.install_dir, wd).context("launch entry has an unsafe working dir")?,
    };
    let mut args = split_args(&i.entry.arguments);
    args.extend(i.extra_args.iter().cloned());
    let appid = i.appid.to_string();
    let mut env = BTreeMap::new();
    for k in ["SteamAppId", "SteamGameId", "SteamOverlayGameId"] {
        env.insert(k.to_string(), appid.clone());
    }
    let base = LaunchPlan {
        appid: i.appid,
        runner: Runner::Native,
        program: exe.clone(),
        args: args.clone(),
        cwd,
        env,
        create_dirs: vec![],
    };

    let luxtorpeda = i.tool.is_some_and(|t| t.kind == ToolKind::Luxtorpeda);
    if i.host == HostOs::Windows {
        return Ok(LaunchPlan { runner: Runner::Direct, ..base });
    }
    if !needs_compat(i.entry, i.host) && !luxtorpeda {
        return Ok(base);
    }
    let tool = i.tool.context("this game needs a compatibility tool (Proton), but none is installed or chosen")?;
    ensure!(tool.kind != ToolKind::Runtime, "{} is a runtime, not a tool to run games with", tool.display_name);
    Ok(compat_plan(i, tool, base, exe, args))
}

/// Windows entry (or a Luxtorpeda replacement) through Proton / umu / the tool's own entry point.
fn compat_plan(i: &PlanInput, tool: &CompatTool, mut p: LaunchPlan, exe: PathBuf, args: Vec<String>) -> LaunchPlan {
    let client = i.steam_root.map(Path::to_path_buf).unwrap_or_else(store::data_dir);
    let prefix = i.compatdata.join("pfx");
    p.create_dirs = vec![i.compatdata.to_path_buf()];
    p.env.insert("STEAM_COMPAT_DATA_PATH".into(), path_str(i.compatdata));
    p.env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), path_str(&client));
    p.env.insert("STEAM_COMPAT_INSTALL_PATH".into(), path_str(i.install_dir));
    p.env.insert("STEAM_COMPAT_APP_ID".into(), i.appid.to_string());

    if let Some(umu) = i.umu.filter(|_| tool.kind == ToolKind::Proton) {
        // umu supplies the runtime container itself and applies per-game fixes keyed by GAMEID.
        p.runner = Runner::Umu;
        p.program = umu.to_path_buf();
        p.args = std::iter::once(path_str(&exe)).chain(args).collect();
        p.env.insert("PROTONPATH".into(), path_str(&tool.dir));
        p.env.insert("GAMEID".into(), format!("umu-{}", i.appid));
        p.env.insert("WINEPREFIX".into(), path_str(&prefix));
        p.env.insert("STORE".into(), "steam".into());
        return p;
    }

    p.runner = Runner::Proton;
    let (program, mut tool_args) = tool.command("waitforexitandrun");
    tool_args.push(path_str(&exe));
    tool_args.extend(args);
    let mut tool_paths = path_str(&tool.dir);
    match i.runtime {
        Some(rt) => {
            // How Steam starts a tool that needs a container: the runtime's entry point wraps the tool.
            let (rt_program, mut wrapped) = rt.command("waitforexitandrun");
            tool_paths = format!("{tool_paths}:{}", path_str(&rt.dir));
            wrapped.push(path_str(&program));
            wrapped.extend(tool_args);
            (p.program, p.args) = (rt_program, wrapped);
        }
        None => {
            p.program = program;
            p.args = tool_args;
        }
    }
    p.env.insert("STEAM_COMPAT_TOOL_PATHS".into(), tool_paths);
    p
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `base/rel`, with Windows separators converted, refusing absolute paths and `..`.
fn resolve_inside(base: &Path, rel: &str) -> Result<PathBuf> {
    let norm = rel.replace('\\', "/");
    let rel = Path::new(&norm);
    ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "path {rel:?} leaves the game folder"
    );
    Ok(base.join(rel))
}

/// Split launch arguments like a shell would for simple cases: whitespace separated, `"` and `'` group.
pub fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote, mut has) = (Vec::new(), String::new(), None::<char>, false);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => (quote, has) = (Some(c), true),
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The entry behind "Play" for `os`: lowest id that is for this OS, not a beta, and of default type;
/// 64-bit entries are preferred over 32-bit ones.
pub fn select_entry<'a>(info: &'a AppInfo, os: &str) -> Option<&'a LaunchEntry> {
    let mut ok: Vec<&LaunchEntry> = info.launch.iter().filter(|e| e.runs_on(os) && e.is_default_kind()).collect();
    if ok.is_empty() {
        ok = info.launch.iter().filter(|e| e.runs_on(os) && e.betakey.is_none()).collect();
    }
    ok.iter().find(|e| e.is_64bit()).or(ok.first()).copied()
}

/// Native entry first on Linux; the Windows entry when there is no native one or when `prefer_compat`
/// (the user or Steam chose a tool for this game). Only entries that name Windows explicitly count as Windows
/// entries: one without an `oslist` runs everywhere, so it is the native one.
pub fn choose_entry(info: &AppInfo, host: HostOs, prefer_compat: bool) -> Option<&LaunchEntry> {
    let native = select_entry(info, host.oslist_name());
    if host == HostOs::Windows {
        return native;
    }
    let windows = select_entry(info, "windows").filter(|e| !e.oslist.is_empty());
    match (native, windows) {
        (Some(n), _) if !prefer_compat => Some(n),
        (n, w) => w.or(n),
    }
}

/// `<library>/steamapps/compatdata/<appid>` for an app installed under `<library>/steamapps/common/<dir>`;
/// for installs elsewhere, our own data dir.
pub fn compatdata_dir(install_dir: &Path, appid: u32) -> PathBuf {
    let steamapps =
        install_dir.parent().and_then(Path::parent).filter(|p| p.file_name().is_some_and(|n| n == "steamapps"));
    match steamapps {
        Some(sa) => sa.join("compatdata").join(appid.to_string()),
        None => store::data_dir().join("compatdata").join(appid.to_string()),
    }
}

/// `umu-run` on `$PATH` or in `~/.local/bin`.
pub fn find_umu() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(h).join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join("umu-run")).find(|p| p.is_file())
}

/// The Steam Linux Runtime a tool asks for, among installed tools.
pub fn runtime_for<'a>(tool: &CompatTool, tools: &'a [CompatTool]) -> Option<&'a CompatTool> {
    let want = match tool.require_tool_appid? {
        1391110 => "SteamLinuxRuntime_soldier",
        1628350 => "SteamLinuxRuntime_sniper",
        4183110 => "SteamLinuxRuntime_4",
        _ => return None,
    };
    tools.iter().find(|t| t.kind == ToolKind::Runtime && t.internal_name == want && t.command("run").0.is_file())
}

// ---- Putting it together -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct PrepareOpts {
    /// Use the Windows entry through the compatibility tool even if the game has a native one.
    pub force_windows: bool,
    /// Tool to use instead of the saved choices (an internal name).
    pub tool: Option<String>,
    pub extra_args: Vec<String>,
    /// The game's launch options text (see [`crate::launch_opts`]); `None` = none.
    pub launch_options: Option<String>,
    /// The performance overlay (MangoHud) to add; the user's own launch options win over it.
    pub hud: Option<crate::mangohud::Hud>,
}

pub struct Prepared {
    pub plan: LaunchPlan,
    pub name: String,
    pub entry: LaunchEntry,
    /// Chosen tool and why, when the plan uses one.
    pub tool: Option<(String, ChoiceSource)>,
    /// False when only Steam's data says where the game would go (a dry run for a game that isn't installed).
    pub installed: bool,
    /// The game's folder (where it would be, when not installed).
    pub install_dir: PathBuf,
}

/// Launch data for `appid` from the official client's `appinfo.vdf` in any Steam root.
pub fn appinfo_for(appid: u32) -> Result<Option<AppInfo>> {
    for root in local::steam_roots() {
        let path = AppInfoFile::path_in(&root);
        if !path.is_file() {
            continue;
        }
        if let Some(info) = AppInfoFile::open(&path)?.get(appid)? {
            return Ok(Some(info));
        }
    }
    Ok(saved_appinfo(appid))
}

/// PICS data the downloader saved when it installed the app: lets games we
/// installed start without the official client ever having seen them.
fn saved_appinfo(appid: u32) -> Option<AppInfo> {
    let mut libraries = vec![crate::content::default_library()];
    for root in local::steam_roots() {
        libraries.extend(local::library_folders(&root));
    }
    libraries.iter().find_map(|lib| {
        let text = std::fs::read_to_string(crate::content::appinfo_path(lib, appid)).ok()?;
        let doc = crate::vdf::parse(text.trim().trim_matches('\0'));
        Some(AppInfo::from_vdf(appid, doc.get("appinfo").unwrap_or(&doc)))
    })
}

/// Discover everything needed and build the plan for `appid`. Use [`prepare_with`] when the caller already has
/// the app info (for example from PICS) and the installed list.
pub fn prepare(appid: u32, opts: &PrepareOpts) -> Result<Prepared> {
    let info = appinfo_for(appid)?.with_context(|| format!("app {appid} is not in Steam's appinfo.vdf"))?;
    let installed = local::installed_apps();
    prepare_with(&info, installed.get(&appid), opts)
}

pub fn prepare_with(info: &AppInfo, installed: Option<&InstalledApp>, opts: &PrepareOpts) -> Result<Prepared> {
    let appid = info.appid;
    let install_dir = match installed {
        Some(a) => a.install_dir.clone(),
        None => local::steam_roots()
            .first()
            .map(|r| r.join("steamapps/common").join(&info.installdir))
            .context("no install location known")?,
    };
    let host = HostOs::current();
    let tools = compat::discover();
    let (tool, source) = pick(&tools, appid, opts)?;
    let prefer_compat = opts.force_windows || matches!(source, Some(ChoiceSource::OurApp | ChoiceSource::SteamApp));
    let entry = choose_entry(info, host, prefer_compat)
        .with_context(|| format!("{} has no launch entry for this system", info.name))?;
    let steam_root = local::steam_roots().into_iter().next();
    let compatdata = compatdata_dir(&install_dir, appid);
    let umu = find_umu();
    let runtime = tool.and_then(|t| runtime_for(t, &tools));
    // Luxtorpeda replaces the engine even for Linux entries; everything else only matters when compat is needed.
    let uses_tool = needs_compat(entry, host) || tool.is_some_and(|t| t.kind == ToolKind::Luxtorpeda && prefer_compat);
    let parsed = opts.launch_options.as_deref().map(launch_opts::parse).unwrap_or_default();
    let extra_args: Vec<String> = parsed.args.iter().chain(&opts.extra_args).cloned().collect();
    let mut plan = plan(&PlanInput {
        appid,
        install_dir: &install_dir,
        entry,
        extra_args: &extra_args,
        host,
        steam_root: steam_root.as_deref(),
        tool: tool.filter(|_| uses_tool),
        runtime,
        umu: umu.as_deref(),
        compatdata: &compatdata,
    })?;
    launch_opts::apply(&mut plan, &parsed);
    if let Some(hud) = &opts.hud {
        crate::mangohud::apply(&mut plan, hud, &parsed);
    }
    let tool = tool.filter(|_| uses_tool).map(|t| (t.internal_name.clone(), source.unwrap_or(ChoiceSource::Newest)));
    Ok(Prepared {
        plan,
        name: info.name.clone(),
        entry: entry.clone(),
        tool,
        installed: installed.is_some(),
        install_dir,
    })
}

fn pick<'a>(
    tools: &'a [CompatTool],
    appid: u32,
    opts: &PrepareOpts,
) -> Result<(Option<&'a CompatTool>, Option<ChoiceSource>)> {
    if let Some(name) = &opts.tool {
        let t = tools
            .iter()
            .find(|t| &t.internal_name == name)
            .with_context(|| format!("no installed tool named {name}"))?;
        return Ok((Some(t), Some(ChoiceSource::OurApp)));
    }
    let picked = compat::pick_tool(tools, appid, &Choices::load(), &compat::steam_mapping_all());
    Ok((picked.map(|(t, _)| t), picked.map(|(_, s)| s)))
}

// ---- Spawning ------------------------------------------------------------------------------------------------

/// What the exit watcher reports.
#[derive(Debug, Clone)]
pub struct Finished {
    pub appid: u32,
    /// Exit code; `None` when killed by a signal.
    pub code: Option<i32>,
    pub success: bool,
    /// Wall-clock time from start to exit.
    pub played: Duration,
}

/// Log file of the last run of a game, next to our other data.
pub fn log_path(appid: u32) -> PathBuf {
    store::data_dir().join("logs").join(format!("game-{appid}.log"))
}

/// Start the plan and return the child's pid at once. A helper thread waits for the child (so it never
/// becomes a zombie) and calls `on_exit` when it ends. stdout and stderr go to [`log_path`].
pub fn spawn(plan: &LaunchPlan, on_exit: impl FnOnce(Finished) + Send + 'static) -> Result<u32> {
    for d in &plan.create_dirs {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    let log = log_path(plan.appid);
    if let Some(dir) = log.parent() {
        store::private_dir(dir)?;
    }
    let out = std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
    let mut cmd = std::process::Command::new(&plan.program);
    cmd.args(&plan.args)
        .current_dir(&plan.cwd)
        .envs(&plan.env)
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // own group: the UI's Ctrl-C doesn't hit the game
    let mut child = cmd.spawn().with_context(|| format!("starting {}", plan.program.display()))?;
    let pid = child.id();
    let appid = plan.appid;
    let started = Instant::now();
    std::thread::Builder::new().name(format!("game-{appid}")).spawn(move || {
        let status = child.wait();
        let (code, success) = status.map(|s| (s.code(), s.success())).unwrap_or((None, false));
        on_exit(Finished { appid, code, success, played: started.elapsed() });
    })?;
    if pid == 0 {
        bail!("process id missing");
    }
    Ok(pid)
}

#[cfg(test)]
mod tests {
    /// An app only we installed (absent from Steam's appinfo.vdf) still gets launch data from the saved PICS text.
    #[test]
    fn saved_appinfo_is_used_when_steam_has_none() {
        let lib = std::env::temp_dir().join(format!("sr-saved-appinfo-{}", std::process::id()));
        let appid = 4_000_000_123;
        let path = crate::content::appinfo_path(&lib, appid);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let pics = r#""appinfo" { "common" { "name" "Only Ours" "oslist" "linux" }
            "config" { "installdir" "Ours" "launch" { "0" { "executable" "run.sh" } } } }"#;
        std::fs::write(&path, pics).unwrap();
        // SAFETY: tests in this module don't read KETTLE_LIBRARY concurrently.
        unsafe { std::env::set_var("KETTLE_LIBRARY", &lib) };
        let info = appinfo_for(appid).unwrap().expect("found in saved data");
        unsafe { std::env::remove_var("KETTLE_LIBRARY") };
        std::fs::remove_dir_all(&lib).unwrap();
        assert_eq!(info.launch.len(), 1);
        assert_eq!(info.launch[0].executable, "run.sh");
    }

    use super::*;
    use crate::compat::ToolOrigin;

    fn entry(exe: &str, oslist: &[&str]) -> LaunchEntry {
        LaunchEntry {
            id: "0".into(),
            executable: exe.into(),
            arguments: "-a \"b c\"".into(),
            oslist: oslist.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn tool(kind: ToolKind, cl: Option<&str>) -> CompatTool {
        CompatTool {
            internal_name: "GE-Proton9-20".into(),
            display_name: "GE-Proton9-20".into(),
            dir: PathBuf::from("/tools/GE-Proton9-20"),
            kind,
            origin: ToolOrigin::Ours,
            version: None,
            commandline: cl.map(str::to_string),
            require_tool_appid: Some(1628350),
        }
    }

    fn input<'a>(entry: &'a LaunchEntry, tool: Option<&'a CompatTool>, host: HostOs) -> PlanInput<'a> {
        PlanInput {
            appid: 730,
            install_dir: Path::new("/lib/steamapps/common/G"),
            entry,
            extra_args: &[],
            host,
            steam_root: Some(Path::new("/steam")),
            tool,
            runtime: None,
            umu: None,
            compatdata: Path::new("/lib/steamapps/compatdata/730"),
        }
    }

    #[test]
    fn native_plan() {
        let e = entry("bin\\game.sh", &["linux"]);
        let p = plan(&input(&e, None, HostOs::Linux)).unwrap();
        assert_eq!(p.runner, Runner::Native);
        assert_eq!(p.program, Path::new("/lib/steamapps/common/G/bin/game.sh"));
        assert_eq!(p.args, ["-a", "b c"]);
        assert_eq!(p.cwd, Path::new("/lib/steamapps/common/G"));
        assert_eq!(p.env["SteamAppId"], "730");
        assert!(!p.env.contains_key("STEAM_COMPAT_DATA_PATH"));
    }

    #[test]
    fn proton_plan_shares_steam_prefix() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Proton, Some("/proton %verb%"));
        let p = plan(&input(&e, Some(&t), HostOs::Linux)).unwrap();
        assert_eq!(p.runner, Runner::Proton);
        assert_eq!(p.program, Path::new("/tools/GE-Proton9-20/proton"));
        assert_eq!(p.args[..2], ["waitforexitandrun", "/lib/steamapps/common/G/game.exe"]);
        assert_eq!(p.env["STEAM_COMPAT_DATA_PATH"], "/lib/steamapps/compatdata/730");
        assert_eq!(p.env["STEAM_COMPAT_CLIENT_INSTALL_PATH"], "/steam");
        assert_eq!(p.create_dirs, [PathBuf::from("/lib/steamapps/compatdata/730")]);
    }

    #[test]
    fn proton_in_runtime_container() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Proton, None);
        let mut rt = tool(ToolKind::Runtime, None);
        rt.dir = PathBuf::from("/lib/steamapps/common/SteamLinuxRuntime_sniper");
        rt.commandline = Some("/_v2-entry-point --verb=%verb% --".into());
        let mut i = input(&e, Some(&t), HostOs::Linux);
        i.runtime = Some(&rt);
        let p = plan(&i).unwrap();
        assert_eq!(p.program, Path::new("/lib/steamapps/common/SteamLinuxRuntime_sniper/_v2-entry-point"));
        assert_eq!(p.args[..3], ["--verb=waitforexitandrun", "--", "/tools/GE-Proton9-20/proton"]);
        assert!(p.env["STEAM_COMPAT_TOOL_PATHS"].contains("SteamLinuxRuntime_sniper"));
    }

    /// Plan with launch options the way `prepare_with` composes them.
    fn with_options(i: &PlanInput, text: &str) -> LaunchPlan {
        let parsed = launch_opts::parse(text);
        let mut p = plan(&PlanInput {
            extra_args: &parsed.args,
            appid: i.appid,
            install_dir: i.install_dir,
            entry: i.entry,
            host: i.host,
            steam_root: i.steam_root,
            tool: i.tool,
            runtime: i.runtime,
            umu: i.umu,
            compatdata: i.compatdata,
        })
        .unwrap();
        launch_opts::apply(&mut p, &parsed);
        p
    }

    #[test]
    fn launch_options_native() {
        let e = entry("game.sh", &["linux"]);
        let i = input(&e, None, HostOs::Linux);
        let p = with_options(&i, "FOO=1 gamemoderun %command% -novid");
        assert_eq!(p.program, Path::new("gamemoderun"));
        assert_eq!(p.args, ["/lib/steamapps/common/G/game.sh", "-a", "b c", "-novid"]);
        assert_eq!(p.env["FOO"], "1");
        // Without %command% the text is plain extra arguments.
        let p = with_options(&i, "-novid 'two words'");
        assert_eq!(p.program, Path::new("/lib/steamapps/common/G/game.sh"));
        assert_eq!(p.args, ["-a", "b c", "-novid", "two words"]);
        // Empty text changes nothing.
        let p = with_options(&i, "");
        assert_eq!(p.args, ["-a", "b c"]);
    }

    #[test]
    fn launch_options_proton_and_umu() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Proton, Some("/proton %verb%"));
        let i = input(&e, Some(&t), HostOs::Linux);
        let p = with_options(&i, "PROTON_LOG=1 mangohud %command% -x");
        assert_eq!(p.program, Path::new("mangohud"));
        assert_eq!(
            p.args,
            ["/tools/GE-Proton9-20/proton", "waitforexitandrun", "/lib/steamapps/common/G/game.exe", "-a", "b c", "-x"]
        );
        assert_eq!(p.env["PROTON_LOG"], "1");
        assert_eq!(p.env["STEAM_COMPAT_APP_ID"], "730");
        let mut i = input(&e, Some(&t), HostOs::Linux);
        i.umu = Some(Path::new("/usr/bin/umu-run"));
        let p = with_options(&i, "DXVK_HUD=fps %command% -x");
        assert_eq!(p.program, Path::new("/usr/bin/umu-run"));
        assert_eq!(p.args, ["/lib/steamapps/common/G/game.exe", "-a", "b c", "-x"]);
        assert_eq!(p.env["DXVK_HUD"], "fps");
    }

    #[test]
    fn umu_plan() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Proton, None);
        let mut i = input(&e, Some(&t), HostOs::Linux);
        i.umu = Some(Path::new("/usr/bin/umu-run"));
        let p = plan(&i).unwrap();
        assert_eq!(p.runner, Runner::Umu);
        assert_eq!(p.program, Path::new("/usr/bin/umu-run"));
        assert_eq!(p.args[0], "/lib/steamapps/common/G/game.exe");
        assert_eq!(p.env["PROTONPATH"], "/tools/GE-Proton9-20");
        assert_eq!(p.env["GAMEID"], "umu-730");
        assert_eq!(p.env["WINEPREFIX"], "/lib/steamapps/compatdata/730/pfx");
        assert_eq!(p.env["STORE"], "steam");
    }

    #[test]
    fn luxtorpeda_uses_own_entry_point_and_no_umu() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Luxtorpeda, Some("/luxtorpeda %verb%"));
        let mut i = input(&e, Some(&t), HostOs::Linux);
        i.umu = Some(Path::new("/usr/bin/umu-run"));
        let p = plan(&i).unwrap();
        assert_eq!(p.runner, Runner::Proton);
        assert_eq!(p.program, Path::new("/tools/GE-Proton9-20/luxtorpeda"));
    }

    #[test]
    fn windows_host_runs_directly_and_missing_tool_errors() {
        let e = entry("game.exe", &["windows"]);
        let p = plan(&input(&e, None, HostOs::Windows)).unwrap();
        assert_eq!(p.runner, Runner::Direct);
        assert!(plan(&input(&e, None, HostOs::Linux)).is_err());
    }

    #[test]
    fn unsafe_paths_rejected() {
        for exe in ["../x", "/bin/sh", "a\\..\\..\\x"] {
            let e = entry(exe, &["linux"]);
            assert!(plan(&input(&e, None, HostOs::Linux)).is_err(), "{exe}");
        }
    }

    #[test]
    fn entry_selection() {
        let mk = |id: &str, os: &[&str], arch: Option<&str>, beta: Option<&str>, kind: Option<&str>| LaunchEntry {
            id: id.into(),
            executable: format!("e{id}"),
            oslist: os.iter().map(|s| s.to_string()).collect(),
            osarch: arch.map(str::to_string),
            betakey: beta.map(str::to_string),
            kind: kind.map(str::to_string),
            ..Default::default()
        };
        let info = AppInfo {
            launch: vec![
                mk("0", &["windows"], None, None, None),
                mk("1", &["linux"], Some("32"), None, None),
                mk("2", &["linux"], Some("64"), None, None),
                mk("3", &["linux"], None, Some("beta"), None),
                mk("4", &["linux"], None, None, Some("server")),
            ],
            ..Default::default()
        };
        assert_eq!(select_entry(&info, "linux").unwrap().id, "2");
        assert_eq!(choose_entry(&info, HostOs::Linux, false).unwrap().id, "2");
        assert_eq!(choose_entry(&info, HostOs::Linux, true).unwrap().id, "0");
        assert_eq!(choose_entry(&info, HostOs::Windows, false).unwrap().id, "0");
        let win_only = AppInfo { launch: vec![mk("0", &["windows"], None, None, None)], ..Default::default() };
        assert_eq!(choose_entry(&win_only, HostOs::Linux, false).unwrap().id, "0");
    }

    #[test]
    fn args_and_paths() {
        assert_eq!(split_args("  -a  'b c' \"\" d"), ["-a", "b c", "", "d"]);
        assert_eq!(compatdata_dir(Path::new("/l/steamapps/common/G"), 5), Path::new("/l/steamapps/compatdata/5"));
        assert!(compatdata_dir(Path::new("/games/G"), 5).ends_with("compatdata/5"));
    }
}
