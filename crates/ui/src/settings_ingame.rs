//! Settings > In Game: the Shift+Tab overlay, the Steam client for games that need it, and the performance
//! overlay (FPS counter and more, drawn by MangoHud).

use crate::{theme, widgets};
use egui::{Id, RichText, Ui};
use sr_client::{
    Client, Command, FPS_LIMIT_CHOICES, HUD_INSTALL_HINTS, HUD_INSTALL_URL, HudColor, HudDetection, HudFontSize,
    HudPosition, HudPreset,
};
use std::time::{Duration, Instant};

/// How long a look at the disk for MangoHud is reused.
const DETECT_EVERY: Duration = Duration::from_secs(5);

/// Key combinations offered for showing and hiding the HUD in the game (MangoHud key names).
const TOGGLE_KEYS: [(&str, &str); 5] = [
    ("Shift_L+F12", "Left Shift + F12"),
    ("Shift_R+F12", "Right Shift + F12"),
    ("Control_L+F12", "Left Ctrl + F12"),
    ("Alt_L+F12", "Left Alt + F12"),
    ("F12", "F12"),
];

fn detection(ui: &Ui) -> HudDetection {
    let id = Id::new("hud-detect");
    let cached: Option<(Instant, HudDetection)> = ui.ctx().data(|d| d.get_temp(id));
    if let Some((at, d)) = cached
        && at.elapsed() < DETECT_EVERY
    {
        return d;
    }
    let d = sr_client::detect_hud();
    ui.ctx().data_mut(|m| m.insert_temp(id, (Instant::now(), d.clone())));
    d
}

fn toggle_row(ui: &mut Ui, label: &str, desc: &str, value: &mut bool) -> bool {
    let mut changed = false;
    widgets::setting_row(ui, label, desc, |ui| changed = widgets::toggle(ui, value).changed());
    changed
}

pub fn show(ui: &mut Ui, client: &mut Client) {
    let mut overlay = client.state().settings.overlay;
    let desc = "While a game started here runs, Shift+Tab shows friends, chat, notes and the performance settings \
                over it. The shortcut is only registered while such a game runs.";
    if toggle_row(ui, "Enable the in-game overlay (Shift+Tab)", desc, &mut overlay) {
        client.send(Command::SetOverlay(overlay));
    }
    widgets::setting_row(
        ui,
        "Overlay shortcut",
        "Shift+Tab shows and hides the overlay. It cannot be changed yet.",
        |ui| {
            ui.label(RichText::new("Shift+Tab").font(theme::medium(14.0)).color(theme::TEXT));
        },
    );
    let mut start = client.state().settings.start_steam;
    let desc = "Most games use the Steamworks API, which only works while the official Steam client runs. We start \
                it minimized before such a game. Nothing is sent to it; it is only started.";
    if toggle_row(ui, "Start the Steam client for games that need it", desc, &mut start) {
        client.send(Command::SetStartSteam(start));
    }
    widgets::section_header(ui, "Performance overlay");
    performance(ui, client);
    widgets::section_header(ui, "Gamescope");
    crate::settings_gamescope::show(ui, client);
}

fn performance(ui: &mut Ui, client: &mut Client) {
    let found = detection(ui);
    if !found.installed() {
        let desc = format!(
            "MangoHud draws the FPS counter inside the game, and it is not installed. {}  More: {HUD_INSTALL_URL}",
            HUD_INSTALL_HINTS.iter().map(|(d, c)| format!("{d}: {c}.")).collect::<Vec<_>>().join("  ")
        );
        widgets::setting_row(ui, "MangoHud is not installed", &desc, |_| {});
    }
    let before = client.state().settings.hud.clone();
    let mut h = before.clone();
    let desc = "Shown inside the game. Changes apply the next time a game starts; the key below shows or hides it \
                while playing.";
    widgets::setting_row(ui, "Show", desc, |ui| {
        widgets::combo(ui, "hud-preset", 230.0, h.preset.label(), |ui| {
            for p in HudPreset::ALL {
                ui.selectable_value(&mut h.preset, p, p.label());
            }
        });
    });
    if h.preset == HudPreset::Custom {
        let mut items = h.custom;
        for (name, field) in items.fields() {
            toggle_row(ui, name, "", field);
        }
        h.custom = items;
    }
    if h.preset != HudPreset::Off {
        widgets::setting_row(ui, "Position", "", |ui| {
            widgets::combo(ui, "hud-pos", 190.0, h.position.label(), |ui| {
                for p in HudPosition::ALL {
                    ui.selectable_value(&mut h.position, p, p.label());
                }
            });
        });
        widgets::setting_row(ui, "Text size", "", |ui| {
            widgets::combo(ui, "hud-font", 190.0, h.font.label(), |ui| {
                for p in HudFontSize::ALL {
                    ui.selectable_value(&mut h.font, p, p.label());
                }
            });
        });
        widgets::setting_row(ui, "Text colour", "", |ui| {
            widgets::combo(ui, "hud-color", 190.0, h.color.label(), |ui| {
                for p in HudColor::ALL {
                    ui.selectable_value(&mut h.color, p, p.label());
                }
            });
        });
        widgets::setting_row(ui, "Background opacity", "", |ui| {
            ui.label(RichText::new(format!("{}%", h.opacity)).font(theme::regular(14.0)).color(theme::TEXT));
            widgets::slider(ui, &mut h.opacity, 0..=100, 180.0);
        });
        widgets::setting_row(ui, "Show / hide key", "Pressed while playing. Handled by MangoHud.", |ui| {
            let current =
                TOGGLE_KEYS.iter().find(|(k, _)| *k == h.toggle_key).map_or(h.toggle_key.as_str(), |(_, l)| *l);
            widgets::combo(ui, "hud-key", 190.0, current.to_string(), |ui| {
                for (key, name) in TOGGLE_KEYS {
                    ui.selectable_value(&mut h.toggle_key, key.to_string(), name);
                }
            });
        });
    }
    let desc = "Caps the frame rate (MangoHud's fps_limit). It applies while the performance overlay is on.";
    widgets::setting_row(ui, "Frame rate limit", desc, |ui| {
        let custom = !FPS_LIMIT_CHOICES.contains(&h.fps_limit);
        let text = if h.fps_limit == 0 { "None".to_string() } else { format!("{} fps", h.fps_limit) };
        // Right to left: the number box (custom values only) first, then the list.
        if custom {
            ui.add(egui::DragValue::new(&mut h.fps_limit).range(1..=1000).suffix(" fps"));
        }
        widgets::combo(ui, "hud-limit", 130.0, text, |ui| {
            for v in FPS_LIMIT_CHOICES {
                let name = if v == 0 { "None".to_string() } else { format!("{v} fps") };
                ui.selectable_value(&mut h.fps_limit, v, name);
            }
            if ui.selectable_label(custom, "Custom").clicked() && !custom {
                h.fps_limit = 90;
            }
        });
    });
    if h != before {
        client.send(Command::SetHud(h));
    }
}
