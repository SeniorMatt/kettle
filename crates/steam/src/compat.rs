//! Installed compatibility tools (Proton and friends): discovery and per-game choice. Read-only.
//!
//! Tools live in three kinds of places:
//! - Valve's own, as Steam apps: `<library>/steamapps/common/Proton*` and `SteamLinuxRuntime_*`;
//! - custom ones in a `compatibilitytools.d` folder (GE-Proton, Proton-CachyOS, Luxtorpeda ...), in any Steam
//!   root, in system folders, or in our own data dir;
//! - nothing else: we never look inside the official client's private state.
//!
//! What the user picked in the official client is read from `config/config.vdf` (`CompatToolMapping`), so we
//! default to the same tool Steam would use. Our own choices are kept in a small JSON in our data dir.

use crate::appinfo::{AppInfoFile, Kv};
use crate::{local, store, vdf};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// What a tool does, which decides how a game is started with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    /// Wine-based: `<dir>/proton <verb> <exe>`.
    Proton,
    /// Steam Linux Runtime container (a helper for Proton, not a tool to pick for a game).
    Runtime,
    /// Luxtorpeda: runs native engine ports instead of the Windows exe. Has its own entry point.
    Luxtorpeda,
    Other,
}

/// Where a tool was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOrigin {
    /// A Steam app in a library (`steamapps/common`).
    SteamLibrary,
    /// `compatibilitytools.d` of a Steam install.
    SteamCustom,
    /// A system-wide `compatibilitytools.d`.
    System,
    /// `compatibilitytools.d` in our own data dir.
    Ours,
}

#[derive(Debug, Clone)]
pub struct CompatTool {
    /// The name Steam stores in `CompatToolMapping` (e.g. `proton_experimental`, `GE-Proton9-20`).
    pub internal_name: String,
    pub display_name: String,
    pub dir: PathBuf,
    pub kind: ToolKind,
    pub origin: ToolOrigin,
    /// From the tool's `version` file, e.g. `proton-9.0-3`.
    pub version: Option<String>,
    /// `commandline` of `toolmanifest.vdf`, e.g. `/proton %verb%`.
    pub commandline: Option<String>,
    /// Steam app the tool needs as its container (`require_tool_appid`), e.g. 1628350 = Linux Runtime 3.0 sniper.
    pub require_tool_appid: Option<u32>,
}

impl CompatTool {
    /// Program and arguments for a verb such as `waitforexitandrun`, from the tool manifest. Falls back to
    /// `<dir>/proton <verb>` for tools without a manifest. The program is absolute.
    pub fn command(&self, verb: &str) -> (PathBuf, Vec<String>) {
        let cl = self.commandline.as_deref().unwrap_or("/proton %verb%");
        let mut parts = cl.split_whitespace();
        let prog = parts.next().unwrap_or("/proton").trim_start_matches('/');
        let args = parts.map(|p| p.replace("%verb%", verb)).collect();
        (self.dir.join(prog), args)
    }
}

/// Every `compatibilitytools.d` folder we know of, with its origin. Folders need not exist.
pub fn custom_tool_dirs() -> Vec<(PathBuf, ToolOrigin)> {
    let mut out = Vec::new();
    let mut roots = local::steam_roots();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // Not every layout is picked up by `steam_roots` (it needs `steamapps` there), so list them again.
        for r in [".steam/root", ".local/share/Steam", ".var/app/com.valvesoftware.Steam/data/Steam"] {
            roots.push(home.join(r));
        }
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    for r in roots {
        push_unique(&mut out, r.join("compatibilitytools.d"), ToolOrigin::SteamCustom);
    }
    for sys in ["/usr/share/steam/compatibilitytools.d", "/usr/local/share/steam/compatibilitytools.d"] {
        push_unique(&mut out, PathBuf::from(sys), ToolOrigin::System);
    }
    push_unique(&mut out, our_tools_dir(), ToolOrigin::Ours);
    out
}

/// Our own install target: `<data_dir>/compatibilitytools.d`. `KETTLE_TOOLS_DIR` overrides it (tests, demos).
pub fn our_tools_dir() -> PathBuf {
    match std::env::var_os("KETTLE_TOOLS_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => store::data_dir().join("compatibilitytools.d"),
    }
}

