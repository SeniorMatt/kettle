//! The Store tab's Browse and Wishlist pages, plus the reviews and trailers on an app page.
//! Reads `client.state().store.more`; the wishlist is read-only (changing it needs the store's web login, so a
//! button opens the wishlist page in the browser).

use super::{Page, StoreView, card, column, elide, image_or_placeholder, platform_icons, price_tag};
use crate::textures::Textures;
use crate::{anim, theme};
use egui::{Align2, Color32, Id, Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{AppDetails, BrowseKind, Client, Command, Load, Reviews, StoreCmd, StoreItem, Tag};

/// Genres and popular tags shown as chips (matched by name against the store's tag list).
const FEATURED_TAGS: [&str; 14] = [
    "Action",
    "Adventure",
    "RPG",
    "Strategy",
    "Simulation",
    "Casual",
    "Indie",
    "Sports",
    "Racing",
    "Puzzle",
    "Horror",
    "Open World",
    "Multiplayer",
    "Free to Play",
];
const POSITIVE: Color32 = theme::VERIFY;
const MIXED: Color32 = Color32::from_rgb(0xb9, 0xa0, 0x74);

// ---- small widgets ---------------------------------------------------------------------------------------------

/// A flat text tab with an accent underline when selected. True when clicked.
fn tab(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let width = ui.painter().layout_no_wrap(label.to_string(), theme::bold(14.0), Color32::WHITE).size().x + 24.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let color = if selected {
        Color32::WHITE
    } else if resp.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    ui.painter().text(rect.center() - Vec2::new(0.0, 1.0), Align2::CENTER_CENTER, label, theme::bold(14.0), color);
    if selected {
        let bar =
            Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.bottom() - 3.0), Vec2::new(rect.width() - 16.0, 2.0));
        ui.painter().rect_filled(bar, egui::CornerRadius::ZERO, theme::ACCENT);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A small rounded filter chip. True when clicked.
fn chip(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let width = ui.painter().layout_no_wrap(label.to_string(), theme::medium(13.0), Color32::WHITE).size().x + 22.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let fill = if selected {
        theme::ACCENT
    } else if resp.hovered() {
        theme::BG_ROW_HOVER
    } else {
        theme::BG_CARD
    };
    ui.painter().rect_filled(rect, theme::RADIUS, fill);
    let color = if selected { Color32::WHITE } else { theme::TEXT };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, theme::medium(13.0), color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Featured / Browse / Wishlist tabs. Returns the page to switch to.
pub(super) fn nav(ui: &mut Ui, page: Page) -> Option<Page> {
    let mut go = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (label, target) in [("Featured", Page::Home), ("Browse", Page::Browse), ("Wishlist", Page::Wishlist)] {
            if tab(ui, label, page == target) && page != target {
                go = Some(target);
            }
        }
    });
    go
}

/// "Prices for: ..." line under the lists.
pub(super) fn region_note(ui: &mut Ui) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!(
            "Store region: {}. The country comes from your Steam login, the language from Settings.",
            sr_client::store_region()
        ))
        .font(theme::regular(12.0))
        .color(theme::TEXT_DIM),
    );
}

/// Ask for what the Browse page shows (the tag list and the current list).
pub(super) fn start_browse(client: &mut Client) {
    let b = &client.state().store.more.browse;
    let (kind, tag) = (b.kind, b.tag);
    client.send(Command::StoreMore(StoreCmd::Tags));
    client.send(Command::StoreMore(StoreCmd::Browse { kind, tag, more: false }));
}

// ---- browse ----------------------------------------------------------------------------------------------------

pub(super) fn browse_page(view: &mut StoreView, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
    let mut goto = None;
    let mut open = None;
    let mut cmds: Vec<StoreCmd> = Vec::new();
    {
        let client: &Client = client;
        egui::ScrollArea::vertical().id_salt("store-browse").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(theme::PAGE_PAD);
            column(ui, |ui| {
                goto = nav(ui, Page::Browse);
                ui.add_space(14.0);
                let b = &client.state().store.more.browse;
                let (kind, tag) = (b.kind, b.tag);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    for k in BrowseKind::ALL {
                        if chip(ui, k.label(), k == kind) {
                            cmds.push(StoreCmd::Browse { kind: k, tag, more: false });
                        }
                    }
                });
                ui.add_space(8.0);
                tag_chips(view, ui, client, kind, tag, &mut cmds);
                ui.add_space(18.0);
                if let Some(id) = grid(ui, client, tex, &mut cmds) {
                    open = Some(id);
                }
                region_note(ui);
            });
            ui.add_space(theme::PAGE_PAD);
        });
    }
    for c in cmds {
        client.send(Command::StoreMore(c));
    }
    if let Some(p) = goto {
        view.go(client, p);
    }
    if let Some(id) = open {
        view.open_app(client, id);
    }
}

