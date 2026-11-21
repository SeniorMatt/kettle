//! The Friends & Chat panel: a right-hand panel that slides in over the page. Friends list (virtualized, grouped
//! In-Game / Online / Offline) and a tab strip of chats. Pure view code: it reads `State` and returns commands.

use crate::textures::{Art, Textures};
use crate::{anim, fmt, theme, widgets};
use egui::{Align, Color32, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, vec2};
use egui_phosphor::regular as icon;
use sr_client::{
    Client, Command, Delivery, Friend, FriendAction, FriendsStatus, GroupMessage, HistoryState, MAX_TEXT, NAME_LOADING,
    PersonaState, Toast,
};
use sr_model::SteamId;
use std::collections::HashMap;
use std::time::Duration;

const HEADER_H: f32 = 66.0;
const TAB_H: f32 = 30.0;
const ROW_H: f32 = 44.0;
const GROUP_H: f32 = 28.0;
const AVATAR: f32 = 32.0;
/// How long the bottom-bar toast stays.
const TOAST_SECS: f64 = 6.0;
const INPUT_ID: &str = "friends-chat-input";
/// Messages from the same sender within this many seconds share one block.
const GROUP_GAP: u64 = 300;

/// The sections of the list, in the order they are shown.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Requests,
    Groups,
    InGame,
    Online,
    Offline,
    Pending,
}

const SECTIONS: [Group; 6] =
    [Group::Requests, Group::Groups, Group::InGame, Group::Online, Group::Offline, Group::Pending];

impl Group {
    fn label(self) -> &'static str {
        match self {
            Group::Requests => "FRIEND REQUESTS",
            Group::Groups => "GROUP CHATS",
            Group::InGame => "IN-GAME",
            Group::Online => "ONLINE",
            Group::Offline => "OFFLINE",
            Group::Pending => "PENDING",
        }
    }
}

enum Row {
    Header { group: Group, count: usize },
    Friend(usize),
    Request(usize),
    Pending(usize),
    Chat(usize),
}

struct Rows {
    key: (u64, String, [bool; 6]),
    items: Vec<(f32, f32, Row)>,
    total: f32,
}

/// Where a typed message goes.
#[derive(Clone, Copy)]
enum InputTarget {
    Friend(SteamId),
    Group(u64, u64),
}

/// "Name" of whoever wrote a group message.
fn group_sender(fs: &sr_client::FriendsState, m: &GroupMessage) -> String {
    if m.from_me { fs.persona_name().unwrap_or("Me").to_string() } else { fs.member_name(m.sender) }
}

/// What the user chose in a friend's right-click menu or on a row.
#[derive(Clone, Copy, PartialEq)]
enum RowAction {
    Chat,
    Profile,
    Remove,
    Block,
}

/// A question waiting for "Yes".
struct Confirm {
    action: FriendAction,
    id: SteamId,
    name: String,
}

#[derive(Default)]
pub struct FriendsView {
    search: String,
    collapsed: [bool; 6],
    /// The "Add a friend" box is open, and what is typed in it.
    adding: bool,
    add_text: String,
    confirm: Option<Confirm>,
    rows: Option<Rows>,
    drafts: HashMap<u64, String>,
    toast_seen: Option<(u64, f64)>,
    last_active: Option<SteamId>,
    /// Drawn inside the in-game overlay: no close button (the overlay has its own).
    embedded: bool,
}

fn group_of(f: &Friend) -> Group {
    if f.game.is_some() && f.state.is_online() {
        Group::InGame
    } else if f.state.is_online() {
        Group::Online
    } else {
        Group::Offline
    }
}

pub(crate) fn name_color(f: &Friend) -> Color32 {
    match group_of(f) {
        Group::InGame => theme::FRIEND_INGAME,
        Group::Online if matches!(f.state, PersonaState::Away | PersonaState::Snooze) => theme::FRIEND_AWAY,
        Group::Online => theme::FRIEND_ONLINE,
        _ => theme::FRIEND_OFFLINE,
    }
}

pub(crate) fn status_text(f: &Friend, now: u64) -> String {
    if !f.has_name() && f.state == PersonaState::Offline && f.last_logoff == 0 {
        return String::new();
    }
    match (&f.game, group_of(f)) {
        (Some(g), Group::InGame) => match &f.rich {
            Some(r) => format!("In-Game: {} - {r}", g.name),
            None => format!("In-Game: {}", g.name),
        },
        (_, Group::Offline) if f.last_logoff > 0 => format!("Last online {}", fmt::ago(f.last_logoff, now)),
        (_, Group::Offline) => "Offline".into(),
        _ => f.state.label().into(),
    }
}

fn state_color(s: PersonaState) -> Color32 {
    match s {
        PersonaState::Offline | PersonaState::Invisible => theme::FRIEND_OFFLINE,
        PersonaState::Away | PersonaState::Snooze => theme::FRIEND_AWAY,
        PersonaState::Busy => theme::ERROR_TEXT,
        _ => theme::FRIEND_ONLINE,
    }
}

impl FriendsView {
    /// Menu: Friends > Add a friend... (the panel is opened by the caller).
    pub(crate) fn open_add(&mut self) {
        self.adding = true;
    }

