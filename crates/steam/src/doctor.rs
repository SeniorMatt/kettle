//! Launch doctor: checks every file, folder and setting a launch plan depends on and says in words which one is
//! wrong. `sr-cli launch <appid> --doctor` prints it; the app appends the short form to a failed launch's
//! "Copy details", so one pasted report shows what is different on the other machine.
//!
//! [`check_plan`] only looks at the disk (and, with `deep`, starts the container once with `true`), so it is
//! tested against temporary folders laid out like the official client's (including the Flatpak one).

use crate::compat::{CompatTool, ToolKind};
use crate::run::{self, LaunchPlan, Runner};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub level: Level,
    pub text: String,
}

impl Check {
    fn new(level: Level, text: impl Into<String>) -> Check {
        Check { level, text: text.into() }
    }
}

/// The checks as text, one per line, `ok` / `WARN` / `FAIL` first. Home folder shown as `~`.
pub fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let tag = match c.level {
            Level::Ok => "ok   ",
            Level::Warn => "WARN ",
            Level::Fail => "FAIL ",
        };
        out.push_str(&format!("{tag}{}\n", c.text));
    }
    match std::env::var("HOME") {
        Ok(h) if h.len() > 1 => out.replace(&h, "~"),
        _ => out,
    }
}

pub fn failed(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

/// Check `plan` (built for the entry running `exe_name` in `install_dir`) against what is on disk.
/// `tool` is the compatibility tool the plan uses, `tools` every installed tool. With `deep` the container is
/// started once to run `true` (can take a minute the first time: the runtime unpacks itself).
pub fn check_plan(
    plan: &LaunchPlan,
    install_dir: &Path,
    exe_name: &str,
    tool: Option<&CompatTool>,
    tools: &[CompatTool],
    deep: bool,
) -> Vec<Check> {
    let mut v = Vec::new();
    let exe = run::resolve_case(install_dir, Path::new(&exe_name.replace('\\', "/")));
    if exe.is_file() {
        v.push(Check::new(Level::Ok, format!("game program: {}", exe.display())));
    } else {
        v.push(Check::new(
            Level::Fail,
            format!("the game's program is missing: {} (verify the game's files)", exe.display()),
        ));
    }
    if plan.cwd.is_dir() {
        v.push(Check::new(Level::Ok, format!("working folder: {}", plan.cwd.display())));
    } else {
        v.push(Check::new(Level::Fail, format!("the working folder is missing: {}", plan.cwd.display())));
    }
    match plan.runner {
        Runner::Native | Runner::Direct => {
            if plan.program.is_absolute() {
                file_check(&mut v, "program", &plan.program, true);
            }
        }
        Runner::Proton | Runner::Umu => compat_checks(&mut v, plan, tool, tools),
    }
    if let Some(d) = plan.env.get("STEAM_COMPAT_DATA_PATH") {
        prefix_checks(&mut v, Path::new(d));
    }
    library_checks(&mut v, plan, install_dir);
    if let Some(c) = plan.env.get("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        client_checks(&mut v, Path::new(c));
    }
    if matches!(plan.runner, Runner::Proton | Runner::Native) && plan.program.to_string_lossy().contains("entry-point")
    {
        userns_checks(&mut v);
        if deep {
            v.push(smoke_test(plan));
        }
    }
    v
}

fn file_check(v: &mut Vec<Check>, what: &str, p: &Path, exec: bool) -> bool {
    match p.metadata() {
        Err(_) => {
            v.push(Check::new(Level::Fail, format!("{what} is missing: {}", p.display())));
            false
        }
        Ok(m) => {
            #[cfg(unix)]
            let not_exec = {
                use std::os::unix::fs::PermissionsExt;
                exec && m.permissions().mode() & 0o111 == 0
            };
            #[cfg(not(unix))]
            let not_exec = false;
            if not_exec {
                v.push(Check::new(Level::Fail, format!("{what} is not executable: {}", p.display())));
                false
            } else {
                v.push(Check::new(Level::Ok, format!("{what}: {}", p.display())));
                true
            }
        }
    }
}

fn compat_checks(v: &mut Vec<Check>, plan: &LaunchPlan, tool: Option<&CompatTool>, tools: &[CompatTool]) {
    let Some(tool) = tool else {
        v.push(Check::new(Level::Fail, "the plan runs through a compatibility tool but none is chosen"));
        return;
    };
    let script = tool.dir.join("proton");
    if plan.runner == Runner::Proton || tool.kind == ToolKind::Proton {
        file_check(
            v,
            &format!("{} ({})", tool.display_name, tool.version.as_deref().unwrap_or("no version file")),
            &script,
            false,
        );
    }
    if !on_path("python3") {
        v.push(Check::new(Level::Fail, "python3 is not installed (Proton's launcher is a Python script)"));
    }
    if plan.runner == Runner::Umu {
        v.push(Check::new(Level::Ok, format!("umu-run starts it: {}", plan.program.display())));
        return;
    }
    if let Some(id) = tool.require_tool_appid {
        match run::runtime_for(tool, tools) {
            None => v.push(Check::new(
                Level::Fail,
                format!(
                    "{} needs {} (app {id}), which is not installed; install it in the Steam client (Library > Tools)",
                    tool.display_name,
                    run::runtime_name(id)
                ),
            )),
            Some(rt) => {
                let entry = rt.command("run").0;
                if file_check(v, &format!("container {}", run::runtime_name(id)), &entry, true) && plan.program != entry
                {
                    v.push(Check::new(
                        Level::Fail,
                        format!(
                            "the plan starts {} but {} needs {}: wrong container",
                            plan.program.display(),
                            tool.display_name,
                            entry.display()
                        ),
                    ));
                }
            }
        }
    }
}

/// The prefix folder: writable, and not half made.
fn prefix_checks(v: &mut Vec<Check>, compatdata: &Path) {
    let probe_dir = if compatdata.is_dir() { Some(compatdata) } else { compatdata.parent().filter(|p| p.is_dir()) };
    match probe_dir {
        None => v.push(Check::new(
            Level::Warn,
            format!("{} and its parent do not exist yet (created at launch)", compatdata.display()),
        )),
        Some(d) => {
            let probe = d.join(format!(".kettle-doctor-{}", std::process::id()));
            match std::fs::write(&probe, b"") {
                Ok(()) => {
                    let _ = std::fs::remove_file(&probe);
                    v.push(Check::new(Level::Ok, format!("prefix folder is writable: {}", d.display())));
                }
                Err(e) => v.push(Check::new(Level::Fail, format!("cannot write in {}: {e}", d.display()))),
            }
        }
    }
    let pfx = compatdata.join("pfx");
    if !pfx.is_dir() {
        v.push(Check::new(Level::Ok, "prefix: not created yet (the tool makes it on the first start)"));
    } else if pfx.join("system.reg").is_file() {
        v.push(Check::new(Level::Ok, "prefix: ready"));
    } else {
        v.push(Check::new(
            Level::Warn,
            "prefix: pfx has no system.reg (an earlier launch did not finish); an empty one is moved aside at launch",
        ));
    }
}

/// Every library folder exists and one of them holds the game (the container only sees listed libraries).
fn library_checks(v: &mut Vec<Check>, plan: &LaunchPlan, install_dir: &Path) {
    let Some(libs) = plan.env.get("STEAM_COMPAT_LIBRARY_PATHS") else { return };
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let game = canon(install_dir);
    let mut holds = false;
    for l in libs.split(':').filter(|l| !l.is_empty()) {
        let lp = Path::new(l);
        if !lp.is_dir() {
            v.push(Check::new(Level::Warn, format!("library folder does not exist: {l}")));
        }
        holds |= game.starts_with(canon(lp));
    }
    if holds {
        v.push(Check::new(Level::Ok, "the game's folder is inside a listed library"));
    } else {
        v.push(Check::new(
            Level::Warn,
            format!("the game's folder {} is in none of the libraries the container is told about", game.display()),
        ));
    }
}

/// The Steam folder handed to Proton as the client install path.
fn client_checks(v: &mut Vec<Check>, client: &Path) {
    if !client.is_dir() {
        v.push(Check::new(Level::Fail, format!("Steam client folder does not exist: {}", client.display())));
        return;
    }
    let has = ["linux64/steamclient.so", "ubuntu12_32/steamclient.so", "linux32/steamclient.so"]
        .iter()
        .any(|f| client.join(f).is_file());
    if has {
        v.push(Check::new(Level::Ok, format!("Steam client folder: {} (steamclient.so found)", client.display())));
    } else {
        v.push(Check::new(
            Level::Warn,
            format!(
                "Steam client folder {} has no steamclient.so (games that need Steamworks may not start)",
                client.display()
            ),
        ));
    }
    if client.to_string_lossy().contains("com.valvesoftware.Steam") {
        v.push(Check::new(
            Level::Ok,
            "Steam is the Flatpak one: its files are used from outside the sandbox, as on the host",
        ));
    }
}

fn read_num(path: &str) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The container needs unprivileged user namespaces; some systems switch them off.
fn userns_checks(v: &mut Vec<Check>) {
    let mut bad = Vec::new();
    if read_num("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") == Some(1) {
        bad.push("kernel.apparmor_restrict_unprivileged_userns=1 (AppArmor blocks bwrap unless it has a profile)");
    }
    if read_num("/proc/sys/kernel/unprivileged_userns_clone") == Some(0) {
        bad.push("kernel.unprivileged_userns_clone=0");
    }
    if read_num("/proc/sys/user/max_user_namespaces") == Some(0) {
        bad.push("user.max_user_namespaces=0");
    }
    if bad.is_empty() {
        v.push(Check::new(Level::Ok, "user namespaces are allowed (the container can start)"));
    } else {
        for b in bad {
            v.push(Check::new(Level::Warn, format!("the container may not start: {b}")));
        }
    }
}

fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

/// Start the container with `true` inside, the same entry point and variables as the real launch.
fn smoke_test(plan: &LaunchPlan) -> Check {
    let mut cmd = std::process::Command::new(&plan.program);
    // Everything up to the first `--` is the entry point's own options; run `true` instead of the game.
    let mut args: Vec<String> = plan.args.iter().take_while(|a| *a != "--").cloned().collect();
    for a in &mut args {
        if let Some(rest) = a.strip_prefix("--verb=") {
            let _ = rest;
            *a = "--verb=run".into();
        }
    }
    args.extend(["--".into(), "true".into()]);
    cmd.args(&args)
        .envs(&plan.env)
        .current_dir(&plan.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Check::new(Level::Fail, format!("could not start the container: {e}")),
    };
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                let tail: Vec<&str> = err.lines().rev().take(6).collect();
                let tail = tail.into_iter().rev().collect::<Vec<_>>().join(" | ");
                return if st.success() {
                    Check::new(Level::Ok, format!("container runs a program ({:.0}s)", t.elapsed().as_secs_f64()))
                } else {
                    Check::new(Level::Fail, format!("the container failed ({st}): {tail}"))
                };
            }
            Ok(None) if t.elapsed() > Duration::from_secs(180) => {
                let _ = child.kill();
                let _ = child.wait();
                return Check::new(Level::Fail, "the container did not finish in 3 minutes");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Check::new(Level::Fail, format!("waiting for the container: {e}")),
        }
    }
}

