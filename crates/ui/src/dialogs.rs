//! Modal dialogs opened from the game page: install options, uninstall confirmation, per-game compatibility
//! tool, and the "could not start" message. View state only; every action is a `Command`.

use crate::widgets::{self, ButtonStyle};
use crate::{Nav, SettingsTab, fmt, theme};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, Command, DownloadRequest, Load, OsChoice, ToolKind, ToolOrigin};
use sr_model::AppId;
use std::path::PathBuf;

const DIALOG_W: f32 = 440.0;

struct InstallDialog {
    app: AppId,
    name: String,
    library: usize,
    os: OsChoice,
    /// The path whose free space was last asked for, so it is asked again only when the choice changes.
    asked: Option<PathBuf>,
}

/// Which dialogs are open.
#[derive(Default)]
pub struct Overlays {
    install: Option<InstallDialog>,
    uninstall: Option<AppId>,
    compat: Option<AppId>,
    /// Navigation a dialog asked for ("Open Proton manager"); the page loop takes it.
    pub goto: Option<Nav>,
}

impl Overlays {
    pub fn open_install(&mut self, app: AppId, name: &str) {
        self.install = Some(InstallDialog { app, name: name.to_string(), library: 0, os: OsChoice::Auto, asked: None });
    }

    pub fn open_uninstall(&mut self, app: AppId) {
        self.uninstall = Some(app);
    }

    pub fn open_compat(&mut self, client: &mut Client, app: AppId) {
        client.send(Command::CompatLoad);
        self.compat = Some(app);
    }

    pub fn show(&mut self, ctx: &Context, client: &mut Client) {
        self.install_dialog(ctx, client);
        self.uninstall_dialog(ctx, client);
        self.compat_dialog(ctx, client);
        self.play_problem(ctx, client);
    }

