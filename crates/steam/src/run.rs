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
use crate::compat::{self, AppCompatConfig, ChoiceSource, Choices, CompatTool, SteamPlay, ToolKind};
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
    /// Every Steam library folder (roots, not `steamapps`): handed to the container as
    /// `STEAM_COMPAT_LIBRARY_PATHS` so it can see the game and the prefix wherever they are.
    pub libraries: &'a [PathBuf],
    /// Valve's per-game Proton settings (`STEAM_COMPAT_CONFIG` and extra variables).
    pub compat: Option<&'a AppCompatConfig>,
    /// Ask Proton for its log (`PROTON_LOG=1`, written to our logs folder).
    pub proton_log: bool,
    /// Steam Linux Runtime container for a NATIVE game that Valve maps to one (e.g. sniper for Dota 2).
    pub native_runtime: Option<&'a CompatTool>,
    /// The legacy Steam runtime's `run.sh` (`<steam>/ubuntu12_32/steam-runtime/run.sh`): how the official client
    /// starts native games that have no container.
    pub legacy_runtime: Option<&'a Path>,
}

/// Does the entry need a compatibility layer on this host?
pub fn needs_compat(entry: &LaunchEntry, host: HostOs) -> bool {
    host == HostOs::Linux && !native_entry(entry)
}

/// Is this entry a Linux program? Only an entry that names Linux says so; one without an `oslist` is native only
/// when its executable doesn't look like a Windows one (old Windows games often leave the list out entirely,
/// and Fallout 2's second entry is `fallout2launcher.exe` with no list).
pub fn native_entry(e: &LaunchEntry) -> bool {
    if e.oslist.is_empty() {
        return !looks_windows(&e.executable);
    }
    e.oslist.iter().any(|o| o.eq_ignore_ascii_case("linux"))
}

