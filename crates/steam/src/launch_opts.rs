//! Per-game launch options, with the same meaning as in the official client.
//!
//! * With `%command%` the text is a template: `ENV=1 gamemoderun %command% -novid` puts the environment
//!   variables and the wrapper before the game and the arguments after it.
//! * Without `%command%` the whole text is extra arguments for the game.
//!
//! Our own values live in `launch_options.json` in the data dir. When we have none for a game, the value the
//! official client has saved is used; its `localconfig.vdf` is only ever read.

use crate::run::{LaunchPlan, split_args};
use crate::{local, store, vdf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A launch options text taken apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// `NAME=value` words in front of `%command%`.
    pub env: Vec<(String, String)>,
    /// The rest in front of `%command%` (`gamemoderun`, `mangohud`, ...): a command that runs the game.
    pub wrapper: Vec<String>,
    /// Arguments for the game itself.
    pub args: Vec<String>,
}

const PLACEHOLDER: &str = "%command%";

/// Parse launch options. Words are split like a shell does for simple cases: whitespace separates, `"..."`
/// and `'...'` group.
pub fn parse(text: &str) -> Parsed {
    let words = split_args(text);
    let Some(at) = words.iter().position(|w| w == PLACEHOLDER) else {
        return Parsed { args: words, ..Parsed::default() };
    };
    let mut out = Parsed { args: words[at + 1..].to_vec(), ..Parsed::default() };
    for w in &words[..at] {
        match env_pair(w).filter(|_| out.wrapper.is_empty()) {
            Some(pair) => out.env.push(pair),
            None => out.wrapper.push(w.clone()),
        }
    }
    out
}

/// `NAME=value` where NAME looks like a variable name.
fn env_pair(word: &str) -> Option<(String, String)> {
    let (name, value) = word.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let ok = (first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| (name.to_string(), value.to_string()))
}

/// Put the parsed options onto a plan that was built with `parsed.args` as its extra arguments: environment
/// variables are added, and the wrapper is put in front of the whole command (also in front of Proton / umu,
/// as the official client does).
pub fn apply(plan: &mut LaunchPlan, parsed: &Parsed) {
    plan.env.extend(parsed.env.iter().cloned());
    if let Some((first, rest)) = parsed.wrapper.split_first() {
        let program = std::mem::replace(&mut plan.program, PathBuf::from(first));
        let old = std::mem::take(&mut plan.args);
        plan.args =
            rest.iter().cloned().chain(std::iter::once(program.to_string_lossy().into_owned())).chain(old).collect();
    }
}

