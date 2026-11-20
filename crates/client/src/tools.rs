//! Compatibility tools (Proton and friends): what is installed, what is available to download, installing and
//! removing, and the per-game choice.
//!
//! Everything slow (scanning folders, GitHub requests, downloads) runs on worker threads; the UI reads
//! [`CompatState`]. Installing is cancelable and reports progress at most four times a second. On Windows these
//! calls still compile, but the UI says the feature is not available.

use super::{Client, Event, Load};
use sr_steam::compat::{self, Choices};
use sr_steam::proton_releases::{self, InstallError, Release};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use sr_steam::compat::{ToolKind, ToolOrigin};
pub use sr_steam::proton_releases::{Phase as ToolPhase, Source};

/// How many releases per source are listed.
const RELEASES_SHOWN: usize = 8;
const FORWARD_EVERY: Duration = Duration::from_millis(250);

/// Where a downloaded tool goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTarget {
    /// Our own `compatibilitytools.d` (default).
    Ours,
    /// The official client's, so Steam sees the tool too.
    Steam,
}

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub internal_name: String,
    pub display_name: String,
    pub dir: PathBuf,
    pub kind: ToolKind,
    pub origin: ToolOrigin,
    pub version: Option<String>,
    /// `None` while the folder is still being measured.
    pub size_bytes: Option<u64>,
}

impl ToolInfo {
    /// Only tools in a `compatibilitytools.d` that is ours or Steam's can be removed here.
    pub fn removable(&self) -> bool {
        matches!(self.origin, ToolOrigin::Ours | ToolOrigin::SteamCustom)
    }
}

/// What a scan found: installed tools and the choices made so far.
#[derive(Debug, Clone)]
pub struct CompatInfo {
    pub tools: Vec<ToolInfo>,
    /// Steam's own per-game picks (`0` = its global default).
    pub steam_map: HashMap<u32, String>,
    pub choices: Choices,
    pub our_dir: PathBuf,
    /// The official client's `compatibilitytools.d`, when a Steam install exists.
    pub steam_dir: Option<PathBuf>,
}

impl CompatInfo {
    /// The tool Steam would use for `app`: its per-game pick, else its global default.
    pub fn steam_choice(&self, app: u32) -> Option<&str> {
        self.steam_map.get(&app).or_else(|| self.steam_map.get(&0)).map(String::as_str)
    }

    /// Our saved override for `app`.
    pub fn override_for(&self, app: u32) -> Option<&str> {
        self.choices.per_app.get(&app.to_string()).map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub published: String,
    pub prerelease: bool,
    pub size_bytes: u64,
}

/// The one tool download in flight (or the last result of one).
#[derive(Debug, Clone)]
pub struct ToolInstall {
    pub source: Source,
    pub tag: String,
    pub phase: ToolPhase,
    pub done: u64,
    pub total: u64,
    pub stopping: bool,
    /// `Some(Ok(name))` installed, `Some(Err(text))` failed or cancelled; `None` while running.
    pub result: Option<Result<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct CompatState {
    pub info: Load<Arc<CompatInfo>>,
    /// One entry per [`Source::ALL`], in that order.
    pub available: Vec<(Source, Load<Arc<Vec<ReleaseInfo>>>)>,
    pub install: Option<ToolInstall>,
}

impl CompatState {
    pub fn releases(&self, source: Source) -> Option<&Load<Arc<Vec<ReleaseInfo>>>> {
        self.available.iter().find(|(s, _)| *s == source).map(|(_, l)| l)
    }

    fn set_releases(&mut self, source: Source, value: Load<Arc<Vec<ReleaseInfo>>>) {
        match self.available.iter_mut().find(|(s, _)| *s == source) {
            Some(slot) => slot.1 = value,
            None => self.available.push((source, value)),
        }
    }
}

/// Running install: the job number is the generation of its events.
pub(crate) struct ToolJob {
    pub job: u64,
    cancel: Arc<AtomicBool>,
}

impl Client {
    /// Scan for tools unless a scan is already running.
    pub(crate) fn compat_load(&mut self) {
        if !self.compat_scanning {
            self.compat_scan(None);
        }
    }

