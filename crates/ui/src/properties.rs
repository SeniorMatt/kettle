//! A game's Properties window: its own native window like the settings window, titled with the game's name.
//! Categories on the left: General (launch options, play time), Compatibility (a tool just for this game) and
//! Installed Files (folder, size, verify, uninstall). View state only; every change is a `Command`.

use crate::dialogs::{Overlays, origin_label};
use crate::widgets::{self, ButtonStyle};
use crate::{Nav, SettingsTab, anim, chrome, fmt, theme};
use egui::{Align, Color32, Context, Layout, Pos2, RichText, Sense, Ui, Vec2, ViewportBuilder, ViewportId};
use sr_client::{Client, Command, DownloadStatus, Load, ToolKind};
use sr_model::{AppId, Game, InstallState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropsTab {
    #[default]
    General,
    Compatibility,
    InstalledFiles,
}

impl PropsTab {
    const ALL: [(PropsTab, &'static str); 3] = [
        (PropsTab::General, "General"),
        (PropsTab::Compatibility, "Compatibility"),
        (PropsTab::InstalledFiles, "Installed Files"),
    ];
}

#[derive(Default)]
pub struct PropertiesView {
    app: Option<AppId>,
    tab: PropsTab,
    /// The launch options field.
    text: String,
    /// The field has been edited and not saved yet.
    dirty: bool,
    confirm_uninstall: bool,
}

impl PropertiesView {
    /// Open the window (or bring it to the front) for `app` on `tab`, and ask for what the pages show.
    pub fn open(&mut self, app: AppId, tab: PropsTab, client: &mut Client, ctx: &Context) {
        match self.app {
            Some(a) if a == app => ctx.send_viewport_cmd_to(Self::viewport_id(), egui::ViewportCommand::Focus),
            _ => {
                self.flush(client);
                self.dirty = false;
                self.confirm_uninstall = false;
                client.send(Command::LoadLaunchOptions(app));
                if cfg!(not(windows)) {
                    client.send(Command::CompatLoad);
                }
            }
        }
        self.app = Some(app);
        self.tab = tab;
    }

    fn viewport_id() -> ViewportId {
        ViewportId::from_hash_of("kettle-properties")
    }

    /// Save an edit that was not committed yet.
    fn flush(&mut self, client: &mut Client) {
        if let (true, Some(app)) = (self.dirty, self.app) {
            client.send(Command::SetLaunchOptions { app, text: self.text.clone() });
            self.dirty = false;
        }
    }

    fn close(&mut self, client: &mut Client) {
        self.flush(client);
        self.app = None;
    }

    /// Draw the window if it is open. Nothing is drawn (and no repaint requested) while it is closed.
    pub fn show(&mut self, ctx: &Context, client: &mut Client, ov: &mut Overlays) {
        let Some(app) = self.app else { return };
        let Some(game) = client.state().game(app).cloned() else {
            self.app = None;
            return;
        };
        let builder = ViewportBuilder::default()
            .with_title(game.name.clone())
            .with_app_id(theme::APP_ID)
            .with_decorations(false)
            .with_inner_size([860.0, 600.0])
            .with_min_inner_size([640.0, 440.0]);
        ctx.show_viewport_immediate(Self::viewport_id(), builder, |ui, _class| {
            let escape = chrome::escape_closes_window(ui);
            if escape || ui.input(|i| i.viewport().close_requested()) {
                self.close(client);
                return;
            }
            self.window(ui, client, &game, ov);
        });
    }

    fn window(&mut self, ui: &mut Ui, client: &mut Client, game: &Game, ov: &mut Overlays) {
        theme::dialog_style(ui);
        let installed = game.install.is_installed();
        if self.tab == PropsTab::InstalledFiles && !installed {
            self.tab = PropsTab::General;
        }
        let title =
            egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin { left: 14, ..Default::default() });
        egui::Panel::top("props-title")
            .exact_size(theme::SETTINGS_TITLE_H)
            .frame(title)
            .show_separator_line(false)
            .show(ui, |ui| {
                chrome::drag_region(ui);
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new(&game.name).font(theme::medium(13.0)).color(theme::TEXT_HEADING));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        chrome::close_button(ui, theme::SETTINGS_TITLE_H);
                    });
                });
            });
        let nav = egui::Frame::new().fill(theme::SETTINGS_NAV_BG).inner_margin(egui::Margin::symmetric(0, 10));
        egui::Panel::left("props-nav")
            .exact_size(theme::SETTINGS_NAV_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(nav)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (tab, label) in PropsTab::ALL {
                    if tab == PropsTab::InstalledFiles && !installed {
                        continue;
                    }
                    if nav_item(ui, self.tab == tab, label) {
                        self.tab = tab;
                    }
                }
            });
        let page = egui::Frame::new().fill(theme::SETTINGS_BG).inner_margin(egui::Margin::symmetric(32, 24));
        egui::CentralPanel::default().frame(page).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let label = PropsTab::ALL.iter().find(|(t, _)| *t == self.tab).map_or("", |(_, l)| l);
                widgets::page_title(ui, label);
                match self.tab {
                    PropsTab::General => self.general(ui, client, game),
                    PropsTab::Compatibility => compatibility(ui, client, game.app_id, ov),
                    PropsTab::InstalledFiles => self.files(ui, client, game, ov),
                }
            });
        });
        chrome::resize_zones(ui);
    }

    // ---- general -----------------------------------------------------------------------------------

    fn general(&mut self, ui: &mut Ui, client: &mut Client, game: &Game) {
        let app = game.app_id;
        let opts = client.state().launch_options.clone();
        if !self.dirty {
            self.text = opts.effective(app);
        }
        let desc = "Advanced users may enter changes to the way the game starts.";
        widgets::setting_block(ui, "Launch options", desc, |ui| {
            let hint = "for example -novid, or MANGOHUD=1 gamemoderun %command%";
            let resp = widgets::text_input(ui, &mut self.text, f32::INFINITY, hint);
            if resp.changed() {
                self.dirty = true;
            }
            if resp.lost_focus() {
                self.flush(client);
            }
            ui.add_space(6.0);
            let help = "Use %command% where the game's own command goes: ENV=1 gamemoderun %command% -novid puts \
                        the variable and the wrapper before the game and the arguments after it. Without \
                        %command%, the text is added after the game's own arguments.";
            ui.label(RichText::new(help).font(theme::regular(12.5)).color(theme::TEXT_DIM));
            if let Some(steam) = opts.steam_value(app).filter(|s| *s != self.text.trim()) {
                ui.add_space(6.0);
                let note = format!("The official client has: {steam}");
                ui.label(RichText::new(note).font(theme::regular(12.5)).color(theme::TEXT_DIM));
                if widgets::link(ui, "Use that").clicked() {
                    self.text = steam.to_string();
                    self.dirty = true;
                    self.flush(client);
                }
            }
        });
        widgets::setting_row(ui, "Play time", "", |ui| value(ui, &fmt::playtime(game.playtime_minutes)));
        let last = fmt::last_played(game.last_played, fmt::now_unix());
        widgets::setting_row(ui, "Last played", "", |ui| value(ui, &last));
        crate::settings_downloads::game_rows(ui, client, game);
        crate::cloud_ui::game_row(ui, client, app);
    }

    // ---- installed files ---------------------------------------------------------------------------

    fn files(&mut self, ui: &mut Ui, client: &mut Client, game: &Game, ov: &mut Overlays) {
        let app = game.app_id;
        let (InstallState::Installed { path, size_bytes, .. } | InstallState::UpdateRequired { path, size_bytes, .. }) =
            &game.install
        else {
            return;
        };
        let running = client.state().running.contains_key(&app);
        let idle = client.state().downloads.status(app) == DownloadStatus::None && !running;
        widgets::setting_block(ui, "Install folder", "", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if widgets::button(ui, "Browse...", ButtonStyle::Secondary, true).clicked() {
                    client.send(Command::BrowseFiles(app));
                }
                ui.add(egui::Label::new(RichText::new(path).color(theme::TEXT)).truncate());
            });
        });
        widgets::setting_row(ui, "Size on disk", "", |ui| value(ui, &fmt::bytes(*size_bytes)));
        let desc = "Checks every file against Steam's version and downloads what is missing or damaged.";
        widgets::setting_row(ui, "Verify integrity of game files", desc, |ui| {
            if widgets::button(ui, "Verify", ButtonStyle::Secondary, idle).clicked() {
                client.send(Command::Verify(app));
                ov.goto = Some(Nav::Downloads);
            }
        });
        let desc = "Deletes the game's folder and its install record. Cloud saves and Proton prefixes are kept.";
        widgets::setting_row(ui, "Uninstall", desc, |ui| {
            if self.confirm_uninstall && idle {
                if widgets::button(ui, "Uninstall", ButtonStyle::Danger, true).clicked() {
                    client.send(Command::Uninstall { app, confirmed: true });
                    self.confirm_uninstall = false;
                }
                if widgets::button(ui, "Keep", ButtonStyle::Secondary, true).clicked() {
                    self.confirm_uninstall = false;
                }
            } else if widgets::button(ui, "Uninstall...", ButtonStyle::Danger, idle).clicked() {
                self.confirm_uninstall = true;
            }
        });
    }
}

