//! egui views for kettle.
//!
//! Architecture invariant: no I/O here. Views read `client.state()`, keep only
//! view-local state (search text, scroll), and act through `client.send(..)`.

mod add_game_ui;
mod anim;
mod chrome;
mod cloud_ui;
mod collections_ui;
mod dialogs;
mod downloads_page;
mod fmt;
mod fonts;
mod friends;
mod game_env_ui;
mod game_menu;
mod game_page;
mod kettle_update_ui;
mod library;
mod login;
mod menubar;
mod nav;
mod overlay;
mod overlay_panel;
mod profile;
mod properties;
mod protondb_badge;
mod settings_downloads;
mod settings_extra;
mod settings_gamescope;
mod settings_ingame;
mod settings_steam;
mod settings_interface;
mod settings_notify;
mod settings_page;
mod settings_storage;
mod sidebar;
mod steam_icons;
mod store;
#[cfg(test)]
mod tests;
mod textures;
pub mod theme;
mod widgets;

use egui::{Align, Id, Layout, RichText, Ui};
use sr_client::{Client, CollCmd, Command, ConnectionStatus, Screen};
use sr_model::AppId;

pub use overlay_panel::{OverlayPanel, PanelOutput};
pub(crate) use settings_page::SettingsTab;

/// A page change asked for by a dialog or a page (handled once per frame by [`AppUi`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Nav {
    /// A game's page in the library.
    Game(AppId),
    Downloads,
    Settings(SettingsTab),
}

/// What a tray menu item asks the window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayGo {
    Store,
    Library,
    Downloads,
    Friends,
    Settings,
    Play(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    Store,
    #[default]
    Library,
    Community,
    Profile,
    /// Reached from the bottom bar and menus, not from the tab row.
    Downloads,
}