fn push_unique(out: &mut Vec<(PathBuf, ToolOrigin)>, dir: PathBuf, origin: ToolOrigin) {
    // `~/.steam/root` and the real folder are the same place; compare canonical paths when they exist.
    let key = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if !out.iter().any(|(d, _)| d.canonicalize().unwrap_or_else(|_| d.clone()) == key) {
        out.push((dir, origin));
    }
}

/// All installed tools, deduplicated by directory, Valve's first.
pub fn discover() -> Vec<CompatTool> {
    let mut tools = Vec::new();
    for root in local::steam_roots() {
        for lib in local::library_folders(&root) {
            tools.extend(scan_library(&lib.join("steamapps/common")));
        }
    }
    for (dir, origin) in custom_tool_dirs() {
        tools.extend(scan_tools_dir(&dir, origin));
    }
    let mut seen = Vec::new();
    tools.retain(|t| {
        let key = (t.dir.canonicalize().unwrap_or_else(|_| t.dir.clone()), t.internal_name.clone());
        let fresh = !seen.contains(&key);
        seen.push(key);
        fresh
    });
    tools
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

/// `steamapps/common`: only folders that look like a tool (manifest or `proton` script) and are named like one.
fn scan_library(common: &Path) -> Vec<CompatTool> {
    subdirs(common)
        .into_iter()
        .filter(|d| {
            let n = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (n.starts_with("Proton") || n.starts_with("SteamLinuxRuntime"))
                && (d.join("toolmanifest.vdf").is_file() || d.join("proton").is_file())
        })
        .map(|d| {
            let name = d.file_name().unwrap().to_string_lossy().into_owned();
            let internal = valve_internal_name(&name).unwrap_or_else(|| name.clone());
            build_tool(&d, internal, name, ToolOrigin::SteamLibrary)
        })
        .collect()
}

/// A `compatibilitytools.d` folder: each subfolder is a tool, described by `compatibilitytool.vdf` when present
/// (which may define several tools, or none that matter).
pub fn scan_tools_dir(dir: &Path, origin: ToolOrigin) -> Vec<CompatTool> {
    let mut out = Vec::new();
    for d in subdirs(dir) {
        let dname = d.file_name().unwrap().to_string_lossy().into_owned();
        let defs = read_tool_defs(&d);
        if defs.is_empty() {
            if d.join("toolmanifest.vdf").is_file() || d.join("proton").is_file() {
                out.push(build_tool(&d, dname.clone(), dname, origin));
            }
            continue;
        }
        for def in defs {
            // `.` is the usual value; keep the plain folder path in that case.
            let root =
                if matches!(def.install_path.as_str(), "." | "") { d.clone() } else { d.join(&def.install_path) };
            out.push(build_tool(&root, def.internal_name, def.display_name.unwrap_or(dname.clone()), origin));
        }
    }
    out
}

struct ToolDef {
    internal_name: String,
    display_name: Option<String>,
    install_path: String,
}

/// Parse `compatibilitytool.vdf`: `compatibilitytools { compat_tools { "<name>" { install_path display_name ... } } }`.
fn read_tool_defs(dir: &Path) -> Vec<ToolDef> {
    let Ok(src) = std::fs::read_to_string(dir.join("compatibilitytool.vdf")) else { return vec![] };
    parse_tool_defs(&src)
}

fn parse_tool_defs(src: &str) -> Vec<ToolDef> {
    let doc = vdf::parse(src);
    let Some(tools) = doc.path(&["compatibilitytools", "compat_tools"]) else { return vec![] };
    tools
        .children()
        .iter()
        .map(|(name, n)| ToolDef {
            internal_name: name.clone(),
            display_name: n.get_str("display_name").map(str::to_string),
            install_path: n.get_str("install_path").unwrap_or(".").to_string(),
        })
        .collect()
}

fn build_tool(dir: &Path, internal_name: String, display_name: String, origin: ToolOrigin) -> CompatTool {
    let manifest = std::fs::read_to_string(dir.join("toolmanifest.vdf")).ok().map(|s| vdf::parse(&s));
    let man = manifest.as_ref().and_then(|m| m.get("manifest"));
    let commandline = man.and_then(|m| m.get_str("commandline")).map(str::to_string);
    let require_tool_appid = man.and_then(|m| m.get_u64("require_tool_appid")).and_then(|n| u32::try_from(n).ok());
    let version =
        std::fs::read_to_string(dir.join("version")).ok().and_then(|v| v.split_whitespace().nth(1).map(str::to_string));
    let kind = classify(&internal_name, dir, commandline.as_deref());
    CompatTool {
        internal_name,
        display_name,
        dir: dir.to_path_buf(),
        kind,
        origin,
        version,
        commandline,
        require_tool_appid,
    }
}

fn classify(internal: &str, dir: &Path, commandline: Option<&str>) -> ToolKind {
    let lname = internal.to_ascii_lowercase();
    let cl = commandline.unwrap_or("");
    if lname.contains("luxtorpeda") || cl.contains("luxtorpeda") {
        ToolKind::Luxtorpeda
    } else if lname.starts_with("steamlinuxruntime") || cl.contains("_v2-entry-point") {
        ToolKind::Runtime
    } else if dir.join("proton").is_file() || cl.contains("/proton") {
        ToolKind::Proton
    } else {
        ToolKind::Other
    }
}

/// The name Steam uses in `CompatToolMapping` for Valve's own Protons, derived from the folder name:
/// "Proton - Experimental" -> `proton_experimental`, "Proton 9.0 (Beta)" -> `proton_9`, "Proton 6.3" -> `proton_63`.
pub fn valve_internal_name(dir_name: &str) -> Option<String> {
    let rest = dir_name.strip_prefix("Proton")?.trim();
    let lower = rest.to_ascii_lowercase();
    if lower.contains("experimental") {
        return Some("proton_experimental".into());
    }
    if lower.contains("hotfix") {
        return Some("proton_hotfix".into());
    }
    let mut nums = rest.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
    let major: u32 = nums.next()?.parse().ok()?;
    if major >= 7 {
        return Some(format!("proton_{major}"));
    }
    let minor = nums.next().unwrap_or("0");
    Some(if minor == "0" { format!("proton_{major}") } else { format!("proton_{major}{minor}") })
}

/// Steam's own per-game choices: `appid -> tool name`; appid 0 is the global default ("Enable Steam Play for all
/// other titles"). Reads `<steam root>/config/config.vdf` only.
pub fn steam_mapping(steam_root: &Path) -> HashMap<u32, String> {
    let Ok(src) = std::fs::read_to_string(steam_root.join("config/config.vdf")) else { return HashMap::new() };
    parse_steam_mapping(&src)
}

fn parse_steam_mapping(src: &str) -> HashMap<u32, String> {
    let doc = vdf::parse(src);
    let Some(map) = doc.path(&["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"]) else {
        return HashMap::new();
    };
    map.children()
        .iter()
        .filter_map(|(id, n)| {
            let name = n.get_str("name").filter(|s| !s.is_empty())?;
            Some((id.parse().ok()?, name.to_string()))
        })
        .collect()
}

/// `steam_mapping` merged over all Steam roots (the first root that has an entry wins).
pub fn steam_mapping_all() -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for root in local::steam_roots() {
        for (id, name) in steam_mapping(&root) {
            out.entry(id).or_insert(name);
        }
    }
    out
}

