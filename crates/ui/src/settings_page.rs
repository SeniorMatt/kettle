//! Settings: compatibility tools (the Proton manager), downloads and privacy.

use crate::dialogs::{kind_label, origin_label};
use crate::widgets::{self, ButtonStyle};
use crate::{anim, fmt, theme};
use egui::{Id, RichText, Ui};
use sr_client::{
    Client, Command, CompatInfo, Load, MAX_WORKERS, MIN_WORKERS, ReleaseInfo, Source, ToolInstall, ToolKind, ToolPhase,
    ToolTarget,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Compatibility,
    Games,
    Downloads,
    Privacy,
}

impl SettingsTab {
    const ALL: [(SettingsTab, &'static str); 4] = [
        (SettingsTab::Compatibility, "Compatibility"),
        (SettingsTab::Games, "Games"),
        (SettingsTab::Downloads, "Downloads"),
        (SettingsTab::Privacy, "Privacy"),
    ];
}

pub struct SettingsView {
    pub tab: SettingsTab,
    target: ToolTarget,
    /// Chosen release (index into the shown list) per source.
    picks: HashMap<&'static str, usize>,
    library_text: String,
    library_synced: bool,
    workers: usize,
    confirm_remove: Option<std::path::PathBuf>,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self {
            tab: SettingsTab::default(),
            target: ToolTarget::Ours,
            picks: HashMap::new(),
            library_text: String::new(),
            library_synced: false,
            workers: 0,
            confirm_remove: None,
        }
    }
}

