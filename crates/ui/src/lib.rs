//! egui views for kettle.
//!
//! Architecture invariant: no I/O here. Views read `client.state()`, keep only
//! view-local state (search text, scroll), and act through `client.send(..)`.

mod anim;
mod chrome;
mod cloud_ui;
mod collections_ui;
mod dialogs;
mod downloads_page;
mod fmt;
mod fonts;
mod friends;
mod game_menu;
mod game_page;
mod library;
mod login;
mod menubar;
mod nav;
mod overlay;
mod overlay_panel;
mod profile;
mod properties;
mod settings_downloads;
mod settings_gamescope;
mod settings_ingame;
mod settings_interface;
mod settings_notify;
mod settings_page;
mod settings_storage;
mod sidebar;
mod store;
mod textures;
pub mod theme;
mod widgets;

use egui::{Align, Id, Layout, Pos2, RichText, Ui};
use egui_phosphor::regular as icon;
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
        let frame = egui::Frame::new().fill(theme::BG_TOP).inner_margin(egui::Margin { left: 8, ..Default::default() });
        egui::Panel::top("menu").exact_size(theme::MENU_BAR_H).frame(frame).show_separator_line(false).show(ui, |ui| {
            // Empty space in the bar drags the window (the title bar).
            chrome::drag_region(ui);
            ui.horizontal_centered(|ui| {
                ui.style_mut().override_font_id = Some(theme::regular(13.0));
                // Plain text items; a box shows only on hover/open.
                ui.visuals_mut().widgets.inactive.fg_stroke.color = theme::TEXT_HEADING;
                ui.visuals_mut().widgets.hovered.weak_bg_fill = theme::BG_ROW_SELECTED;
                ui.visuals_mut().widgets.open.weak_bg_fill = theme::MENU_BG;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                let mut state = self.bar;
                let act = menubar::bar(ui, &mut state, client);
                self.bar = state;
                if let Some(act) = act {
                    self.do_act(&ui.ctx().clone(), act, client);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chrome::window_buttons(ui);
                    ui.add_space(10.0);
                    if let Some(name) = self.display_name(client) {
                        let chip = ui.label(RichText::new(name).font(theme::medium(13.0)).color(theme::TEXT));
                        ui.add_space(6.0);
                        let size = egui::Vec2::splat(20.0);
                        let (av, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        let url = client.state().friends.my_avatar_url().unwrap_or("").to_string();
                        match self.textures.get_web(client, &url) {
                            textures::Art::Ready(t, a) => {
                                widgets::image_cover(ui, av, t, egui::Color32::WHITE.gamma_multiply(a));
                            }
                            _ => {
                                ui.painter().text(
                                    av.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon::USER_CIRCLE,
                                    theme::regular(20.0),
                                    theme::TEXT_DIM,
                                );
                            }
                        }
                        ui.add_space(8.0);
                        match &client.state().connection {
                            ConnectionStatus::Connecting => {
                                ui.add(egui::Spinner::new().size(12.0)).on_hover_text("Connecting to Steam...");
                            }
                            ConnectionStatus::Failed(why) => {
                                let short = "Can't reach Steam. Retrying...";
                                ui.label(RichText::new(short).color(theme::ERROR_TEXT).small()).on_hover_text(why);
                            }
                            _ => {}
                        }
                        chip.on_hover_text(match &client.state().connection {
                            ConnectionStatus::Online => "Connected to Steam",
                            ConnectionStatus::Connecting => "Connecting to Steam...",
                            ConnectionStatus::Failed(_) => "Not connected to Steam",
                            ConnectionStatus::Offline => "Offline",
                        });
                    }
                    if client.state().refreshing && client.state().connection == ConnectionStatus::Online {
                        ui.add(egui::Spinner::new().size(12.0)).on_hover_text("Loading your games...");
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
                let (can_back, can_fwd) = (self.history.can_back(), self.history.can_forward());
                let back = ui.add_enabled_ui(can_back, |ui| {
                    widgets::icon_button(ui, icon::CARET_LEFT, theme::CONTROL_SMALL, "Back (Alt+Left, mouse 4)")
                });
                if back.inner.clicked() {
                    self.step_history(client, true);
                }
                let fwd = ui.add_enabled_ui(can_fwd, |ui| {
                    widgets::icon_button(ui, icon::CARET_RIGHT, theme::CONTROL_SMALL, "Forward (Alt+Right, mouse 5)")
                });
                if fwd.inner.clicked() {
                    self.step_history(client, false);
                }
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.x = 30.0;
                let account = self.display_name(client).map(|n| n.to_uppercase()).unwrap_or_default();
                let mut active_rect = None;
                for (tab, label) in [
                    (Tab::Store, "STORE"),
                    (Tab::Library, "LIBRARY"),
                    (Tab::Community, "COMMUNITY"),
                    (Tab::Profile, account.as_str()),
                ] {
                    let resp = widgets::text_tab(ui, label, theme::bold(theme::NAV_TAB_FONT), self.tab == tab);
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
                if downloads_button(ui, client) {
                    self.goto(Tab::Downloads, client);
                }
                let bar_w = ui.max_rect().width();
                ui.horizontal_centered(|ui| {
                    ui.style_mut().override_font_id = Some(theme::medium(13.0));
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

/// The centered "Downloads" button. While something downloads it also shows the name, the percentage, the
/// speed and a thin gliding progress bar. Returns true when clicked.
fn downloads_button(ui: &mut Ui, client: &Client) -> bool {
    let bar = ui.max_rect();
    let rect = egui::Rect::from_center_size(bar.center(), egui::vec2(380.0, bar.height()));
    let resp = ui.interact(rect, Id::new("downloads-button"), egui::Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let color = theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, hover);
    let painter = ui.painter();
    match &client.state().downloads.active {
        Some(a) => {
            let shown = anim::glide(ui.ctx(), Id::new(("dl-bar", a.item.request.app.0)), a.fraction(), 4.0);
            let line = format!(
                "{}  {}  {:.0}%  {}",
                icon::DOWNLOAD_SIMPLE,
                a.item.request.name,
                shown * 100.0,
                fmt::speed(a.bytes_per_sec)
            );
            painter.text(
                rect.center() - egui::vec2(0.0, 6.0),
                egui::Align2::CENTER_CENTER,
                line,
                theme::medium(12.0),
                color,
            );
            let track = egui::Rect::from_center_size(rect.center() + egui::vec2(0.0, 9.0), egui::vec2(240.0, 3.0));
            painter.rect_filled(track, theme::RADIUS, theme::BG_TOP);
            let fill = egui::Rect::from_min_size(track.min, egui::vec2(track.width() * shown, track.height()));
            painter.rect_filled(fill, theme::RADIUS, theme::ACCENT);
        }
        None => {
            let text = format!("{}  Downloads", icon::DOWNLOAD_SIMPLE);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::medium(13.0), color);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
