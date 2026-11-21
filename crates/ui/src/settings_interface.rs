//! Settings > Interface: animations, UI scale, start page, sidebar icons, scrolling and the tray.

use crate::widgets;
use egui::Ui;
use sr_client::{AnimationMode, Client, Command, MAX_SCALE_PERCENT, MIN_SCALE_PERCENT, StartPage};

fn mode_label(m: AnimationMode) -> &'static str {
    match m {
        AnimationMode::Full => "Full",
        AnimationMode::Reduced => "Reduced (fades only)",
        AnimationMode::Off => "Off",
    }
}

fn page_label(p: StartPage) -> &'static str {
    match p {
        StartPage::Library => "Library",
        StartPage::Store => "Store",
    }
}

fn toggle_row(ui: &mut Ui, label: &str, desc: &str, value: &mut bool) -> bool {
    let mut changed = false;
    widgets::setting_row(ui, label, desc, |ui| changed = widgets::toggle(ui, value).changed());
    changed
}

pub fn show(ui: &mut Ui, client: &mut Client) {
    let mut i = client.state().settings.interface.clone();
    let before = i.clone();

    widgets::setting_row(
        ui,
        "Animations",
        "Reduced keeps fades but nothing moves. Off changes everything at once.",
        |ui| {
            widgets::combo(ui, "anim-mode", 190.0, mode_label(i.animations), |ui| {
                for m in [AnimationMode::Full, AnimationMode::Reduced, AnimationMode::Off] {
                    ui.selectable_value(&mut i.animations, m, mode_label(m));
                }
            });
        },
    );
    widgets::setting_row(ui, "Interface scale", "Also Ctrl + and Ctrl - in the window.", |ui| {
        widgets::combo(ui, "ui-scale", 110.0, format!("{} %", i.scale_percent), |ui| {
            for pct in (MIN_SCALE_PERCENT..=MAX_SCALE_PERCENT).step_by(10) {
                ui.selectable_value(&mut i.scale_percent, pct, format!("{pct} %"));
            }
        });
    });
    widgets::setting_row(ui, "Start page", "What opens when Kettle starts.", |ui| {
        widgets::combo(ui, "start-page", 140.0, page_label(i.start_page), |ui| {
            for p in [StartPage::Library, StartPage::Store] {
                ui.selectable_value(&mut i.start_page, p, page_label(p));
            }
        });
    });
    toggle_row(
        ui,
        "Game icons in the sidebar",
        "Small icons next to the names in the library list.",
        &mut i.sidebar_icons,
    );
    toggle_row(
        ui,
        "Show Kettle in the tray",
        "A tray icon with shortcuts to the Store, Library, Friends, Downloads and Settings.",
        &mut i.tray,
    );
    if i.tray {
        toggle_row(
            ui,
            "Closing the window keeps Kettle in the tray",
            "Exit in the Kettle menu or the tray menu really quits.",
            &mut i.close_to_tray,
        );
        toggle_row(
            ui,
            "Start minimized to the tray",
            "Kettle starts without opening its window.",
            &mut i.start_minimized,
        );
    }

    if i != before {
        client.send(Command::SetInterface(i));
    }
}
