//! Left column of the library: Home, filters, search and the game list.
//! The list is virtualized: only visible rows are laid out.

use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command};
use sr_model::ArtKind;

#[derive(Default)]
pub struct SidebarView {
    search: String,
    installed_only: bool,
    /// Indices into `state.games` that pass the filter.
    rows: Vec<usize>,
    /// What `rows` was computed from: (library_rev, search, installed_only).
    rows_key: Option<(u64, String, bool)>,
}

impl SidebarView {
    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, library_rev: u64) {
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
                self.list(ui, client, tex);
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
        }
    }

    fn filter_row(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new(format!("GAMES {}", icon::CARET_DOWN)).font(theme::bold(12.0)).color(theme::TEXT_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                let tip = if self.installed_only { "Showing installed games" } else { "Show installed games only" };
                let r = widgets::icon_button(ui, icon::DOWNLOAD_SIMPLE, 24.0, tip);
                if self.installed_only {
                    ui.painter().rect_stroke(r.rect, theme::RADIUS, (1.0, theme::ACCENT), egui::StrokeKind::Inside);
                }
                if r.clicked() {
                    self.installed_only = !self.installed_only;
                }
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
        let key = (library_rev, self.search.clone(), self.installed_only);
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let needle = sr_model::sort_key(&self.search);
        self.rows = client
            .state()
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| !self.installed_only || g.install.is_installed())
            .filter(|(_, g)| needle.is_empty() || g.sort_key.contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.rows_key = Some(key);
    }

    fn list(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        let label = if self.installed_only { "INSTALLED" } else { "ALL" };
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let text = format!("{label} ({})", self.rows.len());
            ui.label(RichText::new(text).font(theme::bold(12.0)).color(theme::TEXT_DIM));
        });
        ui.add_space(4.0);
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            theme::LIST_ROW_H,
            self.rows.len(),
            |ui, range| {
                let state = client.state();
                for &i in &self.rows[range] {
                    let g = &state.games[i];
                    let selected = state.selected == Some(g.app_id);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), theme::LIST_ROW_H), Sense::click());
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
                }
            },
        );
        if let Some(app) = clicked {
            client.send(Command::Select(Some(app)));
        }
    }
}
