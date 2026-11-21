//! Per-game environment variables: a list of `KEY=VALUE` lines plus quick switches for common ones
//! (DXVK HUD, NVAPI, WineD3D, GameMode). Saved in `game_env.json`; applied to the launch plan of the game. The
//! variables show up in the launch log like all others (values of secret-looking names are hidden there).

use super::{Client, Command};
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::run::LaunchPlan;
use sr_steam::store;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const MAX_VARS: usize = 64;
pub const MAX_KEY: usize = 128;
pub const MAX_VALUE: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameEnv {
    /// The user's own variables; they win over the switches below.
    pub vars: Vec<EnvVar>,
    /// `DXVK_HUD=fps`.
    pub dxvk_hud_fps: bool,
    /// `PROTON_ENABLE_NVAPI=1`.
    pub nvapi: bool,
    /// `PROTON_USE_WINED3D=1` (OpenGL instead of Vulkan translation).
    pub wined3d: bool,
    /// Start the game through `gamemoderun` (when installed).
    pub gamemode: bool,
}

/// A name the environment accepts: letters, digits and `_`, not starting with a digit.
pub fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    key.len() <= MAX_KEY
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn valid_value(value: &str) -> bool {
    value.len() <= MAX_VALUE && !value.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
}

/// `KEY=VALUE` (spaces around the whole line are dropped; the value may be empty or contain `=`).
pub fn parse_line(line: &str) -> Result<EnvVar, String> {
    let line = line.trim();
    let (key, value) = line.split_once('=').ok_or_else(|| format!("\"{line}\" needs the form KEY=VALUE."))?;
    let key = key.trim();
    if !valid_key(key) {
        return Err(format!(
            "\"{key}\" is not a valid variable name (letters, digits and _, not starting with a digit)."
        ));
    }
    if !valid_value(value) {
        return Err(format!("The value of {key} is too long or has a line break."));
    }
    Ok(EnvVar { key: key.to_string(), value: value.to_string() })
}

/// One variable per line; empty lines and lines starting with `#` are skipped. The first bad line is the error.
pub fn parse_text(text: &str) -> Result<Vec<EnvVar>, String> {
    let mut out: Vec<EnvVar> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let var = parse_line(line)?;
        // The last one wins when a name repeats.
        out.retain(|v| v.key != var.key);
        out.push(var);
    }
    if out.len() > MAX_VARS {
        return Err(format!("At most {MAX_VARS} variables per game."));
    }
    Ok(out)
}

/// The list as editable text, one `KEY=VALUE` per line.
pub fn to_text(vars: &[EnvVar]) -> String {
    vars.iter().map(|v| format!("{}={}\n", v.key, v.value)).collect()
}

impl GameEnv {
    pub fn is_empty(&self) -> bool {
        *self == GameEnv::default()
    }

    /// Keep only valid, non-repeating variables (a hand-edited file can't smuggle in a bad line).
    pub fn cleaned(mut self) -> GameEnv {
        let mut seen: Vec<String> = Vec::new();
        self.vars.retain(|v| {
            valid_key(&v.key) && valid_value(&v.value) && !seen.contains(&v.key) && {
                seen.push(v.key.clone());
                true
            }
        });
        self.vars.truncate(MAX_VARS);
        self
    }

    /// The variables to add: the switches first, then the user's own.
    pub fn variables(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if self.dxvk_hud_fps {
            env.insert("DXVK_HUD".to_string(), "fps".to_string());
        }
        if self.nvapi {
            env.insert("PROTON_ENABLE_NVAPI".to_string(), "1".to_string());
        }
        if self.wined3d {
            env.insert("PROTON_USE_WINED3D".to_string(), "1".to_string());
        }
        for v in self.clone().cleaned().vars {
            env.insert(v.key, v.value);
        }
        env
    }

    /// Add the variables to `plan`, and put `gamemoderun` (when found) in front of the command.
    /// Returns what was done, for the log.
    pub fn apply(&self, plan: &mut LaunchPlan, gamemoderun: Option<PathBuf>) -> Vec<String> {
        let mut done = Vec::new();
        for (k, v) in self.variables() {
            done.push(format!("{k}={v}"));
            plan.env.insert(k, v);
        }
        if self.gamemode
            && let Some(gm) = gamemoderun
        {
            let program = std::mem::replace(&mut plan.program, gm);
            plan.args.insert(0, program.to_string_lossy().into_owned());
            done.push("gamemoderun".to_string());
        }
        done
    }
}

/// `gamemoderun` on `$PATH`, if installed.
pub fn find_gamemoderun() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("gamemoderun")).find(|p| is_executable(p))
}

fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// What the UI may read.
#[derive(Debug, Clone, Default)]
pub struct GameEnvState {
    /// By app id; games without an entry have none.
    pub apps: BTreeMap<String, GameEnv>,
    /// `gamemoderun` is installed (the GameMode switch only works then).
    pub gamemode_installed: bool,
}

