//! Modal dialogs opened from the game page: install options, uninstall confirmation, per-game compatibility
//! tool, and the "could not start" message. View state only; every action is a `Command`.

use crate::properties::PropsTab;
use crate::widgets::{self, ButtonStyle};
use crate::{Nav, SettingsTab, fmt, theme};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, Command, DownloadRequest, Load, ManageCmd, OsChoice, ToolKind, ToolOrigin};
use sr_model::AppId;
use std::collections::HashSet;
use std::path::PathBuf;

const DIALOG_W: f32 = 440.0;

struct InstallDialog {
    app: AppId,
    name: String,
    library: usize,
    os: OsChoice,
    /// The path whose free space was last asked for, so it is asked again only when the choice changes.
    asked: Option<PathBuf>,
    /// Language chosen ("" = the setting's default) and DLC ticked.
    language: String,
    dlc: HashSet<u32>,
    desktop: bool,
    menu: bool,
    /// The platform and language the size lookup was last asked for.
    info_asked: Option<(OsChoice, String)>,
}

/// Which dialogs are open.
#[derive(Default)]
pub struct Overlays {
    install: Option<InstallDialog>,
    uninstall: Option<AppId>,
    /// A Properties window the dialogs or menus asked for; the page loop takes it.
    pub properties: Option<(AppId, PropsTab)>,
    /// Navigation a dialog asked for ("Open Proton manager"); the page loop takes it.
    pub goto: Option<Nav>,
}

impl Overlays {
    pub fn open_install(&mut self, app: AppId, name: &str) {
        self.install = Some(InstallDialog {
            app,
            name: name.to_string(),
            library: 0,
            os: OsChoice::Auto,
            asked: None,
            language: String::new(),
            dlc: HashSet::new(),
            desktop: false,
            menu: false,
            info_asked: None,
        });
    }

    pub fn open_uninstall(&mut self, app: AppId) {
        self.uninstall = Some(app);
    }

    /// The old per-game compatibility dialog now lives in the Properties window.
    pub fn open_compat(&mut self, app: AppId) {
        self.properties = Some((app, PropsTab::Compatibility));
    }

    pub fn open_properties(&mut self, app: AppId) {
        self.properties = Some((app, PropsTab::General));
    }

