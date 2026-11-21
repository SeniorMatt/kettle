//! The menu bar (Kettle, View, Friends, Games, Help) with the behaviour of the official client's:
//! a click opens a menu; while one is open, moving over another title switches to it at once; a click outside,
//! Escape or choosing an item closes it; Left/Right switch menus. egui's own bar does not switch on hover, so
//! this is a small widget of ours. The bar only returns what was chosen ([`Act`]); `AppUi` does it.

use crate::{theme, widgets};
use egui::{Context, Id, Key, Modifiers, Sense, Ui, Vec2};
use sr_client::{Client, PersonaState};

const RELEASES: &str = "https://github.com/SeniorMatt/kettle/releases";
const PRIVACY: &str = "https://github.com/SeniorMatt/kettle/blob/main/PRIVACY.md";
const REDEEM_WALLET: &str = "https://store.steampowered.com/account/redeemwalletcode";
const ACTIVATE: &str = "https://store.steampowered.com/account/registerkey";

/// What a menu item asks for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Act {
    Settings,
    SignOut,
    Quit,
    Url(&'static str),
    GoLibrary,
    GoStore,
    GoDownloads,
    ToggleFriends,
    AddFriend,
    ViewMyProfile,
    EditMyProfile,
    ShowHidden,
    RefreshLibrary,
    /// `+1` zoom in, `-1` zoom out, `0` reset.
    Zoom(i32),
    SetStatus(PersonaState),
    NewCollection,
    ToggleOverlay,
    SystemInfo,
    About,
}

const TITLES: [&str; 5] = [crate::theme::APP_NAME, "View", "Friends", "Games", "Help"];

/// Which menu is open (index into the titles).
#[derive(Default, Clone, Copy)]
pub(crate) struct BarState {
    pub open: Option<usize>,
}

fn item(ui: &mut Ui, label: &str) -> bool {
    ui.button(label).clicked()
}

fn item_sc(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
}

/// One title in the bar and, while it is the open one, its menu.
fn title(ui: &mut Ui, open: &mut Option<usize>, idx: usize, content: impl FnOnce(&mut Ui)) {
    let galley = ui.painter().layout_no_wrap(TITLES[idx].to_string(), theme::regular(13.0), theme::TEXT_HEADING);
    let size = Vec2::new(galley.size().x + 24.0, theme::MENU_BAR_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let is_open = *open == Some(idx);
    if resp.clicked() {
        *open = if is_open { None } else { Some(idx) };
    } else if resp.hovered() && open.is_some() && !is_open {
        // The point of this widget: a menu is open, so moving over another title switches to it.
        *open = Some(idx);
    }
    let is_open = *open == Some(idx);
    if is_open {
        ui.painter().rect_filled(rect, 0.0, theme::MENU_BG);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_SELECTED);
    }
    let color = if is_open || resp.hovered() { theme::TEXT } else { theme::TEXT_HEADING };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    if !is_open {
        return;
    }
    let popup = egui::Popup::menu(&resp)
        .open(true)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .style(theme::menu_style)
        .gap(0.0)
        .show(|ui| {
            ui.set_min_width(theme::MENU_MIN_W);
            content(ui);
        });
    let Some(popup) = popup else { return };
    let (esc, pressed, released, pos) = ui.input(|i| {
        (i.key_pressed(Key::Escape), i.pointer.any_pressed(), i.pointer.primary_released(), i.pointer.interact_pos())
    });
    let inside = pos.is_some_and(|p| popup.response.rect.contains(p));
    if esc || (pressed && !inside && !resp.hovered()) || (released && inside) {
        *open = None;
    }
}