    /// Scan for tools now. `first` runs on the worker before the scan (removing a tool, for example) and its
    /// error becomes a notice. The folder walk for sizes is slow on big Proton trees, so the list is sent
    /// at once without sizes and again with them; the generation drops answers of an older scan.
    fn compat_scan(&mut self, first: Option<Box<dyn FnOnce() -> anyhow::Result<()> + Send>>) {
        self.compat_seq += 1;
        self.compat_scanning = true;
        if !matches!(self.state.compat.info, Load::Ready(_)) {
            self.state.compat.info = Load::Loading;
        }
        let (gen_, tx, wake) = (self.compat_seq, self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let send = |ev: Event| {
                let _ = tx.blocking_send(ev);
                wake();
            };
            if let Some(Err(e)) = first.map(|f| f()) {
                send(Event::Notice(format!("{e:#}")));
            }
            let mut info = scan();
            send(Event::CompatInfo { generation: gen_, result: Ok(info.clone()), last: false });
            for t in &mut info.tools {
                t.size_bytes = Some(dir_size(&t.dir));
            }
            send(Event::CompatInfo { generation: gen_, result: Ok(info), last: true });
        });
    }

    pub(crate) fn apply_compat_info(&mut self, generation: u64, result: Result<CompatInfo, String>, last: bool) {
        if generation != self.compat_seq {
            return;
        }
        self.compat_scanning = !last;
        self.state.compat.info = match result {
            Ok(i) => Load::Ready(Arc::new(i)),
            Err(e) => Load::Failed(e),
        };
    }

    /// Save (or clear, with `None`) our tool override for one game. The list is updated in place: nothing
    /// else changed, so there is no need to walk the tool folders again.
    pub(crate) fn set_compat_tool(&mut self, app: u32, tool: Option<String>) {
        let mut choices = Choices::load();
        choices.set_app(app, tool.as_deref());
        if let Err(e) = choices.save() {
            self.state.notice = Some(format!("Couldn't save the choice: {e:#}"));
            return;
        }
        if let Load::Ready(info) = &self.state.compat.info {
            let mut next = (**info).clone();
            next.choices = choices;
            self.state.compat.info = Load::Ready(Arc::new(next));
        }
    }

    pub(crate) fn proton_releases(&mut self, refresh: bool) {
        for source in Source::ALL {
            if matches!(self.state.compat.releases(source), Some(Load::Loading)) {
                continue;
            }
            self.state.compat.set_releases(source, Load::Loading);
            self.spawn_store(
                move || proton_releases::releases(source, refresh),
                move |result| Event::Releases { source, result },
            );
        }
    }

    pub(crate) fn apply_releases(&mut self, source: Source, result: Result<Vec<Release>, String>) {
        let value = match result {
            Ok(list) => {
                let infos = list.iter().take(RELEASES_SHOWN).map(release_info).collect();
                self.releases.insert(source.id(), list);
                Load::Ready(Arc::new(infos))
            }
            Err(e) => Load::Failed(e),
        };
        self.state.compat.set_releases(source, value);
    }