fn looks_windows(exe: &str) -> bool {
    let l = exe.trim().to_ascii_lowercase();
    [".exe", ".bat", ".cmd", ".com", ".msi", ".lnk"].iter().any(|x| l.ends_with(x))
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
    env.insert("SteamEnv".into(), "1".into());
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
        return Ok(native_plan(i, base, exe, args));
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
    common_compat_env(i, &mut p.env);
    p.env.insert("STEAM_COMPAT_SHADER_PATH".into(), path_str(&shader_path(i)));
    if let Some(c) = i.compat {
        if !c.config.is_empty() {
            p.env.insert("STEAM_COMPAT_CONFIG".into(), c.config.clone());
        }
        p.env.extend(c.environment.iter().cloned());
    }
    if i.proton_log {
        p.env.insert("PROTON_LOG".into(), "1".into());
        p.env.insert("PROTON_LOG_DIR".into(), path_str(&store::data_dir().join("logs")));
    }

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

/// Variables every tool container wants: the game folder, the libraries and the app.
fn common_compat_env(i: &PlanInput, env: &mut BTreeMap<String, String>) {
    env.insert("STEAM_COMPAT_INSTALL_PATH".into(), path_str(i.install_dir));
    env.insert("STEAM_COMPAT_APP_ID".into(), i.appid.to_string());
    let libs: Vec<String> = i.libraries.iter().map(|l| path_str(l)).collect();
    if !libs.is_empty() {
        env.insert("STEAM_COMPAT_LIBRARY_PATHS".into(), libs.join(":"));
    }
}

/// `<library>/steamapps/shadercache/<appid>` next to the game, like the official client.
fn shader_path(i: &PlanInput) -> PathBuf {
    let sa = i.compatdata.parent().and_then(Path::parent).unwrap_or(i.compatdata);
    sa.join("shadercache").join(i.appid.to_string())
}

/// A Linux program: in the container Valve maps it to, else through the legacy runtime's `run.sh`, else as is.
fn native_plan(i: &PlanInput, mut p: LaunchPlan, exe: PathBuf, args: Vec<String>) -> LaunchPlan {
    if let Some(rt) = i.native_runtime {
        let (program, mut wrapped) = rt.command("waitforexitandrun");
        wrapped.push(path_str(&exe));
        wrapped.extend(args);
        common_compat_env(i, &mut p.env);
        p.env.insert("STEAM_COMPAT_TOOL_PATHS".into(), path_str(&rt.dir));
        p.env.insert("STEAM_COMPAT_SHADER_PATH".into(), path_str(&shader_path(i)));
        (p.program, p.args) = (program, wrapped);
    } else if let Some(run_sh) = i.legacy_runtime {
        p.args = std::iter::once(path_str(&exe)).chain(args).collect();
        p.program = run_sh.to_path_buf();
    }
    p
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `base/rel`, with Windows separators converted, refusing absolute paths and `..`.
fn resolve_inside(base: &Path, rel: &str) -> Result<PathBuf> {
    let norm = rel.trim().replace('\\', "/");
    let rel = Path::new(&norm);
    ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "path {rel:?} leaves the game folder"
    );
    Ok(resolve_case(base, rel))
}

/// `base/rel` where each part that doesn't exist as written is matched case-insensitively against the folder
/// (Windows games name `Fallout2.exe` while the depot has `FALLOUT2.EXE`). Parts with no match are kept as they
/// are, so the error that follows names the path from the game's data.
pub fn resolve_case(base: &Path, rel: &Path) -> PathBuf {
    let mut cur = base.to_path_buf();
    for c in rel.components() {
        let Component::Normal(name) = c else { continue };
        let exact = cur.join(name);
        if exact.symlink_metadata().is_ok() {
            cur = exact;
            continue;
        }
        let want = name.to_string_lossy().to_lowercase();
        let found = std::fs::read_dir(&cur).ok().and_then(|rd| {
            let mut names: Vec<_> =
                rd.flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().to_lowercase() == want).collect();
            names.sort();
            names.into_iter().next()
        });
        cur = cur.join(found.as_deref().unwrap_or(name));
    }
    cur
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

/// The entry behind "Play" for `os` like the official client: entries for this OS (a Linux entry must really be
/// one, see [`native_entry`]), not a beta branch, of default type; the lowest id wins, and a 32-bit-only entry
/// only when nothing else fits.
pub fn select_entry<'a>(info: &'a AppInfo, os: &str) -> Option<&'a LaunchEntry> {
    let for_os = |e: &&LaunchEntry| e.runs_on(os) && (os != "linux" || native_entry(e));
    let mut ok: Vec<&LaunchEntry> = info.launch.iter().filter(for_os).filter(|e| e.is_default_kind()).collect();
    if ok.is_empty() {
        ok = info.launch.iter().filter(for_os).filter(|e| e.betakey.is_none()).collect();
    }
    ok.iter().find(|e| e.is_64bit()).or(ok.first()).copied()
}

/// Native entry first on Linux; the Windows entry when there is no native one or when `prefer_compat`
/// (the user or Steam chose a tool for this game). An entry without an `oslist` counts as Windows when it runs
/// an `.exe`.
pub fn choose_entry(info: &AppInfo, host: HostOs, prefer_compat: bool) -> Option<&LaunchEntry> {
    let native = select_entry(info, host.oslist_name());
    if host == HostOs::Windows {
        return native;
    }
    let windows = select_entry(info, "windows");
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

/// The Steam Linux Runtime a tool asks for (`require_tool_appid` of its `toolmanifest.vdf`), among installed
/// tools. Found by the app's manifest in the same library first, then by the folder names Valve uses.
pub fn runtime_for<'a>(tool: &CompatTool, tools: &'a [CompatTool]) -> Option<&'a CompatTool> {
    let id = tool.require_tool_appid?;
    let usable = |t: &&CompatTool| t.kind == ToolKind::Runtime && t.command("run").0.is_file();
    let by_manifest = tools.iter().filter(usable).find(|t| {
        t.dir.parent().and_then(Path::parent).is_some_and(|sa| sa.join(format!("appmanifest_{id}.acf")).is_file())
    });
    let name = match id {
        1070560 => "SteamLinuxRuntime",
        1391110 => "SteamLinuxRuntime_soldier",
        1628350 => "SteamLinuxRuntime_sniper",
        4183110 => "SteamLinuxRuntime_4",
        _ => "",
    };
    by_manifest.or_else(|| tools.iter().filter(usable).find(|t| t.internal_name == name))
}

