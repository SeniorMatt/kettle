//! The settings window ("Kettle Settings"): a separate native window like the official client's, with a
//! category list on the left and the chosen page on the right. Pages are rows of label + description with the
//! control on the right (see `widgets::setting_row`). Compatibility (the Proton manager), downloads, in-game
//! options, library folders, account and privacy; categories without real content are not listed.

use crate::dialogs::{kind_label, origin_label};
use crate::widgets::{self, ButtonStyle};
use crate::{anim, chrome, fmt, theme};
use egui::{Align, Color32, Context, Id, Layout, Pos2, RichText, Sense, Ui, Vec2, ViewportBuilder, ViewportId};
use sr_client::{
    Client, Command, CompatInfo, Load, MAX_WORKERS, MIN_WORKERS, ReleaseInfo, Source, ToolInstall, ToolKind, ToolPhase,
    ToolTarget,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Account,
    Library,
    Downloads,
    Compatibility,
    InGame,
    Privacy,
}

impl SettingsTab {
    const ALL: [(SettingsTab, &'static str); 6] = [
        (SettingsTab::Account, "Account"),
        (SettingsTab::Library, "Library"),
        (SettingsTab::Downloads, "Downloads"),
        (SettingsTab::Compatibility, "Compatibility"),
        (SettingsTab::InGame, "In Game"),
        (SettingsTab::Privacy, "Privacy"),
    ];
}

pub struct SettingsView {
    /// Is the settings window open?
    open: bool,
    tab: SettingsTab,
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
            open: false,
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
    /// Open the window (or bring it to the front) on `tab`, and load what the pages show.
    pub fn open(&mut self, tab: SettingsTab, client: &mut Client, ctx: &Context) {
        if self.open {
            ctx.send_viewport_cmd_to(Self::viewport_id(), egui::ViewportCommand::Focus);
        } else {
            self.library_synced = false;
            if cfg!(not(windows)) {
                client.send(Command::CompatLoad);
                client.send(Command::ProtonReleases(false));
            }
        }
        self.open = true;
        self.tab = tab;
    }

    fn viewport_id() -> ViewportId {
        ViewportId::from_hash_of("kettle-settings")
    }

    /// Draw the window if it is open. It is its own native window and closes on its own; nothing is drawn
    /// (and no repaint requested) while it is closed.
    pub fn show(&mut self, ctx: &Context, client: &mut Client) {
        if !self.open {
            return;
        }
        let builder = ViewportBuilder::default()
            .with_title(format!("{} Settings", theme::APP_NAME))
            .with_app_id(theme::APP_ID)
            // Our own title bar, like the main window.
            .with_decorations(false)
            .with_inner_size([850.0, 620.0])
            .with_min_inner_size([640.0, 440.0]);
        ctx.show_viewport_immediate(Self::viewport_id(), builder, |ui, _class| {
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape || ui.input(|i| i.viewport().close_requested()) {
                self.open = false;
                return;
            }
            self.window(ui, client);
        });
    }

