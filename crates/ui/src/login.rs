//! Sign-in screen: QR code only (no password ever).

use crate::theme;
use egui::{Color32, Pos2, Rect, RichText, Ui, Vec2};
use sr_client::{Client, Command, LoginState};

const CARD: Vec2 = Vec2::new(760.0, 420.0);
const QR_SIZE: f32 = 220.0;

pub fn show(ui: &mut Ui, client: &mut Client, login: &LoginState) {
    let card = Rect::from_center_size(ui.max_rect().center(), CARD);
    ui.painter().rect_filled(card, theme::BUTTON_RADIUS, theme::BG_SIDEBAR);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(40.0)));
    let ui = &mut inner;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(CARD.x - 80.0 - QR_SIZE - 40.0);
            ui.label(RichText::new("Sign in").font(theme::bold(30.0)).color(Color32::WHITE));
            ui.add_space(24.0);
            ui.label(RichText::new("USE THE MOBILE APP").font(theme::bold(12.0)).color(theme::ACCENT));
            ui.add_space(8.0);
            let steps = "1. Open the Steam Mobile App on your phone.\n\
                         2. Tap the shield icon (Steam Guard).\n\
                         3. Tap \"Scan a QR code\" and point it at the code.";
            ui.label(RichText::new(steps).font(theme::regular(15.0)).color(theme::TEXT));
            ui.add_space(24.0);
            ui.label(
                RichText::new("Your password is never entered here. The sign-in token is kept in your system keyring.")
                    .font(theme::regular(13.0))
                    .color(theme::TEXT_DIM),
            );
            if let Some(err) = &login.error {
                ui.add_space(16.0);
                ui.label(RichText::new(err).color(Color32::from_rgb(0xe0, 0x6c, 0x5a)));
                ui.add_space(8.0);
                if ui.button("Try again").clicked() {
                    client.send(Command::StartQrLogin);
                }
            }
        });
        ui.add_space(40.0);
        ui.vertical(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(QR_SIZE), egui::Sense::hover());
            match &login.qr_url {
                Some(url) if !login.scanned => draw_qr(ui, rect, url),
                Some(_) => {
                    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, theme::BG_INPUT);
                    let msg = "Scanned.\nApprove the sign-in\non your phone.";
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        msg,
                        theme::medium(16.0),
                        theme::TEXT,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, theme::BG_INPUT);
                    if login.error.is_none() {
                        ui.put(Rect::from_center_size(rect.center(), Vec2::splat(32.0)), egui::Spinner::new());
                    }
                }
            }
        });
    });
}

/// Draw the QR code natively: a white plate with dark modules.
fn draw_qr(ui: &Ui, rect: Rect, url: &str) {
    let Ok((n, dark)) = sr_steam::auth::qr_matrix(url) else { return };
    let p = ui.painter();
    p.rect_filled(rect, theme::BUTTON_RADIUS, Color32::WHITE);
    let module = rect.width() / (n as f32 + 4.0);
    let origin = rect.min + Vec2::splat(module * 2.0);
    for (i, _) in dark.iter().enumerate().filter(|(_, d)| **d) {
        let (x, y) = ((i % n) as f32, (i / n) as f32);
        let min = Pos2::new(origin.x + x * module, origin.y + y * module);
        p.rect_filled(Rect::from_min_size(min, Vec2::splat(module + 0.3)), 0.0, Color32::BLACK);
    }
}
