//! Library home: the "ALL GAMES" grid of portrait covers, virtualized by row.

use crate::dialogs::Overlays;
use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Color32, Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, CollCmd, Command};
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
    /// Cover width chosen with the size slider (0 = not set yet, use the default).
    cover_w: f32,
    /// Indices into `state.games` in display order.
    order: Vec<usize>,
    order_key: Option<OrderKey>,
    /// The most recently played games (indices into `state.games`), newest first.
    recent: Vec<usize>,
}

/// How many games the "Recent games" shelf can hold.
const RECENT_MAX: usize = 12;

impl LibraryView {
    fn cover(&self) -> Vec2 {
        let w = if self.cover_w <= 0.0 { theme::CAPSULE_W } else { self.cover_w };
        Vec2::new(w, (w * 1.5).round())
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, ov: &mut Overlays) {
        self.refresh_order(client);
        // The soft blue glow behind the top of the page.
        let page = ui.max_rect();
        widgets::vgradient(
            ui,
            Rect::from_min_size(page.min, Vec2::new(page.width(), 340.0_f32.min(page.height()))),
            theme::HOME_GLOW,
            theme::BG_MAIN,
        );
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

        let pad = theme::PAGE_PAD;
        let cover = self.cover();
        let gap = theme::GRID_GAP;
        let usable = (ui.available_width() - 2.0 * pad).max(cover.x);
        let cols = ((usable + gap) / (cover.x + gap)).floor().max(1.0) as usize;
        let rows = self.order.len().div_ceil(cols);
        let row_h = cover.y + gap;
        let in_all = client.state().collections.selected.is_none();
        let shelf_h = if in_all && !self.recent.is_empty() {
            pad + theme::SHELF_TITLE_H + theme::SHELF_BUCKET_H + theme::RECENT_HERO_H + 28.0
        } else {
            0.0
        };
        let top_h = shelf_h + pad.min(12.0) + theme::ALL_BAR_H;
        let mut clicked = None;
        let mut pick: Option<Option<String>> = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let total = top_h + rows as f32 * row_h + pad;
            let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), total), Sense::hover());
            if shelf_h > 0.0 {
                self.shelf(ui, client, tex, ov, full.min, full.width(), &mut clicked);
            }
            // The "ALL GAMES" bar: collection drop-down on the left, sort and cover size on the right.
            let bar = Rect::from_min_size(
                Pos2::new(full.left() + pad, full.top() + shelf_h + pad.min(12.0)),
                Vec2::new(full.width() - 2.0 * pad, theme::ALL_BAR_H),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                ui.horizontal_centered(|ui| self.all_bar(ui, client, &mut pick));
            });
            // Only the rows in view are laid out.
            let first = ((viewport.top() - top_h) / row_h).floor().max(0.0) as usize;
            let last = (((viewport.bottom() - top_h) / row_h).ceil().max(0.0) as usize).min(rows);
            let left = full.left() + pad;
            // Spread the leftover width between the columns so the grid fills the page.
            let pitch = if cols < 2 {
                cover.x + gap
            } else {
                let leftover = usable - (cols as f32 * cover.x + (cols - 1) as f32 * gap);
                cover.x + gap + leftover.max(0.0) / (cols - 1) as f32
            };
            let left = if cols == 1 { left + (usable - cover.x) / 2.0 } else { left };
            for row in first..last {
                let y = full.top() + top_h + row as f32 * row_h;
                for col in 0..cols {
                    let Some(&gi) = self.order.get(row * cols + col) else { break };
                    let rect = Rect::from_min_size(Pos2::new(left + col as f32 * pitch, y), cover);
                    let game = &client.state().games[gi];
                    let id = ui.id().with(("capsule", row * cols + col));
                    let app = game.app_id;
                    let resp = capsule(ui, client, tex, rect, game, id, ArtKind::Capsule);
                    if resp.clicked() {
                        clicked = Some(app);
                    }
                    crate::game_menu::context(&resp, client, app, ov);
                }
            }
        });
        if let Some(sel) = pick {
            client.send(Command::Collections(CollCmd::Show(sel)));
        }
        if let Some(app) = clicked {
            client.send(Command::Select(Some(app)));
        }
    }

    /// "Recent games": time-bucket headers above landscape capsules, the first one larger.
    #[allow(clippy::too_many_arguments)]
    fn shelf(
        &self,
        ui: &mut Ui,
        client: &mut Client,
        tex: &mut Textures,
        ov: &mut Overlays,
        origin: Pos2,
        width: f32,
        clicked: &mut Option<sr_model::AppId>,
    ) {
        let pad = theme::PAGE_PAD;
        let mut y = origin.y + pad;
        ui.painter().text(
            Pos2::new(origin.x + pad, y + theme::SHELF_TITLE_H / 2.0),
            egui::Align2::LEFT_CENTER,
            "Recent games",
            theme::bold(15.0),
            theme::TEXT,
        );
        y += theme::SHELF_TITLE_H;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut x = origin.x + pad;
        let mut last_label = String::new();
        for (i, &gi) in self.recent.iter().enumerate() {
            let (w, h) =
                if i == 0 { (theme::RECENT_HERO_W, theme::RECENT_HERO_H) } else { (theme::RECENT_W, theme::RECENT_H) };
            if x + w > origin.x + width - pad {
                break;
            }
            let label = bucket(now, client.state().games[gi].last_played);
            if label != last_label {
                ui.painter().text(
                    Pos2::new(x, y + theme::SHELF_BUCKET_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    &label,
                    theme::bold(12.0),
                    theme::TEXT_DIM,
                );
                last_label = label;
            }
            let rect = Rect::from_min_size(Pos2::new(x, y + theme::SHELF_BUCKET_H), Vec2::new(w, h));
            let game = &client.state().games[gi];
            let app = game.app_id;
            let id = ui.id().with(("recent", gi));
            let resp = capsule(ui, client, tex, rect, game, id, ArtKind::Header);
            if resp.clicked() {
                *clicked = Some(app);
            }
            crate::game_menu::context(&resp, client, app, ov);
            x += w + theme::GRID_GAP;
        }
    }

    fn all_bar(&mut self, ui: &mut Ui, client: &Client, pick: &mut Option<Option<String>>) {
        let colls = &client.state().collections;
        let title =
            colls.selected.as_deref().and_then(|id| colls.title(id)).map_or("ALL GAMES".to_string(), str::to_uppercase);
        let text = RichText::new(format!("{title} ({})  {}", self.order.len(), icon::CARET_DOWN))
            .font(theme::bold(14.0))
            .color(theme::TEXT_HEADING);
        ui.scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.weak_bg_fill = theme::BG_ROW_HOVER;
            widgets::menu_button(ui, text, |ui| {
                if ui.button("All games").clicked() {
                    *pick = Some(None);
                    ui.close();
                }
                for s in &colls.sections {
                    if ui.button(&s.name).clicked() {
                        *pick = Some(Some(s.id.clone()));
                        ui.close();
                    }
                }
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Narrow window (friends panel open): drop the slider, then the SHOW filter,
            // instead of letting the controls run over the title.
            let room = ui.available_width();
            if room > 540.0 {
                let mut w = if self.cover_w <= 0.0 { theme::CAPSULE_W } else { self.cover_w };
                ui.spacing_mut().slider_width = 110.0;
                if ui
                    .add(egui::Slider::new(&mut w, theme::CAPSULE_W_MIN..=theme::CAPSULE_W_MAX).show_value(false))
                    .on_hover_text("Cover size")
                    .changed()
                {
                    self.cover_w = w;
                }
                ui.add_space(16.0);
            }
            bar_combo(ui, "sort", "SORT BY", self.sort.label(), |ui| {
                for s in Sort::ALL {
                    ui.selectable_value(&mut self.sort, s, s.label());
                }
            });
            if room > 400.0 {
                ui.add_space(12.0);
                bar_combo(ui, "show", "SHOW", self.show.label(), |ui| {
                    for f in Show::ALL {
                        ui.selectable_value(&mut self.show, f, f.label());
                    }
                });
            }
        });
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
        let mut recent: Vec<usize> = (0..games.len())
            .filter(|&i| games[i].last_played > 0 && !colls.hidden.contains(&games[i].app_id.0))
            .collect();
        recent.sort_by_key(|&i| std::cmp::Reverse(games[i].last_played));
        recent.truncate(RECENT_MAX);
        self.recent = recent;
        self.order = order;
        self.order_key = Some(key);
    }
}