    fn window(&mut self, ui: &mut Ui, client: &mut Client) {
        let title =
            egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin { left: 14, ..Default::default() });
        egui::Panel::top("settings-title")
            .exact_size(theme::SETTINGS_TITLE_H)
            .frame(title)
            .show_separator_line(false)
            .show(ui, |ui| {
                chrome::drag_region(ui);
                ui.horizontal_centered(|ui| {
                    let name = format!("{} Settings", theme::APP_NAME);
                    ui.label(RichText::new(name).font(theme::medium(13.0)).color(theme::TEXT_HEADING));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        chrome::close_button(ui, theme::SETTINGS_TITLE_H)
                    });
                });
            });
        let nav = egui::Frame::new().fill(theme::SETTINGS_NAV_BG).inner_margin(egui::Margin::symmetric(0, 10));
        egui::Panel::left("settings-nav")
            .exact_size(theme::SETTINGS_NAV_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(nav)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (tab, label) in SettingsTab::ALL {
                    if self.nav_item(ui, tab, label) {
                        self.tab = tab;
                    }
                }
            });
        let page = egui::Frame::new().fill(theme::SETTINGS_BG).inner_margin(egui::Margin::symmetric(32, 22));
        egui::CentralPanel::default().frame(page).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let label = SettingsTab::ALL.iter().find(|(t, _)| *t == self.tab).map_or("", |(_, l)| l);
                widgets::page_title(ui, label);
                match self.tab {
                    SettingsTab::Account => account(ui, client),
                    SettingsTab::Library => library(ui, client),
                    SettingsTab::Downloads => self.downloads(ui, client),
                    SettingsTab::Compatibility => self.compatibility(ui, client),
                    SettingsTab::InGame => in_game(ui, client),
                    SettingsTab::Privacy => privacy(ui),
                }
            });
        });
        chrome::resize_zones(ui);
    }

    /// One category in the left column: selected = lighter background and white text, hover = subtle.
    fn nav_item(&self, ui: &mut Ui, tab: SettingsTab, label: &str) -> bool {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
        let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
        let selected = self.tab == tab;
        let bg = if selected {
            theme::BG_ROW_SELECTED
        } else {
            theme::SETTINGS_NAV_BG.lerp_to_gamma(theme::BG_ROW_HOVER, hover)
        };
        ui.painter().rect_filled(rect, 0.0, bg);
        let color = if selected { Color32::WHITE } else { theme::TEXT.lerp_to_gamma(Color32::WHITE, hover) };
        ui.painter().text(
            Pos2::new(rect.left() + 24.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            theme::regular(14.0),
            color,
        );
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
        group_title(ui, "Installed");
        match &compat.info {
            Load::Ready(info) => self.installed_list(ui, client, info),
            Load::Failed(e) => {
                ui.label(RichText::new(e).color(theme::ERROR_TEXT));
            }
            _ => {
                ui.add(egui::Spinner::new());
            }
        }
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            group_title(ui, "Available");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::link(ui, "Refresh").clicked() {
                    client.send(Command::ProtonReleases(true));
                }
            });
        });
        self.target_choice(ui, compat.info.ready());
        if let Some(i) = &compat.install {
            ui.add_space(8.0);
            install_status(ui, client, i);
        }
        let busy = compat.install.as_ref().is_some_and(|i| i.result.is_none());
        for (source, load) in &compat.available {
            self.source_block(ui, client, *source, load, busy);
        }
    }

    fn installed_list(&mut self, ui: &mut Ui, client: &mut Client, info: &Arc<CompatInfo>) {
        let mut tools: Vec<_> = info.tools.iter().collect();
        tools.sort_by_key(|t| (t.kind == ToolKind::Runtime, t.display_name.to_lowercase()));
        for t in tools {
            let confirming = self.confirm_remove.as_ref() == Some(&t.dir);
            let mut answer = None;
            // The size is measured in the background; leave it out until it is known.
            let mut caption = format!("{}  |  {}", kind_label(t.kind), origin_label(t.origin));
            if let Some(bytes) = t.size_bytes {
                caption.push_str(&format!("  |  {}", fmt::bytes(bytes)));
            }
            widgets::setting_row(ui, &t.display_name, &caption, |ui| {
                if confirming {
                    // The question sits in the row it is about, so it is never off screen.
                    if widgets::button(ui, "Keep", ButtonStyle::Secondary, true).clicked() {
                        answer = Some(false);
                    }
                    if widgets::button(ui, "Delete", ButtonStyle::Danger, true).clicked() {
                        answer = Some(true);
                    }
                } else if t.removable() && widgets::button(ui, "Remove", ButtonStyle::Secondary, true).clicked() {
                    self.confirm_remove = Some(t.dir.clone());
                }
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
        let ours = info.map_or("Kettle folder".to_string(), |i| format!("Kettle folder ({})", i.our_dir.display()));
        let steam_ok = info.is_none_or(|i| i.steam_dir.is_some());
        let desc = "Tools in Steam's folder are visible to the official client too.";
        widgets::setting_row(ui, "Install tools into", desc, |ui| {
            let shown = if self.target == ToolTarget::Ours { "Kettle folder" } else { "Steam's folder" };
            widgets::combo(ui, "tool-target", 170.0, shown, |ui| {
                ui.selectable_value(&mut self.target, ToolTarget::Ours, ours);
                if steam_ok {
                    ui.selectable_value(&mut self.target, ToolTarget::Steam, "Steam's folder");
                }
            });
        });
    }

    fn source_block(
        &mut self,
        ui: &mut Ui,
        client: &mut Client,
        source: Source,
        load: &Load<Arc<Vec<ReleaseInfo>>>,
        busy: bool,
    ) {
        widgets::setting_block(ui, source.label(), source.description(), |ui| match load {
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
        });
    }

    fn release_row(&mut self, ui: &mut Ui, client: &mut Client, source: Source, list: &[ReleaseInfo], busy: bool) {
        let pick = self.picks.entry(source.id()).or_insert(0);
        *pick = (*pick).min(list.len() - 1);
        let r = &list[*pick];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            widgets::combo(ui, ("release", source.id()), 220.0, release_text(r, *pick == 0), |ui| {
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
        let default = settings.default_library().display().to_string();
        let desc = format!("Games are installed into {default} unless you pick another folder in the install dialog.");
        widgets::setting_block(ui, "Default library folder", &desc, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add(egui::TextEdit::singleline(&mut self.library_text).desired_width(300.0).hint_text(default));
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
        });
        let desc = "How many file pieces are fetched at once. More is faster on a fast line; fewer is gentler.";
        widgets::setting_row(ui, "Parallel downloads", desc, |ui| {
            ui.spacing_mut().slider_width = 150.0;
            let resp = ui.add(egui::Slider::new(&mut self.workers, MIN_WORKERS..=MAX_WORKERS).smallest_positive(1.0));
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                client.send(Command::SetWorkers(self.workers));
            }
        });
    }
}

/// A small heading above a group of rows.
fn group_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).font(theme::bold(12.0)).color(theme::TEXT_HEADING));
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
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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