/// The command line of a plan as one shell-quoted line, for `--dry-run` and logs. Only the variables the
/// launch options set are shown in front; the rest of the environment is listed separately.
pub fn command_line(plan: &LaunchPlan, shown_env: &[(String, String)]) -> String {
    let env = shown_env.iter().map(|(k, v)| format!("{k}={}", quote(v)));
    let program = plan.program.to_string_lossy();
    env.chain(std::iter::once(quote(&program))).chain(plan.args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ")
}

fn quote(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:=+,@%".contains(c));
    if plain { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

// ---- Our own values ------------------------------------------------------------------------------------

/// `appid -> text`. A present but empty text means "none, even if the official client has some".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LaunchOptions {
    pub apps: BTreeMap<String, String>,
}

impl LaunchOptions {
    pub fn path() -> PathBuf {
        store::data_dir().join("launch_options.json")
    }

    pub fn load() -> LaunchOptions {
        store::load_json(&LaunchOptions::path()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        store::save_json(&LaunchOptions::path(), self)
    }

    pub fn get(&self, appid: u32) -> Option<&str> {
        self.apps.get(&appid.to_string()).map(String::as_str)
    }

    /// Set our value. `None`, or text equal to `default` (the official client's), removes it so the default
    /// keeps applying.
    pub fn set(&mut self, appid: u32, text: Option<&str>, default: Option<&str>) {
        let key = appid.to_string();
        match text.map(str::trim) {
            Some(t) if Some(t) != default.map(str::trim) => self.apps.insert(key, t.to_string()),
            _ => self.apps.remove(&key),
        };
    }
}

// ---- The official client's value (read only) -------------------------------------------------------------

const ID64_BASE: u64 = 76_561_197_960_265_728;

/// The 32-bit account id the official client names its `userdata` folders by.
pub fn account_id(steam_id64: u64) -> Option<u32> {
    steam_id64.checked_sub(ID64_BASE).and_then(|n| u32::try_from(n).ok()).filter(|n| *n != 0)
}

/// `LaunchOptions` of an app in the text of a `localconfig.vdf`.
pub fn from_localconfig(text: &str, appid: u32) -> Option<String> {
    let doc = vdf::parse(text);
    let apps = doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])?;
    let s = apps.get(&appid.to_string())?.get_str("LaunchOptions")?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn localconfig(root: &Path, account: u32) -> PathBuf {
    root.join("userdata").join(account.to_string()).join("config/localconfig.vdf")
}

/// What the official client has saved for the app. With `account` it reads that account's file; without
/// (developer tools) the first account that has a value.
pub fn steam_default(appid: u32, account: Option<u32>) -> Option<String> {
    for root in local::steam_roots() {
        let accounts: Vec<u32> = match account {
            Some(a) => vec![a],
            None => std::fs::read_dir(root.join("userdata"))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.parse().ok())
                .collect(),
        };
        for a in accounts {
            if let Some(v) =
                std::fs::read_to_string(localconfig(&root, a)).ok().and_then(|t| from_localconfig(&t, appid))
            {
                return Some(v);
            }
        }
    }
    None
}

/// The text a launch uses: ours when set, else the official client's.
pub fn effective(appid: u32, ours: Option<&str>, account: Option<u32>) -> Option<String> {
    match ours {
        Some(t) => Some(t.to_string()),
        None => steam_default(appid, account),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Runner;
    use std::collections::BTreeMap;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn empty_and_plain_arguments() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("   "), Parsed::default());
        let p = parse("-novid -windowed");
        assert_eq!(p.args, s(&["-novid", "-windowed"]));
        assert!(p.env.is_empty() && p.wrapper.is_empty());
    }

    #[test]
    fn without_placeholder_env_words_are_just_arguments() {
        assert_eq!(parse("FOO=1 -x").args, s(&["FOO=1", "-x"]));
    }

    #[test]
    fn template_splits_env_wrapper_args() {
        let p = parse("ENV=1 gamemoderun %command% -novid");
        assert_eq!(p.env, vec![("ENV".to_string(), "1".to_string())]);
        assert_eq!(p.wrapper, s(&["gamemoderun"]));
        assert_eq!(p.args, s(&["-novid"]));
    }

    #[test]
    fn template_with_only_placeholder_and_with_several_wrappers() {
        assert_eq!(parse("%command%"), Parsed::default());
        let p = parse("A=1 B=2 mangohud --dlsym gamemoderun %command%");
        assert_eq!(p.env.len(), 2);
        assert_eq!(p.wrapper, s(&["mangohud", "--dlsym", "gamemoderun"]));
        assert!(p.args.is_empty());
        // After the wrapper started, NAME=value is an argument of the wrapper.
        assert_eq!(parse("env X=1 %command%").wrapper, s(&["env", "X=1"]));
    }

    #[test]
    fn quotes_group_words() {
        let p = parse(r#"MANGOHUD_CONFIG="fps,cpu_temp" %command% -name 'My Name' "-title=a b""#);
        assert_eq!(p.env, vec![("MANGOHUD_CONFIG".to_string(), "fps,cpu_temp".to_string())]);
        assert_eq!(p.args, s(&["-name", "My Name", "-title=a b"]));
        assert_eq!(parse(r#"X="a b" %command%"#).env, vec![("X".to_string(), "a b".to_string())]);
        // An unterminated quote does not lose text.
        assert_eq!(parse("-x 'abc").args, s(&["-x", "abc"]));
    }

    fn plan(runner: Runner, program: &str, args: &[&str]) -> LaunchPlan {
        LaunchPlan {
            appid: 7,
            runner,
            program: PathBuf::from(program),
            args: s(args),
            cwd: PathBuf::from("/"),
            env: BTreeMap::new(),
            create_dirs: vec![],
        }
    }

    #[test]
    fn native_command_line() {
        // The caller builds the plan with `parsed.args` as extra arguments.
        let parsed = parse("ENV=1 gamemoderun %command% -novid");
        let mut p = plan(Runner::Native, "/g/game", &["-base", "-novid"]);
        apply(&mut p, &parsed);
        assert_eq!(p.program, PathBuf::from("gamemoderun"));
        assert_eq!(p.args, s(&["/g/game", "-base", "-novid"]));
        assert_eq!(p.env.get("ENV").map(String::as_str), Some("1"));
        assert_eq!(command_line(&p, &parsed.env), "ENV=1 gamemoderun /g/game -base -novid");
    }

    #[test]
    fn proton_wrapper_goes_in_front_of_the_tool() {
        let parsed = parse("mangohud %command% -x");
        let mut p = plan(Runner::Proton, "/t/proton", &["waitforexitandrun", "/g/game.exe", "-x"]);
        apply(&mut p, &parsed);
        assert_eq!(p.program, PathBuf::from("mangohud"));
        assert_eq!(p.args, s(&["/t/proton", "waitforexitandrun", "/g/game.exe", "-x"]));
    }

    #[test]
    fn no_wrapper_leaves_the_command_alone_and_quoting_is_safe() {
        let parsed = parse("-a 'b c'");
        let mut p = plan(Runner::Umu, "/bin/umu-run", &["/g/game.exe", "-a", "b c"]);
        let before = p.clone();
        apply(&mut p, &parsed);
        assert_eq!(p, before);
        assert_eq!(command_line(&p, &[]), "/bin/umu-run /g/game.exe -a 'b c'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn our_value_wins_and_default_equal_text_is_dropped() {
        let mut o = LaunchOptions::default();
        o.set(5, Some("-a"), Some("-b"));
        assert_eq!(o.get(5), Some("-a"));
        assert_eq!(effective(5, o.get(5), Some(1)), Some("-a".to_string()));
        o.set(5, Some("-b "), Some("-b"));
        assert_eq!(o.get(5), None);
        o.set(5, Some(""), Some("-b")); // "none" overrides the official client's value
        assert_eq!(o.get(5), Some(""));
        o.set(5, None, Some("-b"));
        assert_eq!(o.get(5), None);
    }

    #[test]
    fn reads_the_official_value() {
        let text = r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
            "730" { "LaunchOptions" "  -novid -console " "LastPlayed" "1" }
            "10" { "LastPlayed" "1" }
            "20" { "LaunchOptions" "" }
        } } } } }"#;
        assert_eq!(from_localconfig(text, 730).as_deref(), Some("-novid -console"));
        assert_eq!(from_localconfig(text, 10), None);
        assert_eq!(from_localconfig(text, 20), None);
        assert_eq!(from_localconfig(text, 99), None);
    }

    #[test]
    fn account_ids() {
        assert_eq!(account_id(76_561_197_960_265_728 + 12345), Some(12345));
        assert_eq!(account_id(0), None);
        assert_eq!(account_id(76_561_197_960_265_728), None);
    }
}
