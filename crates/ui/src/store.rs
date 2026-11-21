//! The Store tab: featured carousel, rows of capsules, live search and an app
//! page. Reads `client.state().store` and sends commands; payments are never
//! handled here, "Add to Cart" opens the store page in the system browser. `extra` has Browse, Wishlist, reviews
//! and trailers.

mod extra;

use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::text::LayoutJob;
use egui::{Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{AppDetails, Client, Command, Load, Platforms, Price, Section, StoreItem};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Widest the store content grows (Steam's own store is a fixed-width column).
const MAX_WIDTH: f32 = 1100.0;
/// Seconds each carousel slide stays; the cross-fade takes [`CROSSFADE`].
const HERO_SECONDS: f64 = 6.0;
const CROSSFADE: f32 = 0.45;
const HERO_SLIDES: usize = 5;
const SEARCH_DEBOUNCE: f64 = 0.3;
const CARD_IMG_H: f32 = 107.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Home,
    App(u32),
    Browse,
    Wishlist,
}

/// History ids of the pages that have no app id (apps use `id + 1`, which stays far below these).
const BROWSE_ID: u64 = 1 << 40;
const WISHLIST_ID: u64 = (1 << 40) + 1;

#[derive(Default)]
pub struct StoreView {
    page: Page,
    search: String,
    /// Last text that was sent as a search, and when the box was last edited.
    sent: String,
    edited_at: f64,
    hero: usize,
    hero_prev: Option<usize>,
    /// egui time the current slide started.
    hero_since: f64,
    /// First visible card per row (the arrows page through a row).
    offsets: HashMap<String, usize>,
    shot: usize,
    /// Text typed in the "More tags" list on the Browse page.
    tag_filter: String,
}

impl StoreView {
    /// Identifies the visible page, for the page-change animation.
    pub fn page_id(&self) -> u64 {
        match self.page {
            Page::Home => 0,
            Page::App(id) => u64::from(id) + 1,
            Page::Browse => BROWSE_ID,
            Page::Wishlist => WISHLIST_ID,
        }
    }

    /// Show the page with this id (see [`StoreView::page_id`]); used by the navigation history.
    pub fn set_page_id(&mut self, client: &mut Client, id: u64) {
        match id {
            0 => self.page = Page::Home,
            BROWSE_ID => self.go(client, Page::Browse),
            WISHLIST_ID => self.go(client, Page::Wishlist),
            n => self.open_app(client, (n - 1) as u32),
        }
    }

    #[allow(dead_code)]
    pub fn back(&mut self) {
        self.page = Page::Home;
    }

    fn open_app(&mut self, client: &mut Client, id: u32) {
        self.page = Page::App(id);
        self.shot = 0;
        client.send(Command::StoreDetails(sr_model::AppId(id)));
        client.send(Command::StoreMore(sr_client::StoreCmd::Reviews(sr_model::AppId(id))));
    }

    /// Switch to Home, Browse or Wishlist and ask for what that page shows.
    fn go(&mut self, client: &mut Client, page: Page) {
        self.page = page;
        match page {
            Page::Browse => extra::start_browse(client),
            Page::Wishlist => {
                client.send(Command::StoreMore(sr_client::StoreCmd::Wishlist { refresh: false }));
            }
            Page::Home => client.send(Command::StoreLoad),
            Page::App(_) => {}
        }
    }

    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        match self.page {
            Page::Home => self.home(ui, client, tex),
            Page::App(id) => self.app_page(ui, client, tex, id),
            Page::Browse => extra::browse_page(self, ui, client, tex),
            Page::Wishlist => extra::wishlist_page(self, ui, client, tex),
        }
    }

    // ---- home ----------------------------------------------------------

    fn home(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        egui::ScrollArea::vertical().id_salt("store-home").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(theme::PAGE_PAD);
            column(ui, |ui| {
                self.search_box(ui, client);
                ui.add_space(10.0);
                if let Some(p) = extra::nav(ui, self.page) {
                    self.go(client, p);
                }
                ui.add_space(18.0);
                if self.search.trim().is_empty() {
                    self.featured(ui, client, tex);
                    extra::region_note(ui);
                } else {
                    self.results(ui, client, tex);
                }
            });
            ui.add_space(theme::PAGE_PAD);
        });
    }

    /// The search box. Typing is debounced so one search runs per pause.
    fn search_box(&mut self, ui: &mut Ui, client: &mut Client) {
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            let edit = egui::TextEdit::singleline(&mut self.search)
                .hint_text(format!("{}  Search the store", icon::MAGNIFYING_GLASS))
                .font(theme::regular(15.0))
                .desired_width(340.0)
                .margin(egui::Margin::symmetric(10, 7));
            if ui.add(edit).changed() {
                self.edited_at = now;
            }
            if client.state().store.search.loading {
                ui.add_space(10.0);
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
        if self.search != self.sent {
            let wait = SEARCH_DEBOUNCE - (now - self.edited_at);
            if wait <= 0.0 {
                self.sent = self.search.clone();
                client.send(Command::StoreSearch(self.search.clone()));
            } else {
                ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
            }
        }
    }

    fn featured(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        let sections = match &client.state().store.sections {
            Load::Ready(s) => Arc::clone(s),
            Load::Failed(e) => {
                let msg = format!("Couldn't load the store: {e}");
                return self.failed(ui, client, &msg);
            }
            _ => {
                let w = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 340.0), Sense::hover());
                return widgets::placeholder(ui, rect);
            }
        };
        if let Some(top) = sections.iter().find(|s| s.id == "top_sellers") {
            self.hero(ui, client, tex, &top.items[..top.items.len().min(HERO_SLIDES)]);
            ui.add_space(26.0);
        }
        for section in sections.iter() {
            self.row(ui, client, tex, section);
            ui.add_space(32.0);
        }
    }

    fn failed(&mut self, ui: &mut Ui, client: &mut Client, msg: &str) {
        ui.label(RichText::new(msg).color(theme::TEXT_DIM));
        ui.add_space(8.0);
        if ui.button("Try again").clicked() {
            client.send(Command::StoreLoad);
        }
    }

    // ---- hero carousel ---------------------------------------------------

    fn hero(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, items: &[StoreItem]) {
        if items.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.hero = self.hero.min(items.len() - 1);
        // Auto-advance; the repaint is scheduled only while this page is drawn.
        if now - self.hero_since >= HERO_SECONDS {
            self.goto_slide((self.hero + 1) % items.len(), now);
        }
        ctx.request_repaint_after(Duration::from_secs_f64((HERO_SECONDS - (now - self.hero_since)).max(0.0)));

        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 340.0), Sense::click());
        widgets::gradient(ui, rect, theme::BG_SIDEBAR, theme::BG_MAIN);
        let img_w = (rect.height() * 616.0 / 353.0).min(w * 0.62);
        let img = Rect::from_min_size(rect.min, Vec2::new(img_w, rect.height()));

        let fade = anim::since(&ctx, self.hero_since, CROSSFADE);
        if let Some(prev) = self.hero_prev.filter(|_| fade < 1.0) {
            slide_image(ui, client, tex, img, &items[prev], 1.0);
        }
        slide_image(ui, client, tex, img, &items[self.hero], if self.hero_prev.is_some() { fade } else { 1.0 });
        self.hero_info(ui, rect, img_w, &items[self.hero], fade);

        if self.hero_arrows(ui, img, items.len(), now) {
            return;
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.open_app(client, items[self.hero].id);
        }
    }

    fn goto_slide(&mut self, next: usize, now: f64) {
        self.hero_prev = Some(self.hero);
        self.hero = next;
        self.hero_since = now;
    }

    fn hero_info(&self, ui: &Ui, rect: Rect, img_w: f32, item: &StoreItem, fade: f32) {
        let x = rect.left() + img_w + 28.0;
        let width = (rect.right() - x - 28.0).max(120.0);
        let painter = ui.painter();
        let alpha = |c: Color32| c.gamma_multiply(if self.hero_prev.is_some() { fade } else { 1.0 });
        let y = rect.top() + 40.0 + 6.0 * (1.0 - fade);
        let title = painter.layout(item.name.clone(), theme::bold(26.0), alpha(Color32::WHITE), width);
        let title_h = title.size().y;
        painter.galley(Pos2::new(x, y), title, Color32::WHITE);
        platform_icons(painter, Pos2::new(x, y + title_h + 14.0), &item.platforms, alpha(theme::TEXT_DIM));
        if let Some(p) = &item.price {
            price_tag(painter, Pos2::new(x, rect.bottom() - 40.0), p, true, false, fade);
        }
    }

    /// Prev/next arrows over the image and the dots below it. True when one was used.
    fn hero_arrows(&mut self, ui: &mut Ui, img: Rect, n: usize, now: f64) -> bool {
        let mut used = false;
        for (dir, x) in [(-1_i32, img.left() + 22.0), (1, img.right() - 22.0)] {
            let glyph = if dir < 0 { icon::CARET_LEFT } else { icon::CARET_RIGHT };
            if round_button(ui, Pos2::new(x, img.center().y), glyph, Id::new(("hero-arrow", dir))) {
                self.goto_slide((self.hero + n).wrapping_add_signed(dir as isize) % n, now);
                used = true;
            }
        }
        let y = img.bottom() - 14.0;
        let x0 = img.center().x - (n as f32 - 1.0) * 8.0;
        for i in 0..n {
            let c = Pos2::new(x0 + i as f32 * 16.0, y);
            let r = Rect::from_center_size(c, Vec2::splat(16.0));
            let resp = ui.interact(r, Id::new(("hero-dot", i)), Sense::click());
            let on = i == self.hero;
            ui.painter().circle_filled(c, 3.5, if on { Color32::WHITE } else { Color32::from_white_alpha(90) });
            if resp.clicked() && !on {
                self.goto_slide(i, now);
                used = true;
            }
        }
        used
    }

    // ---- rows of capsules -----------------------------------------------

    fn row(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, section: &Section) {
        let w = ui.available_width();
        let gap = theme::GRID_GAP;
        let per_page = (((w + gap) / (theme::STORE_CARD_W + gap)).floor() as usize).max(1);
        let offset = self.offsets.entry(section.id.clone()).or_insert(0);
        *offset = (*offset).min(section.items.len().saturating_sub(per_page));
        let (start, total) = (*offset, section.items.len());

        let mut page = 0_i32;
        ui.horizontal(|ui| {
            ui.label(RichText::new(section.title.to_uppercase()).font(theme::bold(14.0)).color(theme::TEXT_HEADING));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if start + per_page < total && widgets::icon_button(ui, icon::CARET_RIGHT, 26.0, "Next").clicked() {
                    page = 1;
                }
                if start > 0 && widgets::icon_button(ui, icon::CARET_LEFT, 26.0, "Previous").clicked() {
                    page = -1;
                }
            });
        });
        ui.add_space(10.0);

        let (row, _) = ui.allocate_exact_size(Vec2::new(w, theme::STORE_CARD_H), Sense::hover());
        let pitch = if per_page > 1 {
            let leftover = w - per_page as f32 * theme::STORE_CARD_W - (per_page - 1) as f32 * gap;
            theme::STORE_CARD_W + gap + leftover.max(0.0) / (per_page - 1) as f32
        } else {
            theme::STORE_CARD_W + gap
        };
        for (slot, item) in section.items.iter().skip(start).take(per_page).enumerate() {
            let min = Pos2::new(row.left() + slot as f32 * pitch, row.top());
            let rect = Rect::from_min_size(min, Vec2::new(theme::STORE_CARD_W, theme::STORE_CARD_H));
            if card(ui, client, tex, rect, item, Id::new((&section.id, item.id, slot))).clicked() {
                self.open_app(client, item.id);
            }
        }
        if page != 0 {
            let step = per_page as i32 * page;
            let o = self.offsets.entry(section.id.clone()).or_insert(0);
            *o = (start as i32 + step).clamp(0, total as i32) as usize;
        }
    }

    // ---- search results --------------------------------------------------

    fn results(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        let search = client.state().store.search.clone();
        if let Some(e) = &search.error {
            return self.failed_search(ui, e);
        }
        if search.items.is_empty() {
            let msg = if search.loading || search.query != self.search.trim() { "Searching..." } else { "No results." };
            ui.label(RichText::new(msg).color(theme::TEXT_DIM));
            return;
        }
        ui.label(
            RichText::new(format!("RESULTS FOR \"{}\"", search.query.to_uppercase()))
                .font(theme::bold(13.0))
                .color(theme::TEXT_HEADING),
        );
        ui.add_space(10.0);
        for item in &search.items {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), Sense::click());
            let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
            ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_SIDEBAR.lerp_to_gamma(theme::BG_ROW_HOVER, hover));
            let img = Rect::from_min_size(rect.min + Vec2::new(8.0, 6.0), Vec2::new(121.0, 48.0));
            image_or_placeholder(ui, client, tex, img, &item.header_image);
            let text_x = img.right() + 14.0;
            let name = elide(ui, &item.name, theme::medium(15.0), theme::TEXT, rect.right() - text_x - 220.0);
            ui.painter().galley(Pos2::new(text_x, rect.top() + 11.0), name, theme::TEXT);
            platform_icons(ui.painter(), Pos2::new(text_x, rect.top() + 34.0), &item.platforms, theme::TEXT_DIM);
            if let Some(p) = &item.price {
                price_tag(ui.painter(), Pos2::new(rect.right() - 12.0, rect.center().y - 15.0), p, false, true, 1.0);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.open_app(client, item.id);
            }
            ui.add_space(4.0);
        }
    }

    fn failed_search(&self, ui: &mut Ui, e: &str) {
        ui.label(RichText::new(format!("Search failed: {e}")).color(theme::TEXT_DIM));
    }

    // ---- app page --------------------------------------------------------

    fn app_page(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, id: u32) {
        let load = client.state().store.details.get(&id).cloned().unwrap_or_default();
        egui::ScrollArea::vertical().id_salt(("store-app", id)).auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(theme::PAGE_PAD);
            column(ui, |ui| match load {
                Load::Ready(d) => self.app_details(ui, client, tex, &d),
                Load::Failed(e) => {
                    ui.label(RichText::new(format!("Couldn't load this page: {e}")).color(theme::TEXT_DIM));
                    ui.add_space(8.0);
                    if ui.button("Try again").clicked() {
                        client.send(Command::StoreDetails(sr_model::AppId(id)));
                    }
                }
                _ => {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 320.0), Sense::hover());
                    widgets::placeholder(ui, rect);
                }
            });
            ui.add_space(theme::PAGE_PAD);
        });
    }

    fn app_details(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, d: &AppDetails) {
        ui.label(RichText::new(&d.name).font(theme::bold(30.0)).color(Color32::WHITE));
        ui.add_space(16.0);
        let w = ui.available_width();
        let side_w = 324.0_f32.min(w * 0.36);
        let main_w = w - side_w - 24.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.vertical(|ui| {
                ui.set_width(main_w);
                self.screenshots(ui, client, tex, d, main_w);
                extra::trailers(ui, client, tex, d);
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.set_width(side_w);
                side_panel(ui, client, tex, d, side_w);
            });
        });
        ui.add_space(24.0);
        purchase_box(ui, client, d);
        ui.add_space(24.0);
        extra::reviews_section(ui, client, d.id);
    }

    fn screenshots(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures, d: &AppDetails, w: f32) {
        if d.screenshots.is_empty() {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w * 215.0 / 460.0), Sense::hover());
            return image_or_placeholder(ui, client, tex, rect, &d.header_image);
        }
        self.shot = self.shot.min(d.screenshots.len() - 1);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w * 9.0 / 16.0), Sense::hover());
        image_or_placeholder(ui, client, tex, rect, &d.screenshots[self.shot].full);
        let n = d.screenshots.len();
        for (dir, x) in [(-1_i32, rect.left() + 22.0), (1, rect.right() - 22.0)] {
            let glyph = if dir < 0 { icon::CARET_LEFT } else { icon::CARET_RIGHT };
            if n > 1 && round_button(ui, Pos2::new(x, rect.center().y), glyph, Id::new(("shot-arrow", dir))) {
                self.shot = (self.shot + n).wrapping_add_signed(dir as isize) % n;
            }
        }
        ui.add_space(8.0);
        egui::ScrollArea::horizontal().id_salt("shots").auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (i, s) in d.screenshots.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(116.0, 65.0), Sense::click());
                    image_or_placeholder(ui, client, tex, r, &s.thumb);
                    if i == self.shot {
                        ui.painter().rect_stroke(r, theme::RADIUS, (2.0, theme::ACCENT), egui::StrokeKind::Inside);
                    } else if !resp.hovered() {
                        ui.painter().rect_filled(r, theme::RADIUS, Color32::from_black_alpha(90));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.shot = i;
                    }
                }
            });
            ui.add_space(8.0);
        });
    }
}