    /// The slide-in panel. Call before the central panel so it takes its space from it.
    pub fn show_panel(&mut self, ui: &mut Ui, client: &mut Client, textures: &mut Textures) {
        let ctx = ui.ctx().clone();
        let open = client.state().friends.open;
        let t = anim::animate_bool(&ctx, Id::new("friends-slide"), open, anim::NORMAL);
        if t <= 0.0 {
            return;
        }
        let frame = egui::Frame::new().fill(theme::BG_SIDEBAR);
        let mut cmds: Vec<Command> = Vec::new();
        egui::Panel::right("friends")
            .exact_size(theme::FRIENDS_W * t)
            .resizable(false)
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                // The content keeps its full width and slides in from the right edge.
                let full = Rect::from_min_max(Pos2::new(rect.right() - theme::FRIENDS_W, rect.top()), rect.max);
                let mut inner = ui.new_child(UiBuilder::new().max_rect(full).layout(Layout::top_down(Align::Min)));
                inner.set_clip_rect(rect.intersect(ui.clip_rect()));
                self.contents(&mut inner, client, textures, &mut cmds);
                ui.painter()
                    .line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, theme::WINDOW_BORDER));
            });
        for c in cmds {
            client.send(c);
        }
    }

    /// The same friends list and chats, drawn into `ui` (the in-game overlay) instead of the slide-in panel.
    pub fn show_embedded(&mut self, ui: &mut Ui, client: &mut Client, textures: &mut Textures) {
        let mut cmds: Vec<Command> = Vec::new();
        self.embedded = true;
        self.contents(ui, client, textures, &mut cmds);
        self.embedded = false;
        for c in cmds {
            client.send(c);
        }
    }

    fn contents(&mut self, ui: &mut Ui, client: &Client, tex: &mut Textures, cmds: &mut Vec<Command>) {
        let now = fmt::now_unix();
        self.header(ui, client, tex, cmds);
        let fs = &client.state().friends;
        match &fs.status {
            FriendsStatus::Connecting => dim_line(ui, "Connecting to Steam...", theme::TEXT_DIM),
            FriendsStatus::Disconnected(why) => {
                dim_line(ui, &format!("Disconnected. Retrying... ({why})"), theme::ERROR_TEXT);
            }
            _ => {}
        }
        if !fs.tabs.is_empty() {
            self.tab_strip(ui, client, cmds);
        }
        let active = fs.active;
        if active != self.last_active {
            self.last_active = active;
            if active.is_some() {
                ui.ctx().memory_mut(|m| m.request_focus(Id::new(INPUT_ID)));
            }
        }
        match (active, fs.active_group) {
            (Some(id), _) => self.chat_view(ui, client, tex, id, now, cmds),
            (None, Some((g, r))) => self.group_view(ui, client, g, r, now, cmds),
            (None, None) => self.list_view(ui, client, tex, now, cmds),
        }
        self.confirm_dialog(ui.ctx(), cmds);
    }

    // ---- header ------------------------------------------------------------------------------------

    fn header(&mut self, ui: &mut Ui, client: &Client, tex: &mut Textures, cmds: &mut Vec<Command>) {
        let fs = &client.state().friends;
        let (rect, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W, HEADER_H), Sense::hover());
        let av = Rect::from_min_size(rect.min + vec2(12.0, 9.0), Vec2::splat(48.0));
        // Our persona name, never the account name; "Loading..." until Steam has sent it.
        let name = fs.persona_name().map_or_else(|| NAME_LOADING.to_string(), str::to_string);
        avatar(ui, tex, client, av, fs.my_avatar_url(), &name, state_color(fs.my_state));
        let me_id = client.state().account.as_ref().map(|a| a.steam_id);
        let name_color = if fs.persona_name().is_some() { theme::TEXT } else { theme::TEXT_DIM };
        let name_rect = Rect::from_min_size(rect.min + vec2(68.0, 10.0), vec2(theme::FRIENDS_W - 68.0 - 48.0, 22.0));
        let name_resp = ui.interact(name_rect.union(av), Id::new("friends-header-me"), Sense::click());
        let p = ui.painter().with_clip_rect(name_rect);
        p.text(name_rect.left_top(), egui::Align2::LEFT_TOP, &name, theme::bold(16.0), name_color);
        if name_resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("View your profile").clicked()
            && let Some(id) = me_id
        {
            cmds.push(Command::ViewProfile(id));
        }
        // State picker, as a menu button under the name.
        let picker = Rect::from_min_size(rect.min + vec2(64.0, 34.0), vec2(190.0, 22.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(picker).layout(Layout::left_to_right(Align::Center)));
        child.style_mut().override_font_id = Some(theme::regular(13.0));
        child.visuals_mut().override_text_color = Some(state_color(fs.my_state));
        child.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        child.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        crate::widgets::menu_button(&mut child, format!("{} {}", fs.my_state.label(), icon::CARET_DOWN), |ui| {
            for s in [
                PersonaState::Online,
                PersonaState::Away,
                PersonaState::Busy,
                PersonaState::Invisible,
                PersonaState::Offline,
            ] {
                if ui.selectable_label(fs.my_state == s, s.label()).clicked() {
                    cmds.push(Command::SetPersonaState(s));
                }
            }
        });
        if !self.embedded {
            let close = Rect::from_min_size(Pos2::new(rect.right() - 36.0, rect.top() + 10.0), Vec2::splat(26.0));
            let mut cui = ui.new_child(UiBuilder::new().max_rect(close));
            if widgets::icon_button(&mut cui, icon::X, 26.0, "Close").clicked() {
                cmds.push(Command::CloseFriends);
            }
        }
        ui.painter().line_segment(
            [Pos2::new(rect.left(), rect.bottom()), Pos2::new(rect.right(), rect.bottom())],
            Stroke::new(1.0, theme::BG_TOP),
        );
    }

    // ---- chat tabs ---------------------------------------------------------------------------------

    fn tab_strip(&mut self, ui: &mut Ui, client: &Client, cmds: &mut Vec<Command>) {
        let fs = &client.state().friends;
        let (strip, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W, TAB_H), Sense::hover());
        ui.painter().rect_filled(strip, 0.0, theme::BG_TOP);
        let mut x = strip.left() + 4.0;
        // The first tab is the list itself.
        let mut entries: Vec<(Option<SteamId>, String)> = vec![(None, icon::USERS.to_string())];
        for id in &fs.tabs {
            let name = fs.friend(*id).map_or_else(|| NAME_LOADING.to_string(), Friend::display_name);
            entries.push((Some(*id), name));
        }
        let max_w = ((theme::FRIENDS_W - 8.0) / entries.len() as f32).clamp(54.0, 120.0);
        for (id, label) in entries {
            let w = if id.is_none() { 34.0 } else { max_w };
            let r = Rect::from_min_size(Pos2::new(x, strip.top()), vec2(w - 2.0, TAB_H));
            x += w;
            let unread = id.map_or(0, |i| fs.unread(i));
            let sel = fs.active == id;
            let resp = ui.interact(r, Id::new(("friends-tab", id.map(|i| i.0))), Sense::click());
            let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
            let color = theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, if sel { 1.0 } else { hover });
            let p = ui.painter().with_clip_rect(r);
            let reserve = if id.is_some() { 18.0 + if unread > 0 { 20.0 } else { 0.0 } } else { 0.0 };
            let galley = p.layout(label, theme::medium(12.5), color, (r.width() - 12.0 - reserve).max(10.0));
            let galley = if id.is_some() { truncated(&p, &galley, r.width() - 12.0 - reserve, color) } else { galley };
            p.galley(Pos2::new(r.left() + 8.0, r.center().y - galley.size().y / 2.0), galley, color);
            if sel {
                p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(2.0, theme::ACCENT));
            }
            if let Some(i) = id {
                if unread > 0 {
                    badge(ui, Pos2::new(r.right() - 32.0, r.center().y), unread);
                }
                let xr = Rect::from_center_size(Pos2::new(r.right() - 10.0, r.center().y), Vec2::splat(16.0));
                let xr_resp = ui.interact(xr, resp.id.with("x"), Sense::click());
                let xc = if xr_resp.hovered() { theme::TEXT } else { theme::TEXT_UNINSTALLED };
                p.text(xr.center(), egui::Align2::CENTER_CENTER, icon::X, theme::regular(11.0), xc);
                if xr_resp.clicked() {
                    cmds.push(Command::CloseChat(i));
                    continue;
                }
            }
            if resp.clicked() {
                cmds.push(match id {
                    Some(i) => Command::OpenChat(i),
                    None => Command::ShowFriendList,
                });
            }
        }
    }

    // ---- friends list ------------------------------------------------------------------------------

    fn list_view(&mut self, ui: &mut Ui, client: &Client, tex: &mut Textures, now: u64, cmds: &mut Vec<Command>) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let edit = egui::TextEdit::singleline(&mut self.search)
                .hint_text(format!("{}  Search friends", icon::MAGNIFYING_GLASS))
                .font(theme::regular(14.0))
                .desired_width(theme::FRIENDS_W - 24.0 - 34.0)
                .margin(egui::Margin::symmetric(8, 5));
            ui.add(edit);
            if widgets::icon_button(ui, icon::USER_PLUS, 26.0, "Add a friend").clicked() {
                self.adding = !self.adding;
            }
        });
        if self.adding {
            self.add_box(ui, cmds);
        }
        ui.add_space(6.0);
        self.refresh_rows(client);
        let Some(rows) = &self.rows else { return };
        let fs = &client.state().friends;
        if rows.items.is_empty() {
            let msg = match (&fs.status, fs.friends.is_empty()) {
                (FriendsStatus::Connected, true) => "No friends yet.",
                (_, true) => "",
                _ => "No friends match.",
            };
            dim_line(ui, msg, theme::TEXT_DIM);
            return;
        }
        let searching = !self.search.trim().is_empty();
        let mut toggle = None;
        let mut confirm = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            ui.set_min_height(rows.total);
            let origin = ui.min_rect().min;
            let w = ui.available_width();
            let first = rows.items.partition_point(|(y, h, _)| y + h < viewport.min.y);
            for (y, h, row) in &rows.items[first..] {
                if *y > viewport.max.y {
                    break;
                }
                let rect = Rect::from_min_size(origin + vec2(0.0, *y), vec2(w, *h));
                match row {
                    Row::Header { group, count } => {
                        if group_header(ui, rect, *group, *count, self.collapsed[*group as usize] && !searching) {
                            toggle = Some(*group as usize);
                        }
                    }
                    Row::Friend(i) => {
                        let Some(f) = fs.friends.get(*i) else { continue };
                        match friend_row(ui, tex, client, rect, f, fs.unread(f.id), now) {
                            Some(RowAction::Chat) => cmds.push(Command::OpenChat(f.id)),
                            Some(RowAction::Profile) => cmds.push(Command::ViewProfile(f.id)),
                            Some(RowAction::Remove) => confirm = Some((FriendAction::Remove, f)),
                            Some(RowAction::Block) => confirm = Some((FriendAction::Block, f)),
                            None => {}
                        }
                    }
                    Row::Request(i) => {
                        if let Some(f) = fs.requests.get(*i) {
                            request_row(ui, tex, client, rect, f, cmds);
                        }
                    }
                    Row::Pending(i) => {
                        if let Some(f) = fs.pending.get(*i) {
                            pending_row(ui, tex, client, rect, f, cmds);
                        }
                    }
                    Row::Chat(i) => {
                        if let Some(g) = fs.groups.get(*i)
                            && group_row(ui, rect, g, fs.group_unread(g.id))
                        {
                            cmds.push(Command::OpenGroup(g.id, g.default_room));
                        }
                    }
                }
            }
        });
        if let Some(g) = toggle {
            self.collapsed[g] = !self.collapsed[g];
        }
        if let Some((action, f)) = confirm {
            self.confirm = Some(Confirm { action, id: f.id, name: f.display_name() });
        }
    }

    /// The "Add a friend" box: a friend code, a SteamID or a profile link.
    fn add_box(&mut self, ui: &mut Ui, cmds: &mut Vec<Command>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let edit = egui::TextEdit::singleline(&mut self.add_text)
                .hint_text("Friend code, SteamID or profile link")
                .font(theme::regular(13.0))
                .desired_width(theme::FRIENDS_W - 24.0 - 76.0)
                .margin(egui::Margin::symmetric(8, 5));
            let resp = ui.add(edit);
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let go = widgets::button(ui, "Add", widgets::ButtonStyle::Primary, !self.add_text.trim().is_empty());
            if (go.clicked() || enter) && !self.add_text.trim().is_empty() {
                cmds.push(Command::AddFriend(std::mem::take(&mut self.add_text)));
                self.adding = false;
            }
        });
    }

    /// "Remove friend?" / "Block?" before anything irreversible is sent.
    fn confirm_dialog(&mut self, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        let Some(c) = &self.confirm else { return };
        let (title, body, yes) = match c.action {
            FriendAction::Block => {
                ("Block", format!("Block {}? They won't be able to message or invite you.", c.name), "Block")
            }
            _ => ("Remove friend", format!("Remove {} from your friends?", c.name), "Remove"),
        };
        let mut answer = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(300.0);
                ui.label(body);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, yes, widgets::ButtonStyle::Danger, true).clicked() {
                        answer = Some(true);
                    }
                    if widgets::button(ui, "Cancel", widgets::ButtonStyle::Secondary, true).clicked() {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                if let Some(c) = self.confirm.take() {
                    cmds.push(Command::FriendAct(c.action, c.id));
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn refresh_rows(&mut self, client: &Client) {
        let fs = &client.state().friends;
        let needle = self.search.trim().to_lowercase();
        let key = (fs.rev, needle.clone(), self.collapsed);
        if self.rows.as_ref().is_some_and(|r| r.key == key) {
            return;
        }
        let matches = |f: &Friend| needle.is_empty() || f.display_name().to_lowercase().contains(&needle);
        let mut lists: [Vec<Row>; 6] = Default::default();
        for (i, f) in fs.requests.iter().enumerate() {
            if matches(f) {
                lists[Group::Requests as usize].push(Row::Request(i));
            }
        }
        for (i, g) in fs.groups.iter().enumerate() {
            if needle.is_empty() || g.name.to_lowercase().contains(&needle) {
                lists[Group::Groups as usize].push(Row::Chat(i));
            }
        }
        for (i, f) in fs.friends.iter().enumerate() {
            if matches(f) {
                lists[group_of(f) as usize].push(Row::Friend(i));
            }
        }
        for (i, f) in fs.pending.iter().enumerate() {
            if matches(f) {
                lists[Group::Pending as usize].push(Row::Pending(i));
            }
        }
        let mut items = Vec::new();
        let mut y = 0.0;
        for group in SECTIONS {
            let g = group as usize;
            let list = std::mem::take(&mut lists[g]);
            if list.is_empty() {
                continue;
            }
            items.push((y, GROUP_H, Row::Header { group, count: list.len() }));
            y += GROUP_H;
            if self.collapsed[g] && needle.is_empty() {
                continue;
            }
            for row in list {
                items.push((y, ROW_H, row));
                y += ROW_H;
            }
        }
        self.rows = Some(Rows { key, items, total: y });
    }

    // ---- chat --------------------------------------------------------------------------------------

    fn chat_view(
        &mut self,
        ui: &mut Ui,
        client: &Client,
        tex: &mut Textures,
        id: SteamId,
        now: u64,
        cmds: &mut Vec<Command>,
    ) {
        let fs = &client.state().friends;
        let friend = fs.friend(id);
        // Top line: back, avatar, name and status.
        let (bar, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W, 48.0), Sense::hover());
        let mut back_ui =
            ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(bar.min + vec2(6.0, 11.0), Vec2::splat(26.0))));
        if widgets::icon_button(&mut back_ui, icon::CARET_LEFT, 26.0, "Friends").clicked() {
            cmds.push(Command::ShowFriendList);
        }
        let av = Rect::from_min_size(bar.min + vec2(40.0, 8.0), Vec2::splat(32.0));
        let (name, color, status) = friend.map_or_else(
            || (NAME_LOADING.to_string(), theme::FRIEND_OFFLINE, String::new()),
            |f| (f.display_name(), name_color(f), status_text(f, now)),
        );
        let ring = friend.map_or(theme::FRIEND_OFFLINE, name_color);
        avatar(ui, tex, client, av, friend.and_then(|f| f.avatar_url.as_deref()), &name, ring);
        let p = ui.painter();
        p.text(bar.min + vec2(80.0, 8.0), egui::Align2::LEFT_TOP, &name, theme::bold(14.0), color);
        let p = p.with_clip_rect(Rect::from_min_max(bar.min + vec2(80.0, 0.0), bar.max));
        p.text(bar.min + vec2(80.0, 27.0), egui::Align2::LEFT_TOP, status, theme::regular(12.0), theme::TEXT_DIM);
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, theme::BG_TOP));

        // The input sits under the messages; reserve its space first.
        let input_h = 56.0;
        let list_h = (ui.available_height() - input_h).max(40.0);
        let chat = fs.chat(id);
        ui.allocate_ui(vec2(theme::FRIENDS_W, list_h), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                ui.set_width(theme::FRIENDS_W - 4.0);
                ui.add_space(8.0);
                if let Some(c) = chat {
                    if c.history == HistoryState::Loading {
                        dim_line(ui, "Loading earlier messages...", theme::TEXT_DIM);
                    }
                    let offset = sr_client::local_utc_offset(now);
                    let mut prev: Option<(bool, u64)> = None;
                    for m in &c.messages {
                        let new_block =
                            prev.is_none_or(|(me, t)| me != m.from_me || m.time.saturating_sub(t) > GROUP_GAP);
                        prev = Some((m.from_me, m.time));
                        if new_block {
                            let who = if m.from_me { me_name(client) } else { name.clone() };
                            block_header(ui, m.from_me, &who, &fmt::clock(m.time, offset));
                        }
                        bubble(ui, m.from_me, &m.text, &m.delivery);
                    }
                } else {
                    dim_line(ui, "No messages yet. Say hello.", theme::TEXT_DIM);
                }
                ui.add_space(6.0);
            });
        });
        self.input(ui, id.0, InputTarget::Friend(id), cmds);
    }

    /// A text channel of a group chat: the same look as a friend chat, plus a channel picker.
    fn group_view(&mut self, ui: &mut Ui, client: &Client, group: u64, room: u64, now: u64, cmds: &mut Vec<Command>) {
        let fs = &client.state().friends;
        let info = fs.group(group);
        let (bar, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W, 48.0), Sense::hover());
        let mut back_ui =
            ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(bar.min + vec2(6.0, 11.0), Vec2::splat(26.0))));
        if widgets::icon_button(&mut back_ui, icon::CARET_LEFT, 26.0, "Friends").clicked() {
            cmds.push(Command::ShowFriendList);
        }
        let title = info.map_or_else(|| "Group chat".to_string(), |g| g.name.clone());
        let room_name =
            info.and_then(|g| g.rooms.iter().find(|r| r.id == room)).map_or(String::new(), |r| r.name.clone());
        let p = ui.painter().with_clip_rect(Rect::from_min_max(bar.min + vec2(40.0, 0.0), bar.max - vec2(8.0, 0.0)));
        p.text(bar.min + vec2(40.0, 8.0), egui::Align2::LEFT_TOP, &title, theme::bold(14.0), theme::TEXT);
        if let Some(g) = info.filter(|g| g.rooms.len() > 1) {
            let picker = Rect::from_min_size(bar.min + vec2(36.0, 24.0), vec2(220.0, 22.0));
            let mut child =
                ui.new_child(UiBuilder::new().max_rect(picker).layout(Layout::left_to_right(Align::Center)));
            child.style_mut().override_font_id = Some(theme::regular(12.0));
            child.visuals_mut().override_text_color = Some(theme::TEXT_DIM);
            child.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            child.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
            widgets::menu_button(&mut child, format!("# {room_name} {}", icon::CARET_DOWN), |ui| {
                for r in &g.rooms {
                    if ui.selectable_label(r.id == room, format!("# {}", r.name)).clicked() {
                        cmds.push(Command::OpenGroup(group, r.id));
                    }
                }
            });
        } else {
            p.text(
                bar.min + vec2(40.0, 27.0),
                egui::Align2::LEFT_TOP,
                format!("# {room_name}"),
                theme::regular(12.0),
                theme::TEXT_DIM,
            );
        }
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, theme::BG_TOP));

        let input_h = 56.0;
        let list_h = (ui.available_height() - input_h).max(40.0);
        let chat = fs.group_chats.get(&(group, room));
        ui.allocate_ui(vec2(theme::FRIENDS_W, list_h), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                ui.set_width(theme::FRIENDS_W - 4.0);
                ui.add_space(8.0);
                match chat {
                    Some(c) if !c.messages.is_empty() || c.history == HistoryState::Loading => {
                        if c.history == HistoryState::Loading {
                            dim_line(ui, "Loading earlier messages...", theme::TEXT_DIM);
                        }
                        let offset = sr_client::local_utc_offset(now);
                        let mut prev: Option<(u32, u64)> = None;
                        for m in &c.messages {
                            let new_block =
                                prev.is_none_or(|(who, t)| who != m.sender || m.time.saturating_sub(t) > GROUP_GAP);
                            prev = Some((m.sender, m.time));
                            if new_block {
                                let who = group_sender(fs, m);
                                block_header(ui, m.from_me, &who, &fmt::clock(m.time, offset));
                            }
                            bubble(ui, m.from_me, &m.text, &m.delivery);
                        }
                    }
                    _ => dim_line(ui, "No messages yet. Say hello.", theme::TEXT_DIM),
                }
                ui.add_space(6.0);
            });
        });
        self.input(ui, room ^ (1 << 63), InputTarget::Group(group, room), cmds);
    }

    fn input(&mut self, ui: &mut Ui, key: u64, target: InputTarget, cmds: &mut Vec<Command>) {
        let input_id = Id::new(INPUT_ID);
        let focused = ui.ctx().memory(|m| m.has_focus(input_id));
        // Enter sends; Shift+Enter adds a line. Consumed before the editor sees it.
        let send = focused
            && ui.input_mut(|i| {
                let mut hit = false;
                i.events.retain(|e| match e {
                    egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift => {
                        hit = true;
                        false
                    }
                    _ => true,
                });
                hit
            });
        let draft = self.drafts.entry(key).or_default();
        if send {
            let text = std::mem::take(draft);
            if !text.trim().is_empty() {
                cmds.push(match target {
                    InputTarget::Friend(id) => Command::SendChat(id, text),
                    InputTarget::Group(g, r) => Command::SendGroupChat(g, r, text),
                });
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            // A blinking cursor would repaint twice a second for as long as the box has focus.
            ui.visuals_mut().text_cursor.blink = false;
            egui::ScrollArea::vertical().max_height(70.0).id_salt("friends-input-scroll").show(ui, |ui| {
                let edit = egui::TextEdit::multiline(draft)
                    .id(input_id)
                    .hint_text("Send a message")
                    .font(theme::regular(14.0))
                    .desired_rows(1)
                    .desired_width(theme::FRIENDS_W - 34.0)
                    .char_limit(MAX_TEXT)
                    .lock_focus(true)
                    .margin(egui::Margin::symmetric(8, 6));
                ui.add(edit);
            });
        });
        if self.drafts.len() > 32 {
            self.drafts.retain(|k, v| *k == key || !v.is_empty());
        }
    }

    // ---- toast -------------------------------------------------------------------------------------

    /// The incoming-message toast, while it should still be on screen.
    pub fn current_toast(&mut self, ctx: &egui::Context, client: &Client) -> Option<Toast> {
        let toast = client.state().friends.toast.clone()?;
        let now = ctx.input(|i| i.time);
        let at = match self.toast_seen {
            Some((seq, at)) if seq == toast.seq => at,
            _ => {
                self.toast_seen = Some((toast.seq, now));
                now
            }
        };
        let left = TOAST_SECS - (now - at);
        if left <= 0.0 {
            return None;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(left + 0.05));
        Some(toast)
    }
}