impl SettingsView {
    /// Called when the page is opened: load what the tabs show.
    pub fn opened(&mut self, client: &mut Client) {
        self.library_synced = false;
        if cfg!(not(windows)) {
            client.send(Command::CompatLoad);
            client.send(Command::ProtonReleases(false));
        }
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client) {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 20)).show(ui, |ui| {
            ui.label(RichText::new("SETTINGS").font(theme::bold(14.0)).color(theme::TEXT_HEADING));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                for (tab, label) in SettingsTab::ALL {
                    if widgets::text_tab(ui, label, theme::bold(15.0), self.tab == tab).clicked() {
                        self.tab = tab;
                    }
                }
            });
            ui.add_space(12.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                match self.tab {
                    SettingsTab::Compatibility => self.compatibility(ui, client),
                    SettingsTab::Games => games(ui, client),
                    SettingsTab::Downloads => self.downloads(ui, client),
                    SettingsTab::Privacy => privacy(ui, client),
                }
            });
        });
    }

    // ---- compatibility ---------------------------------------------------

    fn compatibility(&mut self, ui: &mut Ui, client: &mut Client) {
        if cfg!(windows) {
            ui.label(
                RichText::new("Compatibility tools are not available on Windows: games run directly.")
                    .color(theme::TEXT_DIM),
            );
            return;
        }
        let compat = client.state().compat.clone();
        widgets::section_title(ui, "INSTALLED");
        match &compat.info {
            Load::Ready(info) => self.installed_list(ui, client, info),
            Load::Failed(e) => {
                ui.label(RichText::new(e).color(theme::ERROR_TEXT));
            }
            _ => {
                ui.add(egui::Spinner::new());
            }
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            widgets::section_title(ui, "AVAILABLE");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::link(ui, "Refresh").clicked() {
                    client.send(Command::ProtonReleases(true));
                }
            });
        });
        self.target_choice(ui, compat.info.ready());
        if let Some(i) = &compat.install {
            install_status(ui, client, i);
        }
        let busy = compat.install.as_ref().is_some_and(|i| i.result.is_none());
        for (source, load) in &compat.available {
            self.source_card(ui, client, *source, load, busy);
        }
    }

    fn installed_list(&mut self, ui: &mut Ui, client: &mut Client, info: &Arc<CompatInfo>) {
        let mut tools: Vec<_> = info.tools.iter().collect();
        tools.sort_by_key(|t| (t.kind == ToolKind::Runtime, t.display_name.to_lowercase()));
        for t in tools {
            let confirming = self.confirm_remove.as_ref() == Some(&t.dir);
            let mut answer = None;
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    // Same height with or without buttons.
                    ui.set_min_height(30.0);
                    ui.label(RichText::new(&t.display_name).font(theme::medium(14.0)).color(theme::TEXT));
                    let size = t.size_bytes.map_or("...".to_string(), fmt::bytes);
                    let caption = format!("{}  |  {}  |  {size}", kind_label(t.kind), origin_label(t.origin));
                    ui.label(RichText::new(caption).color(theme::TEXT_DIM).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if confirming {
                            // The question sits in the row it is about, so it is never off screen.
                            if widgets::button(ui, "Keep", ButtonStyle::Secondary, true).clicked() {
                                answer = Some(false);
                            }
                            if widgets::button(ui, "Delete", ButtonStyle::Danger, true).clicked() {
                                answer = Some(true);
                            }
                            ui.label(RichText::new("Delete this tool?").color(theme::ERROR_TEXT));
                        } else if t.removable() && widgets::button(ui, "Remove", ButtonStyle::Secondary, true).clicked()
                        {
                            self.confirm_remove = Some(t.dir.clone());
                        }
                    });
                });
            });
            match answer {
                Some(true) => {
                    client.send(Command::ProtonRemove(t.dir.clone()));
                    self.confirm_remove = None;
                }
                Some(false) => self.confirm_remove = None,
                None => {}
            }
        }
    }

    fn target_choice(&mut self, ui: &mut Ui, info: Option<&Arc<CompatInfo>>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Install into").color(theme::TEXT_DIM));
            let ours = info
                .map_or("steam-rust folder".to_string(), |i| format!("steam-rust folder ({})", i.our_dir.display()));
            ui.radio_value(&mut self.target, ToolTarget::Ours, ours);
            if info.is_none_or(|i| i.steam_dir.is_some()) {
                ui.radio_value(&mut self.target, ToolTarget::Steam, "Steam's folder (Steam sees it too)");
            }
        });
    }

    fn source_card(
        &mut self,
        ui: &mut Ui,
        client: &mut Client,
        source: Source,
        load: &Load<Arc<Vec<ReleaseInfo>>>,
        busy: bool,
    ) {
        card(ui, |ui| {
            ui.label(RichText::new(source.label()).font(theme::medium(14.0)).color(theme::TEXT));
            ui.label(RichText::new(source.description()).color(theme::TEXT_DIM).small());
            ui.add_space(4.0);
            match load {
                Load::Ready(list) if list.is_empty() => {
                    ui.label(RichText::new("No releases found.").color(theme::TEXT_DIM));
                }
                Load::Ready(list) => self.release_row(ui, client, source, list, busy),
                Load::Failed(e) => {
                    ui.label(RichText::new(e).color(theme::ERROR_TEXT).small());
                }
                _ => {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            }
        });
    }

    fn release_row(&mut self, ui: &mut Ui, client: &mut Client, source: Source, list: &[ReleaseInfo], busy: bool) {
        let pick = self.picks.entry(source.id()).or_insert(0);
        *pick = (*pick).min(list.len() - 1);
        let r = &list[*pick];
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("release", source.id()))
                .width(240.0)
                .selected_text(release_text(r, *pick == 0))
                .show_ui(ui, |ui| {
                    for (i, r) in list.iter().enumerate() {
                        ui.selectable_value(pick, i, release_text(r, i == 0));
                    }
                });
            if widgets::button(ui, "Install", ButtonStyle::Primary, !busy).clicked() {
                client.send(Command::ProtonInstall { source, tag: r.tag.clone(), target: self.target });
            }
            ui.label(
                RichText::new(format!("{}  {}", r.published, fmt::bytes(r.size_bytes))).color(theme::TEXT_DIM).small(),
            );
        });
    }

    // ---- downloads -------------------------------------------------------

    fn downloads(&mut self, ui: &mut Ui, client: &mut Client) {
        let settings = client.state().settings.clone();
        if !self.library_synced {
            self.library_text = settings.library.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            self.workers = settings.workers;
            self.library_synced = true;
        }
        widgets::section_title(ui, "DEFAULT LIBRARY FOLDER");
        let default = settings.default_library().display().to_string();
        ui.label(
            RichText::new(format!(
                "Games are installed into {default} unless you pick another folder in the install dialog."
            ))
            .color(theme::TEXT_DIM),
        );
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.library_text).desired_width(420.0).hint_text(default));
            if widgets::button(ui, "Apply", ButtonStyle::Primary, true).clicked() {
                let text = self.library_text.trim();
                let path = (!text.is_empty()).then(|| std::path::PathBuf::from(text));
                client.send(Command::SetLibrary(path));
            }
            if widgets::button(ui, "Use default", ButtonStyle::Secondary, true).clicked() {
                self.library_text.clear();
                client.send(Command::SetLibrary(None));
            }
        });
        ui.add_space(14.0);
        widgets::section_title(ui, "PARALLEL DOWNLOADS");
        ui.label(
            RichText::new("How many file pieces are fetched at once. More is faster on a fast line; fewer is gentler.")
                .color(theme::TEXT_DIM),
        );
        let resp = ui.add(egui::Slider::new(&mut self.workers, MIN_WORKERS..=MAX_WORKERS).smallest_positive(1.0));
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            client.send(Command::SetWorkers(self.workers));
        }
    }
}