/// Our own per-game choices, saved as JSON in our data dir.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Choices {
    /// Tool used when nothing else applies.
    pub default: Option<String>,
    /// `appid -> internal tool name` (string keys, because JSON).
    pub per_app: BTreeMap<String, String>,
}

impl Choices {
    pub fn path() -> PathBuf {
        store::data_dir().join("compat_choices.json")
    }

    pub fn load() -> Choices {
        store::load_json(&Choices::path()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        store::save_json(&Choices::path(), self)
    }

    pub fn set_app(&mut self, appid: u32, tool: Option<&str>) {
        match tool {
            Some(t) => self.per_app.insert(appid.to_string(), t.to_string()),
            None => self.per_app.remove(&appid.to_string()),
        };
    }
}

/// Why a tool was picked; lets the UI say "from Steam" or "your choice".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceSource {
    OurApp,
    SteamApp,
    OurDefault,
    SteamDefault,
    /// Valve's own per-game override (app 891390 `app_mappings`).
    ValveApp,
    /// Valve's default (the `proton-stable` alias of app 891390).
    ValveDefault,
    /// Nothing was chosen anywhere; the newest installed Proton.
    Newest,
}

/// The tool to run `appid` with, in the order the official client uses: our per-game choice, the user's per-game
/// choice in Steam, Valve's per-game override, our default, the user's global default in Steam, Valve's default
/// (`proton-stable`), and finally the newest installed Proton. Choices naming a tool that isn't installed are
/// skipped (the official client would download it; we only use what is there).
pub fn pick_tool<'a>(
    tools: &'a [CompatTool],
    appid: u32,
    ours: &Choices,
    steam: &HashMap<u32, String>,
) -> Option<(&'a CompatTool, ChoiceSource)> {
    pick_tool_with(tools, appid, ours, steam, &SteamPlay::default())
}