// ---- small drawing helpers ------------------------------------------------------------------------

fn me_name(client: &Client) -> String {
    client.state().friends.persona_name().unwrap_or("Me").to_string()
}

fn dim_line(ui: &mut Ui, text: &str, color: Color32) {
    if text.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(text).font(theme::regular(12.5)).color(color)).wrap());
    });
}

/// `galley` cut to `width` with an ellipsis (a plain clip would hide the cut).
fn truncated(
    p: &egui::Painter,
    galley: &std::sync::Arc<egui::Galley>,
    width: f32,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    if galley.size().x <= width {
        return galley.clone();
    }
    let text = galley.text().to_string();
    let mut cut = text.chars().collect::<Vec<_>>();
    while cut.len() > 1 {
        cut.pop();
        let s: String = cut.iter().collect::<String>() + "...";
        let g = p.layout_no_wrap(s, theme::medium(12.5), color);
        if g.size().x <= width {
            return g;
        }
    }
    galley.clone()
}

fn badge(ui: &Ui, center: Pos2, n: u32) {
    let text = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = ui.painter().layout_no_wrap(text, theme::bold(10.5), Color32::WHITE);
    let size = vec2((g.size().x + 10.0).max(16.0), 16.0);
    let r = Rect::from_center_size(center, size);
    ui.painter().rect_filled(r, 8.0, theme::ACCENT);
    ui.painter().galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
}