/// Draw the five titles and the open menu. Returns the chosen action, if any.
pub(crate) fn bar(ui: &mut Ui, state: &mut BarState, client: &Client) -> Option<Act> {
    let mut act: Option<Act> = None;
    let mut open = state.open;
    ui.spacing_mut().item_spacing.x = 0.0;
    let overlay_possible = !client.state().running.is_empty() && client.state().settings.overlay;
    let friends_open = client.state().friends.open;

    title(ui, &mut open, 0, |ui| {
        if item(ui, "Settings") {
            act = Some(Act::Settings);
        }
        if item(ui, "Check for Kettle updates") {
            act = Some(Act::Url(RELEASES));
        }
        widgets::menu_separator(ui);
        // There is no "go offline" command in the client yet, so it is not listed.
        if item(ui, "Sign out") {
            act = Some(Act::SignOut);
        }
        if item(ui, "Exit") {
            act = Some(Act::Quit);
        }
    });
    title(ui, &mut open, 1, |ui| {
        if item(ui, "Library") {
            act = Some(Act::GoLibrary);
        }
        if item(ui, "Store") {
            act = Some(Act::GoStore);
        }
        if item(ui, "Downloads") {
            act = Some(Act::GoDownloads);
        }
        if item(ui, if friends_open { "Hide friends & chat" } else { "Show friends & chat" }) {
            act = Some(Act::ToggleFriends);
        }
        widgets::menu_separator(ui);
        if item(ui, "Show hidden games") {
            act = Some(Act::ShowHidden);
        }
        if item(ui, "Refresh library") {
            act = Some(Act::RefreshLibrary);
        }
        widgets::menu_separator(ui);
        if item_sc(ui, "Zoom in", "Ctrl +") {
            act = Some(Act::Zoom(1));
        }
        if item_sc(ui, "Zoom out", "Ctrl -") {
            act = Some(Act::Zoom(-1));
        }
        if item_sc(ui, "Reset zoom", "Ctrl 0") {
            act = Some(Act::Zoom(0));
        }
    });
    title(ui, &mut open, 2, |ui| {
        if item(ui, "View friends list") {
            act = Some(Act::ToggleFriends);
        }
        if item(ui, "Add a friend...") {
            act = Some(Act::AddFriend);
        }
        if item(ui, "View my profile") {
            act = Some(Act::ViewMyProfile);
        }
        if item(ui, "Edit my profile") {
            act = Some(Act::EditMyProfile);
        }
        widgets::menu_separator(ui);
        for (label, state) in [
            ("Online", PersonaState::Online),
            ("Away", PersonaState::Away),
            ("Invisible", PersonaState::Invisible),
            ("Offline", PersonaState::Offline),
        ] {
            if item(ui, &format!("Status: {label}")) {
                act = Some(Act::SetStatus(state));
            }
        }
    });
    title(ui, &mut open, 3, |ui| {
        if item(ui, "View game library") {
            act = Some(Act::GoLibrary);
        }
        if item(ui, "New collection...") {
            act = Some(Act::NewCollection);
        }
        widgets::menu_separator(ui);
        if item(ui, "Redeem a Steam wallet code...") {
            act = Some(Act::Url(REDEEM_WALLET));
        }
        if item(ui, "Activate a product on Steam...") {
            act = Some(Act::Url(ACTIVATE));
        }
        if overlay_possible {
            widgets::menu_separator(ui);
            if item_sc(ui, "Show overlay", "Shift+Tab") {
                act = Some(Act::ToggleOverlay);
            }
        }
    });
    title(ui, &mut open, 4, |ui| {
        if item(ui, "Kettle on GitHub") {
            act = Some(Act::Url(RELEASES));
        }
        if item(ui, "Privacy") {
            act = Some(Act::Url(PRIVACY));
        }
        widgets::menu_separator(ui);
        if item(ui, "System information") {
            act = Some(Act::SystemInfo);
        }
        if item(ui, "About Kettle") {
            act = Some(Act::About);
        }
    });

    // Keyboard: Left/Right switch menus, F10 opens the first one.
    if let Some(i) = open {
        let n = TITLES.len();
        let (left, right) = ui.input_mut(|inp| {
            (inp.consume_key(Modifiers::NONE, Key::ArrowLeft), inp.consume_key(Modifiers::NONE, Key::ArrowRight))
        });
        if left {
            open = Some((i + n - 1) % n);
        } else if right {
            open = Some((i + 1) % n);
        }
    } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F10)) {
        open = Some(0);
    }
    if act.is_some() {
        open = None;
    }
    state.open = open;
    act
}

/// The two small windows of the Help menu.
#[derive(Default)]
pub(crate) struct HelpDialogs {
    about: bool,
    sysinfo: Option<String>,
}

impl HelpDialogs {
    pub fn open_about(&mut self) {
        self.about = true;
    }

    pub fn open_sysinfo(&mut self, text: String) {
        self.sysinfo = Some(text);
    }

    pub fn show(&mut self, ctx: &Context) {
        if self.about {
            let r = modal("about-dialog").show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(egui::RichText::new(theme::APP_NAME).font(theme::bold(22.0)).color(theme::TEXT));
                ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(theme::TEXT_DIM));
                ui.add_space(10.0);
                ui.label("An unofficial, native Steam client. Not affiliated with Valve.");
                ui.add_space(6.0);
                ui.label(egui::RichText::new("License: MIT OR Apache-2.0").color(theme::TEXT_DIM));
                ui.add_space(14.0);
                widgets::button(ui, "Close", widgets::ButtonStyle::Primary, true).clicked()
            });
            if r.inner || r.should_close() {
                self.about = false;
            }
        }
        if let Some(text) = self.sysinfo.clone() {
            let r = modal("sysinfo-dialog").show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(egui::RichText::new("System information").font(theme::bold(18.0)).color(theme::TEXT));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&text).font(theme::regular(13.0)).color(theme::TEXT_DIM));
                ui.add_space(14.0);
                let mut done = false;
                ui.horizontal(|ui| {
                    if widgets::button(ui, "Copy", widgets::ButtonStyle::Secondary, true).clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    done = widgets::button(ui, "Close", widgets::ButtonStyle::Primary, true).clicked();
                });
                done
            });
            if r.inner || r.should_close() {
                self.sysinfo = None;
            }
        }
    }
}

fn modal(id: &str) -> egui::Modal {
    let frame = egui::Frame::new()
        .fill(theme::BG_SIDEBAR)
        .inner_margin(egui::Margin::same(20))
        .corner_radius(theme::BUTTON_RADIUS)
        .stroke(egui::Stroke::new(1.0, theme::WINDOW_BORDER));
    egui::Modal::new(Id::new(id)).frame(frame)
}