/// [`pick_tool`] with Valve's tables ([`SteamPlay::load`]).
pub fn pick_tool_with<'a>(
    tools: &'a [CompatTool],
    appid: u32,
    ours: &Choices,
    steam: &HashMap<u32, String>,
    valve: &SteamPlay,
) -> Option<(&'a CompatTool, ChoiceSource)> {
    let find = |name: &str| {
        let name = valve.resolve(name);
        tools.iter().find(|t| t.internal_name == name && t.kind != ToolKind::Runtime)
    };
    let valve_app = valve.app_tool.get(&appid).filter(|n| !n.is_empty());
    let valve_default = valve.default_tool();
    let candidates = [
        (ours.per_app.get(&appid.to_string()), ChoiceSource::OurApp),
        (steam.get(&appid), ChoiceSource::SteamApp),
        (valve_app, ChoiceSource::ValveApp),
        (ours.default.as_ref(), ChoiceSource::OurDefault),
        (steam.get(&0), ChoiceSource::SteamDefault),
        (valve_default.as_ref(), ChoiceSource::ValveDefault),
    ];
    for (name, src) in candidates {
        if let Some(t) = name.and_then(|n| find(n)) {
            return Some((t, src));
        }
    }
    newest_proton(tools).map(|t| (t, ChoiceSource::Newest))
}

/// What Valve ships in `appinfo.vdf` app 891390 ("SteamPlay 2.0 Manifests"): which tool each game gets, the
/// aliases of tool names (`proton-stable`), and the per-game Proton settings the official client passes on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SteamPlay {
    /// `internal name -> aliases` of `extended/compat_tools`.
    pub aliases: Vec<(String, Vec<String>)>,
    /// `appid -> tool name or alias` of `extended/app_mappings` (empty value: no tool).
    pub app_tool: HashMap<u32, String>,
    /// `appid -> settings` of `extended/app_compat_configs`.
    pub app_config: HashMap<u32, AppCompatConfig>,
}

/// One `app_compat_configs` row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppCompatConfig {
    /// Comma separated switches (`oldglstr`, `noesync,nofsync`, `cmdlineappend:-changedir`...): the official client
    /// hands them to Proton in `STEAM_COMPAT_CONFIG`.
    pub config: String,
    /// Extra variables for the game.
    pub environment: Vec<(String, String)>,
}

impl SteamPlay {
    pub fn from_kv(root: &Kv) -> SteamPlay {
        let mut out = SteamPlay::default();
        let Some(ext) = root.get("extended") else { return out };
        for (name, t) in ext.get("compat_tools").map(Kv::children).unwrap_or_default() {
            let list = t.get_string("aliases").unwrap_or_default();
            out.aliases.push((name.clone(), list.split(',').map(|a| a.trim().to_string()).collect()));
        }
        for (id, m) in ext.get("app_mappings").map(Kv::children).unwrap_or_default() {
            let (Ok(id), platform) = (id.parse::<u32>(), m.get_string("platform").unwrap_or_default()) else {
                continue;
            };
            if platform.is_empty() || platform == "linux" {
                out.app_tool.insert(id, m.get_string("tool").unwrap_or_default());
            }
        }
        for (id, c) in ext.get("app_compat_configs").map(Kv::children).unwrap_or_default() {
            let Ok(id) = id.parse::<u32>() else { continue };
            let config = c.get_string("config").filter(|s| s != "none").unwrap_or_default();
            let environment = parse_env_list(&c.get_string("environment").unwrap_or_default());
            out.app_config.insert(id, AppCompatConfig { config, environment });
        }
        out
    }

    /// Read from the official client's `appinfo.vdf` in any Steam root. Empty when there is none.
    pub fn load() -> SteamPlay {
        for root in local::steam_roots() {
            let path = AppInfoFile::path_in(&root);
            if let Ok(mut f) = AppInfoFile::open(&path)
                && let Ok(Some(kv)) = f.get_kv(891390)
            {
                return SteamPlay::from_kv(&kv);
            }
        }
        SteamPlay::default()
    }