    pub fn show(&mut self, ctx: &Context, client: &mut Client) {
        self.install_dialog(ctx, client);
        self.uninstall_dialog(ctx, client);
        self.play_problem(ctx, client);
        self.update_prompt(ctx, client);
        self.steam_update_prompt(ctx, client);
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
        // Ask for the size, DLC and languages again when the platform or language changes.
        let wanted = (d.os, d.language.clone());
        if d.info_asked.as_ref() != Some(&wanted) {
            d.info_asked = Some(wanted);
            client.send(Command::Manage(ManageCmd::LoadInstallInfo {
                app: d.app,
                os: d.os,
                language: d.language.clone(),
            }));
        }
        let free = client.state().free_space.iter().find(|(p, _)| *p == lib.path).map(|(_, f)| *f);
        let info = client.state().extra.install_info.get(&d.app.0).cloned();
        let default_language = client.state().settings.downloads.language_or_default().to_string();
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
            let ready = info.as_ref().and_then(|l| l.ready());
            if let Some(i) = ready.filter(|i| i.languages.len() > 1) {
                field(ui, "LANGUAGE");
                let current = if d.language.is_empty() { default_language.as_str() } else { d.language.as_str() };
                let current =
                    if i.languages.iter().any(|l| l == current) { current.to_string() } else { "english".to_string() };
                widgets::combo(
                    ui,
                    "install-lang",
                    DIALOG_W,
                    crate::settings_downloads::language_name(&current),
                    |ui| {
                        for l in &i.languages {
                            if ui.selectable_label(*l == current, crate::settings_downloads::language_name(l)).clicked()
                            {
                                d.language = l.clone();
                            }
                        }
                    },
                );
            }
            if let Some(i) = ready.filter(|i| !i.dlc.is_empty()) {
                field(ui, "DLC");
                egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                    for c in &i.dlc {
                        let mut on = d.dlc.contains(&c.appid);
                        let name = if c.name.is_empty() { format!("DLC {}", c.appid) } else { c.name.clone() };
                        let label = if c.bytes > 0 { format!("{name}  ({})", fmt::bytes(c.bytes)) } else { name };
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                d.dlc.insert(c.appid);
                            } else {
                                d.dlc.remove(&c.appid);
                            }
                        }
                    }
                });
            }
            field(ui, "DISK SPACE");
            let needed = ready.map(|i| {
                let extra: u64 = i.dlc.iter().filter(|c| d.dlc.contains(&c.appid)).map(|c| c.bytes).sum();
                i.disk_bytes + extra
            });
            let free_now = free.flatten();
            match (&info, needed) {
                (_, Some(n)) => {
                    ui.label(RichText::new(format!("Required: {}", fmt::bytes(n))).color(theme::TEXT));
                }
                (Some(Load::Failed(e)), None) => {
                    ui.label(RichText::new(format!("The size is not known: {e}")).color(theme::TEXT_DIM).small());
                }
                _ => {
                    ui.label(RichText::new("Required: checking...").color(theme::TEXT_DIM));
                }
            }
            let free_text = match free {
                Some(Some(b)) => format!("Available: {} on this disk", fmt::bytes(b)),
                Some(None) => "Available: not known for this disk".to_string(),
                None => "Available: checking...".to_string(),
            };
            ui.label(RichText::new(free_text).color(theme::TEXT));
            let too_small = matches!((needed, free_now), (Some(n), Some(f)) if sr_client::with_margin(n) > f);
            if too_small {
                ui.label(RichText::new("There is not enough free space on this disk.").color(theme::ERROR_TEXT));
            }
            field(ui, "SHORTCUTS");
            ui.checkbox(&mut d.desktop, "Create desktop shortcut");
            ui.checkbox(&mut d.menu, "Create application menu shortcut");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                start = widgets::button(ui, "Install", ButtonStyle::Primary, !too_small).clicked();
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        if start {
            let mut req = DownloadRequest::new(d.app, d.name.clone(), lib.path.clone());
            req.os = d.os;
            req.language = d.language.clone();
            req.dlc = d.dlc.iter().copied().collect();
            req.desktop_shortcut = d.desktop;
            req.menu_shortcut = d.menu;
            client.send(Command::Download(req));
            self.install = None;
            self.goto = Some(Nav::Downloads);
        } else if modal.should_close() {
            self.install = None;
        }
    }

    /// Play on a game whose update is waiting.
    fn update_prompt(&mut self, ctx: &Context, client: &mut Client) {
        let Some(app) = client.state().extra.update_prompt else { return };
        let name = client.state().game(app).map_or_else(|| "This game".to_string(), |g| g.name.clone());
        let mut action = None;
        let modal = modal(Id::new("update-prompt")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, "Update required");
            ui.label(RichText::new(format!("{name} has an update on Steam.")).color(theme::TEXT));
            ui.label(
                RichText::new("Playing the old version may not work with other players or newer saves.")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if widgets::button(ui, "Update now", ButtonStyle::Primary, true).clicked() {
                    action = Some(true);
                }
                if widgets::button(ui, "Play anyway", ButtonStyle::Secondary, true).clicked() {
                    action = Some(false);
                }
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        match action {
            Some(true) => {
                client.send(Command::Manage(ManageCmd::DismissUpdatePrompt));
                client.send(Command::Update(app));
                self.goto = Some(Nav::Downloads);
            }
            Some(false) => client.send(Command::Manage(ManageCmd::PlayAnyway(app))),
            None if modal.should_close() => client.send(Command::Manage(ManageCmd::DismissUpdatePrompt)),
            None => {}
        }
    }

    /// Updating a game in the official client's library folder: say what that means and ask.
    fn steam_update_prompt(&mut self, ctx: &Context, client: &mut Client) {
        let Some(app) = client.state().extra.steam_update_prompt else { return };
        let (name, path) = match client.state().game(app) {
            Some(g) => {
                let path = match &g.install {
                    sr_model::InstallState::Installed { path, .. }
                    | sr_model::InstallState::UpdateRequired { path, .. } => path.clone(),
                    _ => String::new(),
                };
                (g.name.clone(), path)
            }
            None => ("This game".to_string(), String::new()),
        };
        use sr_client::SteamUpdatePolicy as P;
        let mut answer = None;
        let modal = modal(Id::new("steam-update-prompt")).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            title(ui, "Update a game the Steam client installed?");
            ui.label(RichText::new(format!("{name} lives in a library folder of the official Steam client:")).color(theme::TEXT));
            ui.label(RichText::new(&path).color(theme::TEXT_DIM).small());
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "If Kettle updates it, both programs write into this folder. The official client may then see the \
                     game as changed and check its files. Nothing is changed unless you agree. You can change this later \
                     in Settings > Downloads.",
                )
                .color(theme::TEXT)
                .small(),
            );
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if widgets::button(ui, "Update this time", ButtonStyle::Primary, true).clicked() {
                    answer = Some((P::Ask, true));
                }
                if widgets::button(ui, "Always allow", ButtonStyle::Secondary, true).clicked() {
                    answer = Some((P::Yes, true));
                }
                if widgets::button(ui, "Never", ButtonStyle::Secondary, true).clicked() {
                    answer = Some((P::Never, false));
                }
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    ui.close();
                }
            });
        });
        if let Some((policy, update)) = answer {
            client.send(Command::Manage(ManageCmd::SteamUpdateAnswer { app, policy, update }));
            if update {
                self.goto = Some(Nav::Downloads);
            }
        } else if modal.should_close() {
            client.send(Command::Manage(ManageCmd::DismissSteamUpdatePrompt));
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