/// The shelf's time label for a last-played time: Today, Yesterday, This week, Last week, a month name, Older.
fn bucket(now: u64, t: u64) -> String {
    let days = now.saturating_sub(t) / 86_400;
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => "This week".into(),
        7..=13 => "Last week".into(),
        14..=365 => {
            // Civil month from days since 1970-01-01.
            let z = (t / 86_400) as i64 + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            const NAMES: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            NAMES[(m - 1).clamp(0, 11) as usize].to_string()
        }
        _ => "Older".into(),
    }
}

/// One cover. Lifts and brightens on hover, with a soft shadow.
fn capsule(
    ui: &mut Ui,
    client: &Client,
    tex: &mut Textures,
    rect: Rect,
    game: &Game,
    id: egui::Id,
    kind: ArtKind,
) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let t = anim::animate_bool(ui.ctx(), id.with("hover"), resp.hovered(), anim::FAST);
    // Scaling and lifting only with full animations; otherwise the cover just brightens.
    let m = if anim::movement(ui.ctx()) { t } else { 0.0 };
    let rect = rect.expand(4.0 * m).translate(Vec2::new(0.0, -3.0 * m));
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
    match tex.get(client, game.app_id, kind) {
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

/// One compact drop-down for the ALL GAMES bar, like Steam's: "SORT BY  Alphabetical v" on a dark box
/// that is only as wide as its text.
fn bar_combo(ui: &mut egui::Ui, id: &str, label: &str, value: &str, list: impl FnOnce(&mut egui::Ui)) {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let dim = TextFormat {
        font_id: theme::bold(11.0),
        color: theme::TEXT_DIM,
        valign: egui::Align::Center,
        ..Default::default()
    };
    let val = TextFormat {
        font_id: theme::regular(13.0),
        color: theme::TEXT_HEADING,
        valign: egui::Align::Center,
        ..Default::default()
    };
    job.append(label, 0.0, dim);
    job.append(value, 8.0, val);
    let text_w = ui.ctx().fonts_mut(|f| f.layout_job(job.clone())).size().x;
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.bg_stroke = egui::Stroke::NONE;
            w.corner_radius = egui::CornerRadius::same(3);
            w.expansion = 0.0;
        }
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2a, 0x30, 0x3b);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x36, 0x3d, 0x4a);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(0x3e, 0x46, 0x55);
        v.widgets.open.weak_bg_fill = Color32::from_rgb(0x3e, 0x46, 0x55);
        ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
        widgets::combo(ui, id, text_w + 26.0, job, list);
    });
}