fn avatar(ui: &Ui, tex: &mut Textures, client: &Client, rect: Rect, url: Option<&str>, name: &str, ring: Color32) {
    let p = ui.painter();
    p.rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
    let mut drawn = false;
    if let Some(u) = url
        && let Art::Ready(t, alpha) = tex.get_web(client, u)
    {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        p.image(t.id(), rect, uv, Color32::WHITE.gamma_multiply(alpha));
        drawn = true;
    }
    if !drawn {
        let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        p.text(rect.center(), egui::Align2::CENTER_CENTER, initial, theme::bold(rect.height() * 0.5), theme::TEXT_DIM);
    }
    p.rect_stroke(rect, theme::RADIUS, Stroke::new(1.5, ring), StrokeKind::Outside);
}

/// A group header ("IN-GAME (3)"); returns true when clicked.
fn group_header(ui: &mut Ui, rect: Rect, group: Group, count: usize, collapsed: bool) -> bool {
    let resp = ui.interact(rect, Id::new(("friends-group", group as u8)), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let label = group.label();
    let caret = if collapsed { icon::CARET_RIGHT } else { icon::CARET_DOWN };
    let color = theme::TEXT_HEADING.lerp_to_gamma(theme::TEXT, hover);
    ui.painter().text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{caret}  {label} ({count})"),
        theme::bold(12.0),
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// One friend: avatar, name colored by state, status line, unread badge. A click opens the chat; the
/// right-click menu has the rest.
fn friend_row(
    ui: &mut Ui,
    tex: &mut Textures,
    client: &Client,
    rect: Rect,
    f: &Friend,
    unread: u32,
    now: u64,
) -> Option<RowAction> {
    let resp = ui.interact(rect, Id::new(("friend", f.id.0)), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_HOVER.gamma_multiply(hover));
    }
    let loading = !f.has_name();
    let name = f.display_name();
    let color = if loading { theme::TEXT_DIM } else { name_color(f) };
    let av = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.center().y - AVATAR / 2.0), Vec2::splat(AVATAR));
    avatar(ui, tex, client, av, f.avatar_url.as_deref(), &name, color);
    let clip =
        Rect::from_min_max(Pos2::new(av.right() + 10.0, rect.top()), Pos2::new(rect.right() - 40.0, rect.bottom()));
    let p = ui.painter().with_clip_rect(clip);
    let font = if unread > 0 { theme::bold(14.0) } else { theme::medium(14.0) };
    p.text(Pos2::new(clip.left(), rect.center().y - 1.0), egui::Align2::LEFT_BOTTOM, name, font, color);
    let sub = if group_of(f) == Group::Offline { theme::TEXT_UNINSTALLED } else { color.gamma_multiply(0.75) };
    p.text(
        Pos2::new(clip.left(), rect.center().y + 1.0),
        egui::Align2::LEFT_TOP,
        status_text(f, now),
        theme::regular(12.0),
        sub,
    );
    if unread > 0 {
        badge(ui, Pos2::new(rect.right() - 24.0, rect.center().y), unread);
    }
    let mut action = None;
    widgets::context_menu(&resp, |ui| {
        if ui.button("Send message").clicked() {
            action = Some(RowAction::Chat);
            ui.close();
        }
        if ui.button("View profile").clicked() {
            action = Some(RowAction::Profile);
            ui.close();
        }
        widgets::menu_separator(ui);
        if ui.button("Remove friend").clicked() {
            action = Some(RowAction::Remove);
            ui.close();
        }
        if ui.button("Block").clicked() {
            action = Some(RowAction::Block);
            ui.close();
        }
    });
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        action = Some(RowAction::Chat);
    }
    action
}