fn value(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::medium(14.0)).color(theme::TEXT));
}

/// One category in the left column (same look as the settings window's page list).
fn nav_item(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.15);
    let bg =
        if selected { theme::BTN_FLAT } else { theme::SETTINGS_NAV_BG.lerp_to_gamma(theme::SETTINGS_NAV_HOVER, hover) };
    ui.painter().rect_filled(rect, 0.0, bg);
    let color = if selected { Color32::WHITE } else { theme::BTN_TEXT.lerp_to_gamma(Color32::WHITE, hover) };
    ui.painter().text(
        Pos2::new(rect.left() + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        theme::regular(16.0),
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

// ---- compatibility ---------------------------------------------------------------------------------

fn compatibility(ui: &mut Ui, client: &mut Client, app: AppId, ov: &mut Overlays) {
    if cfg!(windows) {
        ui.label(
            RichText::new("Games run directly on Windows; no compatibility tool is needed.").color(theme::TEXT_DIM),
        );
        return;
    }
    let info = client.state().compat.info.clone();
    let info = match &info {
        Load::Ready(info) => info,
        Load::Failed(e) => {
            ui.label(RichText::new(e).color(theme::ERROR_TEXT));
            return;
        }
        _ => {
            ui.add(egui::Spinner::new());
            return;
        }
    };
    let mut tools: Vec<_> = info.tools.iter().filter(|t| t.kind != ToolKind::Runtime).collect();
    tools.sort_by_key(|t| t.display_name.to_lowercase());
    let steam = info.steam_choice(app.0).map(str::to_string);
    let mine = info.override_for(app.0).map(str::to_string);
    let fallback = match &steam {
        Some(s) => format!("Without this, the tool the official client chose is used ({s})."),
        None => "Without this, the newest installed Proton is used.".to_string(),
    };
    let mut on = mine.is_some();
    let desc = format!("Choose which compatibility tool starts this game. {fallback}");
    widgets::setting_row(ui, "Force the use of a specific compatibility tool", &desc, |ui| {
        if widgets::toggle(ui, &mut on).changed() {
            let pick = steam
                .as_ref()
                .and_then(|s| tools.iter().find(|t| &t.internal_name == s))
                .or(tools.first())
                .map(|t| t.internal_name.clone());
            client.send(Command::SetCompatTool { app, tool: if on { pick } else { None } });
        }
    });
    if let Some(chosen) = &mine {
        let shown =
            tools.iter().find(|t| &t.internal_name == chosen).map_or(chosen.clone(), |t| t.display_name.clone());
        widgets::setting_row(ui, "Compatibility tool", "", |ui| {
            widgets::combo(ui, ("props-tool", app.0), 240.0, shown, |ui| {
                for t in &tools {
                    let label = format!("{}  ({})", t.display_name, origin_label(t.origin));
                    let selected = &t.internal_name == chosen;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        client.send(Command::SetCompatTool { app, tool: Some(t.internal_name.clone()) });
                    }
                }
            });
        });
    }
    if tools.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("No compatibility tool is installed.").color(theme::ERROR_TEXT));
    }
    crate::settings_gamescope::per_game(ui, client, app);
    ui.add_space(12.0);
    if widgets::link(ui, "Open Proton manager").clicked() {
        ov.goto = Some(Nav::Settings(SettingsTab::Compatibility));
    }
}
