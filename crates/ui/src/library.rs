//! Library home: the "ALL GAMES" grid of portrait covers, virtualized by row.

use crate::dialogs::Overlays;
use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Color32, Pos2, Rect, RichText, Sense, Ui, Vec2};
use sr_client::{Client, Command};
use sr_model::{ArtKind, Game};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Alphabetical,
    LastPlayed,
    HoursPlayed,
}

impl Sort {
    const ALL: [Sort; 3] = [Sort::Alphabetical, Sort::LastPlayed, Sort::HoursPlayed];

    fn label(self) -> &'static str {
        match self {
            Sort::Alphabetical => "Alphabetical",
            Sort::LastPlayed => "Last Played",
            Sort::HoursPlayed => "Hours Played",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    All,
    Installed,
    NotInstalled,
}

impl Show {
    const ALL: [Show; 3] = [Show::All, Show::Installed, Show::NotInstalled];

    fn label(self) -> &'static str {
        match self {
            Show::All => "All",
            Show::Installed => "Installed",
            Show::NotInstalled => "Not installed",
        }
    }

    fn pass(self, g: &Game) -> bool {
        match self {
            Show::All => true,
            Show::Installed => g.install.is_installed(),
            Show::NotInstalled => !g.install.is_installed(),
        }
    }
}

/// What `order` was computed from: library revision, collections revision, shown list, sort, filter.
type OrderKey = (u64, u64, Option<String>, Sort, Show);

#[derive(Default)]
pub struct LibraryView {
    sort: Sort,
    show: Show,
    /// Indices into `state.games` in display order.
    order: Vec<usize>,
    order_key: Option<OrderKey>,
}

impl LibraryView {
    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays) {
        self.refresh_order(client);
        let pad = theme::PAGE_PAD;
        ui.add_space(pad);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let n = self.order.len();
            let colls = &client.state().collections;
            let title = colls
                .selected
                .as_deref()
                .and_then(|id| colls.title(id))
                .map_or("ALL GAMES".to_string(), str::to_uppercase);
            ui.label(RichText::new(format!("{title} ({n})")).font(theme::bold(14.0)).color(theme::TEXT_HEADING));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad);
                widgets::combo(ui, "sort", 140.0, self.sort.label(), |ui| {
                    for s in Sort::ALL {
                        ui.selectable_value(&mut self.sort, s, s.label());
                    }
                });
                ui.label(RichText::new("SORT BY").font(theme::bold(12.0)).color(theme::TEXT_DIM));
                ui.add_space(16.0);
                widgets::combo(ui, "show", 120.0, self.show.label(), |ui| {
                    for f in Show::ALL {
                        ui.selectable_value(&mut self.show, f, f.label());
                    }
                });
                ui.label(RichText::new("SHOW").font(theme::bold(12.0)).color(theme::TEXT_DIM));
            });
        });
        ui.add_space(12.0);

        if self.order.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if client.state().refreshing {
                    "Loading your library..."
                } else if client.state().games.is_empty() {
                    "No games yet."
                } else {
                    "No games here."
                };
                ui.label(RichText::new(msg).color(theme::TEXT_DIM));
            });
            return;
        }

        let cell = Vec2::new(theme::CAPSULE_W + theme::GRID_GAP, theme::CAPSULE_H + theme::GRID_GAP);
        let usable = ui.available_width() - 2.0 * pad;
        let cols = ((usable + theme::GRID_GAP) / cell.x).floor().max(1.0) as usize;
        let pitch = column_pitch(usable, cols);
        // With a single column the cover is centered instead.
        let left = pad + if cols == 1 { (usable - theme::CAPSULE_W) / 2.0 } else { 0.0 };
        let rows = self.order.len().div_ceil(cols);
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, cell.y, rows, |ui, range| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for row in range {
                let (row_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), cell.y), Sense::hover());
                for col in 0..cols {
                    let Some(&gi) = self.order.get(row * cols + col) else { break };
                    let min = Pos2::new(row_rect.left() + left + col as f32 * pitch, row_rect.top());
                    let rect = Rect::from_min_size(min, Vec2::new(theme::CAPSULE_W, theme::CAPSULE_H));
                    let game = &client.state().games[gi];
                    let id = ui.id().with(("capsule", row * cols + col));
                    let app = game.app_id;
                    let resp = capsule(ui, client, tex, rect, game, id);
                    if resp.clicked() {
                        clicked = Some(app);
                    }
                    crate::game_menu::context(&resp, client, app, ov);
                }
            }
        });
        if let Some(app) = clicked {
            client.send(Command::Select(Some(app)));
        }
    }

    fn refresh_order(&mut self, client: &Client) {
        let colls = &client.state().collections;
        let key: OrderKey = (client.state().library_rev, colls.rev, colls.selected.clone(), self.sort, self.show);
        if self.order_key.as_ref() == Some(&key) {
            return;
        }
        let games = &client.state().games;
        // A collection shows its members; all games leaves out the hidden ones.
        let members = colls.selected.as_deref().and_then(|id| colls.members(id));
        let mut order: Vec<usize> = (0..games.len())
            .filter(|&i| {
                let app = games[i].app_id.0;
                self.show.pass(&games[i])
                    && match members {
                        Some(m) => m.contains(&app),
                        None => !colls.hidden.contains(&app),
                    }
            })
            .collect();
        match self.sort {
            Sort::Alphabetical => {} // `games` is already sorted by name, and filtering keeps that order.
            Sort::LastPlayed => order.sort_by_key(|&i| std::cmp::Reverse(games[i].last_played)),
            Sort::HoursPlayed => order.sort_by_key(|&i| std::cmp::Reverse(games[i].playtime_minutes)),
        }
        self.order = order;
        self.order_key = Some(key);
    }
}