/// A small flat button inside a row; returns true when clicked.
fn row_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, accent: bool) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let base = if accent { theme::ACCENT } else { theme::BTN_FLAT };
    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, base.lerp_to_gamma(Color32::WHITE, 0.12 * hover));
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, theme::medium(12.0), Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Someone who asked to be our friend: Accept / Ignore.
fn request_row(ui: &mut Ui, tex: &mut Textures, client: &Client, rect: Rect, f: &Friend, cmds: &mut Vec<Command>) {
    let hover_resp = ui.interact(rect, Id::new(("friend-request", f.id.0)), Sense::click());
    if hover_resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_HOVER);
    }
    let name = f.display_name();
    let av = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.center().y - AVATAR / 2.0), Vec2::splat(AVATAR));
    avatar(ui, tex, client, av, f.avatar_url.as_deref(), &name, theme::FRIEND_ONLINE);
    let clip =
        Rect::from_min_max(Pos2::new(av.right() + 10.0, rect.top()), Pos2::new(rect.right() - 130.0, rect.bottom()));
    let p = ui.painter().with_clip_rect(clip);
    let color = if f.has_name() { theme::TEXT } else { theme::TEXT_DIM };
    p.text(Pos2::new(clip.left(), rect.center().y - 1.0), egui::Align2::LEFT_BOTTOM, name, theme::medium(14.0), color);
    p.text(
        Pos2::new(clip.left(), rect.center().y + 1.0),
        egui::Align2::LEFT_TOP,
        "Wants to be friends",
        theme::regular(12.0),
        theme::TEXT_DIM,
    );
    let accept = Rect::from_min_size(Pos2::new(rect.right() - 124.0, rect.center().y - 12.0), vec2(58.0, 24.0));
    let ignore = Rect::from_min_size(Pos2::new(rect.right() - 62.0, rect.center().y - 12.0), vec2(54.0, 24.0));
    if row_button(ui, accept, Id::new(("req-accept", f.id.0)), "Accept", true) {
        cmds.push(Command::FriendAct(FriendAction::Add, f.id));
    }
    if row_button(ui, ignore, Id::new(("req-ignore", f.id.0)), "Ignore", false) {
        cmds.push(Command::FriendAct(FriendAction::Remove, f.id));
    }
    let mut profile = false;
    widgets::context_menu(&hover_resp, |ui| {
        if ui.button("View profile").clicked() {
            profile = true;
            ui.close();
        }
        if ui.button("Block").clicked() {
            cmds.push(Command::FriendAct(FriendAction::Block, f.id));
            ui.close();
        }
    });
    if profile || hover_resp.clicked() {
        cmds.push(Command::ViewProfile(f.id));
    }
}