// ---- the other pages ---------------------------------------------------------

fn account(ui: &mut Ui, client: &mut Client) {
    if let Some(a) = &client.state().account {
        widgets::setting_row(ui, "Signed in as", "Your Steam account name.", |ui| {
            ui.label(RichText::new(&a.account_name).font(theme::medium(14.0)).color(Color32::WHITE));
        });
    }
    let desc = "Deletes the token from the keyring and removes the cached library of this account. To end the \
                session on Steam's side too, remove Kettle under authorized devices in the Steam Mobile App.";
    widgets::setting_row(ui, "Sign out", desc, |ui| {
        if widgets::button(ui, "Sign out", ButtonStyle::Danger, true).clicked() {
            client.send(Command::Logout);
        }
    });
}

fn library(ui: &mut Ui, client: &Client) {
    let desc = "Folders games can be installed into. Folders of the official Steam client are listed too and are \
                only written to when you pick them in the install dialog.";
    ui.label(RichText::new(desc).font(theme::regular(12.5)).color(theme::TEXT_DIM));
    ui.add_space(6.0);
    for l in &client.state().libraries {
        let kind = if l.ours { "Kettle library" } else { "Steam library folder" };
        widgets::setting_row(ui, &l.path.display().to_string(), &format!("{kind}  |  {}", l.label), |_| {});
    }
}

fn in_game(ui: &mut Ui, client: &mut Client) {
    let mut overlay = client.state().settings.overlay;
    let desc = "While a game started here runs, Shift+Tab shows friends, chat and notes over it. The shortcut is \
                only registered while such a game runs.";
    widgets::setting_row(ui, "Enable the in-game overlay (Shift+Tab)", desc, |ui| {
        if widgets::toggle(ui, &mut overlay).changed() {
            client.send(Command::SetOverlay(overlay));
        }
    });
    let mut start = client.state().settings.start_steam;
    let desc = "Most games use the Steamworks API, which only works while the official Steam client runs. We start \
                it minimized before such a game. Nothing is sent to it; it is only started.";
    widgets::setting_row(ui, "Start the Steam client for games that need it", desc, |ui| {
        if widgets::toggle(ui, &mut start).changed() {
            client.send(Command::SetStartSteam(start));
        }
    });
}

fn privacy(ui: &mut Ui) {
    let rows = [
        (
            "No telemetry",
            "No analytics or crash reports, and no servers of our own. The app only talks to Valve's servers.",
        ),
        ("QR sign-in only", "A password is never typed into, seen by or stored by this app."),
        (
            "Token in the keyring",
            "Your sign-in token lives in the system keyring and in memory only. It never goes to a file, a log or the screen.",
        ),
        (
            "Files readable only by you",
            "Our files (accounts list, library cache, play time, settings) are private to your user. We never write to the official Steam client's configuration; downloads go to a library folder you choose.",
        ),
        ("Full text", "PRIVACY.md in the project folder."),
    ];
    for (label, desc) in rows {
        widgets::setting_row(ui, label, desc, |_| {});
    }
}

/// A framed block (the progress of a tool install).
fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let frame = egui::Frame::new()
        .fill(theme::BG_CARD)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .corner_radius(theme::RADIUS);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}
