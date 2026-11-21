//! The in-game overlay: a transparent, always-on-top window over the whole monitor, shown while a game we started
//! runs. The app (not this crate) registers the global shortcut and calls [`OverlayView::toggle`]; this file only
//! draws. Hidden, it costs nothing: no viewport exists and no repaint is requested.

use crate::friends::FriendsView;
use crate::textures::Textures;
use crate::{anim, fmt, theme, widgets};
use egui::{
    Align2, Color32, Context, CornerRadius, Id, Key, Margin, Modifiers, Rect, RichText, Stroke, Ui, UiBuilder,
    ViewportBuilder, ViewportCommand, ViewportId, vec2,
};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command, MAX_NOTE_CHARS};
use sr_model::AppId;
use std::time::Duration;

/// Fade-in time of the dimmed background.
const FADE_SECS: f32 = 0.15;
const MARGIN: f32 = 48.0;
const TOP_H: f32 = 72.0;
const BOTTOM_H: f32 = 96.0;
/// Opacity of the dimmed background when fully faded in.
const DIM_ALPHA: f32 = 0.84;

#[derive(Default)]
pub struct OverlayView {
    open: bool,
    /// Egui time of the first frame since opening (the fade starts there).
    since: Option<f64>,
    focus_sent: bool,
    /// The note being edited, with the game it belongs to.
    note: Option<(AppId, String)>,
}

/// "under a minute", "12 min", "1 h 05 min".
pub fn session_text(secs: u64) -> String {
    let m = secs / 60;
    match m {
        0 => "under a minute".into(),
        1..=59 => format!("{m} min"),
        _ => format!("{} h {:02} min", m / 60, m % 60),
    }
}

/// The game the overlay is about: the one that started first.
fn current_game(client: &Client) -> Option<AppId> {
    client.state().running.iter().min_by_key(|(app, g)| (g.started, app.0)).map(|(app, _)| *app)
}

impl OverlayView {
    /// Show or hide. Showing needs a running game and the setting on.
    pub fn toggle(&mut self, client: &mut Client) {
        if self.open {
            self.close(client);
        } else if client.state().settings.overlay
            && let Some(app) = current_game(client)
        {
            self.open = true;
            self.since = None;
            self.focus_sent = false;
            self.note = Some((app, client.state().note(app).to_string()));
        }
    }

    fn close(&mut self, client: &mut Client) {
        self.flush_note(client);
        self.open = false;
        self.note = None;
    }

    fn flush_note(&mut self, client: &mut Client) {
        if let Some((app, text)) = &self.note
            && client.state().note(*app) != text
        {
            client.send(Command::SetNote(*app, text.clone()));
        }
    }

    /// Draw the overlay window when it is open. Call once per frame from the main window's update.
    pub fn show(&mut self, ctx: &Context, client: &mut Client, textures: &mut Textures, friends: &mut FriendsView) {
        if !self.open {
            return;
        }
        let game = current_game(client);
        if game.is_none() || !client.state().settings.overlay {
            self.close(client);
            return;
        }
        let game = game.unwrap_or(AppId(0));
        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(vec2(1920.0, 1080.0));
        let builder = ViewportBuilder::default()
            .with_title(format!("{} overlay", theme::APP_NAME))
            .with_app_id(format!("{}-overlay", theme::APP_ID))
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_position([0.0, 0.0])
            .with_inner_size(monitor)
            .with_fullscreen(true);
        let mut hide = false;
        let started = client.state().running.get(&game).map(|g| g.started);
        let name = client.state().game(game).map_or_else(|| format!("App {}", game.0), |g| g.name.clone());
        ctx.show_viewport_immediate(ViewportId::from_hash_of("kettle-overlay"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            let now = ctx.input(|i| i.time);
            let since = *self.since.get_or_insert(now);
            if !self.focus_sent {
                self.focus_sent = true;
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            if ctx.input(|i| i.viewport().close_requested())
                || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                // When the global shortcut is not available, Shift+Tab reaches us as a plain key press.
                || ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab))
            {
                hide = true;
            }
            let fade = anim::since(&ctx, since, FADE_SECS);
            let bg = Color32::from_rgba_unmultiplied(8, 10, 14, (255.0 * DIM_ALPHA * fade) as u8);
            let panel = egui::Frame::new().fill(bg);
            egui::CentralPanel::default().frame(panel).show(ui, |ui| {
                ui.set_opacity(fade);
                let secs = started.map_or(0, |s| s.elapsed().as_secs());
                if body(ui, client, textures, friends, &mut self.note, &name, secs) {
                    hide = true;
                }
            });
            // The clock and the session time change once a minute.
            let wall = fmt::now_unix();
            let to_minute = (60 - wall % 60).min(60 - secs_of(started) % 60);
            ctx.request_repaint_after(Duration::from_secs(to_minute) + Duration::from_millis(200));
        });
        if hide {
            self.close(client);
        }
    }
}

