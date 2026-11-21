//! Settings > Downloads: updates (automatic, schedule, official-client policy), bandwidth, language, the
//! default library and the number of parallel downloads. Every change is one `Command`.

use crate::widgets::{self, ButtonStyle};
use crate::{fmt, theme};
use egui::{Id, RichText, Ui};
use sr_client::{
    Client, Command, DownloadSettings, MAX_WORKERS, MIN_WORKERS, ManageCmd, SteamUpdatePolicy, UpdateCheck,
};

/// Steam's depot language names with the words shown.
pub(crate) const LANGUAGES: [(&str, &str); 13] = [
    ("english", "English"),
    ("german", "German"),
    ("french", "French"),
    ("spanish", "Spanish"),
    ("russian", "Russian"),
    ("brazilian", "Brazilian Portuguese"),
    ("italian", "Italian"),
    ("polish", "Polish"),
    ("turkish", "Turkish"),
    ("ukrainian", "Ukrainian"),
    ("schinese", "Simplified Chinese"),
    ("japanese", "Japanese"),
    ("koreana", "Korean"),
];

pub(crate) fn language_name(id: &str) -> &str {
    LANGUAGES.iter().find(|(k, _)| *k == id).map_or(id, |(_, n)| *n)
}

/// What the page remembers between frames (editing buffers).
#[derive(Clone, Default)]
struct Page {
    library_text: String,
    synced: bool,
    workers: usize,
    kbps: u32,
    editing_kbps: bool,
}

fn hour_text(h: u8) -> String {
    format!("{h:02}:00")
}

fn hour_combo(ui: &mut Ui, id: &str, value: &mut u8) {
    widgets::combo(ui, id.to_string(), 90.0, hour_text(*value), |ui| {
        for h in 0..24u8 {
            ui.selectable_value(value, h, hour_text(h));
        }
    });
}

fn toggle_row(ui: &mut Ui, label: &str, desc: &str, value: &mut bool) {
    widgets::setting_row(ui, label, desc, |ui| {
        widgets::toggle(ui, value);
    });
}

pub fn show(ui: &mut Ui, client: &mut Client) {
    let key = Id::new("settings-downloads-page");
    let mut page: Page = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let settings = client.state().settings.clone();
    if !page.synced {
        page.library_text = settings.library.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        page.workers = settings.workers;
        page.synced = true;
    }
    let mut d: DownloadSettings = settings.downloads.clone();
    let before = d.clone();

    group(ui, "Updates");
    toggle_row(
        ui,
        "Automatically update games",
        "Look for updates every 45 minutes and download them.",
        &mut d.auto_update,
    );
    toggle_row(
        ui,
        "Only update between set hours",
        "Automatic updates start only inside this window (your local time). Downloads you start yourself are not limited.",
        &mut d.schedule_enabled,
    );
    if d.schedule_enabled {
        widgets::setting_row(
            ui,
            "Update window",
            "From and to, on the clock of this computer. 22:00 to 06:00 runs overnight.",
            |ui| {
                hour_combo(ui, "sched-to", &mut d.schedule_to);
                ui.label(RichText::new("to").color(theme::TEXT_DIM));
                hour_combo(ui, "sched-from", &mut d.schedule_from);
            },
        );
    }
    let desc = "Games the official Steam client installed live in its library folders. Updating them from here writes \
                into those folders, so the official client and Kettle would both change them. \"Ask me\" asks the \
                first time; until then they are left alone.";
    widgets::setting_row(ui, "Update games installed by the Steam client", desc, |ui| {
        widgets::combo(ui, "steam-policy", 120.0, d.steam_policy.label(), |ui| {
            for p in SteamUpdatePolicy::ALL {
                ui.selectable_value(&mut d.steam_policy, p, p.label());
            }
        });
    });
    update_status(ui, client);

    group(ui, "Bandwidth");
    let mut limited = d.limit_kbps > 0;
    let desc = "Caps the total download speed of all downloads. Rates below are shown in KB/s (1 MB/s = 1024 KB/s).";
    widgets::setting_row(ui, "Limit download speed", desc, |ui| {
        if !page.editing_kbps {
            page.kbps = d.limit_kbps;
        }
        let resp = ui
            .add_enabled(limited, egui::DragValue::new(&mut page.kbps).range(64..=1_000_000).speed(64).suffix(" KB/s"));
        page.editing_kbps = resp.has_focus() || resp.dragged();
        if resp.drag_stopped() || resp.lost_focus() {
            d.limit_kbps = page.kbps.max(64);
            page.editing_kbps = false;
        }
        if widgets::toggle(ui, &mut limited).changed() {
            d.limit_kbps = if limited { page.kbps.max(8192) } else { 0 };
            page.kbps = d.limit_kbps;
        }
    });
    toggle_row(
        ui,
        "Allow downloads during gameplay",
        "Off: while a game started by Kettle runs, downloads wait and continue when it closes.",
        &mut d.allow_during_gameplay,
    );
    toggle_row(
        ui,
        "Display download rates in bits per second",
        "Shows Mbps instead of MB/s on the Downloads page.",
        &mut d.bits_per_second,
    );
    let mut workers = page.workers;
    let desc = "How many file pieces are fetched at once. More is faster on a fast line; fewer is gentler.";
    widgets::setting_row(ui, "Parallel downloads", desc, |ui| {
        // Right to left: the number first (it ends up on the right), then the slider.
        ui.label(RichText::new(workers.to_string()).font(theme::regular(14.0)).color(theme::TEXT));
        let resp = widgets::slider(ui, &mut workers, MIN_WORKERS..=MAX_WORKERS, 180.0);
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            client.send(Command::SetWorkers(workers));
        }
    });
    page.workers = workers;

    group(ui, "Installing");
    widgets::setting_row(ui, "Language of new installs", "Used when a game has files per language.", |ui| {
        let shown = language_name(d.language_or_default()).to_string();
        widgets::combo(ui, "dl-language", 190.0, shown, |ui| {
            for (id, name) in LANGUAGES {
                let on = d.language_or_default() == id;
                if ui.selectable_label(on, name).clicked() {
                    d.language = if id == "english" { String::new() } else { id.to_string() };
                }
            }
        });
    });
    let default = settings.default_library().display().to_string();
    let desc = format!("Games are installed into {default} unless you pick another folder in the install dialog.");
    widgets::setting_block(ui, "Default library folder", &desc, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::text_input(ui, &mut page.library_text, 300.0, &default);
            if widgets::button(ui, "Apply", ButtonStyle::Primary, true).clicked() {
                let text = page.library_text.trim();
                let path = (!text.is_empty()).then(|| std::path::PathBuf::from(text));
                client.send(Command::SetLibrary(path));
            }
            if widgets::button(ui, "Use default", ButtonStyle::Secondary, true).clicked() {
                page.library_text.clear();
                client.send(Command::SetLibrary(None));
            }
        });
    });
    widgets::setting_row(
        ui,
        "Clear download cache",
        "Deletes the pictures and lists Kettle keeps in its cache folder. Installed games are not touched.",
        |ui| {
            if widgets::button(ui, "Clear cache", ButtonStyle::Secondary, true).clicked() {
                client.send(Command::Manage(ManageCmd::ClearCache));
            }
        },
    );

    if d != before {
        client.send(Command::Manage(ManageCmd::SetDownloads(d)));
    }
    ui.data_mut(|data| data.insert_temp(key, page));
}