/// All view-local state.
#[derive(Default)]
pub struct AppUi {
    textures: textures::Textures,
    tab: Tab,
    library: library::LibraryView,
    sidebar: sidebar::SidebarView,
    store: store::StoreView,
    settings: settings_page::SettingsView,
    properties: properties::PropertiesView,
    overlays: dialogs::Overlays,
    /// The "Add a Non-Steam Game" dialog while it is open.
    add_game: Option<add_game_ui::AddGameDialog>,
    friends: friends::FriendsView,
    profile: profile::ProfileView,
    /// The last `nav_seq` of the profile page we reacted to.
    profile_seen: u64,
    overlay: overlay::OverlayView,
    /// What the central page showed last frame, and when it changed (egui time).
    page: Option<(Tab, Option<AppId>, u64)>,
    page_since: f64,
    /// Which menu of the menu bar is open.
    bar: menubar::BarState,
    help: menubar::HelpDialogs,
    history: nav::History,
    /// The graphics adapter name (set by the app, shown in System information).
    gpu: String,
    started: bool,
    focus_sent: Option<bool>,
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
        // Zoom (Ctrl +/-/0) is ours, so it is saved with the interface settings.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
    }

    /// A tray menu choice other than "show": go to a page, or start a game.
    pub fn tray_go(&mut self, ctx: &egui::Context, client: &mut Client, go: TrayGo) {
        match go {
            TrayGo::Store => self.goto(Tab::Store, client),
            TrayGo::Library => {
                self.goto(Tab::Library, client);
                client.send(Command::Select(None));
            }
            TrayGo::Downloads => self.goto(Tab::Downloads, client),
            TrayGo::Friends => client.send(Command::OpenFriends),
            TrayGo::Settings => self.navigate(ctx, Nav::Settings(SettingsTab::default()), client),
            TrayGo::Play(id) => client.send(Command::Play(AppId(id))),
        }
    }

    /// The graphics adapter name, for Help > System information.
    pub fn set_gpu_name(&mut self, name: String) {
        self.gpu = name;
    }

    /// True when the user chose Exit (as opposed to closing the window): the app really quits then, even if
    /// closing the window would only hide it in the tray.
    pub fn quit_requested(ctx: &egui::Context) -> bool {
        ctx.data(|d| d.get_temp::<bool>(Id::new("kettle-quit"))).unwrap_or(false)
    }

    /// Apply the saved interface settings (animation mode, scale) once per frame.
    fn apply_interface(&mut self, ctx: &egui::Context, client: &mut Client) {
        let i = client.state().settings.interface.clone();
        anim::set_mode(ctx, i.animations);
        if (ctx.zoom_factor() - i.zoom()).abs() > 0.001 {
            ctx.set_zoom_factor(i.zoom());
        }
        if !self.started {
            self.started = true;
            if i.start_page == sr_client::StartPage::Store {
                self.goto(Tab::Store, client);
            }
            // Screenshots in the check workflow: `KETTLE_SHOT_OPEN=game:<appid> | settings:<page> | downloads | store`.
            if let Ok(open) = std::env::var("KETTLE_SHOT_OPEN") {
                let (what, arg) = open.split_once(':').unwrap_or((open.as_str(), ""));
                match what {
                    "game" => {
                        if let Ok(id) = arg.parse::<u32>() {
                            self.navigate(ctx, Nav::Game(AppId(id)), client);
                        }
                    }
                    "settings" => {
                        let tab = match arg {
                            "interface" => SettingsTab::Interface,
                            "library" => SettingsTab::Library,
                            "storage" => SettingsTab::Storage,
                            "downloads" => SettingsTab::Downloads,
                            "notifications" => SettingsTab::Notifications,
                            "compatibility" => SettingsTab::Compatibility,
                            "ingame" => SettingsTab::InGame,
                            "privacy" => SettingsTab::Privacy,
                            _ => SettingsTab::Account,
                        };
                        self.navigate(ctx, Nav::Settings(tab), client);
                    }
                    "downloads" => self.navigate(ctx, Nav::Downloads, client),
                    "store" => self.goto(Tab::Store, client),
                    _ => {}
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client) {
        client.poll();
        self.apply_interface(&ui.ctx().clone(), client);
        self.textures.begin_frame(ui.ctx(), client);
        let focused = ui.input(|i| i.viewport().focused.unwrap_or(true));
        if self.focus_sent != Some(focused) {
            self.focus_sent = Some(focused);
            client.send(Command::WindowFocus(focused));
        }

        if let Screen::Login(login) = &client.state().screen {
            let login = login.clone();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(ui, |ui| {
                login::show(ui, client, &login);
            });
            return;
        }

        // "View profile" (friends list, profile tab) asks for the profile page.
        let profile_seq = client.state().profile.nav_seq;
        if profile_seq != self.profile_seen {
            self.profile_seen = profile_seq;
            self.tab = Tab::Profile;
        }
        self.menu_bar(ui, client);
        self.nav_bar(ui, client);
        self.bottom_bar(ui, client);
        self.shortcuts(ui, client);
        let mut nav = self.overlays.goto.take();
        let selected = client.state().selected;
        let store_page = self.store.page_id();
        let key = (self.tab, selected, store_page);
        {
            let lib = self.tab == Tab::Library;
            self.history.record(nav::Place {
                tab: self.tab,
                game: if lib { selected } else { None },
                store: if self.tab == Tab::Store { store_page } else { 0 },
                collection: if lib { client.state().collections.selected.clone() } else { None },
            });
        }
        if self.page != Some(key) {
            // The very first frame shows its page at once; later changes fade in.
            self.page_since = if self.page.is_some() { ui.input(|i| i.time) } else { f64::NEG_INFINITY };
            self.page = Some(key);
        }
        let enter = anim::since(ui.ctx(), self.page_since, 0.12);
        if self.tab == Tab::Library {
            let library_rev = client.state().library_rev;
            self.sidebar.show(ui, client, &mut self.textures, &mut self.overlays, library_rev);
        }
        self.friends.show_panel(ui, client, &mut self.textures);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_MAIN)).show(ui, |ui| {
            // Pages fade in and slide up by a few points.
            ui.set_opacity(enter);
            if anim::movement(ui.ctx()) {
                ui.add_space(theme::PAGE_SLIDE * (1.0 - enter));
            }
            kettle_update_ui::banner(ui, client);
            match (self.tab, selected) {
                (Tab::Library, Some(app)) => {
                    game_page::show(ui, client, &mut self.textures, app, &mut self.overlays);
                }
                (Tab::Downloads, _) => downloads_page::show(ui, client, &mut self.textures, &mut nav),
                (Tab::Library, None) => self.library.show(ui, client, &mut self.textures, &mut self.overlays),
                (Tab::Store, _) => self.store.show(ui, client, &mut self.textures),
                (Tab::Profile, _) => self.profile.show(ui, client, &mut self.textures),
                (tab, _) => {
                    ui.centered_and_justified(|ui| {
                        let what = if tab == Tab::Community { "Community" } else { "Profiles" };
                        ui.label(RichText::new(format!("{what} is coming in a later version.")).color(theme::TEXT_DIM));
                    });
                }
            }
        });
        self.overlays.show(ui.ctx(), client);
        cloud_ui::conflict_dialog(ui.ctx(), client);
        if let Some(d) = &mut self.add_game
            && !d.show(ui.ctx(), client)
        {
            self.add_game = None;
        }
        collections_ui::show_prompt(ui.ctx(), client);
        if let Some((app, tab)) = self.overlays.properties.take() {
            self.properties.open(app, tab, client, ui.ctx());
        }
        self.properties.show(ui.ctx(), client, &mut self.overlays);
        self.settings.show(ui.ctx(), client);
        self.overlay.show(ui.ctx(), client, &mut self.textures, &mut self.friends);
        self.help.show(ui.ctx());
        if let Some(n) = nav.or_else(|| self.overlays.goto.take()) {
            self.navigate(ui.ctx(), n, client);
        }
        chrome::alt_drag(ui);
        chrome::resize_zones(ui);
    }

    /// The Shift+Tab shortcut (or the "Show overlay" menu item): show or hide the in-game overlay.
    /// The overlay is drawn by a helper process: the menu item then only records a request (see `take_overlay_request`).
    pub fn set_overlay_external(&mut self, on: bool) {
        self.overlay.set_external(on);
    }

    /// True once after "Show overlay" was chosen while the helper process draws the overlay.
    pub fn take_overlay_request(&mut self) -> bool {
        self.overlay.take_request()
    }

    pub fn toggle_overlay(&mut self, client: &mut Client) {
        self.overlay.toggle(client);
    }

    /// Switch tab; the store loads its rows when it is opened.
    fn goto(&mut self, tab: Tab, client: &mut Client) {
        if tab == Tab::Store && self.tab != Tab::Store {
            client.send(Command::StoreLoad);
        }
        if tab == Tab::Profile
            && self.tab != Tab::Profile
            && let Some(a) = client.state().account.as_ref().map(|a| a.steam_id)
        {
            // The profile tab shows our own profile.
            client.send(Command::ViewProfile(a));
        }
        self.tab = tab;
    }

    /// Back or forward in the history (title bar buttons, mouse buttons 4 and 5, Alt+Left/Right, Esc).
    fn step_history(&mut self, client: &mut Client, back: bool) {
        let place = if back { self.history.back() } else { self.history.forward() };
        let Some(p) = place else { return };
        self.goto(p.tab, client);
        if p.tab == Tab::Library {
            client.send(Command::Select(p.game));
            client.send(Command::Collections(CollCmd::Show(p.collection)));
        }
        if p.tab == Tab::Store {
            self.store.set_page_id(client, p.store);
        }
    }

    fn do_act(&mut self, ctx: &egui::Context, act: menubar::Act, client: &mut Client) {
        use menubar::Act;
        match act {
            Act::Settings => self.navigate(ctx, Nav::Settings(SettingsTab::default()), client),
            Act::SignOut => client.send(Command::Logout),
            Act::Quit => {
                ctx.data_mut(|d| d.insert_temp(Id::new("kettle-quit"), true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Act::Url(u) => client.send(Command::OpenUrl(u.to_string())),
            Act::GoLibrary => {
                self.goto(Tab::Library, client);
                client.send(Command::Select(None));
            }
            Act::GoStore => self.goto(Tab::Store, client),
            Act::GoDownloads => self.goto(Tab::Downloads, client),
            Act::ToggleFriends => {
                let open = client.state().friends.open;
                client.send(if open { Command::CloseFriends } else { Command::OpenFriends });
            }
            Act::AddFriend => {
                client.send(Command::OpenFriends);
                self.friends.open_add();
            }
            Act::ViewMyProfile => self.goto(Tab::Profile, client),
            Act::EditMyProfile => {
                self.goto(Tab::Profile, client);
                client.send(Command::ProfileEditStart);
            }
            Act::ShowHidden => {
                self.goto(Tab::Library, client);
                client.send(Command::Select(None));
                client.send(Command::Collections(CollCmd::Show(Some(sr_client::HIDDEN.to_string()))));
            }
            Act::RefreshLibrary => client.send(Command::RefreshLibrary),
            Act::Zoom(d) => {
                let mut i = client.state().settings.interface.clone();
                i.scale_percent = if d == 0 { 100 } else { (i.scale_percent as i32 + d * 10).max(0) as u32 };
                client.send(Command::SetInterface(i));
            }
            Act::SetStatus(s) => client.send(Command::SetPersonaState(s)),
            Act::NewCollection => collections_ui::open_prompt(ctx, None),
            Act::ToggleOverlay => self.overlay.toggle(client),
            Act::SystemInfo => self.help.open_sysinfo(sr_client::system_info(client.state(), &self.gpu)),
            Act::About => self.help.open_about(),
        }
    }

    fn navigate(&mut self, ctx: &egui::Context, nav: Nav, client: &mut Client) {
        match nav {
            Nav::Game(app) => {
                self.goto(Tab::Library, client);
                client.send(Command::Select(Some(app)));
            }
            Nav::Downloads => self.goto(Tab::Downloads, client),
            Nav::Settings(tab) => self.settings.open(tab, client, ctx),
        }
    }

    /// Ctrl+Tab and Ctrl+Shift+Tab cycle the top tabs.
    fn shortcuts(&mut self, ui: &Ui, client: &mut Client) {
        use egui::{Key, Modifiers};
        let next = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab));
        let prev = ui.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab));
        if next || prev {
            self.goto(self.tab.step(prev), client);
        }
        let ctx = ui.ctx().clone();
        let (b4, b5, alt_left, alt_right, esc, zin, zout, zreset) = ui.input_mut(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
                i.consume_key(Modifiers::ALT, Key::ArrowLeft),
                i.consume_key(Modifiers::ALT, Key::ArrowRight),
                i.key_pressed(Key::Escape),
                i.consume_key(Modifiers::CTRL, Key::Plus) | i.consume_key(Modifiers::CTRL, Key::Equals),
                i.consume_key(Modifiers::CTRL, Key::Minus),
                i.consume_key(Modifiers::CTRL, Key::Num0),
            )
        });
        if b4 || alt_left {
            self.step_history(client, true);
        } else if b5 || alt_right {
            self.step_history(client, false);
        }
        if zin {
            self.do_act(&ctx, menubar::Act::Zoom(1), client);
        } else if zout {
            self.do_act(&ctx, menubar::Act::Zoom(-1), client);
        } else if zreset {
            self.do_act(&ctx, menubar::Act::Zoom(0), client);
        }
        // Escape goes back, unless it is closing something (a menu, a dialog) or the user is typing.
        let busy = self.bar.open.is_some()
            || egui::Popup::is_any_open(&ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some() || m.focused().is_some());
        if esc && !busy {
            self.step_history(client, true);
        }
        // Space or Enter on a game's page plays it.
        if self.tab == Tab::Library && !busy {
            let go = ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Space)
            });
            if let (true, Some(app)) = (go, client.state().selected) {
                let ready = client.state().game(app).is_some_and(|g| g.install.is_installed())
                    && !client.state().running.contains_key(&app);
                if ready {
                    client.send(Command::Play(app));
                }
            }
        }
    }

    /// Our persona name once Steam sent it, else the account name.
    fn display_name(&self, client: &Client) -> Option<String> {
        let s = client.state();
        s.friends.persona_name().map(str::to_string).or_else(|| s.account.as_ref().map(|a| a.account_name.clone()))
    }

    fn menu_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        let frame = egui::Frame::new().fill(theme::BG_TOP);
        egui::Panel::top("menu").exact_size(theme::MENU_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            // Empty space in the bar drags the window (the title bar).
            chrome::drag_region(ui);
            ui.horizontal_centered(|ui| {
                ui.style_mut().override_font_id = Some(theme::regular(13.0));
                ui.spacing_mut().item_spacing.x = 0.0;
                // Plain text items: no box at rest, brighter text on hover.
                ui.visuals_mut().widgets.open.weak_bg_fill = theme::MENU_BG;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                ui.add_space(14.0);
                let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                steam_icons::kettle_mark(ui.painter(), mark.center(), 16.0, theme::TEXT_DIM);
                ui.add_space(2.0);
                let mut state = self.bar;
                let act = menubar::bar(ui, &mut state, client);
                self.bar = state;
                if let Some(act) = act {
                    self.do_act(&ui.ctx().clone(), act, client);
                }
                let mut account_act = None;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chrome::window_buttons(ui);
                    self.top_right(ui, client, &mut account_act);
                });
                if let Some(act) = account_act {
                    self.do_act(&ui.ctx().clone(), act, client);
                }
            });
        });
    }

    /// Right side of the top bar, laid out right to left: screen button, account pill, bell, announcements.
    fn top_right(&mut self, ui: &mut Ui, client: &mut Client, act: &mut Option<menubar::Act>) {
        use egui::{Rect, Sense, vec2};
        let ctx = ui.ctx().clone();
        // The "big screen" button: here it toggles full screen.
        let (r, resp) = ui.allocate_exact_size(vec2(24.0, 26.0), Sense::click());
        let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
        steam_icons::monitor(ui.painter(), r.center(), 14.0, theme::TOP_ICON.lerp_to_gamma(egui::Color32::WHITE, t));
        if resp.on_hover_text("Full screen").clicked() {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
        ui.add_space(13.0);
        if let Some(name) = self.display_name(client) {
            let galley = ui.painter().layout_no_wrap(name, theme::regular(13.0), theme::TOP_NAME);
            let w = 3.0 + 24.0 + 6.0 + galley.size().x + 8.0 + 10.0 + 10.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
            let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
            ui.painter().rect_filled(rect, 2.0, theme::TOP_ACCOUNT.lerp_to_gamma(theme::BG_ROW_SELECTED, 0.3 * t));
            let av = Rect::from_min_size(rect.min + vec2(2.0, 1.0), vec2(24.0, 24.0));
            let url = client.state().friends.my_avatar_url().unwrap_or("").to_string();
            match self.textures.get_web(client, &url) {
                textures::Art::Ready(tex, a) => {
                    widgets::image_cover(ui, av, tex, egui::Color32::WHITE.gamma_multiply(a))
                }
                _ => {
                    ui.painter().rect_filled(av, 2.0, theme::BG_CAPSULE);
                }
            }
            let text_pos = egui::pos2(av.right() + 6.0, rect.center().y - galley.size().y / 2.0);
            let name_right = text_pos.x + galley.size().x;
            ui.painter().galley(text_pos, galley, theme::TOP_NAME);
            steam_icons::chevron_down(
                ui.painter(),
                egui::pos2(name_right + 11.0, rect.center().y + 1.0),
                8.0,
                theme::TOP_WALLET,
            );
            let tip = match &client.state().connection {
                ConnectionStatus::Online => "Connected to Steam",
                ConnectionStatus::Connecting => "Connecting to Steam...",
                ConnectionStatus::Failed(_) => "Not connected to Steam",
                ConnectionStatus::Offline => "Offline",
            };
            let resp = resp.on_hover_text(tip);
            widgets::popup_menu(&resp, |ui| {
                use menubar::Act;
                use sr_client::PersonaState;
                let mut pick = |ui: &mut Ui, label: &str, a: Act| {
                    if ui.button(label).clicked() {
                        *act = Some(a);
                        ui.close();
                    }
                };
                pick(ui, "View my profile", Act::ViewMyProfile);
                pick(ui, "Edit my profile", Act::EditMyProfile);
                widgets::menu_separator(ui);
                pick(ui, "Online", Act::SetStatus(PersonaState::Online));
                pick(ui, "Away", Act::SetStatus(PersonaState::Away));
                pick(ui, "Invisible", Act::SetStatus(PersonaState::Invisible));
                pick(ui, "Offline", Act::SetStatus(PersonaState::Offline));
                widgets::menu_separator(ui);
                pick(ui, "Settings", Act::Settings);
                pick(ui, "Sign out", Act::SignOut);
            });
        }
        ui.add_space(8.0);
        // Bell: notifications (none yet).
        let (r, resp) = ui.allocate_exact_size(vec2(48.0, 26.0), Sense::click());
        let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
        ui.painter().rect_filled(r, 2.0, theme::TOP_BELL_BTN.lerp_to_gamma(theme::BG_ROW_SELECTED, 0.3 * t));
        steam_icons::bell(ui.painter(), r.center(), 14.0, theme::TOP_ICON.lerp_to_gamma(egui::Color32::WHITE, t));
        widgets::popup_menu(&resp, |ui| widgets::menu_label(ui, "No new notifications", true));
        ui.add_space(7.0);
        // Announcements: the blue button.
        let (r, resp) = ui.allocate_exact_size(vec2(52.0, 26.0), Sense::click());
        let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
        ui.painter().rect_filled(r, 2.0, widgets::shade(theme::BTN_PRIMARY_A, t, resp.is_pointer_button_down_on()));
        steam_icons::megaphone(ui.painter(), r.center(), 15.0, egui::Color32::WHITE);
        if resp.on_hover_text("Announcements").clicked() {
            client.send(Command::OpenUrl("https://store.steampowered.com/news/".to_string()));
        }
        if client.state().refreshing && client.state().connection == ConnectionStatus::Online {
            ui.add_space(8.0);
            ui.add(egui::Spinner::new().size(12.0)).on_hover_text("Loading your games...");
        }
        if let ConnectionStatus::Failed(why) = &client.state().connection {
            ui.add_space(8.0);
            ui.label(RichText::new("Can't reach Steam. Retrying...").color(theme::ERROR_TEXT).small())
                .on_hover_text(why);
        }
        if client.state().connection == ConnectionStatus::Connecting {
            ui.add_space(8.0);
            ui.add(egui::Spinner::new().size(12.0)).on_hover_text("Connecting to Steam...");
        }
    }

    fn nav_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        use egui::{Rect, Sense, pos2, vec2};
        let frame = egui::Frame::new().fill(theme::BG_TOP);
        egui::Panel::top("nav").exact_size(theme::NAV_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            chrome::drag_region(ui);
            let bar = ui.max_rect();
            let cy = bar.top() + 12.0;
            let ctx = ui.ctx().clone();
            let (can_back, can_fwd) = (self.history.can_back(), self.history.can_forward());
            // History arrows (mouse buttons 4 and 5 and Alt+Left/Right do the same).
            for (i, left) in [true, false].into_iter().enumerate() {
                let enabled = if left { can_back } else { can_fwd };
                let c = pos2(bar.left() + 23.0 + 31.0 * i as f32, cy);
                let resp =
                    ui.interact(Rect::from_center_size(c, vec2(28.0, 28.0)), Id::new(("nav-arrow", i)), Sense::click());
                let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered() && enabled, anim::FAST);
                let base = if enabled { theme::TEXT_DIM } else { theme::TEXT_DIM.gamma_multiply(0.45) };
                steam_icons::arrow(ui.painter(), c, 16.0, left, base.lerp_to_gamma(egui::Color32::WHITE, t));
                if enabled
                    && resp
                        .on_hover_text(if left { "Back (Alt+Left, mouse 4)" } else { "Forward (Alt+Right, mouse 5)" })
                        .clicked()
                {
                    self.step_history(client, left);
                }
            }
            let account = self.display_name(client).map(|n| n.to_uppercase()).unwrap_or_default();
            let mut x = bar.left() + 77.0;
            let mut active_rect = None;
            for (tab, label) in [
                (Tab::Store, "STORE"),
                (Tab::Library, "LIBRARY"),
                (Tab::Community, "COMMUNITY"),
                (Tab::Profile, account.as_str()),
            ] {
                let active = self.tab == tab;
                let galley =
                    ui.painter().layout_no_wrap(label.to_string(), theme::medium(theme::NAV_TAB_FONT), theme::TEXT);
                let size = galley.size();
                let rect = Rect::from_min_size(pos2(x, cy - size.y / 2.0), size);
                let resp = ui.interact(rect.expand2(vec2(8.0, 10.0)), Id::new(("nav-tab", label)), Sense::click());
                let hover = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered() && !active, anim::FAST);
                let color = if active { theme::ACCENT } else { theme::TEXT.lerp_to_gamma(egui::Color32::WHITE, hover) };
                ui.painter().galley_with_override_text_color(rect.min, galley, color);
                if active {
                    active_rect = Some(rect);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.goto(tab, client);
                }
                x += size.x + 21.0;
            }
            if let Some(r) = active_rect {
                // One underline (3 px, the width of the tab text) that slides to the active tab.
                let left = anim::animate_to(&ctx, Id::new("tab-underline-l"), r.left(), anim::NORMAL);
                let right = anim::animate_to(&ctx, Id::new("tab-underline-r"), r.right(), anim::NORMAL);
                let y = bar.top() + 25.0;
                ui.painter().rect_filled(Rect::from_min_max(pos2(left, y), pos2(right, y + 3.0)), 1.0, theme::ACCENT);
            }
        });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, client: &mut Client) {
        use egui::{Rect, Sense, pos2, vec2};
        let frame = egui::Frame::new().fill(theme::BG_BOTTOM).inner_margin(egui::Margin {
            left: 0,
            right: 16,
            top: 0,
            bottom: 0,
        });
        egui::Panel::bottom("bottom").exact_size(theme::BOTTOM_BAR_H).frame(frame).show_separator_line(false).show(
            ui,
            |ui| {
                // "Add a Game" at the left: a small menu.
                let bar = ui.max_rect();
                let ctx = ui.ctx().clone();
                let r = Rect::from_min_size(pos2(bar.left() + 14.0, bar.top()), vec2(110.0, bar.height()));
                let resp = ui.interact(r, Id::new("add-game"), Sense::click());
                let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
                let color = theme::BOTTOM_TEXT.lerp_to_gamma(theme::TEXT, t);
                steam_icons::add_square(ui.painter(), pos2(r.left() + 9.0, r.center().y), 16.0, color, theme::BG_TOP);
                ui.painter().text(
                    pos2(r.left() + 26.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Add a Game",
                    theme::regular(13.0),
                    color,
                );
                let add_game = &mut self.add_game;
                widgets::popup_menu(&resp.on_hover_cursor(egui::CursorIcon::PointingHand), |ui| {
                    if ui.button("Add a Non-Steam Game...").clicked() {
                        *add_game = Some(add_game_ui::AddGameDialog::default());
                        ui.close();
                    }
                    if ui.button("Activate a product on Steam...").clicked() {
                        client.send(Command::OpenUrl("https://store.steampowered.com/account/registerkey".to_string()));
                        ui.close();
                    }
                    if ui.button("Redeem a Steam wallet code...").clicked() {
                        client.send(Command::OpenUrl(
                            "https://store.steampowered.com/account/redeemwalletcode".to_string(),
                        ));
                        ui.close();
                    }
                });
                if downloads_button(ui, client) {
                    self.goto(Tab::Downloads, client);
                }
                let bar_w = ui.max_rect().width();
                ui.horizontal_centered(|ui| {
                    ui.style_mut().override_font_id = Some(theme::regular(13.0));
                    let notice = client.state().notice.clone();
                    let toast = self.friends.current_toast(ui.ctx(), client);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if friends::bottom_button(ui, client) {
                            let open = client.state().friends.open;
                            client.send(if open { Command::CloseFriends } else { Command::OpenFriends });
                        }
                        // A fixed-width slot, so a long message is cut short instead of running
                        // into the centered Downloads button or the left item.
                        let slot_w = (bar_w / 2.0 - 354.0).max(80.0);
                        if let Some(t) = toast {
                            ui.add_space(24.0);
                            if friends::toast_label(ui, &t, slot_w) {
                                client.send(Command::OpenChat(t.friend));
                            }
                        } else if let Some(n) = notice {
                            ui.add_space(24.0);
                            let slot = egui::vec2(slot_w, ui.available_height());
                            let layout = Layout::right_to_left(Align::Center);
                            ui.allocate_ui_with_layout(slot, layout, |ui| {
                                let text = RichText::new(&n).color(theme::TEXT_DIM).small();
                                ui.add(egui::Label::new(text).truncate()).on_hover_text(&n);
                            });
                        }
                    });
                });
            },
        );
    }
}

