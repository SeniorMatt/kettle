//! Left column of the library: Home, filters, search and the game list.
//! The list is virtualized: only visible rows are laid out.

use crate::dialogs::Overlays;
use crate::steam_icons as si;
use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Id, Rect, Sense, Ui, Vec2, pos2, vec2};
use sr_client::{Client, CollCmd, Command, FAVORITE, HIDDEN};
use sr_model::{ArtKind, Game, InstallState};
use std::collections::HashSet;

/// Which games the list shows: checks from the filter button (they combine) and the drop-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Filter {
    installed: bool,
    /// Installed and up to date, so Play works at once.
    ready: bool,
    played: bool,
    unplayed: bool,
}

impl Filter {
    fn pass(self, g: &Game) -> bool {
        (!self.installed || g.install.is_installed())
            && (!self.ready || matches!(g.install, InstallState::Installed { .. }))
            && (!self.played || g.playtime_minutes > 0)
            && (!self.unplayed || g.playtime_minutes == 0)
    }

    fn is_active(self) -> bool {
        self.installed || self.ready || self.played || self.unplayed
    }

    fn all_label(self) -> String {
        let mut parts = Vec::new();
        for (on, name) in [
            (self.installed, "INSTALLED"),
            (self.ready, "READY TO PLAY"),
            (self.played, "PLAYED"),
            (self.unplayed, "UNPLAYED"),
        ] {
            if on {
                parts.push(name);
            }
        }
        if parts.is_empty() { "ALL".to_string() } else { parts.join(" + ") }
    }
}

/// Order of the games inside each section (the sort button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Name,
    NameReversed,
    Recent,
    PlayTime,
}

/// One line of the list: a section header or a game.
enum Row {
    Header {
        id: String,
        title: String,
        count: usize,
        total: usize,
        collapsed: bool,
        can_open: bool,
        ours: bool,
        dynamic: bool,
    },
    /// Games index.
    Game(usize),
}

/// What `rows` was computed from.
#[derive(PartialEq)]
struct RowsKey {
    library_rev: u64,
    collections_rev: u64,
    search: String,
    filter: Filter,
    sort: Sort,
    collapsed_rev: u64,
}

#[derive(Default)]
pub struct SidebarView {
    search: String,
    filter: Filter,
    sort: Sort,
    /// Section ids folded away (kept in memory only). "hidden" starts folded.
    collapsed: HashSet<String>,
    collapsed_rev: u64,
    collapsed_init: bool,
    rows: Vec<Row>,
    rows_key: Option<RowsKey>,
}

