//! "Run games in Gamescope": the settings section and the per-game row.
//!
//! Gamescope is Valve's micro-compositor (the Steam Deck's). A game started inside it can't be minimized or
//! pushed behind other windows by the desktop, and the Shift+Tab overlay is drawn over it by Gamescope itself.
//! Call [`show`] from a settings page and [`per_game`] from a game's properties.
#![allow(dead_code)] // until the pages call these

use crate::{theme, widgets};
use egui::{Id, RichText, Ui};
use sr_client::{
    Client, Command, FPS_LIMIT_CHOICES, GAMESCOPE_INSTALL_HINTS, GAMESCOPE_INSTALL_URL, GAMESCOPE_MAX_SHARPNESS,
    GAMESCOPE_RESOLUTIONS, GamescopeDetection, GamescopeOverride, GamescopeUpscaler, GamescopeWindow,
};
use sr_model::AppId;
use std::time::{Duration, Instant};

/// How long a look at the disk for Gamescope is reused.
const DETECT_EVERY: Duration = Duration::from_secs(5);

fn detection(ui: &Ui) -> GamescopeDetection {
    let id = Id::new("gamescope-detect");
    let cached: Option<(Instant, GamescopeDetection)> = ui.ctx().data(|d| d.get_temp(id));
    if let Some((at, d)) = cached
        && at.elapsed() < DETECT_EVERY
    {
        return d;
    }
    let d = sr_client::detect_gamescope();
    ui.ctx().data_mut(|m| m.insert_temp(id, (Instant::now(), d.clone())));
    d
}

fn toggle_row(ui: &mut Ui, label: &str, desc: &str, value: &mut bool) {
    widgets::setting_row(ui, label, desc, |ui| {
        widgets::toggle(ui, value);
    });
}

fn size_text(w: u32, h: u32) -> String {
    if w == 0 || h == 0 { "Same as the screen".to_string() } else { format!("{w} x {h}") }
}

fn size_row(ui: &mut Ui, id: &str, label: &str, desc: &str, w: &mut u32, h: &mut u32, auto: &str) {
    widgets::setting_row(ui, label, desc, |ui| {
        let text = if *w == 0 || *h == 0 { auto.to_string() } else { size_text(*w, *h) };
        widgets::combo(ui, id.to_string(), 220.0, text, |ui| {
            for (rw, rh) in GAMESCOPE_RESOLUTIONS {
                let name = if rw == 0 { auto.to_string() } else { size_text(rw, rh) };
                if ui.selectable_label((*w, *h) == (rw, rh), name).clicked() {
                    (*w, *h) = (rw, rh);
                }
            }
        });
    });
}

/// Where to get it, one line per package manager.
fn install_hint() -> String {
    format!(
        "Gamescope is not installed. Install it: {}  More: {GAMESCOPE_INSTALL_URL}",
        GAMESCOPE_INSTALL_HINTS.iter().map(|(d, c)| format!("{d}: {c}.")).collect::<Vec<_>>().join("  ")
    )
}