/// The report for `appid` as the launch would run it now. `deep` also starts the container once.
pub fn report(appid: u32, opts: &run::PrepareOpts, deep: bool) -> String {
    let p = match run::prepare(appid, opts) {
        Ok(p) => p,
        Err(e) => {
            return render(&[Check::new(Level::Fail, format!("the launch cannot be planned: {e:#}"))]);
        }
    };
    let tools = crate::compat::discover();
    let tool = p.tool.as_ref().and_then(|(n, _)| tools.iter().find(|t| &t.internal_name == n));
    let mut checks = check_plan(&p.plan, &p.install_dir, &p.entry.executable, tool, &tools, deep);
    let umu = crate::run::find_umu();
    checks.push(Check::new(
        Level::Ok,
        format!("umu-run: {}", umu.as_ref().map_or("not installed".to_string(), |u| u.display().to_string())),
    ));
    render(&checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::ToolOrigin;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-doctor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch_exec(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn runtime(dir: &Path, name: &str) -> CompatTool {
        CompatTool {
            internal_name: name.into(),
            display_name: name.into(),
            dir: dir.to_path_buf(),
            kind: ToolKind::Runtime,
            origin: ToolOrigin::SteamLibrary,
            version: None,
            commandline: Some("/_v2-entry-point --verb=%verb% --".into()),
            require_tool_appid: None,
        }
    }

    /// A Flatpak-layout library holding Proton 11 (needs runtime 4.0), scout and 4.0, Fallout Shelter like data.
    fn flatpak_library(root: &Path) -> (PathBuf, Vec<CompatTool>, CompatTool) {
        let steam = root.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        let common = steam.join("steamapps/common");
        touch_exec(&common.join("SteamLinuxRuntime_4/_v2-entry-point"));
        touch_exec(&common.join("SteamLinuxRuntime/_v2-entry-point"));
        touch_exec(&common.join("Proton 11.0/proton"));
        std::fs::write(
            steam.join("steamapps/appmanifest_4183110.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\"4183110\"\n\t\"installdir\"\t\"SteamLinuxRuntime_4\"\n}\n",
        )
        .unwrap();
        std::fs::write(
            steam.join("steamapps/appmanifest_1070560.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\"1070560\"\n\t\"installdir\"\t\"SteamLinuxRuntime\"\n}\n",
        )
        .unwrap();
        // The scout runtime sorts first, as in the real library.
        let tools = vec![
            runtime(&common.join("SteamLinuxRuntime"), "SteamLinuxRuntime"),
            runtime(&common.join("SteamLinuxRuntime_4"), "SteamLinuxRuntime_4"),
        ];
        let mut proton = runtime(&common.join("Proton 11.0"), "proton_11");
        proton.kind = ToolKind::Proton;
        proton.commandline = Some("/proton %verb%".into());
        proton.require_tool_appid = Some(4183110);
        (steam, tools, proton)
    }

    fn plan_for(steam: &Path, program: PathBuf, tool_dir: PathBuf) -> (LaunchPlan, PathBuf) {
        let game = steam.join("steamapps/common/Fallout Shelter");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("FalloutShelter.exe"), b"MZ").unwrap();
        let mut env = BTreeMap::new();
        env.insert(
            "STEAM_COMPAT_DATA_PATH".into(),
            steam.join("steamapps/compatdata/588430").to_string_lossy().into_owned(),
        );
        env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), steam.to_string_lossy().into_owned());
        env.insert("STEAM_COMPAT_LIBRARY_PATHS".into(), steam.to_string_lossy().into_owned());
        let plan = LaunchPlan {
            appid: 588430,
            runner: Runner::Proton,
            program,
            args: vec![
                "--verb=waitforexitandrun".into(),
                "--".into(),
                tool_dir.join("proton").to_string_lossy().into_owned(),
            ],
            cwd: game.clone(),
            env,
            create_dirs: vec![],
        };
        (plan, game)
    }

    #[test]
    fn flatpak_layout_passes_with_the_right_container() {
        let root = tmp("ok");
        let (steam, tools, proton) = flatpak_library(&root);
        let rt = run::runtime_for(&proton, &tools).unwrap();
        assert_eq!(rt.internal_name, "SteamLinuxRuntime_4", "the manifest names the folder, not just any runtime");
        let (plan, game) = plan_for(&steam, rt.command("run").0, proton.dir.clone());
        let checks = check_plan(&plan, &game, "FalloutShelter.exe", Some(&proton), &tools, false);
        assert!(!failed(&checks), "{}", render(&checks));
        let text = render(&checks);
        assert!(text.contains("Flatpak"), "{text}");
        assert!(text.contains("prefix: not created yet"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wrong_container_is_reported() {
        let root = tmp("wrong");
        let (steam, tools, proton) = flatpak_library(&root);
        // What the old code did: the first runtime in the library.
        let (plan, game) = plan_for(&steam, tools[0].command("run").0, proton.dir.clone());
        let checks = check_plan(&plan, &game, "FalloutShelter.exe", Some(&proton), &tools, false);
        assert!(failed(&checks));
        assert!(render(&checks).contains("wrong container"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_pieces_fail_in_words() {
        let root = tmp("missing");
        let (steam, tools, proton) = flatpak_library(&root);
        let (plan, game) = plan_for(&steam, tools[1].command("run").0, proton.dir.clone());
        // No runtime installed at all for the tool.
        let checks = check_plan(&plan, &game, "FalloutShelter.exe", Some(&proton), &[], false);
        assert!(render(&checks).contains("which is not installed"));
        // Missing game program and half-made prefix.
        std::fs::remove_file(game.join("FalloutShelter.exe")).unwrap();
        std::fs::create_dir_all(steam.join("steamapps/compatdata/588430/pfx")).unwrap();
        let text = render(&check_plan(&plan, &game, "FalloutShelter.exe", Some(&proton), &tools, false));
        assert!(text.contains("program is missing"), "{text}");
        assert!(text.contains("no system.reg"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
