//! egui views for steam-rust.
//!
//! Architecture invariant: no I/O here. Views read `client.state()`, keep only
//! view-local state (search text, scroll), and act through `client.send(..)`.

mod anim;
mod chrome;
mod fmt;
mod fonts;
mod game_page;
mod library;
mod login;
mod sidebar;
mod store;
mod textures;
pub mod theme;
mod widgets;

use egui::{Align, Id, Layout, Pos2, RichText, Ui};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command, Screen};
use sr_model::AppId;

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
    store: store::StoreView,
    /// What the central page showed last frame, and when it changed (egui time).
    page: Option<(Tab, Option<AppId>, u64)>,
    page_since: f64,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Store, Tab::Library, Tab::Community, Tab::Profile];

    /// The next tab in the row, wrapping around (Ctrl+Tab).
    fn step(self, back: bool) -> Tab {
        let i = Tab::ALL.iter().position(|t| *t == self).unwrap_or(0);
        let n = Tab::ALL.len();
        Tab::ALL[(i + if back { n - 1 } else { 1 }) % n]
    }
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
        self.shortcuts(ui, client);
        let selected = client.state().selected;
        let store_page = self.store.page_id();
        let key = (self.tab, selected, store_page);
        if self.page != Some(key) {
            // The very first frame shows its page at once; later changes fade in.
            self.page_since = if self.page.is_some() { ui.input(|i| i.time) } else { f64::NEG_INFINITY };
            self.page = Some(key);
        }
        let enter = anim::since(ui.ctx(), self.page_since, 0.18);
        if self.tab == Tab::Library {
            let library_rev = client.state().library_rev;
            self.sidebar.show(ui, client, &mut self.textures, library_rev);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(ui, |ui| {
            // Pages fade in and slide up by a few points.
            ui.set_opacity(enter);
            ui.add_space(theme::PAGE_SLIDE * (1.0 - enter));
            match (self.tab, selected) {
                (Tab::Library, Some(app)) => game_page::show(ui, client, &mut self.textures, app),
                (Tab::Library, None) => self.library.show(ui, client, &mut self.textures),
                (Tab::Store, _) => self.store.show(ui, client, &mut self.textures),
                (tab, _) => {
                    ui.centered_and_justified(|ui| {
                        let what = if tab == Tab::Community { "Community" } else { "Profiles" };
                        ui.label(RichText::new(format!("{what} is coming in a later version.")).color(theme::TEXT_DIM));
                    });
                }
            }
        });
        chrome::alt_drag(ui);
        chrome::resize_zones(ui);
    }

    /// Switch tab; the store loads its rows when it is opened.
    fn goto(&mut self, tab: Tab, client: &mut Client) {
        if tab == Tab::Store && self.tab != Tab::Store {
            client.send(Command::StoreLoad);
        }
        self.tab = tab;
    }

    /// Ctrl+Tab and Ctrl+Shift+Tab cycle the top tabs.
    fn shortcuts(&mut self, ui: &Ui, client: &mut Client) {
        use egui::{Key, Modifiers};
        let next = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab));
        let prev = ui.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab));
        if next || prev {
            self.goto(self.tab.step(prev), client);
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin { left: 8, ..Default::default() });
        egui::Panel::top("menu").exact_size(theme::MENU_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            // Empty space in the bar drags the window (the title bar).
            chrome::drag_region(ui);
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
                    if ui.button("Store").clicked() {
                        self.goto(Tab::Store, client);
                    }
                    if ui.button("Library").clicked() {
                        self.goto(Tab::Library, client);
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
                        self.goto(Tab::Library, client);
                        client.send(Command::Select(None));
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("{} {}", theme::APP_NAME, env!("CARGO_PKG_VERSION")));
                    ui.label(RichText::new("Unofficial. Not affiliated with Valve.").color(theme::TEXT_DIM));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chrome::window_buttons(ui);
                    ui.add_space(10.0);
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
            chrome::drag_region(ui);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let back = widgets::icon_button(ui, icon::CARET_LEFT, 26.0, "Back");
                if back.clicked() {
                    if self.tab == Tab::Store {
                        self.store.back();
                    } else {
                        client.send(Command::Select(None));
                    }
                }
                widgets::icon_button(ui, icon::CARET_RIGHT, 26.0, "Forward");
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.x = 26.0;
                let account =
                    client.state().account.as_ref().map(|a| a.account_name.to_uppercase()).unwrap_or_default();
                let mut active_rect = None;
                for (tab, label) in [
                    (Tab::Store, "STORE"),
                    (Tab::Library, "LIBRARY"),
                    (Tab::Community, "COMMUNITY"),
                    (Tab::Profile, account.as_str()),
                ] {
                    let resp = widgets::text_tab(ui, label, theme::bold(19.0), self.tab == tab);
                    if self.tab == tab {
                        active_rect = Some(resp.rect);
                    }
                    if resp.clicked() {
                        self.goto(tab, client);
                    }
                }
                if let Some(r) = active_rect {
                    // One underline that slides to the active tab.
                    let ctx = ui.ctx().clone();
                    let left = anim::animate_to(&ctx, Id::new("tab-underline-l"), r.left(), anim::NORMAL);
                    let right = anim::animate_to(&ctx, Id::new("tab-underline-r"), r.right(), anim::NORMAL);
                    let y = r.bottom() + 1.0;
                    ui.painter().line_segment([Pos2::new(left, y), Pos2::new(right, y)], (2.0, theme::ACCENT));
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_BOTTOM).inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("bottom").exact_size(theme::BOTTOM_BAR_H).frame(frame).show_separator_line(false).show(
            ui,
            |ui| {
                // Truly centered, whatever the side items measure.
                ui.painter().text(
                    ui.max_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}  Downloads", icon::DOWNLOAD_SIMPLE),
                    theme::medium(13.0),
                    theme::TEXT_DIM,
                );
                ui.horizontal_centered(|ui| {
                    ui.style_mut().override_font_id = Some(theme::medium(13.0));
                    ui.label(RichText::new(format!("{}  Add a Game", icon::PLUS)).color(theme::TEXT_DIM));
                    let notice = client.state().notice.clone();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Friends & Chat  {}", icon::CHAT_CIRCLE)).color(theme::TEXT_DIM),
                        );
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