impl GameEnvState {
    pub fn get(&self, app: AppId) -> GameEnv {
        self.apps.get(&app.0.to_string()).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    apps: BTreeMap<String, GameEnv>,
}

fn path() -> PathBuf {
    store::data_dir().join("game_env.json")
}

impl GameEnvState {
    pub(crate) fn load() -> GameEnvState {
        let file: File = store::load_json(&path()).unwrap_or_default();
        let apps = file.apps.into_iter().map(|(k, v)| (k, v.cleaned())).filter(|(_, v)| !v.is_empty()).collect();
        GameEnvState { apps, gamemode_installed: find_gamemoderun().is_some() }
    }
}

impl Client {
    pub(crate) fn set_game_env(&mut self, app: AppId, env: GameEnv) {
        let env = env.cleaned();
        let key = app.0.to_string();
        if env.is_empty() {
            self.state.game_env.apps.remove(&key);
        } else {
            self.state.game_env.apps.insert(key, env);
        }
        let file = File { apps: self.state.game_env.apps.clone() };
        if let Err(e) = store::save_json(&path(), &file) {
            self.state.notice = Some(format!("Couldn't save the environment variables: {e:#}"));
        }
    }
}

pub(crate) fn handle(client: &mut Client, cmd: Command) {
    if let Command::SetGameEnv { app, env } = cmd {
        client.set_game_env(app, env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_steam::run::Runner;

    fn plan() -> LaunchPlan {
        LaunchPlan {
            appid: 1,
            runner: Runner::Native,
            program: PathBuf::from("/games/run"),
            args: vec!["-a".into()],
            cwd: PathBuf::from("/games"),
            env: BTreeMap::new(),
            create_dirs: vec![],
        }
    }

    #[test]
    fn keys_and_values_are_checked() {
        assert!(valid_key("DXVK_HUD"));
        assert!(valid_key("_x1"));
        assert!(!valid_key(""));
        assert!(!valid_key("1A"));
        assert!(!valid_key("A B"));
        assert!(!valid_key("A=B"));
        assert!(!valid_key(&"A".repeat(MAX_KEY + 1)));
        assert!(valid_value(""));
        assert!(valid_value("a=b c"));
        assert!(!valid_value("a\nb"));
        assert!(!valid_value("a\0b"));
    }

    #[test]
    fn parses_lines() {
        assert_eq!(parse_line(" FOO=bar=baz ").unwrap(), EnvVar { key: "FOO".into(), value: "bar=baz".into() });
        assert_eq!(parse_line("EMPTY=").unwrap().value, "");
        assert!(parse_line("FOO").is_err());
        assert!(parse_line("=x").is_err());
    }

    #[test]
    fn parses_text() {
        let v = parse_text("# comment\n\nFOO=1\nBAR=a b=c\nFOO=2\n").unwrap();
        assert_eq!(
            v,
            vec![EnvVar { key: "BAR".into(), value: "a b=c".into() }, EnvVar { key: "FOO".into(), value: "2".into() }]
        );
        assert!(parse_text("NOEQUALS").is_err());
        assert!(parse_text("1BAD=x").is_err());
        let many: String = (0..=MAX_VARS).map(|i| format!("V{i}=1\n")).collect();
        assert!(parse_text(&many).is_err());
        assert_eq!(parse_text(&to_text(&v)).unwrap(), v);
    }

    #[test]
    fn switches_and_own_variables_merge() {
        let env = GameEnv {
            vars: vec![
                EnvVar { key: "DXVK_HUD".into(), value: "full".into() },
                EnvVar { key: "X".into(), value: "y".into() },
            ],
            dxvk_hud_fps: true,
            nvapi: true,
            ..Default::default()
        };
        let vars = env.variables();
        assert_eq!(vars["DXVK_HUD"], "full", "the user's own value wins");
        assert_eq!(vars["PROTON_ENABLE_NVAPI"], "1");
        assert_eq!(vars["X"], "y");
        assert!(!vars.contains_key("PROTON_USE_WINED3D"));
    }

    #[test]
    fn applies_to_a_plan_with_gamemode() {
        let env = GameEnv { gamemode: true, wined3d: true, ..Default::default() };
        let mut p = plan();
        let done = env.apply(&mut p, Some(PathBuf::from("/usr/bin/gamemoderun")));
        assert_eq!(p.program, PathBuf::from("/usr/bin/gamemoderun"));
        assert_eq!(p.args, vec!["/games/run".to_string(), "-a".to_string()]);
        assert_eq!(p.env["PROTON_USE_WINED3D"], "1");
        assert!(done.contains(&"gamemoderun".to_string()));
        // Not installed: the command stays as it was.
        let mut p = plan();
        env.apply(&mut p, None);
        assert_eq!(p.program, PathBuf::from("/games/run"));
    }

    #[test]
    fn bad_saved_entries_are_dropped() {
        let env = GameEnv {
            vars: vec![
                EnvVar { key: "OK".into(), value: "1".into() },
                EnvVar { key: "bad key".into(), value: "1".into() },
                EnvVar { key: "OK".into(), value: "2".into() },
            ],
            ..Default::default()
        }
        .cleaned();
        assert_eq!(env.vars.len(), 1);
        assert!(GameEnv::default().is_empty());
        let old: GameEnv = serde_json::from_str(r#"{"gamemode": true}"#).unwrap();
        assert!(old.gamemode && old.vars.is_empty());
    }
}
