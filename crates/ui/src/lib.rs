//! egui views for steam-rust.
//!
//! Architecture invariant: no I/O here. Views read `client.state()`, keep only
//! view-local state (search text, scroll), and act through `client.send(..)`.

mod fmt;
mod fonts;
mod game_page;
mod library;
mod login;
mod sidebar;
mod textures;
pub mod theme;
mod widgets;

use egui::{Align, Layout, RichText, Ui};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command, Screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    Store,
    #[default]
    Library,
    Community,
    Profile,
}

/// All view-local state.
#[derive(Default)]
pub struct AppUi {
    textures: textures::Textures,
    tab: Tab,
    library: library::LibraryView,
    sidebar: sidebar::SidebarView,
}

impl AppUi {
    /// Call once at startup.
    pub fn setup(ctx: &egui::Context) {
        fonts::install(ctx);
        theme::apply(ctx);
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client) {
        client.poll();
        self.textures.begin_frame(ui.ctx(), client);

        if let Screen::Login(login) = &client.state().screen {
            let login = login.clone();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(ui, |ui| {
                login::show(ui, client, &login);
            });
            return;
        }

        self.menu_bar(ui, client);
        self.nav_bar(ui, client);
        self.bottom_bar(ui, client);
        match self.tab {
            Tab::Library => {
                let library_rev = client.state().library_rev;
                self.sidebar.show(ui, client, &mut self.textures, library_rev);
                egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(
                    ui,
                    |ui| match client.state().selected {
                        Some(app) => game_page::show(ui, client, &mut self.textures, app),
                        None => self.library.show(ui, client, &mut self.textures),
                    },
                );
            }
            tab => {
                egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        let what = match tab {
                            Tab::Store => "The store",
                            Tab::Community => "Community",
                            _ => "Profiles",
                        };
                        ui.label(RichText::new(format!("{what} is coming in a later version.")).color(theme::TEXT_DIM));
                    });
                });
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin::symmetric(8, 0));
        egui::Panel::top("menu").exact_size(theme::MENU_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.style_mut().override_font_id = Some(theme::regular(13.0));
                ui.visuals_mut().override_text_color = Some(theme::TEXT_HEADING);
                // Plain text items; a box shows only on hover/open.
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                ui.menu_button(theme::APP_NAME, |ui| {
                    if ui.button("Sign out").clicked() {
                        client.send(Command::Logout);
                    }
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Library").clicked() {
                        self.tab = Tab::Library;
                    }
                    if ui.button("Refresh library").clicked() {
                        client.send(Command::RefreshLibrary);
                    }
                });
                ui.menu_button("Friends", |ui| {
                    ui.label(RichText::new("Friends are coming in a later version.").color(theme::TEXT_DIM));
                });
                ui.menu_button("Games", |ui| {
                    if ui.button("View game library").clicked() {
                        self.tab = Tab::Library;
                        client.send(Command::Select(None));
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("{} {}", theme::APP_NAME, env!("CARGO_PKG_VERSION")));
                    ui.label(RichText::new("Unofficial. Not affiliated with Valve.").color(theme::TEXT_DIM));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(a) = &client.state().account {
                        ui.label(RichText::new(format!("{} {}", icon::USER_CIRCLE, a.account_name)).color(theme::TEXT));
                    }
                    if client.state().refreshing {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                });
            });
        });
    }

    fn nav_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::top("nav").exact_size(theme::NAV_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let back = widgets::icon_button(ui, icon::CARET_LEFT, 26.0, "Back");
                if back.clicked() {
                    client.send(Command::Select(None));
                }
                widgets::icon_button(ui, icon::CARET_RIGHT, 26.0, "Forward");
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.x = 26.0;
                let account =
                    client.state().account.as_ref().map(|a| a.account_name.to_uppercase()).unwrap_or_default();
                for (tab, label) in [
                    (Tab::Store, "STORE"),
                    (Tab::Library, "LIBRARY"),
                    (Tab::Community, "COMMUNITY"),
                    (Tab::Profile, account.as_str()),
                ] {
                    if widgets::text_tab(ui, label, theme::bold(19.0), self.tab == tab).clicked() {
                        self.tab = tab;
                    }
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_BOTTOM).inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("bottom").exact_size(theme::BOTTOM_BAR_H).frame(frame).show_separator_line(false).show(
            ui,
            |ui| {
                ui.horizontal_centered(|ui| {
                    ui.style_mut().override_font_id = Some(theme::medium(13.0));
                    ui.label(RichText::new(format!("{}  Add a Game", icon::PLUS)).color(theme::TEXT_DIM));
                    let notice = client.state().notice.clone();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Friends & Chat  {}", icon::CHAT_CIRCLE)).color(theme::TEXT_DIM),
                        );
                        ui.add_space(ui.available_width() / 2.0 - 90.0);
                        ui.label(RichText::new(format!("{}  Downloads", icon::DOWNLOAD_SIMPLE)).color(theme::TEXT_DIM));
                        if let Some(n) = notice {
                            ui.add_space(24.0);
                            ui.label(RichText::new(n).color(theme::TEXT_DIM).small()).on_hover_text("Last message");
                        }
                    });
                });
            },
        );
    }
}
