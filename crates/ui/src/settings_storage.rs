//! Settings > Library (library folders: add, remove, make default) and Settings > Storage (per folder: space
//! used and free, the installed games by size, move and uninstall).

use crate::widgets::{self, ButtonStyle};
use crate::{fmt, theme};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, Command, LibraryChoice, ManageCmd, library_of_path};
use sr_model::{AppId, InstallState};
use std::collections::HashSet;
use std::path::PathBuf;

/// Forget which folders were asked about, so the next time a page opens it measures free space again.
pub fn reset(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<Asked>(Id::new("storage-asked")));
}

#[derive(Clone, Default)]
struct Asked(HashSet<PathBuf>);

/// Ask once per page visit how much space each library has left.
fn ask_free_space(ui: &Ui, client: &mut Client) {
    let key = Id::new("storage-asked");
    let mut asked: Asked = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    for l in client.state().libraries.clone() {
        if asked.0.insert(l.path.clone()) {
            client.send(Command::CheckFree(l.path));
        }
    }
    ui.data_mut(|d| d.insert_temp(key, asked));
}

fn free_of(client: &Client, path: &std::path::Path) -> Option<u64> {
    client.state().free_space.iter().find(|(p, _)| p == path).and_then(|(_, f)| *f)
}

// ---- library folders ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FoldersPage {
    new_path: String,
}

pub fn folders(ui: &mut Ui, client: &mut Client) {
    ask_free_space(ui, client);
    let key = Id::new("settings-folders-page");
    let mut page: FoldersPage = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let desc = "Folders games can be installed into. Folders of the official Steam client are listed too: you can \
                install into them from the install dialog, but they are managed by that client and can't be removed \
                here.";
    ui.label(RichText::new(desc).font(theme::regular(12.5)).color(theme::TEXT_DIM));
    ui.add_space(6.0);
    let settings = client.state().settings.clone();
    let default = settings.default_library();
    for l in client.state().libraries.clone() {
        let removable = settings.downloads.libraries.contains(&l.path);
        let is_default = l.path == default;
        let mut caption =
            if l.ours { "Kettle library".to_string() } else { "Steam library folder (read-only here)".to_string() };
        match free_of(client, &l.path) {
            Some(b) => caption.push_str(&format!("  |  {} free", fmt::bytes(b))),
            None => caption.push_str("  |  free space unknown"),
        }
        if is_default {
            caption.push_str("  |  default");
        }
        widgets::setting_row(ui, &l.path.display().to_string(), &caption, |ui| {
            if removable && widgets::button(ui, "Remove", ButtonStyle::Secondary, true).clicked() {
                client.send(Command::Manage(ManageCmd::RemoveLibrary(l.path.clone())));
            }
            if l.ours && !is_default && widgets::button(ui, "Make default", ButtonStyle::Secondary, true).clicked() {
                client.send(Command::SetLibrary(Some(l.path.clone())));
            }
        });
    }
    widgets::setting_block(
        ui,
        "Add a library folder",
        "An absolute path. The folder is created if needed, with a steamapps folder in it, the shape the official client reads too.",
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::text_input(ui, &mut page.new_path, 300.0, "/mnt/games/KettleLibrary");
                let text = page.new_path.trim().to_string();
                if widgets::button(ui, "Add folder", ButtonStyle::Primary, !text.is_empty()).clicked() {
                    client.send(Command::Manage(ManageCmd::AddLibrary(PathBuf::from(text))));
                    page.new_path.clear();
                }
            });
        },
    );
    ui.data_mut(|d| d.insert_temp(key, page));
}

// ---- storage ------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Size,
    Name,
}

#[derive(Clone, Default)]
struct StoragePage {
    sort: Sort,
    /// A row waiting for "Uninstall?" confirmation.
    uninstall: Option<AppId>,
    /// A move waiting for confirmation: the game and the target library.
    mover: Option<(AppId, PathBuf)>,
    /// Chosen move target per game (index into the library list).
    target: std::collections::HashMap<u32, usize>,
}

struct Row {
    app: AppId,
    name: String,
    bytes: u64,
    needs_update: bool,
}

pub fn storage(ui: &mut Ui, client: &mut Client) {
    ask_free_space(ui, client);
    let key = Id::new("settings-storage-page");
    let mut page: StoragePage = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    move_status(ui, client);
    widgets::setting_row(ui, "Sort games by", "", |ui| {
        widgets::combo(ui, "storage-sort", 120.0, if page.sort == Sort::Size { "Size" } else { "Name" }, |ui| {
            ui.selectable_value(&mut page.sort, Sort::Size, "Size");
            ui.selectable_value(&mut page.sort, Sort::Name, "Name");
        });
    });
    let libs = client.state().libraries.clone();
    for lib in &libs {
        library_block(ui, client, &mut page, lib, &libs);
    }
    ui.data_mut(|d| d.insert_temp(key, page));
}

fn rows_of(client: &Client, lib: &LibraryChoice, sort: Sort) -> Vec<Row> {
    let mut rows: Vec<Row> = client
        .state()
        .games
        .iter()
        .filter_map(|g| match &g.install {
            InstallState::Installed { size_bytes, path } | InstallState::UpdateRequired { size_bytes, path } => {
                (library_of_path(path).as_deref() == Some(lib.path.as_path())).then(|| Row {
                    app: g.app_id,
                    name: g.name.clone(),
                    bytes: *size_bytes,
                    needs_update: matches!(g.install, InstallState::UpdateRequired { .. }),
                })
            }
            _ => None,
        })
        .collect();
    match sort {
        Sort::Size => rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name))),
        Sort::Name => rows.sort_by_key(|r| r.name.to_lowercase()),
    }
    rows
}

