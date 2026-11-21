//! The collection parts of the game menu, and the "New collection" prompt.
//! View state only: every change is a `Command::Collections`.

use crate::theme;
use crate::widgets::{self, ButtonStyle};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, CollCmd, Command};
use sr_model::{AppId, Game};

/// The name being typed, and the game it is for (it goes into the new collection at once).
#[derive(Clone, Default)]
struct Prompt {
    app: Option<AppId>,
    name: String,
    focus: bool,
}

fn prompt_id() -> Id {
    Id::new("new-collection-prompt")
}

/// Open the "New collection" prompt (from any menu).
pub fn open_prompt(ctx: &Context, app: Option<AppId>) {
    ctx.data_mut(|d| d.insert_temp(prompt_id(), Prompt { app, name: String::new(), focus: true }));
}

fn send(client: &mut Client, cmd: CollCmd) {
    client.send(Command::Collections(cmd));
}

/// Favorite, hide and "Add to" items of the right-click menu.
pub fn menu_items(ui: &mut Ui, client: &mut Client, game: &Game) {
    let app = game.app_id;
    let fav = client.state().collections.is_favorite(app);
    let hidden = client.state().collections.is_hidden(app);
    if ui.button(if fav { "Remove from Favorites" } else { "Add to Favorites" }).clicked() {
        send(client, CollCmd::SetFavorite(app, !fav));
        ui.close();
    }
    if ui.button(if hidden { "Unhide this game" } else { "Hide this game" }).clicked() {
        send(client, CollCmd::SetHidden(app, !hidden));
        ui.close();
    }
    let mine: Vec<(String, String, bool)> = client
        .state()
        .collections
        .ours_for(app)
        .into_iter()
        .map(|(id, name, has)| (id.to_string(), name.to_string(), has))
        .collect();
    ui.menu_button("Add to", |ui| {
        ui.set_min_width(theme::MENU_MIN_W);
        for (id, name, has) in &mine {
            let label = if *has { format!("\u{2713}  {name}") } else { format!("     {name}") };
            if ui.button(label).clicked() {
                send(client, CollCmd::SetMember { collection: id.clone(), app, on: !has });
                ui.close();
            }
        }
        if !mine.is_empty() {
            widgets::menu_separator(ui);
        }
        if ui.button("New collection...").clicked() {
            open_prompt(ui.ctx(), Some(app));
            ui.close();
        }
    });
}

/// The prompt window, if open. Call once per frame.
pub fn show_prompt(ctx: &Context, client: &mut Client) {
    let Some(mut p) = ctx.data(|d| d.get_temp::<Prompt>(prompt_id())) else { return };
    let frame = egui::Frame::new()
        .fill(theme::BG_SIDEBAR)
        .inner_margin(egui::Margin::same(20))
        .corner_radius(theme::BUTTON_RADIUS)
        .stroke(egui::Stroke::new(1.0, theme::WINDOW_BORDER));
    let (mut create, mut cancel) = (false, false);
    let modal = egui::Modal::new(prompt_id()).frame(frame).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new("New collection").font(theme::bold(18.0)).color(theme::TEXT));
        ui.add_space(10.0);
        let edit = egui::TextEdit::singleline(&mut p.name)
            .hint_text("Name")
            .font(theme::regular(14.0))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6));
        let resp = ui.add(edit);
        if p.focus {
            resp.request_focus();
            p.focus = false;
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let ok = !p.name.trim().is_empty();
            create = widgets::button(ui, "Create", ButtonStyle::Primary, ok).clicked() || (enter && ok);
            ui.add_space(8.0);
            cancel = widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked();
        });
    });
    if create {
        send(client, CollCmd::New { name: p.name.clone(), app: p.app });
    }
    if create || cancel || modal.should_close() {
        ctx.data_mut(|d| d.remove::<Prompt>(prompt_id()));
    } else {
        ctx.data_mut(|d| d.insert_temp(prompt_id(), p));
    }
}
