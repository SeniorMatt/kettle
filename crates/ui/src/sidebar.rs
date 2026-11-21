//! Left column of the library: Home, filters, search and the game list.
//! The list is virtualized: only visible rows are laid out.

use crate::dialogs::Overlays;
use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, CollCmd, Command, FAVORITE, HIDDEN};
use sr_model::{ArtKind, Game, InstallState};
use std::collections::HashSet;

/// Which games the list shows (the GAMES drop-down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Installed,
    /// Installed and up to date, so Play works at once.
    Ready,
}

impl Filter {
    fn pass(self, g: &Game) -> bool {
        match self {
            Filter::All => true,
            Filter::Installed => g.install.is_installed(),
            Filter::Ready => matches!(g.install, InstallState::Installed { .. }),
        }
    }

    fn all_label(self) -> &'static str {
        match self {
            Filter::All => "ALL",
            Filter::Installed => "INSTALLED",
            Filter::Ready => "READY TO PLAY",
        }
    }
}

/// One line of the list: a section header or a game.
enum Row {
    Header {
        id: String,
        title: String,
        count: usize,
        collapsed: bool,
        can_open: bool,
        ours: bool,
    },
    /// Games index.
    Game(usize),
    Note(&'static str),
}

/// What `rows` was computed from.
#[derive(PartialEq)]
struct RowsKey {
    library_rev: u64,
    collections_rev: u64,
    search: String,
    filter: Filter,
    collapsed_rev: u64,
}

#[derive(Default)]
pub struct SidebarView {
    search: String,
    filter: Filter,
    /// Section ids folded away (kept in memory only). "hidden" starts folded.
    collapsed: HashSet<String>,
    collapsed_rev: u64,
    collapsed_init: bool,
    rows: Vec<Row>,
    rows_key: Option<RowsKey>,
}

impl SidebarView {
    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays, library_rev: u64) {
        let frame = egui::Frame::new().fill(theme::BG_SIDEBAR);
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.home_row(ui, client);
                self.filter_row(ui);
                self.search_box(ui);
                self.refresh_rows(client, library_rev);
                self.list(ui, client, tex, ov);
            });
    }

    fn home_row(&self, ui: &mut Ui, client: &mut Client) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
        let home = client.state().selected.is_none();
        let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
        let bg =
            if home { theme::BG_ROW_SELECTED } else { theme::BG_SIDEBAR.lerp_to_gamma(theme::BG_ROW_HOVER, hover) };
        ui.painter().rect_filled(rect.shrink2(Vec2::new(8.0, 4.0)), theme::RADIUS, bg);
        let text = format!("{}   Home", icon::HOUSE);
        ui.painter().text(
            Pos2::new(rect.left() + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            theme::medium(15.0),
            theme::TEXT,
        );
        if resp.clicked() {
            client.send(Command::Select(None));
            client.send(Command::Collections(CollCmd::Show(None)));
        }
    }

    fn filter_row(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            // A plain text drop-down: a box shows only on hover or while open.
            ui.scope(|ui| {
                let v = ui.visuals_mut();
                v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                v.widgets.hovered.weak_bg_fill = theme::BG_ROW_HOVER;
                v.widgets.open.weak_bg_fill = theme::MENU_BG;
                let mut text = format!("GAMES {}", icon::CARET_DOWN);
                if self.filter != Filter::All {
                    text = format!("GAMES: {} {}", self.filter.all_label(), icon::CARET_DOWN);
                }
                let text = RichText::new(text).font(theme::bold(12.0)).color(theme::TEXT_DIM);
                widgets::menu_button(ui, text, |ui| {
                    for (f, label) in [
                        (Filter::All, "All games"),
                        (Filter::Installed, "Installed only"),
                        (Filter::Ready, "Ready to play"),
                    ] {
                        let mark = if self.filter == f { "\u{2713}  " } else { "     " };
                        if ui.button(format!("{mark}{label}")).clicked() {
                            self.filter = f;
                            ui.close();
                        }
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    fn search_box(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let edit = egui::TextEdit::singleline(&mut self.search)
                .hint_text(format!("{}  Search", icon::MAGNIFYING_GLASS))
                .font(theme::regular(14.0))
                .desired_width(theme::SIDEBAR_W - 24.0)
                .margin(egui::Margin::symmetric(8, 5));
            ui.add(edit);
        });
        ui.add_space(8.0);
    }

    fn refresh_rows(&mut self, client: &Client, library_rev: u64) {
        if !self.collapsed_init {
            self.collapsed.insert(HIDDEN.to_string());
            self.collapsed_init = true;
        }
        let colls = &client.state().collections;
        let key = RowsKey {
            library_rev,
            collections_rev: colls.rev,
            search: self.search.clone(),
            filter: self.filter,
            collapsed_rev: self.collapsed_rev,
        };
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let needle = sr_model::sort_key(&self.search);
        let narrowed = !needle.is_empty() || self.filter != Filter::All;
        let games = &client.state().games;
        let pass = |g: &Game| self.filter.pass(g) && (needle.is_empty() || g.sort_key.contains(&needle));
        let mut rows = Vec::new();
        let mut section =
            |id: &str, title: String, members: Option<&HashSet<u32>>, hide_hidden: bool, ours: bool, dynamic: bool| {
                let idx: Vec<usize> = games
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| members.is_none_or(|m| m.contains(&g.app_id.0)))
                    .filter(|(_, g)| !hide_hidden || !colls.hidden.contains(&g.app_id.0))
                    .filter(|(_, g)| pass(g))
                    .map(|(i, _)| i)
                    .collect();
                if idx.is_empty() && (narrowed || id == FAVORITE || id == HIDDEN) {
                    return;
                }
                let collapsed = self.collapsed.contains(id);
                let can_open = id != "all";
                rows.push(Row::Header { id: id.to_string(), title, count: idx.len(), collapsed, can_open, ours });
                if collapsed {
                    return;
                }
                if idx.is_empty() && dynamic {
                    rows.push(Row::Note("Dynamic collection: its filter isn't evaluated yet"));
                }
                rows.extend(idx.into_iter().map(Row::Game));
            };
        section(FAVORITE, "FAVORITES".into(), Some(&colls.favorites), true, false, false);
        for s in &colls.sections {
            section(&s.id, s.name.to_uppercase(), Some(&s.apps), true, s.ours, s.dynamic);
        }
        section("all", self.filter.all_label().into(), None, true, false, false);
        section(HIDDEN, "HIDDEN".into(), Some(&colls.hidden), false, false, false);
        self.rows = rows;
        self.rows_key = Some(key);
    }

    fn list(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays) {
        let mut clicked = None;
        let mut toggled = None;
        let mut opened = None;
        // Rows whose right-click menu is open (or was just asked for); the menu needs `client` mutably, so it
        // is drawn after the list, which holds `client` shared.
        let mut menus = Vec::new();
        let mut header_menus = Vec::new();
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            theme::LIST_ROW_H,
            self.rows.len(),
            |ui, range| {
                let state = client.state();
                for row in &self.rows[range] {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), theme::LIST_ROW_H), Sense::click());
                    let g = match row {
                        Row::Header { id, title, count, collapsed, can_open, ours } => {
                            let caret = if *collapsed { icon::CARET_RIGHT } else { icon::CARET_DOWN };
                            let text = format!("{caret}  {title} ({count})");
                            let color = theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, resp.hovered() as u8 as f32);
                            ui.painter().text(
                                Pos2::new(rect.left() + 16.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                text,
                                theme::bold(12.0),
                                color,
                            );
                            if *can_open {
                                let b = Rect::from_center_size(
                                    Pos2::new(rect.right() - 20.0, rect.center().y),
                                    Vec2::splat(20.0),
                                );
                                let r = ui.interact(b, resp.id.with("open"), Sense::click());
                                let c = if r.hovered() { theme::ACCENT } else { theme::TEXT_UNINSTALLED };
                                ui.painter().text(
                                    b.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon::SQUARES_FOUR,
                                    theme::regular(14.0),
                                    c,
                                );
                                if r.on_hover_text("Show in the library").clicked() {
                                    opened = Some(id.clone());
                                } else if resp.clicked() {
                                    toggled = Some(id.clone());
                                }
                            } else if resp.clicked() {
                                toggled = Some(id.clone());
                            }
                            if *ours {
                                header_menus.push((resp, id.clone()));
                            }
                            continue;
                        }
                        Row::Note(text) => {
                            ui.painter().text(
                                Pos2::new(rect.left() + 28.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                text,
                                theme::regular(12.0),
                                theme::TEXT_UNINSTALLED,
                            );
                            continue;
                        }
                        Row::Game(i) => &state.games[*i],
                    };
                    let selected = state.selected == Some(g.app_id);
                    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
                    if selected {
                        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_SELECTED);
                    } else if hover > 0.0 {
                        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_HOVER.gamma_multiply(hover));
                    }
                    let icon_rect =
                        Rect::from_center_size(Pos2::new(rect.left() + 28.0, rect.center().y), Vec2::splat(16.0));
                    match tex.get(client, g.app_id, ArtKind::Icon) {
                        Art::Ready(t, a) => {
                            widgets::image_cover(ui, icon_rect, t, egui::Color32::WHITE.gamma_multiply(a))
                        }
                        _ => {
                            ui.painter().rect_filled(icon_rect, 1.0, theme::BG_CAPSULE);
                        }
                    }
                    let color = if g.install.is_installed() { theme::TEXT } else { theme::TEXT_UNINSTALLED };
                    let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 44.0, rect.top()), rect.right_bottom());
                    let galley = ui.painter().layout(g.name.clone(), theme::medium(14.0), color, f32::INFINITY);
                    ui.painter().with_clip_rect(text_rect).galley(
                        Pos2::new(text_rect.left(), rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    if resp.clicked() {
                        clicked = Some(g.app_id);
                    }
                    if resp.secondary_clicked() || resp.context_menu_opened() {
                        menus.push((resp, g.app_id));
                    }
                }
            },
        );
        for (resp, app) in menus {
            crate::game_menu::context(&resp, client, app, ov);
        }
        for (resp, id) in header_menus {
            widgets::context_menu(&resp, |ui| {
                if ui.button("Delete collection").clicked() {
                    client.send(Command::Collections(CollCmd::Delete(id.clone())));
                    ui.close();
                }
            });
        }
        if let Some(id) = toggled {
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }
            self.collapsed_rev += 1;
        }
        if let Some(id) = opened {
            client.send(Command::Select(None));
            client.send(Command::Collections(CollCmd::Show(Some(id))));
        }
        if let Some(app) = clicked {
            client.send(Command::Select(Some(app)));
        }
    }
}