// ---- pieces -----------------------------------------------------------------

/// A centered column no wider than [`MAX_WIDTH`], with the page padding at the sides.
fn column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let avail = ui.available_width();
    let w = (avail - 2.0 * theme::PAGE_PAD).clamp(300.0, MAX_WIDTH);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.add_space(((avail - w) / 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui);
        });
    });
}

fn side_panel(ui: &mut Ui, client: &Client, tex: &mut Textures, d: &AppDetails, w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w * 215.0 / 460.0), Sense::hover());
    image_or_placeholder(ui, client, tex, rect, &d.header_image);
    ui.add_space(14.0);
    ui.add(egui::Label::new(RichText::new(&d.short_description).font(theme::regular(14.0)).color(theme::TEXT)).wrap());
    ui.add_space(16.0);
    let mut line = |label: &str, value: String| {
        if value.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{label}  ")).font(theme::bold(12.0)).color(theme::TEXT_DIM));
            ui.label(RichText::new(value).font(theme::regular(13.0)).color(theme::TEXT));
        });
        ui.add_space(4.0);
    };
    line("RELEASE DATE", d.release_date.clone());
    line("DEVELOPER", d.developers.join(", "));
    line("GENRES", d.genres.join(", "));
    ui.horizontal(|ui| {
        ui.label(RichText::new("PLATFORMS  ").font(theme::bold(12.0)).color(theme::TEXT_DIM));
        let (r, _) = ui.allocate_exact_size(Vec2::new(70.0, 18.0), Sense::hover());
        platform_icons(ui.painter(), r.left_top(), &d.platforms, theme::TEXT);
    });
}