/// Human name of the runtime a tool needs, for messages.
pub fn runtime_name(id: u32) -> String {
    match id {
        1070560 => "Steam Linux Runtime 1.0 (scout)".into(),
        1391110 => "Steam Linux Runtime 2.0 (soldier)".into(),
        1628350 => "Steam Linux Runtime 3.0 (sniper)".into(),
        4183110 => "Steam Linux Runtime 4.0".into(),
        n => format!("Steam tool {n}"),
    }
}

/// The container Valve or the user maps a NATIVE game to (`steamlinuxruntime_sniper` ...), when it is installed.
pub fn native_runtime<'a>(
    tools: &'a [CompatTool],
    appid: u32,
    ours: &Choices,
    steam: &std::collections::HashMap<u32, String>,
    valve: &SteamPlay,
) -> Option<&'a CompatTool> {
    let name = ours
        .per_app
        .get(&appid.to_string())
        .or_else(|| steam.get(&appid))
        .or_else(|| valve.app_tool.get(&appid))
        .filter(|n| !n.is_empty())?;
    let name = valve.resolve(name).to_ascii_lowercase();
    tools.iter().find(|t| {
        t.kind == ToolKind::Runtime && t.internal_name.to_ascii_lowercase() == name && t.command("run").0.is_file()
    })
}

/// The Steam root that owns the library holding `install_dir`: what Proton must get as the client path, so a
/// game in the Flatpak client's library is started against the Flatpak client's files.
pub fn steam_root_for(install_dir: &Path) -> Option<PathBuf> {
    let roots = local::steam_roots();
    let inside = |lib: &Path| {
        let (a, b) = (install_dir.canonicalize().ok(), lib.canonicalize().ok());
        matches!((a, b), (Some(a), Some(b)) if a.starts_with(&b))
    };
    roots.iter().find(|r| local::library_folders(r).iter().any(|l| inside(l))).or(roots.first()).cloned()
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
    /// "Write Proton logs": `PROTON_LOG=1`, the file goes to `<data dir>/logs`.
    pub proton_log: bool,
    /// Run the game inside Gamescope (see [`crate::gamescope`]); `None` = as usual. Ignored on Windows and
    /// when the user's own launch options already start `gamescope`.
    pub gamescope: Option<crate::gamescope::Launch>,
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
    /// The plan runs inside Gamescope.
    pub gamescope: bool,
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
    let valve = SteamPlay::load();
    let steam_map = compat::steam_mapping_all();
    let ours = Choices::load();
    let (tool, source) = pick(&tools, appid, opts, &ours, &steam_map, &valve)?;
    // A tool the user or Steam named for this very game wins over a native build, as in the official client.
    let prefer_compat = opts.force_windows
        || matches!(source, Some(ChoiceSource::OurApp | ChoiceSource::SteamApp))
        || (source == Some(ChoiceSource::ValveApp) && tool.is_some_and(|t| t.kind == ToolKind::Proton));
    let entry = choose_entry(info, host, prefer_compat)
        .with_context(|| format!("{} has no launch entry for this system", info.name))?;
    let steam_root = steam_root_for(&install_dir);
    let compatdata = compatdata_dir(&install_dir, appid);
    let umu = find_umu();
    let runtime = tool.and_then(|t| runtime_for(t, &tools));
    // Luxtorpeda replaces the engine even for Linux entries; everything else only matters when compat is needed.
    let uses_tool = needs_compat(entry, host) || tool.is_some_and(|t| t.kind == ToolKind::Luxtorpeda && prefer_compat);
    if let (true, Some(t), None, None) = (uses_tool, tool, runtime, umu.as_ref()) {
        // Valve's own Protons only work inside their container; a custom one may still run without it.
        if let Some(id) = t.require_tool_appid.filter(|_| t.origin == compat::ToolOrigin::SteamLibrary) {
            anyhow::bail!(
                "{} needs {}, which is not installed. Install it in the Steam client (Library > Tools), or choose a \
                 different compatibility tool for this game.",
                t.display_name,
                runtime_name(id)
            );
        }
    }
    let parsed = opts.launch_options.as_deref().map(launch_opts::parse).unwrap_or_default();
    let extra_args: Vec<String> = parsed.args.iter().chain(&opts.extra_args).cloned().collect();
    let libraries = all_libraries();
    let legacy = steam_root
        .as_ref()
        .map(|r| r.join("ubuntu12_32/steam-runtime/run.sh"))
        .filter(|p| p.is_file() && std::env::var_os("KETTLE_NO_STEAM_RUNTIME").is_none());
    let native_rt = native_runtime(&tools, appid, &ours, &steam_map, &valve);
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
        libraries: &libraries,
        compat: valve.app_config.get(&appid),
        proton_log: opts.proton_log,
        native_runtime: native_rt,
        legacy_runtime: legacy.as_deref(),
    })?;
    if installed.is_some() {
        check_files(&plan, &install_dir, entry)?;
    }
    launch_opts::apply(&mut plan, &parsed);
    let gs = opts.gamescope.as_ref().filter(|_| host == HostOs::Linux && !crate::gamescope::user_wrapped(&parsed));
    if let Some(hud) = &opts.hud {
        // Gamescope draws the HUD itself with --mangoapp: then the game gets neither the layer nor the wrapper.
        let by_gamescope = gs.is_some_and(|g| g.mangoapp) && !crate::mangohud::user_owns(&parsed);
        if by_gamescope {
            crate::mangohud::apply(&mut plan, &crate::gamescope::mangoapp_hud(hud), &parsed);
        } else {
            crate::mangohud::apply(&mut plan, hud, &parsed);
        }
    }
    let wrapped = gs.is_some();
    if let Some(g) = gs {
        let mut g = g.clone();
        g.mangoapp = g.mangoapp && opts.hud.is_some() && !crate::mangohud::user_owns(&parsed);
        crate::gamescope::apply(&mut plan, &g);
    }
    let tool = tool.filter(|_| uses_tool).map(|t| (t.internal_name.clone(), source.unwrap_or(ChoiceSource::Newest)));
    Ok(Prepared {
        plan,
        name: info.name.clone(),
        entry: entry.clone(),
        tool,
        installed: installed.is_some(),
        install_dir,
        gamescope: wrapped,
    })
}