/// The lighter top of the sidebar list area, fading into the list colour.
fn side_fade(ui: &Ui) {
    let r = ui.max_rect();
    let top = r.top() + 40.0;
    let bottom = (top + 280.0).min(r.bottom());
    let mut mesh = egui::epaint::Mesh::default();
    for (p, c) in [
        (pos2(r.left(), top), theme::SIDE_BG_TOP),
        (pos2(r.right(), top), theme::SIDE_BG_TOP),
        (pos2(r.right(), bottom), theme::BG_SIDEBAR),
        (pos2(r.left(), bottom), theme::BG_SIDEBAR),
    ] {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
}

/// 14 px sidebar text cut to `max_w` with "..." at the end.
fn ellipsized(ui: &Ui, text: &str, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let layout = |t: &str| ui.painter().layout_no_wrap(t.to_string(), theme::regular(14.0), theme::SIDE_TEXT);
    let full = layout(text);
    if full.size().x <= max_w {
        return full;
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.pop().is_some() {
        let g = layout(&format!("{}...", chars.iter().collect::<String>().trim_end()));
        if g.size().x <= max_w {
            return g;
        }
    }
    full
}

impl SidebarView {
    /// Number of folded sections (for tests).
    #[cfg(test)]
    pub(crate) fn collapsed_count(&self) -> usize {
        self.collapsed.len()
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays, library_rev: u64) {
        let frame = egui::Frame::new().fill(theme::BG_SIDEBAR);
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                side_fade(ui);
                self.home_row(ui, client);
                ui.add_space(10.0);
                self.filter_row(ui);
                ui.add_space(12.0);
                self.search_box(ui);
                self.refresh_rows(client, library_rev);
                self.list(ui, client, tex, ov);
                // The dark 2 px line between the sidebar and the page.
                let r = ui.clip_rect();
                let line = Rect::from_min_max(pos2(r.right() - 2.0, r.top()), r.right_bottom());
                ui.painter().rect_filled(line, 0.0, theme::DIVIDER);
            });
    }

    /// Dark strip with the Home box and the grid button (y 78-117 of the reference).
    fn home_row(&self, ui: &mut Ui, client: &mut Client) {
        let w = ui.available_width();
        let (zone, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
        ui.painter().rect_filled(zone, 0.0, theme::SIDE_HOME_ZONE);
        let home = client.state().selected.is_none() && client.state().collections.selected.is_none();
        let box_rect = Rect::from_min_size(zone.min + vec2(5.0, 4.0), vec2(224.0, 30.0));
        let grid_rect = Rect::from_min_size(zone.min + vec2(233.0, 4.0), vec2(32.0, 30.0));
        let mut go = false;
        for (r, is_grid) in [(box_rect, false), (grid_rect, true)] {
            let resp = ui.interact(r, Id::new(("side-home", is_grid)), Sense::click());
            let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
            let base = if home && !is_grid {
                theme::BG_ROW_SELECTED
            } else if is_grid {
                theme::SIDE_GRID_BG
            } else {
                theme::SIDE_BOX
            };
            ui.painter().rect_filled(r, 2.0, base.lerp_to_gamma(theme::BG_ROW_HOVER, hover));
            if is_grid {
                si::grid(ui.painter(), r.center(), 16.0, theme::SIDE_GRID_ICON.lerp_to_gamma(theme::SIDE_ICON, hover));
            } else {
                ui.painter().text(
                    pos2(r.left() + 8.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Home",
                    theme::regular(14.0),
                    theme::SIDE_TEXT,
                );
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                go = true;
            }
        }
        if go {
            client.send(Command::Select(None));
            client.send(Command::Collections(CollCmd::Show(None)));
        }
    }

    /// "Games and Software" drop-down, sort button and the quick filter icons.
    fn filter_row(&mut self, ui: &mut Ui) {
        let w = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        let ctx = ui.ctx().clone();
        // Drop-down box.
        let dd = Rect::from_min_size(row.min + vec2(5.0, 0.0), vec2(152.0, 28.0));
        let resp = ui.interact(dd, Id::new("side-scope"), Sense::click());
        let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
        ui.painter().rect_filled(dd, 2.0, theme::SIDE_BOX.lerp_to_gamma(theme::BG_ROW_HOVER, t));
        let label = if self.filter.ready {
            "Ready to play"
        } else if self.filter.installed {
            "Installed"
        } else {
            "Games and Software"
        };
        let galley = ellipsized(ui, label, dd.width() - 8.0 - 14.0);
        ui.painter().galley(pos2(dd.left() + 8.0, dd.center().y - galley.size().y / 2.0), galley, theme::SIDE_TEXT);
        si::chevron_down(ui.painter(), pos2(dd.right() - 12.0, dd.center().y), 10.0, theme::SIDE_ICON);
        let f = &mut self.filter;
        widgets::popup_menu(&resp.on_hover_cursor(egui::CursorIcon::PointingHand), |ui| {
            let cur = (f.installed, f.ready);
            for (label, installed, ready) in
                [("Games and Software", false, false), ("Installed", true, false), ("Ready to play", false, true)]
            {
                let mark = if cur == (installed, ready) { "\u{2713}  " } else { "     " };
                if ui.button(format!("{mark}{label}")).clicked() {
                    f.installed = installed;
                    f.ready = ready;
                    ui.close();
                }
            }
        });
        // Sort button.
        let sb = Rect::from_min_size(row.min + vec2(161.0, 0.0), vec2(32.0, 28.0));
        let resp = ui.interact(sb, Id::new("side-sort"), Sense::click());
        let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
        ui.painter().rect_filled(sb, 2.0, theme::SIDE_SORT_BTN.lerp_to_gamma(theme::BG_ROW_HOVER, 0.6 * t));
        si::arrow_up(ui.painter(), sb.center(), 14.0, theme::SIDE_ICON.lerp_to_gamma(egui::Color32::WHITE, t));
        let sort = &mut self.sort;
        widgets::popup_menu(&resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Sort"), |ui| {
            for (s, label) in [
                (Sort::Name, "Alphabetical"),
                (Sort::NameReversed, "Alphabetical, reversed"),
                (Sort::Recent, "Recently played"),
                (Sort::PlayTime, "Play time"),
            ] {
                let mark = if *sort == s { "\u{2713}  " } else { "     " };
                if ui.button(format!("{mark}{label}")).clicked() {
                    *sort = s;
                    ui.close();
                }
            }
        });
        // Quick filters at the right: recently played order and ready to play.
        let c1 = pos2(row.left() + 233.0, row.center().y);
        let c2 = pos2(row.left() + 259.0, row.center().y);
        let recent = self.sort == Sort::Recent;
        let ready = self.filter.ready;
        for (c, on, tip, id) in
            [(c1, recent, "Recently played first", "side-q-recent"), (c2, ready, "Ready to play", "side-q-ready")]
        {
            let resp = ui.interact(Rect::from_center_size(c, vec2(24.0, 24.0)), Id::new(id), Sense::click());
            let t = anim::animate_bool(&ctx, resp.id.with("h"), resp.hovered(), anim::FAST);
            let color = if on { theme::ACCENT } else { theme::SIDE_ICON.lerp_to_gamma(egui::Color32::WHITE, t) };
            if id == "side-q-recent" {
                si::clock(ui.painter(), c, 14.0, color);
            } else {
                si::play_circle(ui.painter(), c, 14.0, color);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                if id == "side-q-recent" {
                    self.sort = if recent { Sort::Name } else { Sort::Recent };
                } else {
                    self.filter.ready = !ready;
                    self.filter.installed = false;
                }
            }
        }
    }

    /// Search field with a magnifier on the left and the filter button on the right.
    fn search_box(&mut self, ui: &mut Ui) {
        let w = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        let b = Rect::from_min_max(row.min + vec2(5.0, 0.0), row.max - vec2(7.0, 0.0));
        ui.painter().rect_filled(b, 2.0, theme::SIDE_SEARCH_BG);
        si::magnifier(ui.painter(), pos2(b.left() + 15.0, b.center().y), 14.0, theme::SIDE_ICON);
        let edit_rect = Rect::from_min_max(pos2(b.left() + 30.0, b.top()), pos2(b.right() - 34.0, b.bottom()));
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.add(
            egui::TextEdit::singleline(&mut self.search)
                .frame(egui::Frame::NONE)
                .font(theme::regular(14.0))
                .desired_width(edit_rect.width())
                .margin(egui::Margin::symmetric(0, 4)),
        );
        // Filter button.
        let fb = Rect::from_center_size(pos2(b.right() - 17.0, b.center().y), vec2(28.0, 26.0));
        let resp = ui.interact(fb, Id::new("side-filter"), Sense::click());
        let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
        let color = if self.filter.is_active() {
            theme::ACCENT
        } else {
            theme::SIDE_ICON.lerp_to_gamma(egui::Color32::WHITE, t)
        };
        si::filter_lines(ui.painter(), fb.center(), 15.0, color);
        let f = &mut self.filter;
        widgets::popup_menu(&resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Filters"), |ui| {
            let item = |ui: &mut Ui, on: bool, label: &str| {
                let mark = if on { "\u{2713}  " } else { "     " };
                let clicked = ui.button(format!("{mark}{label}")).clicked();
                if clicked {
                    ui.close();
                }
                clicked
            };
            if item(ui, f.installed, "Installed") {
                f.installed = !f.installed;
            }
            if item(ui, f.ready, "Ready to play") {
                f.ready = !f.ready;
            }
            widgets::menu_separator(ui);
            if item(ui, f.played, "Played") {
                f.played = !f.played;
                f.unplayed = false;
            }
            if item(ui, f.unplayed, "Unplayed") {
                f.unplayed = !f.unplayed;
                f.played = false;
            }
            if f.is_active() {
                widgets::menu_separator(ui);
                if ui.button("     Clear filters").clicked() {
                    *f = Filter::default();
                    ui.close();
                }
            }
        });
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
            sort: self.sort,
            collapsed_rev: self.collapsed_rev,
        };
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let needle = sr_model::sort_key(&self.search);
        let narrowed = !needle.is_empty() || self.filter.is_active();
        let games = &client.state().games;
        let pass = |g: &Game| self.filter.pass(g) && (needle.is_empty() || g.sort_key.contains(&needle));
        let sort = self.sort;
        let mut rows = Vec::new();
        let mut section =
            |id: &str, title: String, members: Option<&HashSet<u32>>, hide_hidden: bool, ours: bool, dynamic: bool| {
                let in_section = |g: &Game| {
                    members.is_none_or(|m| m.contains(&g.app_id.0))
                        && (!hide_hidden || !colls.hidden.contains(&g.app_id.0))
                };
                let total = games.iter().filter(|g| in_section(g)).count();
                let mut idx: Vec<usize> =
                    games.iter().enumerate().filter(|(_, g)| in_section(g) && pass(g)).map(|(i, _)| i).collect();
                match sort {
                    Sort::Name => {}
                    Sort::NameReversed => idx.reverse(),
                    Sort::Recent => idx.sort_by_key(|&i| std::cmp::Reverse(games[i].last_played)),
                    Sort::PlayTime => idx.sort_by_key(|&i| std::cmp::Reverse(games[i].playtime_minutes)),
                }
                if idx.is_empty() && (narrowed || id == FAVORITE || id == HIDDEN) {
                    return;
                }
                let collapsed = self.collapsed.contains(id);
                let can_open = id != "all";
                rows.push(Row::Header {
                    id: id.to_string(),
                    title,
                    count: idx.len(),
                    total,
                    collapsed,
                    can_open,
                    ours,
                    dynamic,
                });
                if !collapsed {
                    rows.extend(idx.into_iter().map(Row::Game));
                }
            };
        section(FAVORITE, "FAVORITES".into(), Some(&colls.favorites), true, false, false);
        for s in &colls.sections {
            section(&s.id, s.name.to_uppercase(), Some(&s.apps), true, s.ours, s.dynamic);
        }
        section("all", self.filter.all_label(), None, true, false, false);
        section(HIDDEN, "HIDDEN".into(), Some(&colls.hidden), false, false, false);
        self.rows = rows;
        self.rows_key = Some(key);
    }

    fn list(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays) {
        let mut clicked = None;
        let mut dbl = None;
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
                        ui.allocate_exact_size(vec2(ui.available_width(), theme::LIST_ROW_H), Sense::click());
                    let g = match row {
                        Row::Header { id, title, count, total, collapsed, can_open, ours, dynamic } => {
                            // Steam's header: a small square at the left folds the section, the rest of the
                            // row opens the collection in the main area.
                            let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
                            let color = theme::HEADER_TEXT.lerp_to_gamma(egui::Color32::WHITE, 0.4 * hover);
                            let cy = rect.center().y;
                            let box_rect = Rect::from_center_size(pos2(rect.left() + 14.0, cy), Vec2::splat(13.0));
                            let toggle = ui.interact(box_rect.expand(3.0), resp.id.with("fold"), Sense::click());
                            let th =
                                anim::animate_bool(ui.ctx(), resp.id.with("fold-hover"), toggle.hovered(), anim::FAST);
                            let line = theme::SIDE_ICON.lerp_to_gamma(egui::Color32::WHITE, 0.6 * th);
                            ui.painter().rect_filled(rect, 0.0, theme::HEADER_BG);
                            let p = ui.painter();
                            p.rect_stroke(box_rect, 1.0, egui::Stroke::new(1.0, line), egui::StrokeKind::Inside);
                            let dash = box_rect.center();
                            p.line_segment(
                                [dash - vec2(3.0, 0.0), dash + vec2(3.0, 0.0)],
                                egui::Stroke::new(1.0, line),
                            );
                            if *collapsed {
                                p.line_segment(
                                    [dash - vec2(0.0, 3.0), dash + vec2(0.0, 3.0)],
                                    egui::Stroke::new(1.0, line),
                                );
                            }
                            let title_g = p.layout_no_wrap(title.clone(), theme::regular(13.0), color);
                            let tx = rect.left() + 30.0;
                            let tw = title_g.size().x;
                            p.galley(pos2(tx, cy - title_g.size().y / 2.0), title_g, color);
                            let count_text =
                                if count == total { format!("({count})") } else { format!("({count}/{total})") };
                            let cg = p.layout_no_wrap(count_text, theme::regular(13.0), theme::HEADER_COUNT);
                            let cw = cg.size().x;
                            p.galley(pos2(tx + tw + 6.0, cy - cg.size().y / 2.0), cg, theme::HEADER_COUNT);
                            if *dynamic {
                                si::bolt(p, pos2(tx + tw + 6.0 + cw + 14.0, cy), 12.0, theme::HEADER_COUNT);
                            }
                            if toggle.clicked() {
                                toggled = Some(id.clone());
                            } else if resp.clicked() {
                                opened = Some(if *can_open { Some(id.clone()) } else { None });
                            }
                            if *ours {
                                header_menus.push((resp, id.clone()));
                            }
                            continue;
                        }
                        Row::Game(i) => &state.games[*i],
                    };
                    let selected = state.selected == Some(g.app_id);
                    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
                    let band = Rect::from_min_max(
                        pos2(rect.left(), rect.top() + 1.0),
                        pos2(rect.right() - 12.0, rect.bottom() - 1.0),
                    );
                    if selected {
                        ui.painter().rect_filled(band, 0.0, theme::BG_ROW_SELECTED);
                    } else if hover > 0.0 {
                        ui.painter().rect_filled(band, 0.0, theme::BG_ROW_HOVER.gamma_multiply(hover));
                    }
                    let icons = client.state().settings.interface.sidebar_icons;
                    let running = state.running.contains_key(&g.app_id);
                    if running {
                        // Steam's little green figure for a running game.
                        let c = pos2(rect.left() + 12.0, rect.center().y);
                        ui.painter().circle_filled(c - vec2(0.0, 4.0), 2.3, theme::RUNNING);
                        ui.painter().rect_filled(
                            Rect::from_center_size(c + vec2(0.0, 2.5), vec2(6.0, 7.0)),
                            2.0,
                            theme::RUNNING,
                        );
                    }
                    let icon_rect =
                        Rect::from_center_size(pos2(rect.left() + 34.0, rect.center().y), Vec2::splat(20.0));
                    if icons {
                        match tex.get(client, g.app_id, ArtKind::Icon) {
                            Art::Ready(t, a) => {
                                widgets::image_cover(ui, icon_rect, t, egui::Color32::WHITE.gamma_multiply(a))
                            }
                            _ => {
                                ui.painter().rect_filled(icon_rect, 1.0, theme::BG_CAPSULE);
                            }
                        }
                    }
                    let updating =
                        matches!(g.install, InstallState::Downloading { .. } | InstallState::UpdateRequired { .. });
                    let color = if updating {
                        theme::ACCENT
                    } else if selected || g.install.is_installed() {
                        theme::ROW_TEXT
                    } else {
                        theme::TEXT_UNINSTALLED
                    };
                    let label = if updating { format!("{} - Updating", g.name) } else { g.name.clone() };
                    let text_rect = Rect::from_min_max(
                        pos2(rect.left() + if icons { 50.0 } else { 26.0 }, rect.top()),
                        pos2(rect.right() - 12.0, rect.bottom()),
                    );
                    let galley = ui.painter().layout(label, theme::regular(theme::SIDE_ROW_FONT), color, f32::INFINITY);
                    ui.painter().with_clip_rect(text_rect).galley(
                        pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    if resp.clicked() {
                        clicked = Some(g.app_id);
                    }
                    if resp.double_clicked() || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        dbl = Some(g.app_id);
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
            client.send(Command::Collections(CollCmd::Show(id)));
        }
        if let Some(app) = clicked {
            client.send(Command::Select(Some(app)));
        }
        if let Some(app) = dbl {
            crate::game_page::activate(client, ov, app);
        }
    }
}