fn group(ui: &mut Ui, text: &str) {
    widgets::section_header(ui, text);
}

/// "Check now", the state of the last check, and the games that have an update.
fn update_status(ui: &mut Ui, client: &mut Client) {
    let state = client.state();
    let status = match &state.extra.update_check {
        UpdateCheck::Idle => "No check yet. It runs when you are signed in and connected.".to_string(),
        UpdateCheck::Running => "Checking...".to_string(),
        UpdateCheck::Done { at, found } => {
            let when = fmt::ago(*at, fmt::now_unix());
            if *found == 0 {
                format!("All games are up to date (checked {when}).")
            } else {
                format!("{found} game(s) have an update (checked {when}).")
            }
        }
        UpdateCheck::Failed(e) => format!("The last check failed: {e}"),
    };
    let waiting: Vec<(sr_model::AppId, String)> = state
        .extra
        .updates
        .iter()
        .map(|&id| {
            let app = sr_model::AppId(id);
            (app, state.game(app).map_or_else(|| format!("App {id}"), |g| g.name.clone()))
        })
        .collect();
    let running = state.extra.update_check == UpdateCheck::Running;
    widgets::setting_row(ui, "Check for updates", &status, |ui| {
        if widgets::button(ui, "Check now", ButtonStyle::Secondary, !running).clicked() {
            client.send(Command::Manage(ManageCmd::CheckUpdates));
        }
    });
    for (app, name) in waiting {
        widgets::setting_row(ui, &name, "Update available", |ui| {
            if widgets::button(ui, "Update", ButtonStyle::Primary, true).clicked() {
                client.send(Command::Update(app));
            }
        });
    }
}

/// Properties > General for an installed game: when it gets updates, and shortcuts.
pub fn game_rows(ui: &mut Ui, client: &mut Client, game: &sr_model::Game) {
    if !game.install.is_installed() {
        return;
    }
    let app = game.app_id;
    let current = client.state().settings.downloads.behavior_of(app);
    let mut chosen = current;
    let desc = "Applies to this game only. The schedule in Settings > Downloads still applies to automatic updates.";
    widgets::setting_row(ui, "Automatic updates", desc, |ui| {
        widgets::combo(ui, ("update-behavior", app.0), 300.0, current.label(), |ui| {
            for b in sr_client::UpdateBehavior::ALL {
                ui.selectable_value(&mut chosen, b, b.label());
            }
        });
    });
    if chosen != current {
        client.send(Command::Manage(ManageCmd::SetUpdateBehavior(app, chosen)));
    }
    let desc = "Writes a .desktop file that starts this game with Kettle (kettle --play <id>).";
    widgets::setting_row(ui, "Shortcuts", desc, |ui| {
        if widgets::button(ui, "Application menu", ButtonStyle::Secondary, true).clicked() {
            client.send(Command::Manage(ManageCmd::CreateShortcuts { app, desktop: false, menu: true }));
        }
        if widgets::button(ui, "Desktop", ButtonStyle::Secondary, true).clicked() {
            client.send(Command::Manage(ManageCmd::CreateShortcuts { app, desktop: true, menu: false }));
        }
    });
}