/// Plan for a program that is not a Steam game (Kettle's "Add a Game"): a Linux program runs as it is, a
/// Windows one (`.exe` and friends) goes through the default compatibility tool with its own prefix in our data
/// dir. `appid` is only an identifier for the prefix folder and the launch log. `args` is one text, split like
/// launch options. Does no writes.
pub fn prepare_exe(appid: u32, exe: &Path, args: &str, proton_log: bool, tool: Option<&str>) -> Result<LaunchPlan> {
    ensure!(exe.is_absolute(), "the program's path must be absolute");
    ensure!(exe.is_file(), "The program is missing: {}", exe.display());
    let dir = exe.parent().context("the program has no folder")?;
    let name = exe.file_name().and_then(|n| n.to_str()).context("the program's file name is not text")?;
    let host = HostOs::current();
    let windows = looks_windows(name);
    if !windows && host == HostOs::Linux {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(exe)?.permissions().mode();
            ensure!(mode & 0o111 != 0, "{} is not marked as executable (chmod +x it).", exe.display());
        }
    }
    let entry = LaunchEntry {
        id: "0".into(),
        executable: name.to_string(),
        arguments: args.to_string(),
        oslist: vec![if windows { "windows" } else { "linux" }.to_string()],
        ..Default::default()
    };
    let tools = compat::discover();
    let valve = SteamPlay::load();
    let steam_map = compat::steam_mapping_all();
    let ours = Choices::load();
    let opts = PrepareOpts { tool: tool.map(str::to_string), ..Default::default() };
    let (tool, _) = if windows { pick(&tools, appid, &opts, &ours, &steam_map, &valve)? } else { (None, None) };
    let runtime = tool.and_then(|t| runtime_for(t, &tools));
    let compatdata = store::data_dir().join("compatdata").join(appid.to_string());
    let steam_root = local::steam_roots().first().cloned();
    let umu = find_umu();
    let libraries = all_libraries();
    plan(&PlanInput {
        appid,
        install_dir: dir,
        entry: &entry,
        extra_args: &[],
        host,
        steam_root: steam_root.as_deref(),
        tool,
        runtime,
        umu: umu.as_deref(),
        compatdata: &compatdata,
        libraries: &libraries,
        compat: None,
        proton_log,
        native_runtime: None,
        legacy_runtime: None,
    })
}

