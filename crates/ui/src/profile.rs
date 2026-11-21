//! The profile page: anyone's public profile, and a form to edit your own.
//!
//! Pure view code. The data comes from `client.state().profile` (see `sr_client::ProfileState`); actions go
//! out as commands. The avatar picture is the full-size one from the community CDN.

use crate::textures::{Art, Textures};
use crate::{fmt, theme, widgets};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};
use sr_client::{
    Client, Command, EditPhase, EditableProfile, FriendAction, NAME_LOADING, Privacy, ProfileEntry, ProfileLoad,
    avatar_full_url,
};
use sr_model::SteamId;

const AVATAR: f32 = 132.0;
const PAGE_W: f32 = 760.0;

#[derive(Default)]
pub struct ProfileView {
    /// The profile we already asked for (so a missing one is requested once, not every frame).
    asked: Option<u64>,
    /// The edit form's fields while editing.
    draft: Option<EditableProfile>,
}

/// "12.5 hrs" style text for hours played.
fn hours(h: f32) -> String {
    if h >= 100.0 { format!("{h:.0} hrs") } else { format!("{h:.1} hrs") }
}

/// "3 h 20 min" style text for minutes of play time.
fn minutes(m: u32) -> String {
    if m < 60 { format!("{m} min") } else { format!("{:.0} hrs", f64::from(m) / 60.0) }
}

impl ProfileView {
    pub fn show(&mut self, ui: &mut Ui, client: &mut Client, tex: &mut Textures) {
        let mut cmds: Vec<Command> = Vec::new();
        self.contents(ui, client, tex, &mut cmds);
        for c in cmds {
            client.send(c);
        }
    }