    fn install_dialog(&mut self, ctx: &Context, client: &mut Client) {
        let Some(d) = &mut self.install else { return };
        let libs = client.state().libraries.clone();
        d.library = d.library.min(libs.len().saturating_sub(1));
        let Some(lib) = libs.get(d.library) else {
            self.install = None;
            return;
        };
        if d.asked.as_ref() != Some(&lib.path) {
            d.asked = Some(lib.path.clone());
            client.send(Command::CheckFree(lib.path.clone()));
        }
        let free = client.state().free_space.iter().find(|(p, _)| *p == lib.path).map(|(_, f)| *f);
        let mut start = false;
        let modal = modal(Id::new("install-dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, &format!("Install {}", d.name));
            field(ui, "INSTALL TO");
            let chosen = format!("{}  ({})", lib.path.display(), lib.label);
            widgets::combo(ui, "install-lib", DIALOG_W, chosen, |ui| {
                for (i, l) in libs.iter().enumerate() {
                    ui.selectable_value(&mut d.library, i, format!("{}  ({})", l.path.display(), l.label));
                }
            });
            if !lib.ours {
                ui.label(
                    RichText::new("This is a Steam library folder. The game will be written into it.")
                        .color(theme::TEXT_DIM)
                        .small(),
                );
            }
            field(ui, "PLATFORM");
            widgets::combo(ui, "install-os", DIALOG_W, d.os.label(), |ui| {
                for os in [OsChoice::Auto, OsChoice::Linux, OsChoice::Windows] {
                    ui.selectable_value(&mut d.os, os, os.label());
                }
            });
            field(ui, "DISK SPACE");
            let free_text = match free {
                Some(Some(b)) => format!("{} free on this disk", fmt::bytes(b)),
                Some(None) => "Free space is not known for this disk".to_string(),
                None => "Checking...".to_string(),
            };
            ui.label(RichText::new(free_text).color(theme::TEXT));
            ui.label(
                RichText::new(
                    "The size is known once the download starts; it stops with a message if the disk is too small.",
                )
                .color(theme::TEXT_DIM)
                .small(),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                start = widgets::button(ui, "Install", ButtonStyle::Primary, true).clicked();
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        if start {
            let (app, name, os) = (d.app, d.name.clone(), d.os);
            client.send(Command::Download(DownloadRequest { app, name, library: lib.path.clone(), os, verify: false }));
            self.install = None;
            self.goto = Some(Nav::Downloads);
        } else if modal.should_close() {
            self.install = None;
        }
    }

    fn uninstall_dialog(&mut self, ctx: &Context, client: &mut Client) {
        let Some(app) = self.uninstall else { return };
        let Some(game) = client.state().game(app).cloned() else {
            self.uninstall = None;
            return;
        };
        let path = match &game.install {
            sr_model::InstallState::Installed { path, .. } | sr_model::InstallState::UpdateRequired { path, .. } => {
                path.clone()
            }
            _ => String::new(),
        };
        let ours = client.state().libraries.iter().any(|l| l.ours && path.starts_with(&l.path.display().to_string()));
        let mut confirm = false;
        let modal = modal(Id::new("uninstall-dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, &format!("Uninstall {}", game.name));
            ui.label(RichText::new("This deletes the game's folder and its install record:").color(theme::TEXT));
            ui.label(RichText::new(&path).color(theme::TEXT_DIM));
            if !ours {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "This folder is in a Steam library. The official client will see the game as removed.",
                    )
                    .color(theme::ERROR_TEXT),
                );
            }
            ui.label(
                RichText::new("Save games kept in the cloud or in Proton prefixes are not touched.")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                confirm = widgets::button(ui, "Uninstall", ButtonStyle::Danger, true).clicked();
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        if confirm {
            client.send(Command::Uninstall { app, confirmed: true });
            self.uninstall = None;
        } else if modal.should_close() {
            self.uninstall = None;
        }
    }

    fn compat_dialog(&mut self, ctx: &Context, client: &mut Client) {
        let Some(app) = self.compat else { return };
        let name = client.state().game(app).map(|g| g.name.clone()).unwrap_or_default();
        let info = client.state().compat.info.clone();
        let mut goto = false;
        let modal = modal(Id::new("compat-dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, &format!("Compatibility: {name}"));
            if cfg!(windows) {
                ui.label(
                    RichText::new("Games run directly on Windows; no compatibility tool is needed.")
                        .color(theme::TEXT_DIM),
                );
                return;
            }
            match &info {
                Load::Ready(info) => {
                    let steam = info.steam_choice(app.0).map(str::to_string);
                    let mine = info.override_for(app.0).map(str::to_string);
                    let default_label = match &steam {
                        Some(s) => format!("Use Steam's choice ({s})"),
                        None => "Use Steam's choice (none set, the newest Proton is used)".to_string(),
                    };
                    if ui.radio(mine.is_none(), default_label).clicked() && mine.is_some() {
                        client.send(Command::SetCompatTool { app, tool: None });
                    }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for t in info.tools.iter().filter(|t| t.kind != ToolKind::Runtime) {
                            let on = mine.as_deref() == Some(t.internal_name.as_str());
                            let steams = steam.as_deref() == Some(t.internal_name.as_str());
                            let tag = if steams { "  (Steam's choice)" } else { "" };
                            let label = format!("{}  {}{tag}", t.display_name, origin_label(t.origin));
                            if ui.radio(on, label).clicked() && !on {
                                client.send(Command::SetCompatTool { app, tool: Some(t.internal_name.clone()) });
                            }
                        }
                    });
                    if info.tools.iter().all(|t| t.kind == ToolKind::Runtime) {
                        ui.label(RichText::new("No compatibility tool is installed.").color(theme::ERROR_TEXT));
                    }
                }
                Load::Failed(e) => {
                    ui.label(RichText::new(e).color(theme::ERROR_TEXT));
                }
                _ => {
                    ui.add(egui::Spinner::new());
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if widgets::button(ui, "Close", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
                goto = widgets::link(ui, "Open Proton manager").clicked();
            });
        });
        if goto {
            self.compat = None;
            self.goto = Some(Nav::Settings(SettingsTab::Compatibility));
        } else if modal.should_close() {
            self.compat = None;
        }
    }

    fn play_problem(&mut self, ctx: &Context, client: &mut Client) {
        let Some(p) = client.state().play_problem.clone() else { return };
        let mut action = None;
        let modal = modal(Id::new("play-problem")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, &format!("Can't start {}", p.name));
            ui.label(RichText::new(&p.message).color(theme::TEXT));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if p.needs_compat_tool
                    && widgets::button(ui, "Open Proton manager", ButtonStyle::Primary, true).clicked()
                {
                    action = Some(Nav::Settings(SettingsTab::Compatibility));
                }
                if widgets::button(ui, "Launch via Steam client", ButtonStyle::Secondary, true).clicked() {
                    client.send(Command::LaunchViaSteam(p.app));
                    ui.close();
                }
                if widgets::button(ui, "Close", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        if action.is_some() {
            self.goto = action;
            client.send(Command::DismissPlayProblem);
        } else if modal.should_close() {
            client.send(Command::DismissPlayProblem);
        }
    }
}

/// A modal with our own frame: padded, themed, with a hairline border.
fn modal(id: Id) -> egui::Modal {
    let frame = egui::Frame::new()
        .fill(theme::BG_SIDEBAR)
        .inner_margin(egui::Margin::same(20))
        .corner_radius(theme::BUTTON_RADIUS)
        .stroke(egui::Stroke::new(1.0, theme::WINDOW_BORDER));
    egui::Modal::new(id).frame(frame)
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::bold(18.0)).color(theme::TEXT));
    ui.add_space(8.0);
}

fn field(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::bold(11.0)).color(theme::TEXT_DIM));
    ui.add_space(2.0);
}

pub fn origin_label(o: ToolOrigin) -> &'static str {
    match o {
        ToolOrigin::SteamLibrary => "Steam",
        ToolOrigin::SteamCustom => "Steam, custom",
        ToolOrigin::System => "System",
        ToolOrigin::Ours => "Kettle",
    }
}

pub fn kind_label(k: ToolKind) -> &'static str {
    match k {
        ToolKind::Proton => "Proton",
        ToolKind::Runtime => "Runtime",
        ToolKind::Luxtorpeda => "Native engine ports",
        ToolKind::Other => "Other",
    }
}