fn tag_chips(
    view: &mut StoreView,
    ui: &mut Ui,
    client: &Client,
    kind: BrowseKind,
    tag: Option<u32>,
    cmds: &mut Vec<StoreCmd>,
) {
    let tags = client.state().store.more.tags.ready().cloned();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        if chip(ui, "All genres", tag.is_none()) {
            cmds.push(StoreCmd::Browse { kind, tag: None, more: false });
        }
        let Some(tags) = &tags else {
            if matches!(client.state().store.more.tags, Load::Loading) {
                ui.add(egui::Spinner::new().size(14.0));
            }
            return;
        };
        for name in FEATURED_TAGS {
            if let Some(t) = tags.iter().find(|t| t.name.eq_ignore_ascii_case(name))
                && chip(ui, &t.name, tag == Some(t.id))
            {
                cmds.push(StoreCmd::Browse { kind, tag: Some(t.id), more: false });
            }
        }
        let current = tag.and_then(|id| tags.iter().find(|t| t.id == id)).map_or("More tags", |t| t.name.as_str());
        egui::ComboBox::from_id_salt("store-more-tags").selected_text(current).width(180.0).show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut view.tag_filter).hint_text("Filter tags"));
            let needle = view.tag_filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                let shown = tags.iter().filter(|t: &&Tag| needle.is_empty() || t.name.to_lowercase().contains(&needle));
                for t in shown.take(120) {
                    if ui.selectable_label(tag == Some(t.id), &t.name).clicked() {
                        cmds.push(StoreCmd::Browse { kind, tag: Some(t.id), more: false });
                    }
                }
            });
        });
    });
}

/// The card grid with endless scrolling. Returns the app that was clicked.
fn grid(ui: &mut Ui, client: &Client, tex: &mut Textures, cmds: &mut Vec<StoreCmd>) -> Option<u32> {
    let b = &client.state().store.more.browse;
    if let Some(e) = &b.error {
        ui.label(RichText::new(format!("Couldn't load this list: {e}")).color(theme::TEXT_DIM));
        ui.add_space(8.0);
        if ui.button("Try again").clicked() {
            cmds.push(StoreCmd::Browse { kind: b.kind, tag: b.tag, more: false });
        }
        return None;
    }
    if b.items.is_empty() {
        let msg = if b.loading || !b.started() { "Loading..." } else { "Nothing found." };
        ui.label(RichText::new(msg).color(theme::TEXT_DIM));
        return None;
    }
    let w = ui.available_width();
    let gap = theme::GRID_GAP;
    let per_row = (((w + gap) / (theme::STORE_CARD_W + gap)).floor() as usize).max(1);
    let pitch = if per_row > 1 {
        let leftover = w - per_row as f32 * theme::STORE_CARD_W - (per_row - 1) as f32 * gap;
        theme::STORE_CARD_W + gap + leftover.max(0.0) / (per_row - 1) as f32
    } else {
        theme::STORE_CARD_W + gap
    };
    let row_h = theme::STORE_CARD_H + gap;
    let rows = b.items.len().div_ceil(per_row);
    let (area, _) = ui.allocate_exact_size(Vec2::new(w, rows as f32 * row_h), Sense::hover());
    let mut clicked = None;
    for (i, item) in b.items.iter().enumerate() {
        let (r, c) = (i / per_row, i % per_row);
        let min = area.min + Vec2::new(c as f32 * pitch, r as f32 * row_h);
        let rect = Rect::from_min_size(min, Vec2::new(theme::STORE_CARD_W, theme::STORE_CARD_H));
        // Cards far outside the window are neither drawn nor their images loaded.
        if !ui.is_rect_visible(rect.expand(80.0)) {
            continue;
        }
        if card(ui, client, tex, rect, item, Id::new(("browse", item.id))).clicked() {
            clicked = Some(item.id);
        }
    }
    // Sentinel under the grid: when it comes near the window, the next page is asked for.
    let (end, _) = ui.allocate_exact_size(Vec2::new(w, 56.0), Sense::hover());
    if b.loading {
        ui.put(Rect::from_center_size(end.center(), Vec2::splat(18.0)), egui::Spinner::new().size(18.0));
    } else if b.more {
        let near = Rect::from_min_max(end.min - Vec2::new(0.0, 600.0), end.max);
        if ui.is_rect_visible(near) {
            cmds.push(StoreCmd::Browse { kind: b.kind, tag: b.tag, more: true });
        }
    }
    clicked
}