fn release_text(r: &ReleaseInfo, latest: bool) -> String {
    let mut s = r.tag.clone();
    if latest {
        s.push_str("  (latest)");
    }
    if r.prerelease {
        s.push_str("  (pre-release)");
    }
    s
}

fn install_status(ui: &mut Ui, client: &mut Client, i: &ToolInstall) {
    card(ui, |ui| match &i.result {
        None => {
            let phase = match i.phase {
                ToolPhase::Download => "Downloading",
                ToolPhase::Verify => "Checking the download",
                ToolPhase::Extract => "Unpacking",
            };
            let f = if i.total == 0 { 0.0 } else { (i.done as f64 / i.total as f64) as f32 };
            ui.horizontal(|ui| {
                let text = if i.stopping {
                    "Cancelling...".to_string()
                } else {
                    format!("{phase} {} {:.0}%", i.tag, f * 100.0)
                };
                ui.label(RichText::new(text).color(theme::TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, "Cancel", ButtonStyle::Secondary, !i.stopping).clicked() {
                        client.send(Command::ProtonCancel);
                    }
                });
            });
            let shown = anim::glide(ui.ctx(), Id::new("tool-bar"), f, 4.0);
            widgets::progress_bar(ui, ui.available_width(), 6.0, shown, theme::ACCENT);
        }
        Some(Ok(name)) => {
            ui.label(RichText::new(format!("Installed {name}.")).color(theme::VERIFY));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(theme::ERROR_TEXT));
        }
    });
}

// ---- privacy ---------------------------------------------------------------

fn games(ui: &mut Ui, client: &mut Client) {
    widgets::section_title(ui, "STEAM CLIENT");
    let mut on = client.state().settings.start_steam;
    if ui.checkbox(&mut on, "Start the Steam client in the background for games that need it").changed() {
        client.send(Command::SetStartSteam(on));
    }
    ui.label(
        RichText::new(
            "Most games use the Steamworks API, which only works while the official Steam client runs. \
             We start it minimized before such a game. Nothing is sent to it; it is only started.",
        )
        .color(theme::TEXT_DIM),
    );
}

fn privacy(ui: &mut Ui, client: &mut Client) {
    widgets::section_title(ui, "PRIVACY");
    for line in [
        "No telemetry, analytics or crash reports, and no servers of our own. The app only talks to Valve's servers.",
        "Sign-in is by QR code only. A password is never typed into, seen by or stored by this app.",
        "Your sign-in token lives in the system keyring and in memory only. It never goes to a file, a log or the screen.",
        "Our files (accounts list, library cache, play time, settings) are readable only by you. We never write to the official Steam client's configuration; downloads go to a library folder you choose.",
        "The full text is in PRIVACY.md in the project folder.",
    ] {
        ui.label(RichText::new(line).color(theme::TEXT));
    }
    ui.add_space(14.0);
    widgets::section_title(ui, "SIGN OUT");
    ui.label(RichText::new("Deletes the token from the keyring, revokes it on Steam's servers so no copy keeps working, and removes the cached library of this account.").color(theme::TEXT_DIM));
    if widgets::button(ui, "Sign out and revoke", ButtonStyle::Danger, true).clicked() {
        client.send(Command::Logout);
    }
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let frame = egui::Frame::new()
        .fill(theme::BG_CARD)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .corner_radius(theme::RADIUS);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}