    /// An alias (`proton-stable`) as the internal tool name; other names unchanged.
    pub fn resolve(&self, name: &str) -> String {
        if self.aliases.iter().any(|(n, _)| n == name) {
            return name.to_string();
        }
        self.aliases
            .iter()
            .find(|(_, al)| al.iter().any(|a| a == name))
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| name.to_string())
    }

    /// Valve's default tool: what `proton-stable` stands for.
    pub fn default_tool(&self) -> Option<String> {
        let n = self.resolve("proton-stable");
        self.aliases.iter().any(|(k, _)| *k == n).then_some(n)
    }
}

/// `A=1:B=2` as pairs (Valve's separator is a colon; a colon inside a value stays when what follows isn't `NAME=`).
fn parse_env_list(s: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in s.split(':').filter(|p| !p.is_empty()) {
        let pair = part.split_once('=').filter(|(k, _)| {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit())
        });
        match (pair, out.last_mut()) {
            (Some((k, v)), _) => out.push((k.to_string(), v.to_string())),
            (None, Some(last)) => {
                last.1.push(':');
                last.1.push_str(part);
            }
            _ => {}
        }
    }
    out
}

fn newest_proton(tools: &[CompatTool]) -> Option<&CompatTool> {
    tools.iter().filter(|t| t.kind == ToolKind::Proton).max_by_key(|t| version_key(&t.display_name))
}