/// A request we sent that is still waiting: Cancel.
fn pending_row(ui: &mut Ui, tex: &mut Textures, client: &Client, rect: Rect, f: &Friend, cmds: &mut Vec<Command>) {
    let resp = ui.interact(rect, Id::new(("friend-pending", f.id.0)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_HOVER);
    }
    let name = f.display_name();
    let av = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.center().y - AVATAR / 2.0), Vec2::splat(AVATAR));
    avatar(ui, tex, client, av, f.avatar_url.as_deref(), &name, theme::FRIEND_OFFLINE);
    let clip =
        Rect::from_min_max(Pos2::new(av.right() + 10.0, rect.top()), Pos2::new(rect.right() - 76.0, rect.bottom()));
    let p = ui.painter().with_clip_rect(clip);
    let color = if f.has_name() { theme::FRIEND_OFFLINE } else { theme::TEXT_DIM };
    p.text(Pos2::new(clip.left(), rect.center().y - 1.0), egui::Align2::LEFT_BOTTOM, name, theme::medium(14.0), color);
    p.text(
        Pos2::new(clip.left(), rect.center().y + 1.0),
        egui::Align2::LEFT_TOP,
        "Request sent",
        theme::regular(12.0),
        theme::TEXT_UNINSTALLED,
    );
    let cancel = Rect::from_min_size(Pos2::new(rect.right() - 68.0, rect.center().y - 12.0), vec2(60.0, 24.0));
    if row_button(ui, cancel, Id::new(("pend-cancel", f.id.0)), "Cancel", false) {
        cmds.push(Command::FriendAct(FriendAction::Remove, f.id));
    }
    if resp.clicked() {
        cmds.push(Command::ViewProfile(f.id));
    }
}

