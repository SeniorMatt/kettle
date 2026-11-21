//! Per-game environment variables: switches for the common ones and a text box for the rest.
//!
//! Hook: call [`section`] in the game's Properties window (Compatibility tab), after the launch options.
//! Text edits are kept in view state until "Apply", so half-typed lines are never saved.

use crate::theme;
use crate::widgets::{self, ButtonStyle};
use egui::{Id, RichText, Ui};
use sr_client::{Client, Command, env_to_text, parse_env_text};
use sr_model::AppId;

#[derive(Clone)]
struct Edit {
    text: String,
    /// The saved text this edit started from; when the saved value changes elsewhere the edit restarts.
    from: String,
}

pub fn section(ui: &mut Ui, client: &mut Client, app: AppId) {
    let saved = client.state().game_env.get(app);
    let gamemode_installed = client.state().game_env.gamemode_installed;
    let mut env = saved.clone();
    widgets::section_header(ui, "Environment variables");
    widgets::setting_row(ui, "Show the DXVK frame rate", "Sets DXVK_HUD=fps.", |ui| {
        widgets::toggle(ui, &mut env.dxvk_hud_fps);
    });
    widgets::setting_row(ui, "Enable NVAPI", "Sets PROTON_ENABLE_NVAPI=1 (DLSS on Nvidia cards).", |ui| {
        widgets::toggle(ui, &mut env.nvapi);
    });
    widgets::setting_row(ui, "Use WineD3D instead of DXVK", "Sets PROTON_USE_WINED3D=1 (OpenGL).", |ui| {
        widgets::toggle(ui, &mut env.wined3d);
    });
    let desc = if gamemode_installed {
        "Starts the game through gamemoderun."
    } else {
        "GameMode is not installed (gamemoderun was not found), so this does nothing yet."
    };
    widgets::setting_row(ui, "Use GameMode", desc, |ui| {
        widgets::toggle(ui, &mut env.gamemode);
    });

    let id = Id::new(("game-env-edit", app.0));
    let saved_text = env_to_text(&saved.vars);
    let mut edit: Edit = ui
        .data(|d| d.get_temp::<Edit>(id))
        .filter(|e| e.from == saved_text)
        .unwrap_or_else(|| Edit { text: saved_text.clone(), from: saved_text.clone() });
    let desc = "One KEY=VALUE per line. They are added when the game starts and win over the switches above. \
                Names: letters, digits and _.";
    widgets::setting_block(ui, "Your own variables", desc, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut edit.text)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .hint_text("DXVK_ASYNC=1")
                .font(theme::regular(theme::BTN_FONT)),
        );
        match parse_env_text(&edit.text) {
            Err(e) => {
                ui.label(RichText::new(e).font(theme::regular(12.5)).color(theme::ERROR_TEXT));
            }
            Ok(vars) => {
                ui.add_space(6.0);
                let changed = vars != saved.vars;
                if widgets::button(ui, "Apply", ButtonStyle::Primary, changed).clicked() {
                    env.vars = vars;
                }
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, edit));
    if env != saved {
        client.send(Command::SetGameEnv { app, env });
    }
}