/// The centered "Downloads" button: Steam's square download icon and "Downloads"; while something downloads it
/// adds the name, the percentage and the speed with a thin progress bar. Returns true when clicked.
fn downloads_button(ui: &mut Ui, client: &Client) -> bool {
    let bar = ui.max_rect();
    let rect = egui::Rect::from_center_size(bar.center(), egui::vec2(380.0, bar.height()));
    let resp = ui.interact(rect, Id::new("downloads-button"), egui::Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let color = theme::BOTTOM_TEXT.lerp_to_gamma(theme::TEXT, hover);
    let line = match &client.state().downloads.active {
        Some(a) => {
            let shown = anim::glide(ui.ctx(), Id::new(("dl-bar", a.item.request.app.0)), a.fraction(), 4.0);
            let track = egui::Rect::from_min_size(
                egui::pos2(rect.center().x - 120.0, bar.bottom() - 6.0),
                egui::vec2(240.0, 3.0),
            );
            ui.painter().rect_filled(track, theme::RADIUS, theme::BG_TOP);
            let fill = egui::Rect::from_min_size(track.min, egui::vec2(track.width() * shown, track.height()));
            ui.painter().rect_filled(fill, theme::RADIUS, theme::ACCENT);
            format!("Downloads - {}  {:.0}%  {}", a.item.request.name, shown * 100.0, fmt::speed(a.bytes_per_sec))
        }
        None => "Downloads".to_string(),
    };
    let galley = ui.painter().layout_no_wrap(line, theme::regular(13.0), color);
    let total = 16.0 + 8.0 + galley.size().x;
    let left = rect.center().x - total / 2.0;
    steam_icons::download_square(ui.painter(), egui::pos2(left + 8.0, rect.center().y), 16.0, color, theme::BG_TOP);
    ui.painter().galley_with_override_text_color(
        egui::pos2(left + 24.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