// ---- wishlist --------------------------------------------------------------------------------------------------

pub(super) fn wishlist_page(view: &mut StoreView, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
    let mut goto = None;
    let mut open = None;
    let mut refresh = false;
    let mut browser = false;
    {
        let client: &Client = client;
        egui::ScrollArea::vertical().id_salt("store-wishlist").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(theme::PAGE_PAD);
            column(ui, |ui| {
                goto = nav(ui, Page::Wishlist);
                ui.add_space(14.0);
                let st = client.state();
                let own = st.account.as_ref().map_or(0, |a| a.steam_id.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("YOUR WISHLIST").font(theme::bold(14.0)).color(theme::TEXT_HEADING));
                    ui.add_space(14.0);
                    if own != 0 && ui.button("Refresh").clicked() {
                        refresh = true;
                    }
                    ui.add_space(6.0);
                    if ui.button("Add or remove games in the browser").clicked() {
                        browser = true;
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Read-only here: Kettle has no store login, so adding and removing games happens on the store page.",
                    )
                    .font(theme::regular(12.0))
                    .color(theme::TEXT_DIM),
                );
                ui.add_space(14.0);
                let more = &st.store.more;
                if own == 0 {
                    ui.label(RichText::new("Sign in to see your wishlist.").color(theme::TEXT_DIM));
                    return;
                }
                match &more.wishlist {
                    Load::Ready(w) if more.wishlist_for == own => {
                        if w.items.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Nothing here. Your wishlist is empty, or your Steam profile's \"Game details\" are not public.",
                                )
                                .color(theme::TEXT_DIM),
                            );
                        }
                        for (n, entry) in w.items.iter().enumerate() {
                            if let Some(id) = wish_row(ui, client, tex, n, entry.priority, entry.added, &entry.item) {
                                open = Some(id);
                            }
                        }
                        if w.total > w.items.len() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!("Showing the first {} of {} games.", w.items.len(), w.total))
                                    .color(theme::TEXT_DIM),
                            );
                        }
                    }
                    Load::Failed(e) if more.wishlist_for == own => {
                        ui.label(RichText::new(format!("Couldn't load your wishlist: {e}")).color(theme::TEXT_DIM));
                    }
                    _ => {
                        ui.label(RichText::new("Loading...").color(theme::TEXT_DIM));
                    }
                }
            });
            ui.add_space(theme::PAGE_PAD);
        });
    }
    if refresh {
        client.send(Command::StoreMore(StoreCmd::Wishlist { refresh: true }));
    }
    if browser {
        client.send(Command::OpenUrl("https://store.steampowered.com/wishlist/".into()));
    }
    if let Some(p) = goto {
        view.go(client, p);
    }
    if let Some(id) = open {
        view.open_app(client, id);
    }
}

fn wish_row(
    ui: &mut Ui,
    client: &Client,
    tex: &mut Textures,
    n: usize,
    priority: u32,
    added: u64,
    item: &StoreItem,
) -> Option<u32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_SIDEBAR.lerp_to_gamma(theme::BG_ROW_HOVER, hover));
    let rank = if priority > 0 { priority as usize } else { n + 1 };
    ui.painter().text(
        Pos2::new(rect.left() + 28.0, rect.center().y),
        Align2::CENTER_CENTER,
        rank.to_string(),
        theme::bold(14.0),
        theme::TEXT_DIM,
    );
    let img = Rect::from_min_size(rect.min + Vec2::new(52.0, 6.0), Vec2::new(121.0, 48.0));
    image_or_placeholder(ui, client, tex, img, &item.header_image);
    let text_x = img.right() + 14.0;
    let name = elide(ui, &item.name, theme::medium(15.0), theme::TEXT, rect.right() - text_x - 220.0);
    ui.painter().galley(Pos2::new(text_x, rect.top() + 9.0), name, theme::TEXT);
    platform_icons(ui.painter(), Pos2::new(text_x, rect.top() + 32.0), &item.platforms, theme::TEXT_DIM);
    if added > 0 {
        let at = Pos2::new(text_x + 90.0, rect.top() + 34.0);
        ui.painter().text(at, Align2::LEFT_TOP, format!("Added {}", ymd(added)), theme::regular(12.0), theme::TEXT_DIM);
    }
    if let Some(p) = &item.price {
        price_tag(ui.painter(), Pos2::new(rect.right() - 12.0, rect.center().y - 16.0), p, false, true, 1.0);
    }
    ui.add_space(4.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked().then_some(item.id)
}