/// Distance between the left edges of neighbouring covers: the leftover width
/// is shared out between the columns so the grid fills the page like Steam's.
fn column_pitch(usable: f32, cols: usize) -> f32 {
    let base = theme::CAPSULE_W + theme::GRID_GAP;
    if cols < 2 {
        return base;
    }
    let leftover = usable - (cols as f32 * theme::CAPSULE_W + (cols - 1) as f32 * theme::GRID_GAP);
    base + leftover.max(0.0) / (cols - 1) as f32
}

/// One cover. Lifts and brightens on hover, with a soft shadow.
fn capsule(ui: &mut Ui, client: &Client, tex: &mut Textures, rect: Rect, game: &Game, id: egui::Id) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let t = anim::animate_bool(ui.ctx(), id.with("hover"), resp.hovered(), anim::FAST);
    let rect = rect.expand(4.0 * t).translate(Vec2::new(0.0, -3.0 * t));
    if t > 0.0 {
        let shadow = egui::epaint::Shadow {
            offset: [0, (6.0 * t) as i8],
            blur: (14.0 * t) as u8,
            spread: 0,
            color: Color32::from_black_alpha((110.0 * t) as u8),
        };
        ui.painter().add(shadow.as_shape(rect, theme::RADIUS));
    }
    let painter = ui.painter();
    match tex.get(client, game.app_id, ArtKind::Capsule) {
        Art::Ready(texture, alpha) => {
            let tint = Color32::from_gray((215.0 + 40.0 * t) as u8).gamma_multiply(alpha);
            widgets::image_cover(ui, rect, texture, tint);
        }
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            // No portrait art: fall back to the landscape header, centered.
            painter.rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
            let header = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), rect.width() * 215.0 / 460.0));
            match tex.get(client, game.app_id, ArtKind::Header) {
                Art::Ready(h, alpha) => widgets::image_cover(ui, header, h, Color32::WHITE.gamma_multiply(alpha)),
                _ => {
                    let text_rect = rect.shrink(10.0);
                    let galley =
                        ui.painter().layout(game.name.clone(), theme::medium(15.0), theme::TEXT, text_rect.width());
                    ui.painter().galley(text_rect.left_top(), galley, theme::TEXT);
                }
            }
        }
    }
    if t > 0.0 {
        ui.painter().rect_stroke(
            rect,
            theme::RADIUS,
            (1.0, Color32::from_white_alpha((60.0 * t) as u8)),
            egui::StrokeKind::Outside,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&game.name)
}