    pub(crate) fn proton_install(&mut self, source: Source, tag: String, target: ToolTarget) {
        if self.tool_job.is_some() {
            self.state.notice = Some("A compatibility tool is already being installed.".into());
            return;
        }
        let Some(release) = self.releases.get(source.id()).and_then(|l| l.iter().find(|r| r.tag == tag)).cloned()
        else {
            self.state.notice = Some("That release is no longer in the list. Refresh it first.".into());
            return;
        };
        let dir = match target {
            ToolTarget::Ours => compat::our_tools_dir(),
            ToolTarget::Steam => match steam_tools_dir() {
                Some(d) => d,
                None => {
                    self.state.notice = Some("No Steam install found to install into.".into());
                    return;
                }
            },
        };
        self.tool_seq += 1;
        let job = self.tool_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        self.tool_job = Some(ToolJob { job, cancel: cancel.clone() });
        self.state.compat.install = Some(ToolInstall {
            source,
            tag,
            phase: ToolPhase::Download,
            done: 0,
            total: release.archive.size,
            stopping: false,
            result: None,
        });
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let mut last = Instant::now().checked_sub(FORWARD_EVERY).unwrap_or_else(Instant::now);
            let mut phase = None;
            let mut report = |p: proton_releases::Progress| {
                // A new phase is always shown at once; within a phase, at most four times a second.
                if phase != Some(p.phase) || last.elapsed() >= FORWARD_EVERY {
                    (last, phase) = (Instant::now(), Some(p.phase));
                    let _ = tx.blocking_send(Event::ToolProgress { job, progress: p });
                    wake();
                }
            };
            let result = match proton_releases::install(&release, &dir, &cancel, &mut report) {
                Ok(i) => Ok(i.name),
                Err(e) if matches!(e.downcast_ref::<InstallError>(), Some(InstallError::Cancelled)) => {
                    Err("Installation cancelled.".to_string())
                }
                Err(e) => Err(format!("{e:#}")),
            };
            let _ = tx.blocking_send(Event::ToolDone { job, result });
            wake();
        });
    }

    pub(crate) fn proton_cancel(&mut self) {
        if let (Some(j), Some(i)) = (&self.tool_job, &mut self.state.compat.install) {
            j.cancel.store(true, Ordering::SeqCst);
            i.stopping = true;
        }
    }

    pub(crate) fn tool_progress(&mut self, job: u64, p: proton_releases::Progress) {
        if self.tool_job.as_ref().map(|j| j.job) != Some(job) {
            return;
        }
        if let Some(i) = &mut self.state.compat.install {
            (i.phase, i.done, i.total) = (p.phase, p.done, p.total);
        }
    }

    pub(crate) fn tool_done(&mut self, job: u64, result: Result<String, String>) {
        if self.tool_job.as_ref().map(|j| j.job) != Some(job) {
            return;
        }
        self.tool_job = None;
        if let Some(i) = &mut self.state.compat.install {
            i.result = Some(result);
            i.stopping = false;
        }
        self.compat_scan(None);
    }

    /// Remove an installed tool. Only folders from the last scan are accepted, and the backend re-checks that
    /// the folder sits directly inside a `compatibilitytools.d`.
    pub(crate) fn proton_remove(&mut self, dir: PathBuf) {
        let known = match &self.state.compat.info {
            Load::Ready(info) => info.tools.iter().any(|t| t.dir == dir && t.removable()),
            _ => false,
        };
        if !known {
            self.state.notice = Some("That tool can't be removed from here.".into());
            return;
        }
        // Remove, then scan again, in one job so the list never shows a half-removed tool.
        self.compat_scan(Some(Box::new(move || proton_releases::remove(&dir))));
    }
}

fn release_info(r: &Release) -> ReleaseInfo {
    ReleaseInfo {
        tag: r.tag.clone(),
        published: r.published.get(..10).unwrap_or(&r.published).to_string(),
        prerelease: r.prerelease,
        size_bytes: r.archive.size,
    }
}

/// The official client's custom-tools folder, if there is a Steam install.
fn steam_tools_dir() -> Option<PathBuf> {
    sr_steam::local::steam_roots().into_iter().next().map(|r| r.join("compatibilitytools.d"))
}

fn scan() -> CompatInfo {
    let tools = compat::discover()
        .into_iter()
        .map(|t| ToolInfo {
            size_bytes: None,
            internal_name: t.internal_name,
            display_name: t.display_name,
            dir: t.dir,
            kind: t.kind,
            origin: t.origin,
            version: t.version,
        })
        .collect();
    CompatInfo {
        tools,
        steam_map: compat::steam_mapping_all(),
        choices: Choices::load(),
        our_dir: compat::our_tools_dir(),
        steam_dir: steam_tools_dir(),
    }
}

/// Total size of the regular files under `dir` (symlinks are not followed). Bounded in depth and count.
fn dir_size(dir: &Path) -> u64 {
    const MAX_ENTRIES: usize = 200_000;
    let mut total = 0;
    let mut seen = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return total;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                total += e.metadata().map_or(0, |m| m.len());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_choice_falls_back_to_global_default() {
        let info = CompatInfo {
            tools: vec![],
            steam_map: HashMap::from([(0, "proton_9".to_string()), (7, "GE".to_string())]),
            choices: Choices::default(),
            our_dir: PathBuf::new(),
            steam_dir: None,
        };
        assert_eq!(info.steam_choice(7), Some("GE"));
        assert_eq!(info.steam_choice(8), Some("proton_9"));
        assert_eq!(info.override_for(7), None);
    }

    #[test]
    fn dir_size_counts_files() {
        let d = std::env::temp_dir().join(format!("sr-dirsize-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::write(d.join("a/x"), [0u8; 10]).unwrap();
        std::fs::write(d.join("y"), [0u8; 5]).unwrap();
        assert_eq!(dir_size(&d), 15);
        let _ = std::fs::remove_dir_all(&d);
    }
}