/// The settings section (put it under a section header such as "Gamescope").
pub fn show(ui: &mut Ui, client: &mut Client) {
    let found = detection(ui);
    match (&found.binary, &found.version) {
        (None, _) => {
            widgets::setting_row(ui, "Gamescope is not installed", &install_hint(), |_| {});
            if found.flatpak {
                let desc = "The Flatpak version (org.freedesktop.Platform.VulkanLayer.gamescope) is for Flatpak \
                            Steam. Games started here don't use it; install the normal package.";
                widgets::setting_row(ui, "Flatpak Gamescope found", desc, |_| {});
            }
        }
        (Some(path), version) => {
            let v = version.as_deref().map_or(String::new(), |v| format!("version {v}, "));
            widgets::setting_row(ui, "Gamescope", &format!("Installed: {v}{}", path.display()), |_| {});
        }
    }
    let before = client.state().settings.gamescope.clone();
    let mut g = before.clone();
    let desc = "Starts each game inside Gamescope, as on the Steam Deck. The desktop can't minimize the game or \
                cover it, and the Shift+Tab overlay is drawn over it. A game can follow this or choose for \
                itself in its properties. Applies from the next game start.";
    toggle_row(ui, "Run games in Gamescope", desc, &mut g.enabled);
    if g.enabled {
        widgets::setting_row(ui, "Window", "", |ui| {
            widgets::combo(ui, "gs-window", 220.0, g.window.label(), |ui| {
                for m in GamescopeWindow::ALL {
                    ui.selectable_value(&mut g.window, m, m.label());
                }
            });
        });
        let desc = "The size of Gamescope's window. Leave it as the screen unless you want a smaller window.";
        size_row(ui, "gs-out", "Window size", desc, &mut g.output_width, &mut g.output_height, "Same as the screen");
        let desc = "What the game renders at; Gamescope scales it up to the window. Lower is faster.";
        size_row(ui, "gs-game", "Game resolution", desc, &mut g.game_width, &mut g.game_height, "Same as the window");
        widgets::setting_row(ui, "Upscaling", "How a lower game resolution is scaled up.", |ui| {
            widgets::combo(ui, "gs-upscaler", 220.0, g.upscaler.label(), |ui| {
                for u in GamescopeUpscaler::ALL {
                    ui.selectable_value(&mut g.upscaler, u, u.label());
                }
            });
        });
        if g.upscaler.sharpens() {
            widgets::setting_row(ui, "Sharpness", "0 is the sharpest, 20 the softest.", |ui| {
                ui.label(RichText::new(g.sharpness.to_string()).font(theme::regular(14.0)).color(theme::TEXT));
                widgets::slider(ui, &mut g.sharpness, 0..=GAMESCOPE_MAX_SHARPNESS, 180.0);
            });
        }
        let desc = "Caps the frame rate in Gamescope. With none here, the performance overlay's limit is used.";
        widgets::setting_row(ui, "Frame rate limit", desc, |ui| {
            let custom = !FPS_LIMIT_CHOICES.contains(&g.fps_limit);
            let text = if g.fps_limit == 0 { "None".to_string() } else { format!("{} fps", g.fps_limit) };
            if custom {
                ui.add(egui::DragValue::new(&mut g.fps_limit).range(1..=1000).suffix(" fps"));
            }
            widgets::combo(ui, "gs-fps", 130.0, text, |ui| {
                for v in FPS_LIMIT_CHOICES {
                    let name = if v == 0 { "None".to_string() } else { format!("{v} fps") };
                    ui.selectable_value(&mut g.fps_limit, v, name);
                }
                if ui.selectable_label(custom, "Custom").clicked() && !custom {
                    g.fps_limit = 90;
                }
            });
        });
        toggle_row(ui, "Adaptive sync", "Variable refresh rate, when the screen supports it.", &mut g.adaptive_sync);
        let desc = "Needs an HDR screen and a desktop with HDR on. Only turn it on if you want HDR.";
        toggle_row(ui, "HDR", desc, &mut g.hdr);
        let desc = "Always capture the mouse inside the game. Turn it on if the mouse leaves the window or the \
                    game doesn't get it.";
        toggle_row(ui, "Always grab the mouse", desc, &mut g.grab_cursor);
        if found.installed() && !found.mangoapp {
            let desc = "The performance overlay works through the MangoHud layer in the game; Gamescope's own \
                        version needs the mangoapp program (part of the MangoHud package).";
            widgets::setting_row(ui, "Performance overlay", desc, |_| {});
        }
    }
    if g != before {
        client.send(Command::SetGamescope(g));
    }
}

/// One row for a game's properties: follow the global setting, always, or never.
pub fn per_game(ui: &mut Ui, client: &mut Client, app: AppId) {
    let global = client.state().settings.gamescope.enabled;
    let current = client.state().gamescope_games.get(app.0);
    let mut choice = current;
    let desc = format!(
        "Run this game inside Gamescope. The global setting is {}.{}",
        if global { "on" } else { "off" },
        if detection(ui).installed() { "" } else { " Gamescope is not installed." }
    );
    widgets::setting_row(ui, "Gamescope", &desc, |ui| {
        widgets::combo(ui, ("gs-game-choice", app.0), 230.0, choice.label(), |ui| {
            for o in GamescopeOverride::ALL {
                ui.selectable_value(&mut choice, o, o.label());
            }
        });
    });
    if choice != current {
        client.send(Command::SetGameGamescope(app, choice));
    }
}