/// Every library folder of every Steam root, plus our own.
fn all_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in local::steam_roots() {
        for l in local::library_folders(&root) {
            if !libs.contains(&l) {
                libs.push(l);
            }
        }
    }
    libs
}

/// The files a launch needs, checked with words the user can act on (the official client says much the same).
fn check_files(plan: &LaunchPlan, install_dir: &Path, entry: &LaunchEntry) -> Result<()> {
    let exe = resolve_inside(install_dir, &entry.executable)?;
    if !exe.exists() {
        let place = if install_dir.is_dir() { "" } else { " The game folder itself is not there." };
        anyhow::bail!(
            "The game's program is missing: {}\nSteam's data says this game starts \"{}\" from its folder.{place} \
             Verify the game's files in the Steam client or reinstall it.",
            exe.display(),
            entry.executable,
        );
    }
    if !plan.cwd.is_dir() {
        anyhow::bail!("The game's working folder is missing: {}", plan.cwd.display());
    }
    Ok(())
}

fn pick<'a>(
    tools: &'a [CompatTool],
    appid: u32,
    opts: &PrepareOpts,
    ours: &Choices,
    steam: &std::collections::HashMap<u32, String>,
    valve: &SteamPlay,
) -> Result<(Option<&'a CompatTool>, Option<ChoiceSource>)> {
    if let Some(name) = &opts.tool {
        let t = tools
            .iter()
            .find(|t| &t.internal_name == name)
            .with_context(|| format!("no installed tool named {name}"))?;
        return Ok((Some(t), Some(ChoiceSource::OurApp)));
    }
    let picked = compat::pick_tool_with(tools, appid, ours, steam, valve);
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

/// Log file of the last launch of a game, next to our other data: what we ran, then everything it printed.
pub fn log_path(appid: u32) -> PathBuf {
    store::data_dir().join("logs").join(format!("launch-{appid}.log"))
}

/// Home folder replaced by `~`, so a log can be pasted into a bug report.
fn tidy(text: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if h.len() > 1 => text.replace(&h, "~"),
        _ => text.to_string(),
    }
}

/// Is this variable one whose value must not appear in a log?
fn secret_name(k: &str) -> bool {
    let k = k.to_ascii_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "PASSWD", "AUTH", "KEY", "COOKIE", "CREDENTIAL"].iter().any(|w| k.contains(w))
}

/// The plan as text: command, working folder and the variables we add (secrets hidden, home folder as `~`).
/// Same text as the top of the launch log and the "details" of a failure.
pub fn describe(plan: &LaunchPlan) -> String {
    let env: Vec<(String, String)> = plan
        .env
        .iter()
        .map(|(k, v)| (k.clone(), if secret_name(k) { "<hidden>".to_string() } else { v.clone() }))
        .collect();
    let mut out = format!("runner:  {:?}\ncwd:     {}\n", plan.runner, plan.cwd.display());
    for (k, v) in &env {
        out.push_str(&format!("env:     {k}={v}\n"));
    }
    out.push_str(&format!("command: {}\n", launch_opts::command_line(plan, &[])));
    tidy(&out)
}

/// The last `lines` lines of the launch log (empty when there is none).
pub fn log_tail(appid: u32, lines: usize) -> String {
    let text = std::fs::read_to_string(log_path(appid)).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Is the Wine prefix of `compatdata` half-made (a `pfx` folder without `system.reg`)? Such a folder is what a
/// first launch leaves behind when Proton dies early, and every later launch then fails with
/// "C:\\windows ... failed". Returns what was done, if anything.
pub fn heal_prefix(compatdata: &Path) -> Option<String> {
    let pfx = compatdata.join("pfx");
    if !pfx.is_dir() || pfx.join("system.reg").is_file() {
        return None;
    }
    if dir_has_files(&pfx.join("drive_c/users/steamuser"), 4) {
        return Some(format!(
            "note: {} has no system.reg (an earlier launch did not finish) but holds files, so it was left alone",
            pfx.display()
        ));
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let aside = compatdata.join(format!("pfx.incomplete-{stamp}"));
    match std::fs::rename(&pfx, &aside) {
        Ok(()) => Some(format!(
            "note: {} was incomplete and empty; moved to {} so Proton makes a new one",
            pfx.display(),
            aside.display()
        )),
        Err(e) => Some(format!("note: {} is incomplete and could not be moved aside: {e}", pfx.display())),
    }
}

/// Any regular file within `depth` levels of `dir`.
fn dir_has_files(dir: &Path, depth: usize) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| match e.file_type() {
        Ok(t) if t.is_dir() => depth > 0 && dir_has_files(&e.path(), depth - 1),
        Ok(_) => true,
        Err(_) => false,
    })
}