// ---- reviews and trailers on an app page ---------------------------------------------------------------

pub(super) fn reviews_section(ui: &mut Ui, client: &Client, app: u32) {
    ui.label(RichText::new("USER REVIEWS").font(theme::bold(14.0)).color(theme::TEXT_HEADING));
    ui.add_space(8.0);
    let load = client.state().store.more.reviews.get(&app).cloned().unwrap_or_default();
    match load {
        Load::Ready(r) => reviews_list(ui, &r),
        Load::Failed(e) => {
            ui.label(RichText::new(format!("Reviews are not available: {e}")).color(theme::TEXT_DIM));
        }
        _ => {
            ui.label(RichText::new("Loading reviews...").color(theme::TEXT_DIM));
        }
    }
}

fn reviews_list(ui: &mut Ui, r: &Reviews) {
    let s = &r.summary;
    let color = if s.total == 0 {
        theme::TEXT_DIM
    } else if s.percent_positive() >= 70 {
        POSITIVE
    } else if s.percent_positive() >= 40 {
        MIXED
    } else {
        theme::ERROR_TEXT
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(s.text()).font(theme::bold(17.0)).color(color));
        if s.total > 0 {
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("{}% positive", s.percent_positive()))
                    .font(theme::regular(13.0))
                    .color(theme::TEXT_DIM),
            );
        }
    });
    ui.add_space(10.0);
    if r.items.is_empty() {
        ui.label(RichText::new("No written reviews in your language yet.").color(theme::TEXT_DIM));
        return;
    }
    ui.label(RichText::new("Most helpful").font(theme::bold(12.0)).color(theme::TEXT_DIM));
    ui.add_space(6.0);
    for rev in &r.items {
        let frame = egui::Frame::new().fill(theme::BG_CARD).inner_margin(egui::Margin::symmetric(14, 10));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (glyph, text, c) = if rev.recommended {
                    (icon::THUMBS_UP, "Recommended", POSITIVE)
                } else {
                    (icon::THUMBS_DOWN, "Not recommended", theme::ERROR_TEXT)
                };
                ui.label(RichText::new(format!("{glyph}  {text}")).font(theme::bold(14.0)).color(c));
                ui.add_space(10.0);
                let mut meta = format!("{:.1} hrs played", rev.hours);
                if rev.created > 0 {
                    meta.push_str(&format!("  -  {}", ymd(rev.created)));
                }
                if rev.votes_up > 0 {
                    meta.push_str(&format!("  -  {} found this helpful", sr_client::thousands(rev.votes_up)));
                }
                ui.label(RichText::new(meta).font(theme::regular(12.0)).color(theme::TEXT_DIM));
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&rev.text).font(theme::regular(13.0)).color(theme::TEXT)).wrap());
        });
        ui.add_space(6.0);
    }
}

/// Trailer thumbnails. A click opens the video in the system's player or browser.
pub(super) fn trailers(ui: &mut Ui, client: &mut Client, tex: &mut Textures, d: &AppDetails) {
    if d.movies.is_empty() {
        return;
    }
    ui.add_space(16.0);
    ui.label(RichText::new("TRAILERS").font(theme::bold(14.0)).color(theme::TEXT_HEADING));
    ui.add_space(8.0);
    let mut open = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        for m in d.movies.iter().take(8) {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(176.0, 99.0), Sense::click());
            if m.thumb.is_empty() {
                ui.painter().rect_filled(r, theme::RADIUS, theme::BG_CAPSULE);
            } else {
                image_or_placeholder(ui, client, tex, r, &m.thumb);
            }
            let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
            ui.painter().circle_filled(r.center(), 20.0, Color32::from_black_alpha((120.0 + 80.0 * t) as u8));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon::PLAY, theme::regular(20.0), Color32::WHITE);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&m.name);
            if resp.clicked() {
                open = Some(m.url.clone());
            }
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Opens in your video player.").font(theme::regular(12.0)).color(theme::TEXT_DIM));
    if let Some(u) = open {
        client.send(Command::OpenUrl(u));
    }
}

/// "2024-05-01" from Unix seconds (proleptic Gregorian, UTC).
fn ymd(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::ymd;

    #[test]
    fn dates_from_unix_seconds() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_700_000_000), "2023-11-14");
        assert_eq!(ymd(951_782_400), "2000-02-29");
    }
}