fn purchase_box(ui: &mut Ui, client: &mut Client, d: &AppDetails) {
    let frame = egui::Frame::new().fill(theme::BG_ACTION_BAR).inner_margin(egui::Margin::symmetric(20, 16));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(format!("Buy {}", d.name)).font(theme::bold(18.0)).color(Color32::WHITE));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let label = match &d.price {
                Some(p) => p.label(),
                None if d.is_free => "Free".to_string(),
                None => "Not available".to_string(),
            };
            if widgets::action_button(ui, icon::SHOPPING_CART, "ADD TO CART", (theme::PLAY_A, theme::PLAY_B)).clicked()
            {
                client.send(Command::OpenStorePage(sr_model::AppId(d.id)));
            }
            if let Some(p) = d.price.as_ref().filter(|p| p.discount_percent > 0) {
                let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 46.0), Sense::hover());
                price_tag(ui.painter(), Pos2::new(r.left(), r.top() + 4.0), p, true, false, 1.0);
            } else {
                ui.label(RichText::new(label).font(theme::bold(18.0)).color(theme::TEXT));
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new("Opens the store page in your browser.").font(theme::regular(12.0)).color(theme::TEXT_DIM),
        );
    });
}

/// One capsule in a row: header image, name below, price tag on the image.
fn card(ui: &mut Ui, client: &Client, tex: &mut Textures, rect: Rect, item: &StoreItem, id: Id) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let t = anim::animate_bool(ui.ctx(), id.with("hover"), resp.hovered(), anim::FAST);
    let img = Rect::from_min_size(rect.min, Vec2::new(rect.width(), CARD_IMG_H))
        .expand(3.0 * t)
        .translate(Vec2::new(0.0, -2.0 * t));
    if t > 0.0 {
        let shadow = egui::epaint::Shadow {
            offset: [0, (6.0 * t) as i8],
            blur: (14.0 * t) as u8,
            spread: 0,
            color: Color32::from_black_alpha((110.0 * t) as u8),
        };
        ui.painter().add(shadow.as_shape(img, theme::RADIUS));
    }
    image_or_placeholder(ui, client, tex, img, &item.header_image);
    if let Some(p) = &item.price {
        price_tag(ui.painter(), Pos2::new(img.right() - 6.0, img.bottom() - 28.0), p, false, true, 1.0);
    }
    let name = elide(ui, &item.name, theme::medium(13.0), theme::TEXT.lerp_to_gamma(Color32::WHITE, t), rect.width());
    ui.painter().galley(Pos2::new(rect.left(), rect.top() + CARD_IMG_H + 8.0), name, theme::TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn slide_image(ui: &Ui, client: &Client, tex: &mut Textures, rect: Rect, item: &StoreItem, alpha: f32) {
    let url = if item.large_image.is_empty() { &item.header_image } else { &item.large_image };
    match tex.get_web(client, url) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a.min(alpha))),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
        }
    }
}