/// Numbers in a name as a sortable key: "GE-Proton9-20" -> [9, 20].
fn version_key(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(internal: &str, kind: ToolKind) -> CompatTool {
        CompatTool {
            internal_name: internal.into(),
            display_name: internal.into(),
            dir: PathBuf::from("/t").join(internal),
            kind,
            origin: ToolOrigin::Ours,
            version: None,
            commandline: None,
            require_tool_appid: None,
        }
    }

    #[test]
    fn valve_names() {
        assert_eq!(valve_internal_name("Proton - Experimental").as_deref(), Some("proton_experimental"));
        assert_eq!(valve_internal_name("Proton Hotfix").as_deref(), Some("proton_hotfix"));
        assert_eq!(valve_internal_name("Proton 9.0 (Beta)").as_deref(), Some("proton_9"));
        assert_eq!(valve_internal_name("Proton 10.0").as_deref(), Some("proton_10"));
        assert_eq!(valve_internal_name("Proton 6.3").as_deref(), Some("proton_63"));
        assert_eq!(valve_internal_name("SteamLinuxRuntime_sniper"), None);
    }

    #[test]
    fn parses_tool_vdf_and_mapping() {
        let defs = parse_tool_defs(
            r#""compatibilitytools" { "compat_tools" { "GE-Proton9-20" {
                "install_path" "." "display_name" "GE-Proton9-20" "from_oslist" "windows" "to_oslist" "linux" } } }"#,
        );
        assert_eq!(defs.len(), 1);
        assert_eq!(
            (defs[0].internal_name.as_str(), defs[0].display_name.as_deref()),
            ("GE-Proton9-20", Some("GE-Proton9-20"))
        );
        let m = parse_steam_mapping(
            r#""InstallConfigStore" { "Software" { "Valve" { "Steam" { "CompatToolMapping" {
                "0" { "name" "proton_9" } "730" { "name" "GE-Proton9-20" } "5" { "name" "" } } } } } }"#,
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m[&730], "GE-Proton9-20");
    }

    #[test]
    fn manifest_command_and_kinds() {
        let mut t = tool("lux", ToolKind::Luxtorpeda);
        t.commandline = Some("/luxtorpeda %verb%".into());
        assert_eq!(t.command("run"), (PathBuf::from("/t/lux/luxtorpeda"), vec!["run".to_string()]));
        assert_eq!(classify("luxtorpeda", Path::new("/nonexistent"), None), ToolKind::Luxtorpeda);
        assert_eq!(classify("SteamLinuxRuntime_sniper", Path::new("/nonexistent"), None), ToolKind::Runtime);
        assert_eq!(classify("x", Path::new("/nonexistent"), Some("/proton %verb%")), ToolKind::Proton);
    }

    #[test]
    fn pick_order() {
        let tools = vec![
            tool("proton_9", ToolKind::Proton),
            tool("GE-Proton9-20", ToolKind::Proton),
            tool("lux", ToolKind::Luxtorpeda),
        ];
        let steam =
            HashMap::from([(730, "GE-Proton9-20".to_string()), (0, "proton_9".to_string()), (1, "gone".to_string())]);
        let mut ours = Choices::default();
        let name = |r: Option<(&CompatTool, ChoiceSource)>| r.map(|(t, s)| (t.internal_name.clone(), s));
        assert_eq!(name(pick_tool(&tools, 730, &ours, &steam)), Some(("GE-Proton9-20".into(), ChoiceSource::SteamApp)));
        ours.set_app(730, Some("lux"));
        assert_eq!(name(pick_tool(&tools, 730, &ours, &steam)), Some(("lux".into(), ChoiceSource::OurApp)));
        assert_eq!(name(pick_tool(&tools, 1, &ours, &steam)), Some(("proton_9".into(), ChoiceSource::SteamDefault)));
        assert_eq!(
            name(pick_tool(&tools, 1, &ours, &HashMap::new())),
            Some(("GE-Proton9-20".into(), ChoiceSource::Newest))
        );
    }

    #[test]
    fn valve_tables_decide_before_the_global_default() {
        let root = crate::vdf::parse(
            r#""appinfo" { "extended" {
                "compat_tools" {
                    "proton_11" { "aliases" "proton-stable,proton-11.0" }
                    "proton_experimental" { "aliases" "proton-experimental" } }
                "app_mappings" {
                    "5" { "appid" "5" "platform" "linux" "tool" "proton-experimental" }
                    "6" { "appid" "6" "platform" "linux" "tool" "" }
                    "7" { "appid" "7" "platform" "windows" "tool" "proton_11" } }
                "app_compat_configs" {
                    "8" { "config" "noesync,nofsync" "environment" "A=1:B=x:y:C=3" }
                    "9" { "config" "none" } } } }"#,
        );
        let sp = SteamPlay::from_kv(&Kv::from_vdf(root.get("appinfo").unwrap()));
        assert_eq!(sp.default_tool().as_deref(), Some("proton_11"));
        assert_eq!(sp.resolve("proton-experimental"), "proton_experimental");
        assert_eq!(sp.resolve("GE-Proton9-20"), "GE-Proton9-20");
        assert!(!sp.app_tool.contains_key(&7), "other platforms ignored");
        assert_eq!(sp.app_config[&8].config, "noesync,nofsync");
        assert_eq!(
            sp.app_config[&8].environment,
            [("A".into(), "1".into()), ("B".into(), "x:y".into()), ("C".into(), "3".into())]
        );
        assert_eq!(sp.app_config[&9].config, "");

        let tools = vec![
            tool("proton_11", ToolKind::Proton),
            tool("proton_experimental", ToolKind::Proton),
            tool("GE-Proton9-20", ToolKind::Proton),
        ];
        let name = |r: Option<(&CompatTool, ChoiceSource)>| r.map(|(t, s)| (t.internal_name.clone(), s));
        let ours = Choices::default();
        let none = HashMap::new();
        // Valve's per-game override beats nothing else but the user's per-game choice.
        assert_eq!(
            name(pick_tool_with(&tools, 5, &ours, &none, &sp)),
            Some(("proton_experimental".into(), ChoiceSource::ValveApp))
        );
        let steam = HashMap::from([(5, "GE-Proton9-20".to_string()), (0, "GE-Proton9-20".to_string())]);
        assert_eq!(name(pick_tool_with(&tools, 5, &ours, &steam, &sp)).unwrap().1, ChoiceSource::SteamApp);
        // The user's global default beats Valve's default; Valve's default is `proton-stable`.
        assert_eq!(name(pick_tool_with(&tools, 1, &ours, &steam, &sp)).unwrap().1, ChoiceSource::SteamDefault);
        assert_eq!(
            name(pick_tool_with(&tools, 1, &ours, &none, &sp)),
            Some(("proton_11".into(), ChoiceSource::ValveDefault))
        );
        // An empty Valve mapping means no override.
        assert_eq!(name(pick_tool_with(&tools, 6, &ours, &none, &sp)).unwrap().1, ChoiceSource::ValveDefault);
    }
}