/// Words for a failed `spawn`: which file is missing when the OS only says "No such file or directory".
fn spawn_error(plan: &LaunchPlan, e: &std::io::Error) -> anyhow::Error {
    let why = if !plan.cwd.is_dir() {
        format!("the working folder {} does not exist", plan.cwd.display())
    } else if plan.program.is_absolute() && !plan.program.exists() {
        format!("{} does not exist", plan.program.display())
    } else {
        e.to_string()
    };
    anyhow::anyhow!("Could not start {}: {why} ({e})", plan.program.display())
}

/// Start the plan and return the child's pid at once. A helper thread waits for the child (so it never
/// becomes a zombie) and calls `on_exit` when it ends. stdout and stderr go to [`log_path`], below a header
/// with the command.
pub fn spawn(plan: &LaunchPlan, on_exit: impl FnOnce(Finished) + Send + 'static) -> Result<u32> {
    spawn_with_preamble(plan, "", on_exit)
}

/// Add lines to the end of a game's launch log (for what happens after it ended, such as the Cloud upload).
pub fn append_log(appid: u32, text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(log_path(appid)) {
        let _ = write!(f, "{}", tidy(text));
    }
}

/// Like [`spawn`], with `preamble` written to the log before the launch description (what was done before the
/// start, such as the Cloud download).
pub fn spawn_with_preamble(
    plan: &LaunchPlan,
    preamble: &str,
    on_exit: impl FnOnce(Finished) + Send + 'static,
) -> Result<u32> {
    use std::io::Write;
    for d in &plan.create_dirs {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    let note = plan.env.get("STEAM_COMPAT_DATA_PATH").and_then(|d| heal_prefix(Path::new(d)));
    let log = log_path(plan.appid);
    if let Some(dir) = log.parent() {
        store::private_dir(dir)?;
    }
    let mut out = std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
    if !preamble.is_empty() {
        let _ = write!(out, "{}", tidy(preamble));
    }
    let _ = write!(out, "{}", describe(plan));
    if let Some(n) = note {
        let _ = writeln!(out, "{}", tidy(&n));
    }
    let _ = writeln!(out, "---- output ----");
    let mut cmd = std::process::Command::new(&plan.program);
    cmd.args(&plan.args)
        .current_dir(&plan.cwd)
        .envs(&plan.env)
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // own group: the UI's Ctrl-C doesn't hit the game
    let mut child = cmd.spawn().map_err(|e| spawn_error(plan, &e))?;
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
            libraries: &[],
            compat: None,
            proton_log: false,
            native_runtime: None,
            legacy_runtime: None,
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
            libraries: i.libraries,
            compat: i.compat,
            proton_log: i.proton_log,
            native_runtime: i.native_runtime,
            legacy_runtime: i.legacy_runtime,
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

    fn mk_entry(id: &str, exe: &str, os: &[&str]) -> LaunchEntry {
        LaunchEntry {
            id: id.into(),
            executable: exe.into(),
            oslist: os.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// Fallout 2 as Steam has it: entry 0 for Windows, entry 1 with no list at all but the same `.exe`. The
    /// second one used to be picked as "native" and started as a Linux program: "no such file".
    #[test]
    fn fallout2_layout_runs_through_proton() {
        let info = AppInfo {
            appid: 38410,
            launch: vec![
                mk_entry("0", "fallout2launcher.exe", &["windows"]),
                LaunchEntry { arguments: "-classic".into(), ..mk_entry("1", "fallout2launcher.exe", &[]) },
            ],
            ..Default::default()
        };
        assert!(select_entry(&info, "linux").is_none());
        let e = choose_entry(&info, HostOs::Linux, false).unwrap();
        assert_eq!(e.id, "0");
        assert!(needs_compat(e, HostOs::Linux));
        // The entry without a list alone is still a Windows program because of its `.exe`.
        let only = AppInfo { launch: vec![mk_entry("1", "game.EXE", &[])], ..Default::default() };
        let e = choose_entry(&only, HostOs::Linux, false).unwrap();
        assert!(needs_compat(e, HostOs::Linux));
        // ... while a script without a list is native.
        let script = AppInfo { launch: vec![mk_entry("0", "run.sh", &[])], ..Default::default() };
        let e = choose_entry(&script, HostOs::Linux, false).unwrap();
        assert!(!needs_compat(e, HostOs::Linux));
    }

    #[test]
    fn entry_filters_arch_type_beta() {
        let mk = |id: &str, os: &[&str], arch: Option<&str>, beta: Option<&str>, kind: Option<&str>| LaunchEntry {
            osarch: arch.map(str::to_string),
            betakey: beta.map(str::to_string),
            kind: kind.map(str::to_string),
            ..mk_entry(id, &format!("e{id}"), os)
        };
        let only32 = AppInfo { launch: vec![mk("0", &["windows"], Some("32"), None, None)], ..Default::default() };
        assert_eq!(select_entry(&only32, "windows").unwrap().id, "0", "32-bit is used when it is all there is");
        let info = AppInfo {
            launch: vec![
                mk("0", &["windows"], None, Some("beta"), None),
                mk("1", &["windows"], None, None, Some("vr")),
                mk("2", &["windows"], Some("32"), None, None),
                mk("3", &["windows"], Some("64"), None, None),
                mk("4", &["windows"], None, None, Some("none")),
            ],
            ..Default::default()
        };
        assert_eq!(select_entry(&info, "windows").unwrap().id, "3");
    }

    #[test]
    fn exe_case_is_resolved() {
        let dir = std::env::temp_dir().join(format!("sr-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Data/Bin")).unwrap();
        std::fs::write(dir.join("Data/Bin/FALLOUT2.EXE"), b"").unwrap();
        let p = resolve_inside(&dir, "data\\bin\\Fallout2.exe").unwrap();
        assert_eq!(p, dir.join("Data/Bin/FALLOUT2.EXE"));
        // No match: kept as written, so the message names what Steam's data says.
        assert_eq!(resolve_inside(&dir, "Nope/x.exe").unwrap(), dir.join("Nope/x.exe"));
        assert!(resolve_inside(&dir, "..\\x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn container_env_and_valve_config() {
        let e = entry("game.exe", &["windows"]);
        let t = tool(ToolKind::Proton, None);
        let mut rt = tool(ToolKind::Runtime, None);
        rt.dir = PathBuf::from("/lib/steamapps/common/SteamLinuxRuntime_sniper");
        rt.commandline = Some("/_v2-entry-point --verb=%verb% --".into());
        let libs = [PathBuf::from("/lib"), PathBuf::from("/mnt/games")];
        let cfg = AppCompatConfig { config: "noesync,oldglstr".into(), environment: vec![("A".into(), "1".into())] };
        let mut i = input(&e, Some(&t), HostOs::Linux);
        i.runtime = Some(&rt);
        i.libraries = &libs;
        i.compat = Some(&cfg);
        i.proton_log = true;
        let p = plan(&i).unwrap();
        assert_eq!(p.env["STEAM_COMPAT_LIBRARY_PATHS"], "/lib:/mnt/games");
        assert_eq!(p.env["STEAM_COMPAT_SHADER_PATH"], "/lib/steamapps/shadercache/730");
        assert_eq!(p.env["STEAM_COMPAT_CONFIG"], "noesync,oldglstr");
        assert_eq!(p.env["A"], "1");
        assert_eq!(p.env["SteamEnv"], "1");
        assert_eq!(p.env["PROTON_LOG"], "1");
        assert_eq!(
            p.env["STEAM_COMPAT_TOOL_PATHS"],
            "/tools/GE-Proton9-20:/lib/steamapps/common/SteamLinuxRuntime_sniper"
        );
        assert_eq!(p.args[..3], ["--verb=waitforexitandrun", "--", "/tools/GE-Proton9-20/proton"]);
        assert_eq!(p.args[3], "waitforexitandrun");
    }

    #[test]
    fn native_games_use_the_legacy_runtime_or_their_container() {
        let e = entry("game.sh", &["linux"]);
        let mut i = input(&e, None, HostOs::Linux);
        i.legacy_runtime = Some(Path::new("/steam/ubuntu12_32/steam-runtime/run.sh"));
        let p = plan(&i).unwrap();
        assert_eq!(p.program, Path::new("/steam/ubuntu12_32/steam-runtime/run.sh"));
        assert_eq!(p.args[0], "/lib/steamapps/common/G/game.sh");
        let mut rt = tool(ToolKind::Runtime, None);
        rt.dir = PathBuf::from("/rt/sniper");
        rt.commandline = Some("/_v2-entry-point --verb=%verb% --".into());
        i.native_runtime = Some(&rt);
        let p = plan(&i).unwrap();
        assert_eq!(p.program, Path::new("/rt/sniper/_v2-entry-point"));
        assert_eq!(p.args[..3], ["--verb=waitforexitandrun", "--", "/lib/steamapps/common/G/game.sh"]);
        assert_eq!(p.env["STEAM_COMPAT_TOOL_PATHS"], "/rt/sniper");
        assert!(!p.env.contains_key("STEAM_COMPAT_DATA_PATH"));
    }

    #[test]
    fn runtime_is_found_by_manifest_and_flatpak_paths_stay_in_the_flatpak() {
        let root = std::env::temp_dir().join(format!("sr-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A Flatpak-style Steam root: everything below ~/.var/app/<id>/.local/share/Steam.
        let steam = root.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        let rtdir = steam.join("steamapps/common/SteamLinuxRuntime_sniper");
        std::fs::create_dir_all(&rtdir).unwrap();
        std::fs::write(rtdir.join("_v2-entry-point"), b"").unwrap();
        std::fs::write(steam.join("steamapps/appmanifest_1628350.acf"), b"").unwrap();
        let mut rt = tool(ToolKind::Runtime, None);
        rt.internal_name = "whatever".into();
        rt.dir = rtdir.clone();
        rt.commandline = Some("/_v2-entry-point --verb=%verb% --".into());
        let proton = tool(ToolKind::Proton, None);
        let tools = vec![rt];
        assert_eq!(runtime_for(&proton, &tools).map(|t| t.dir.clone()), Some(rtdir));
        let mut none = tool(ToolKind::Proton, None);
        none.require_tool_appid = None;
        assert!(runtime_for(&none, &tools).is_none());
        // compatdata for a game in that library is in the same library: prefix shared with the official client.
        let game = steam.join("steamapps/common/Fallout 2");
        assert_eq!(compatdata_dir(&game, 38410), steam.join("steamapps/compatdata/38410"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn half_made_prefix_is_moved_aside_only_when_empty() {
        let dir = std::env::temp_dir().join(format!("sr-pfx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pfx/drive_c/windows")).unwrap();
        let said = heal_prefix(&dir).unwrap();
        assert!(said.contains("moved"), "{said}");
        assert!(!dir.join("pfx").exists());
        // With saves inside it is left alone; a complete one is not touched.
        std::fs::create_dir_all(dir.join("pfx/drive_c/users/steamuser/Documents")).unwrap();
        std::fs::write(dir.join("pfx/drive_c/users/steamuser/Documents/save.dat"), b"x").unwrap();
        assert!(heal_prefix(&dir).unwrap().contains("left alone"));
        assert!(dir.join("pfx").is_dir());
        std::fs::write(dir.join("pfx/system.reg"), b"").unwrap();
        assert!(heal_prefix(&dir).is_none());
        assert!(heal_prefix(&dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describe_hides_secrets_and_home() {
        let mut p = plan(&input(&entry("g.sh", &["linux"]), None, HostOs::Linux)).unwrap();
        p.env.insert("MY_API_KEY".into(), "hunter2".into());
        let text = describe(&p);
        assert!(!text.contains("hunter2") && text.contains("MY_API_KEY=<hidden>"));
        assert!(text.contains("command:"));
    }
}