fn library_block(
    ui: &mut Ui,
    client: &mut Client,
    page: &mut StoragePage,
    lib: &LibraryChoice,
    libs: &[LibraryChoice],
) {
    let rows = rows_of(client, lib, page.sort);
    let used: u64 = rows.iter().map(|r| r.bytes).sum();
    let free = free_of(client, &lib.path);
    ui.add_space(18.0);
    ui.label(RichText::new(lib.path.display().to_string()).font(theme::medium(14.0)).color(egui::Color32::WHITE));
    let kind = if lib.ours { "Kettle library" } else { "Steam library folder" };
    let caption = match free {
        Some(f) => format!("{kind}  |  {} used by games, {} free", fmt::bytes(used), fmt::bytes(f)),
        None => format!("{kind}  |  {} used by games", fmt::bytes(used)),
    };
    ui.label(RichText::new(caption).font(theme::regular(12.5)).color(theme::TEXT_DIM));
    ui.add_space(4.0);
    let total = used.saturating_add(free.unwrap_or(0));
    let fraction = if total == 0 { 0.0 } else { used as f32 / total as f32 };
    widgets::progress_bar(ui, ui.available_width(), 6.0, fraction, theme::ACCENT);
    ui.add_space(4.0);
    if rows.is_empty() {
        ui.label(RichText::new("No games installed here.").color(theme::TEXT_UNINSTALLED));
    }
    for r in rows {
        game_row(ui, client, page, &r, lib, libs);
    }
}

fn game_row(
    ui: &mut Ui,
    client: &mut Client,
    page: &mut StoragePage,
    r: &Row,
    lib: &LibraryChoice,
    libs: &[LibraryChoice],
) {
    let busy = client.state().running.contains_key(&r.app)
        || client.state().downloads.status(r.app) != sr_client::DownloadStatus::None
        || client.state().extra.moving.as_ref().is_some_and(|m| m.result.is_none());
    let mut caption = fmt::bytes(r.bytes);
    if r.needs_update {
        caption.push_str("  |  update available");
    }
    let confirming_uninstall = page.uninstall == Some(r.app);
    let confirming_move = page.mover.as_ref().filter(|(a, _)| *a == r.app).map(|(_, p)| p.clone());
    widgets::setting_row(ui, &r.name, &caption, |ui| {
        if confirming_uninstall {
            if widgets::button(ui, "Keep", ButtonStyle::Secondary, true).clicked() {
                page.uninstall = None;
            }
            if widgets::button(ui, "Delete", ButtonStyle::Danger, true).clicked() {
                client.send(Command::Uninstall { app: r.app, confirmed: true });
                page.uninstall = None;
            }
        } else if let Some(to) = &confirming_move {
            let steam = !lib.ours || libs.iter().any(|l| l.path == *to && !l.ours);
            let text = if steam { "Move (touches Steam's folder)" } else { "Move" };
            if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                page.mover = None;
            }
            if widgets::button(ui, text, ButtonStyle::Primary, true).clicked() {
                client.send(Command::Manage(ManageCmd::MoveGame { app: r.app, to: to.clone(), confirmed: true }));
                page.mover = None;
            }
        } else {
            if widgets::button(ui, "Uninstall", ButtonStyle::Secondary, !busy).clicked() {
                page.uninstall = Some(r.app);
            }
            let others: Vec<(usize, &LibraryChoice)> =
                libs.iter().enumerate().filter(|(_, l)| l.path != lib.path).collect();
            if !others.is_empty() {
                let pick = page.target.entry(r.app.0).or_insert(others[0].0);
                if !others.iter().any(|(i, _)| i == pick) {
                    *pick = others[0].0;
                }
                if widgets::button(ui, "Move", ButtonStyle::Secondary, !busy).clicked() {
                    page.mover = Some((r.app, libs[*pick].path.clone()));
                }
                let shown = libs[*pick].path.display().to_string();
                widgets::combo(ui, ("move-to", r.app.0), 190.0, shown, |ui| {
                    for (i, l) in &others {
                        ui.selectable_value(pick, *i, l.path.display().to_string());
                    }
                });
            }
        }
    });
}

/// The running or finished move, with its progress bar.
fn move_status(ui: &mut Ui, client: &mut Client) {
    let Some(m) = client.state().extra.moving.clone() else { return };
    egui::Frame::new()
        .fill(theme::BG_CARD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .corner_radius(theme::RADIUS)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &m.result {
                None => {
                    ui.horizontal(|ui| {
                        let text = format!(
                            "Moving {} to {}  ({} of {})",
                            m.name,
                            m.to.display(),
                            fmt::bytes(m.done),
                            fmt::bytes(m.total)
                        );
                        ui.label(RichText::new(text).color(theme::TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                                client.send(Command::Manage(ManageCmd::CancelMove));
                            }
                        });
                    });
                    widgets::progress_bar(ui, ui.available_width(), 6.0, m.fraction(), theme::ACCENT);
                }
                Some(result) => {
                    ui.horizontal(|ui| {
                        match result {
                            Ok(()) => ui.label(RichText::new(format!("{} was moved.", m.name)).color(theme::VERIFY)),
                            Err(e) => ui.label(RichText::new(format!("Move failed: {e}")).color(theme::ERROR_TEXT)),
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, "OK", ButtonStyle::Secondary, true).clicked() {
                                client.send(Command::Manage(ManageCmd::DismissMove));
                            }
                        });
                    });
                }
            }
        });
}