    fn contents(&mut self, ui: &mut Ui, client: &Client, tex: &mut Textures, cmds: &mut Vec<Command>) {
        let st = client.state();
        let Some(own) = st.account.as_ref().map(|a| a.steam_id) else { return };
        let is_own = st.profile.viewing.is_none();
        let target: SteamId = st.profile.viewing.unwrap_or(own);
        let entry = st.profile.entry(target);
        if entry.is_none() && self.asked != Some(target.0) {
            self.asked = Some(target.0);
            cmds.push(Command::ViewProfile(target));
        }
        self.asked = Some(target.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(18.0);
            let width = ui.available_width().min(PAGE_W);
            let side = ((ui.available_width() - width) / 2.0).max(16.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    self.card(ui, client, tex, target, is_own, entry, cmds);
                });
            });
            ui.add_space(24.0);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn card(
        &mut self,
        ui: &mut Ui,
        client: &Client,
        tex: &mut Textures,
        target: SteamId,
        is_own: bool,
        entry: Option<&ProfileEntry>,
        cmds: &mut Vec<Command>,
    ) {
        let st = client.state();
        let fs = &st.friends;
        let friend = fs.friend(target);
        let xml = entry.and_then(|e| e.xml.as_ref());
        let now = fmt::now_unix();

        let name = xml
            .map(|x| x.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| friend.filter(|f| f.has_name()).map(|f| f.name.clone()))
            .or_else(|| is_own.then(|| fs.persona_name().map(str::to_string)).flatten())
            .unwrap_or_else(|| NAME_LOADING.to_string());
        let avatar_url = xml
            .and_then(|x| x.avatar_hash.as_deref().map(avatar_full_url))
            .or_else(|| if is_own { fs.my_avatar_url().map(str::to_string) } else { None })
            .or_else(|| friend.and_then(|f| f.avatar_url.clone()));
        let status = if is_own {
            fs.my_state.label().to_string()
        } else if let Some(f) = friend {
            crate::friends::status_text(f, now)
        } else {
            xml.map(|x| x.state_message.replace('\n', " ")).unwrap_or_default()
        };
        let ring = match (is_own, friend) {
            (true, _) => theme::FRIEND_ONLINE,
            (false, Some(f)) => crate::friends::name_color(f),
            _ => theme::FRIEND_OFFLINE,
        };

        // Header: picture on the left, name and facts on the right.
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), AVATAR), Sense::hover());
        let av = Rect::from_min_size(rect.min, Vec2::splat(AVATAR));
        let p = ui.painter();
        p.rect_filled(av, theme::RADIUS, theme::BG_CAPSULE);
        let mut drawn = false;
        if let Some(u) = &avatar_url
            && let Art::Ready(t, alpha) = tex.get_web(client, u)
        {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(t.id(), av, uv, Color32::WHITE.gamma_multiply(alpha));
            drawn = true;
        }
        if !drawn {
            let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            ui.painter().text(av.center(), egui::Align2::CENTER_CENTER, initial, theme::bold(54.0), theme::TEXT_DIM);
        }
        ui.painter().rect_stroke(av, theme::RADIUS, Stroke::new(2.0, ring), StrokeKind::Outside);

        let text_left = av.right() + 20.0;
        let ptr = ui.painter().clone();
        ptr.text(
            Pos2::new(text_left, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            &name,
            theme::bold(26.0),
            Color32::WHITE,
        );
        ptr.text(Pos2::new(text_left, rect.top() + 36.0), egui::Align2::LEFT_TOP, &status, theme::regular(14.0), ring);
        let mut y = rect.top() + 62.0;
        let public = is_own || xml.is_some_and(|x| x.privacy == Privacy::Public);
        let mut line = |label: &str, value: &str| {
            if !value.is_empty() {
                ptr.text(
                    Pos2::new(text_left, y),
                    egui::Align2::LEFT_TOP,
                    format!("{label}  {value}"),
                    theme::regular(13.0),
                    theme::TEXT_DIM,
                );
                y += 18.0;
            }
        };
        if let Some(x) = xml {
            if public {
                line("Name", &x.real_name);
                line("Location", &x.location);
            }
            line("Member since", &x.member_since);
        }
        if let Some(l) = entry.and_then(|e| e.level) {
            line("Steam level", &l.to_string());
        }
        if is_own {
            line("Friends", &fs.friends.len().to_string());
            line("Games", &st.games.len().to_string());
        }

        // Buttons.
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if is_own {
                let editing = st.profile.edit != EditPhase::Idle;
                if widgets::button(ui, "Edit profile", widgets::ButtonStyle::Primary, !editing).clicked() {
                    cmds.push(Command::ProfileEditStart);
                }
            } else if friend.is_some() {
                if widgets::button(ui, "Send message", widgets::ButtonStyle::Primary, true).clicked() {
                    cmds.push(Command::OpenChat(target));
                }
            } else if widgets::button(ui, "Add friend", widgets::ButtonStyle::Primary, true).clicked() {
                cmds.push(Command::FriendAct(FriendAction::Add, target));
            }
            if widgets::button(ui, "Refresh", widgets::ButtonStyle::Secondary, true).clicked() {
                cmds.push(Command::ViewProfile(target));
            }
        });
        ui.add_space(14.0);

        match entry.map(|e| &e.load) {
            None | Some(ProfileLoad::Loading) if xml.is_none() => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new("Loading profile...").color(theme::TEXT_DIM));
                });
                return;
            }
            Some(ProfileLoad::Failed(why)) if xml.is_none() => {
                ui.label(RichText::new(format!("Couldn't load this profile: {why}")).color(theme::ERROR_TEXT));
                return;
            }
            _ => {}
        }
        if let Some(ProfileLoad::Failed(why)) = entry.map(|e| &e.load) {
            ui.label(RichText::new(format!("Couldn't refresh: {why}")).color(theme::ERROR_TEXT));
        }
        if !public {
            ui.label(RichText::new("This profile is private.").color(theme::TEXT_DIM));
            return;
        }

        if is_own && st.profile.edit != EditPhase::Idle {
            self.edit_form(ui, client, cmds);
            return;
        }
        self.draft = None;

        if let Some(x) = xml {
            if !x.headline.is_empty() {
                ui.label(RichText::new(&x.headline).font(theme::medium(16.0)).color(theme::TEXT));
                ui.add_space(6.0);
            }
            widgets::section_title(ui, "SUMMARY");
            ui.add_space(4.0);
            let summary = if x.summary.is_empty() { "No information given." } else { x.summary.as_str() };
            ui.add(egui::Label::new(RichText::new(summary).color(theme::TEXT)).wrap());
            ui.add_space(16.0);
        }

        // Recently played: our own from the library, others from the profile page.
        widgets::section_title(ui, "RECENT ACTIVITY");
        ui.add_space(4.0);
        if is_own {
            let mut recent: Vec<_> = st.games.iter().filter(|g| g.last_played > 0).collect();
            recent.sort_by_key(|g| std::cmp::Reverse(g.last_played));
            if recent.is_empty() {
                ui.label(RichText::new("Nothing played yet.").color(theme::TEXT_DIM));
            }
            for g in recent.into_iter().take(5) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&g.name).color(theme::TEXT));
                    ui.label(
                        RichText::new(format!(
                            "{} on record  -  last played {}",
                            minutes(g.playtime_minutes),
                            fmt::last_played(g.last_played, now)
                        ))
                        .color(theme::TEXT_DIM),
                    );
                });
            }
        } else if let Some(x) = xml {
            if x.most_played.is_empty() {
                ui.label(RichText::new("No recent games shown.").color(theme::TEXT_DIM));
            } else {
                ui.label(RichText::new(format!("{} past 2 weeks", hours(x.hours_2wk))).color(theme::TEXT_DIM));
            }
            for g in &x.most_played {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&g.name).color(theme::TEXT));
                    ui.label(
                        RichText::new(format!(
                            "{} past 2 weeks, {} on record",
                            hours(g.hours_2wk),
                            hours(g.hours_total)
                        ))
                        .color(theme::TEXT_DIM),
                    );
                });
            }
        }
        if let Some(x) = xml
            && (x.vac_banned || x.limited)
        {
            ui.add_space(12.0);
            let text = if x.vac_banned { "This account has a ban on record." } else { "This is a limited account." };
            ui.label(RichText::new(text).color(theme::TEXT_DIM));
        }
    }

    /// The form that edits our profile name, real name and summary.
    fn edit_form(&mut self, ui: &mut Ui, client: &Client, cmds: &mut Vec<Command>) {
        let st = client.state();
        let phase = &st.profile.edit;
        let is_loading = matches!(phase, EditPhase::Loading);
        widgets::section_title(ui, "EDIT PROFILE");
        ui.add_space(6.0);
        if is_loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new("Signing in to the community website...").color(theme::TEXT_DIM));
            });
            return;
        }
        if self.draft.is_none() {
            self.draft = st.profile.edit_original.clone();
        }
        if let EditPhase::Failed(why) = phase {
            ui.label(RichText::new(why).color(theme::ERROR_TEXT));
            ui.add_space(6.0);
        }
        let Some(draft) = &mut self.draft else {
            // Loading the form failed: offer to close it or to try again.
            ui.horizontal(|ui| {
                if widgets::button(ui, "Try again", widgets::ButtonStyle::Primary, true).clicked() {
                    cmds.push(Command::ProfileEditStart);
                }
                if widgets::button(ui, "Close", widgets::ButtonStyle::Secondary, true).clicked() {
                    cmds.push(Command::ProfileEditCancel);
                }
            });
            return;
        };
        let saving = matches!(phase, EditPhase::Saving);
        ui.add_enabled_ui(!saving, |ui| {
            ui.label(RichText::new("Profile name").color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut draft.persona_name).char_limit(32).desired_width(360.0));
            ui.add_space(8.0);
            ui.label(RichText::new("Real name").color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut draft.real_name).char_limit(64).desired_width(360.0));
            ui.add_space(8.0);
            ui.label(RichText::new("Summary").color(theme::TEXT_DIM));
            ui.add(
                egui::TextEdit::multiline(&mut draft.summary)
                    .char_limit(2000)
                    .desired_rows(6)
                    .desired_width(PAGE_W - 40.0),
            );
        });
        ui.add_space(10.0);
        let mut cancel = false;
        ui.horizontal(|ui| {
            let ok = !draft.persona_name.trim().is_empty();
            if widgets::button(
                ui,
                if saving { "Saving..." } else { "Save" },
                widgets::ButtonStyle::Primary,
                ok && !saving,
            )
            .clicked()
            {
                cmds.push(Command::ProfileEditSave(draft.clone()));
            }
            if widgets::button(ui, "Cancel", widgets::ButtonStyle::Secondary, !saving).clicked() {
                cmds.push(Command::ProfileEditCancel);
                cancel = true;
            }
        });
        if cancel {
            self.draft = None;
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("Avatar and privacy settings can't be changed here yet.")
                .font(theme::regular(12.0))
                .color(theme::TEXT_DIM),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_time_text() {
        assert_eq!(hours(0.0), "0.0 hrs");
        assert_eq!(hours(12.5), "12.5 hrs");
        assert_eq!(hours(1234.5), "1234 hrs");
        assert_eq!(minutes(45), "45 min");
        assert_eq!(minutes(600), "10 hrs");
    }
}