fn secs_of(started: Option<std::time::Instant>) -> u64 {
    started.map_or(0, |s| s.elapsed().as_secs())
}

/// The overlay's content. Returns true when "Return to game" was clicked.
fn body(
    ui: &mut Ui,
    client: &mut Client,
    textures: &mut Textures,
    friends: &mut FriendsView,
    note: &mut Option<(AppId, String)>,
    name: &str,
    session_secs: u64,
) -> bool {
    let full = ui.max_rect();
    let mut leave = false;
    // ---- top bar
    let top = Rect::from_min_size(full.min + vec2(MARGIN, 0.0), vec2(full.width() - 2.0 * MARGIN, TOP_H));
    let p = ui.painter();
    p.text(
        top.left_center(),
        Align2::LEFT_CENTER,
        format!("{}  {name}", icon::GAME_CONTROLLER),
        theme::bold(26.0),
        theme::TEXT,
    );
    let offset = sr_client::local_utc_offset(fmt::now_unix());
    let clock = fmt::clock(fmt::now_unix(), offset);
    let clock_rect = p.text(
        top.right_center(),
        Align2::RIGHT_CENTER,
        format!("{} {clock}", icon::CLOCK),
        theme::medium(18.0),
        theme::TEXT,
    );
    p.text(
        clock_rect.left_center() - vec2(28.0, 0.0),
        Align2::RIGHT_CENTER,
        format!("{} Session {}", icon::TIMER, session_text(session_secs)),
        theme::medium(16.0),
        theme::TEXT_DIM,
    );
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(24)));

    // ---- columns
    let body_top = full.top() + TOP_H + 24.0;
    let body_bottom = full.bottom() - BOTTOM_H;
    let friends_rect = Rect::from_min_max(
        egui::pos2(full.right() - MARGIN - theme::FRIENDS_W, body_top),
        egui::pos2(full.right() - MARGIN, body_bottom),
    );
    let notes_right = (friends_rect.left() - 28.0).min(full.left() + MARGIN + 760.0);
    let notes_rect =
        Rect::from_min_max(egui::pos2(full.left() + MARGIN, body_top), egui::pos2(notes_right, body_bottom));

    ui.painter().rect_filled(friends_rect, CornerRadius::same(4), theme::BG_SIDEBAR);
    let mut fui = ui.new_child(UiBuilder::new().max_rect(friends_rect));
    fui.set_clip_rect(friends_rect);
    friends.show_embedded(&mut fui, client, textures);

    ui.painter().rect_filled(notes_rect, CornerRadius::same(4), theme::BG_CARD);
    let mut nui = ui.new_child(UiBuilder::new().max_rect(notes_rect.shrink(18.0)));
    if let Some((app, text)) = note {
        widgets::section_title(&mut nui, "NOTES");
        nui.label(RichText::new("Saved for this game, on this computer.").color(theme::TEXT_DIM).small());
        nui.add_space(8.0);
        let h = (nui.available_height() - 4.0).max(60.0);
        let edit = egui::TextEdit::multiline(text)
            .id(Id::new("overlay-note"))
            .hint_text("Write something for this game...")
            .font(theme::regular(15.0))
            .desired_width(f32::INFINITY)
            .char_limit(MAX_NOTE_CHARS)
            .margin(Margin::symmetric(10, 8));
        let resp = egui::ScrollArea::vertical()
            .max_height(h)
            .id_salt("overlay-note-scroll")
            .show(&mut nui, |ui| {
                ui.visuals_mut().text_cursor.blink = false;
                ui.add_sized(vec2(ui.available_width(), h - 4.0), edit)
            })
            .inner;
        if resp.lost_focus() && client.state().note(*app) != text.as_str() {
            client.send(Command::SetNote(*app, text.clone()));
        }
    }

    // ---- bottom: return button and hint
    let mid = (full.center().x, full.bottom() - BOTTOM_H / 2.0 - 8.0);
    let btn_rect = Rect::from_center_size(egui::pos2(mid.0, mid.1 - 10.0), vec2(180.0, 34.0));
    let mut bui = ui.new_child(UiBuilder::new().max_rect(btn_rect).layout(egui::Layout::top_down(egui::Align::Center)));
    if widgets::button(
        &mut bui,
        &format!("{}  Return to game", icon::ARROW_U_UP_LEFT),
        widgets::ButtonStyle::Primary,
        true,
    )
    .clicked()
    {
        leave = true;
    }
    ui.painter().text(
        egui::pos2(mid.0, mid.1 + 24.0),
        Align2::CENTER_CENTER,
        "Shift+Tab to close",
        theme::regular(13.0),
        theme::TEXT_DIM,
    );
    leave
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_text_steps() {
        assert_eq!(session_text(0), "under a minute");
        assert_eq!(session_text(59), "under a minute");
        assert_eq!(session_text(60 * 12 + 5), "12 min");
        assert_eq!(session_text(3600 + 300), "1 h 05 min");
    }
}