/// A group chat: icon, name, member count and tagline. Returns true when clicked.
fn group_row(ui: &mut Ui, rect: Rect, g: &sr_client::GroupInfo, unread: u32) -> bool {
    let resp = ui.interact(rect, Id::new(("friend-group-chat", g.id)), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, 0.0, theme::BG_ROW_HOVER.gamma_multiply(hover));
    }
    let av = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.center().y - AVATAR / 2.0), Vec2::splat(AVATAR));
    ui.painter().rect_filled(av, theme::RADIUS, theme::BG_CAPSULE);
    ui.painter().text(
        av.center(),
        egui::Align2::CENTER_CENTER,
        icon::USERS_THREE,
        theme::regular(18.0),
        theme::TEXT_DIM,
    );
    let clip =
        Rect::from_min_max(Pos2::new(av.right() + 10.0, rect.top()), Pos2::new(rect.right() - 40.0, rect.bottom()));
    let p = ui.painter().with_clip_rect(clip);
    p.text(
        Pos2::new(clip.left(), rect.center().y - 1.0),
        egui::Align2::LEFT_BOTTOM,
        &g.name,
        theme::medium(14.0),
        theme::TEXT,
    );
    let sub = match (g.members, g.tagline.is_empty()) {
        (0, true) => String::new(),
        (n, true) => format!("{n} members"),
        (0, false) => g.tagline.clone(),
        (n, false) => format!("{n} members - {}", g.tagline),
    };
    p.text(
        Pos2::new(clip.left(), rect.center().y + 1.0),
        egui::Align2::LEFT_TOP,
        sub,
        theme::regular(12.0),
        theme::TEXT_UNINSTALLED,
    );
    if unread > 0 {
        badge(ui, Pos2::new(rect.right() - 24.0, rect.center().y), unread);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// "Name  14:05" above a run of messages from one sender.
fn block_header(ui: &mut Ui, mine: bool, who: &str, time: &str) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W - 4.0, 16.0), Sense::hover());
    let align = if mine { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
    let x = if mine { rect.right() - 12.0 } else { rect.left() + 12.0 };
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        align,
        format!("{who}  {time}"),
        theme::regular(11.0),
        theme::TEXT_DIM,
    );
}