fn image_or_placeholder(ui: &Ui, client: &Client, tex: &mut Textures, rect: Rect, url: &str) {
    match tex.get_web(client, url) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a)),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
        }
    }
}

/// A round translucent arrow button over an image. True when clicked.
fn round_button(ui: &mut Ui, center: Pos2, glyph: &str, id: Id) -> bool {
    let resp = ui.interact(Rect::from_center_size(center, Vec2::splat(34.0)), id, Sense::click());
    let t = anim::animate_bool(ui.ctx(), id.with("hover"), resp.hovered(), anim::FAST);
    let bg = Color32::from_black_alpha((90.0 + 80.0 * t) as u8);
    ui.painter().circle_filled(center, 17.0, bg);
    ui.painter().text(center, Align2::CENTER_CENTER, glyph, theme::regular(18.0), Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// One line of text cut with "..." to `width`.
fn elide(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::simple(text.to_string(), font, color, width.max(10.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// Small Windows / macOS / Linux glyphs for the supported platforms.
fn platform_icons(painter: &egui::Painter, at: Pos2, p: &Platforms, color: Color32) {
    let mut text = String::new();
    for (on, glyph) in [(p.windows, icon::WINDOWS_LOGO), (p.mac, icon::APPLE_LOGO), (p.linux, icon::LINUX_LOGO)] {
        if on {
            text.push_str(glyph);
            text.push_str("  ");
        }
    }
    painter.text(at, Align2::LEFT_TOP, text, theme::regular(15.0), color);
}

/// Price like the store: a green discount box next to the (struck-through)
/// old and the new price. `right_aligned` anchors `at` to the right edge.
fn price_tag(painter: &egui::Painter, at: Pos2, price: &Price, big: bool, right_aligned: bool, alpha: f32) {
    let discounted = price.discount_percent > 0;
    // A discounted tag stacks old and new price, so it is taller.
    let (h, font, small) = match (big, discounted) {
        (true, true) => (44.0, 18.0, 12.0),
        (true, false) => (34.0, 18.0, 12.0),
        (false, true) => (32.0, 14.0, 11.0),
        (false, false) => (24.0, 14.0, 11.0),
    };
    let a = |c: Color32| c.gamma_multiply(alpha);
    let pad = 7.0;
    let new = painter.layout_no_wrap(price.label(), theme::bold(font), a(Color32::WHITE));
    let badge = discounted.then(|| {
        painter.layout_no_wrap(format!("-{}%", price.discount_percent), theme::bold(font), a(theme::DISCOUNT_TEXT))
    });
    let old =
        discounted.then(|| painter.layout_no_wrap(price.original_label(), theme::regular(small), a(theme::TEXT_DIM)));
    let badge_w = badge.as_ref().map_or(0.0, |b| b.size().x + 2.0 * pad);
    let price_w = new.size().x.max(old.as_ref().map_or(0.0, |o| o.size().x)) + 2.0 * pad;
    let total = badge_w + price_w;
    let x = if right_aligned { at.x - total } else { at.x };
    let top = Pos2::new(x, at.y);
    if let Some(b) = badge {
        let r = Rect::from_min_size(top, Vec2::new(badge_w, h));
        painter.rect_filled(r, theme::RADIUS, a(theme::DISCOUNT_BG));
        painter.galley(r.center() - b.size() / 2.0, b, Color32::WHITE);
    }
    let pr = Rect::from_min_size(Pos2::new(x + badge_w, at.y), Vec2::new(price_w, h));
    painter.rect_filled(
        pr,
        theme::RADIUS,
        a(if discounted { theme::PRICE_BG } else { Color32::from_black_alpha(150) }),
    );
    match old {
        Some(o) => {
            let oy = pr.top() + 2.0;
            let ox = pr.right() - pad - o.size().x;
            let line_y = oy + o.size().y / 2.0;
            painter.galley(Pos2::new(ox, oy), o.clone(), Color32::WHITE);
            painter
                .line_segment([Pos2::new(ox, line_y), Pos2::new(ox + o.size().x, line_y)], (1.0, a(theme::TEXT_DIM)));
            painter.galley(
                Pos2::new(pr.right() - pad - new.size().x, pr.bottom() - new.size().y - 1.0),
                new,
                Color32::WHITE,
            );
        }
        None => {
            painter.galley(pr.center() - new.size() / 2.0, new, Color32::WHITE);
        }
    }
}