fn bubble(ui: &mut Ui, mine: bool, text: &str, delivery: &Delivery) {
    let max_text = theme::FRIENDS_W * 0.74 - 20.0;
    let alpha = if *delivery == Delivery::Sending { 0.6 } else { 1.0 };
    let galley = ui.painter().layout(text.to_string(), theme::regular(13.5), theme::TEXT, max_text);
    let size = galley.size() + vec2(20.0, 14.0);
    let (row, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W - 4.0, size.y + 2.0), Sense::hover());
    let x = if mine { row.right() - 10.0 - size.x } else { row.left() + 10.0 };
    let r = Rect::from_min_size(Pos2::new(x, row.top() + 1.0), size);
    let fill = if mine { theme::CHAT_MINE } else { theme::CHAT_THEIRS };
    ui.painter().rect_filled(r, 9.0, fill.gamma_multiply(alpha));
    ui.painter().galley(r.min + vec2(10.0, 7.0), galley, theme::TEXT);
    if let Delivery::Failed(why) = delivery {
        let (note, _) = ui.allocate_exact_size(vec2(theme::FRIENDS_W - 4.0, 14.0), Sense::hover());
        ui.painter().text(
            Pos2::new(note.right() - 12.0, note.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("Not sent: {why}"),
            theme::regular(11.0),
            theme::ERROR_TEXT,
        );
    }
}

/// The bottom-bar button. Shows the unread count; returns true when clicked.
pub fn bottom_button(ui: &mut Ui, client: &Client) -> bool {
    let fs = &client.state().friends;
    let unread = fs.unread_total();
    let on = anim::animate_bool(ui.ctx(), Id::new("friends-button-on"), fs.open, anim::FAST);
    let galley = ui.painter().layout_no_wrap("Friends & Chat".to_string(), theme::regular(13.0), theme::BOTTOM_TEXT);
    let badge_w = if unread > 0 { 30.0 } else { 0.0 };
    let (rect, resp) =
        ui.allocate_exact_size(vec2(galley.size().x + badge_w + 34.0, ui.available_height()), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let color = theme::BOTTOM_TEXT.lerp_to_gamma(theme::TEXT, hover).lerp_to_gamma(theme::ACCENT, on);
    // Text, then Steam's small square icon at the right edge.
    ui.painter().galley_with_override_text_color(
        Pos2::new(rect.right() - 28.0 - galley.size().x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    crate::steam_icons::friends_square(
        ui.painter(),
        Pos2::new(rect.right() - 12.0, rect.center().y),
        16.0,
        color,
        theme::BG_TOP,
    );
    if unread > 0 {
        badge(ui, Pos2::new(rect.left() + 14.0, rect.center().y), unread);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The toast in the bottom bar; returns true when clicked (open that chat).
pub fn toast_label(ui: &mut Ui, toast: &Toast, width: f32) -> bool {
    let text = format!("{}  {}: {}", icon::CHAT_CIRCLE_TEXT, toast.name, toast.text.replace('\n', " "));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ui.available_height()), Sense::click());
    let p = ui.painter().with_clip_rect(rect);
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let color = theme::FRIEND_ONLINE.lerp_to_gamma(Color32::WHITE, 0.3 * hover);
    let g = p.layout_no_wrap(text, theme::medium(12.0), color);
    let g = truncated(&p, &g, width, color);
    p.galley_with_override_text_color(
        Pos2::new(rect.right() - g.size().x, rect.center().y - g.size().y / 2.0),
        g,
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_client::GamePlayed;
    use sr_model::SteamId;

    fn friend(state: PersonaState, game: Option<&str>, logoff: u64) -> Friend {
        Friend {
            id: SteamId(76_561_198_000_000_001),
            name: "Ann".into(),
            avatar_url: None,
            state,
            game: game.map(|n| GamePlayed { app_id: 1, name: n.into() }),
            last_logoff: logoff,
            rich: None,
        }
    }

    #[test]
    fn status_lines_follow_steam() {
        let now = 1_000_000;
        assert_eq!(status_text(&friend(PersonaState::Online, Some("Dota 2"), 0), now), "In-Game: Dota 2");
        assert_eq!(status_text(&friend(PersonaState::Online, None, 0), now), "Online");
        assert_eq!(status_text(&friend(PersonaState::Away, None, 0), now), "Away");
        assert_eq!(status_text(&friend(PersonaState::Offline, None, now - 360), now), "Last online 6 min ago");
        assert_eq!(status_text(&friend(PersonaState::Offline, None, now - 7300), now), "Last online 2 h ago");
        assert_eq!(status_text(&friend(PersonaState::Offline, None, 0), now), "Offline");
        let mut rich = friend(PersonaState::Online, Some("Dota 2"), 0);
        rich.rich = Some("Hero Pick".into());
        assert_eq!(status_text(&rich, now), "In-Game: Dota 2 - Hero Pick");
    }

    #[test]
    fn sections_follow_state() {
        assert!(group_of(&friend(PersonaState::Online, Some("G"), 0)) == Group::InGame);
        assert!(group_of(&friend(PersonaState::Busy, None, 0)) == Group::Online);
        assert!(group_of(&friend(PersonaState::Offline, Some("G"), 0)) == Group::Offline);
    }
}
