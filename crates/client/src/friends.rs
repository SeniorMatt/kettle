//! Friends list and chat: the state the UI reads, the reducer for session events, and the connection
//! supervisor that keeps one friends session running on the shared Steam connection (`cm_hub`).
//!
//! Everything is bounded: at most [`MAX_FRIENDS`] friends, [`MAX_MESSAGES`] messages per chat, [`MAX_CHATS`]
//! chats in memory and [`MAX_TABS`] open chat tabs. Tokens never enter the state.

use crate::cm_hub::HubState;
use crate::{Client, Event};
use sr_model::SteamId;
use sr_steam::friends::{
    ChatLine, FriendsEvent, FriendsHandle, FriendsSession, GroupLine, PersonaUpdate, Relationship, parse_friend_target,
};
pub use sr_steam::friends::{FriendAction, GamePlayed, GroupInfo, PersonaState, RoomInfo};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shown instead of a name that hasn't arrived yet.
pub const LOADING: &str = "Loading\u{2026}";
pub const MAX_FRIENDS: usize = 2000;
/// Messages kept per group chat channel.
pub const MAX_GROUP_MESSAGES: usize = 300;
pub const MAX_MESSAGES: usize = 500;
pub const MAX_CHATS: usize = 64;
pub const MAX_TABS: usize = 8;
/// Accounts whose data is kept (friends, requests and a margin).
const MAX_PEOPLE: usize = 2 * MAX_FRIENDS;
/// Longest message we send (characters).
pub const MAX_TEXT: usize = 2000;
/// Pause before the friends session is restarted on a connection that is still up.
const RETRY_MIN: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Default)]
pub enum FriendsStatus {
    /// Not signed in, or demo mode.
    #[default]
    Offline,
    Connecting,
    Connected,
    /// The connection failed or dropped; the supervisor is waiting to retry. Holds the reason.
    Disconnected(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Friend {
    pub id: SteamId,
    pub name: String,
    pub avatar_url: Option<String>,
    pub state: PersonaState,
    pub game: Option<GamePlayed>,
    /// Unix seconds; 0 = unknown.
    pub last_logoff: u64,
    /// Simple rich-presence text, e.g. "In a match".
    pub rich: Option<String>,
}

/// What the friends list is sorted by, like the official client: in-game first, then online (including
/// away, busy and snooze), then offline by most recent logoff. Within a group by name.
pub fn sort_key(f: &Friend) -> (u8, u64, String, u64) {
    let (rank, recent) = if f.game.is_some() && f.state.is_online() {
        (0, 0)
    } else if f.state.is_online() {
        (1, 0)
    } else {
        (2, u64::MAX - f.last_logoff)
    };
    (rank, recent, f.display_name().to_lowercase(), f.id.0)
}

impl Friend {
    fn new(id: u64) -> Self {
        Friend {
            id: SteamId(id),
            name: String::new(),
            avatar_url: None,
            state: PersonaState::Offline,
            game: None,
            last_logoff: 0,
            rich: None,
        }
    }

    /// Steam has told us the name.
    pub fn has_name(&self) -> bool {
        !self.name.is_empty()
    }

    /// The name to show. Never a raw SteamID: "Loading…" until the name arrives.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { LOADING.to_string() } else { self.name.clone() }
    }
}

/// Our own persona.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Me {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Delivery {
    Sent,
    Sending,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub from_me: bool,
    pub text: String,
    /// Unix seconds.
    pub time: u64,
    pub ordinal: u32,
    /// Ties a message we sent to Steam's answer (0 for received ones).
    pub local_id: u64,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryState {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
}

#[derive(Debug, Clone, Default)]
pub struct Chat {
    pub messages: VecDeque<Message>,
    pub unread: u32,
    pub history: HistoryState,
    /// Bumped on every change, so the view can keep its scroll-to-bottom logic cheap.
    pub rev: u64,
    touched: u64,
}

/// A message that arrived for a chat that isn't on screen. The UI shows it for a few seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub seq: u64,
    pub friend: SteamId,
    pub name: String,
    pub text: String,
}

/// One line in a group chat channel.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMessage {
    pub from_me: bool,
    /// 32-bit account id of the sender.
    pub sender: u32,
    pub text: String,
    pub time: u64,
    pub ordinal: u32,
    pub local_id: u64,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Default)]
pub struct GroupChat {
    pub messages: VecDeque<GroupMessage>,
    pub unread: u32,
    pub history: HistoryState,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum GroupsState {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct FriendsState {
    /// The panel is shown.
    pub open: bool,
    pub status: FriendsStatus,
    pub me: Me,
    /// The state we asked for (shown in the picker).
    pub my_state: PersonaState,
    /// Friends only (relationship "friend"), sorted by [`sort_key`].
    pub friends: Vec<Friend>,
    /// People who asked to be our friend.
    pub requests: Vec<Friend>,
    /// People we asked, who haven't answered.
    pub pending: Vec<Friend>,
    /// How many accounts we have blocked (they are not listed).
    pub blocked: usize,
    /// Chat groups we are in.
    pub groups: Vec<GroupInfo>,
    pub groups_state: GroupsState,
    pub group_chats: HashMap<(u64, u64), GroupChat>,
    /// The group channel shown in the panel (group id, channel id).
    pub active_group: Option<(u64, u64)>,
    /// Bumped whenever `friends` changes.
    pub rev: u64,
    pub chats: HashMap<u64, Chat>,
    /// Open chat tabs, oldest first.
    pub tabs: Vec<SteamId>,
    /// The chat shown in the panel (`None` = the friends list).
    pub active: Option<SteamId>,
    pub toast: Option<Toast>,
    /// The window is not focused (set by the UI; desktop notifications show only then).
    pub window_unfocused: bool,
    index: HashMap<u64, usize>,
}

impl FriendsState {
    pub fn friend(&self, id: SteamId) -> Option<&Friend> {
        self.index.get(&id.0).and_then(|i| self.friends.get(*i))
    }

    pub fn chat(&self, id: SteamId) -> Option<&Chat> {
        self.chats.get(&id.0)
    }

    pub fn unread(&self, id: SteamId) -> u32 {
        self.chats.get(&id.0).map_or(0, |c| c.unread)
    }

    pub fn unread_total(&self) -> u32 {
        self.chats.values().map(|c| c.unread).sum::<u32>() + self.group_chats.values().map(|c| c.unread).sum::<u32>()
    }

    /// Our persona name once Steam has sent it (not the account name).
    pub fn persona_name(&self) -> Option<&str> {
        Some(self.me.name.as_str()).filter(|n| !n.is_empty())
    }

    /// URL of our avatar once known.
    pub fn my_avatar_url(&self) -> Option<&str> {
        self.me.avatar_url.as_deref()
    }

    /// Unread messages in all channels of a group chat.
    pub fn group_unread(&self, group: u64) -> u32 {
        self.group_chats.iter().filter(|((g, _), _)| *g == group).map(|(_, c)| c.unread).sum()
    }

    pub fn group(&self, id: u64) -> Option<&GroupInfo> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Name of a group chat member by account id: a friend's name, else "Member".
    pub fn member_name(&self, account: u32) -> String {
        let id = sr_steam::friends::id_from_friend_code(account);
        if id == self.me.id {
            return self.persona_name().unwrap_or("Me").to_string();
        }
        match self.friend(SteamId(id)) {
            Some(f) if f.has_name() => f.name.clone(),
            _ => "Member".to_string(),
        }
    }

    fn reindex(&mut self) {
        self.friends.sort_by_cached_key(sort_key);
        self.index = self.friends.iter().enumerate().map(|(i, f)| (f.id.0, i)).collect();
        self.rev += 1;
    }
}

/// Everything the client keeps about friends that the UI doesn't read.
#[derive(Default)]
pub struct FriendsInner {
    /// The live session's handle, set by the supervisor.
    handle: Arc<Mutex<Option<FriendsHandle>>>,
    /// The persona state we want (`PersonaState::to_wire`), read when a session (re)connects.
    wanted: Arc<AtomicU32>,
    /// Relationships from the friends list (`None` until the first full list arrives).
    rels: Option<HashMap<u64, Relationship>>,
    /// Everything Steam told us about accounts (merged); bounded by [`MAX_PEOPLE`].
    people: HashMap<u64, Friend>,
    /// Set once the user picked a state (or the saved choice was loaded).
    state_loaded: bool,
    next_local: u64,
    toast_seq: u64,
    /// Chat counter for the "touched" order.
    clock: u64,
    demo: bool,
}

impl Client {
    pub(crate) fn friends_open(&mut self, open: bool) {
        self.state.friends.open = open;
        if open && let Some(id) = self.state.friends.active {
            self.open_chat(id);
        }
    }

    pub(crate) fn open_chat(&mut self, id: SteamId) {
        let f = &mut self.state.friends;
        f.open = true;
        f.active = Some(id);
        f.active_group = None;
        if !f.tabs.contains(&id) {
            f.tabs.push(id);
            while f.tabs.len() > MAX_TABS {
                f.tabs.remove(0);
            }
        }
        if f.toast.as_ref().is_some_and(|t| t.friend == id) {
            f.toast = None;
        }
        let clock = self.friends.tick();
        let chat = f.chats.entry(id.0).or_default();
        chat.unread = 0;
        chat.touched = clock;
        chat.rev += 1;
        if chat.history == HistoryState::NotLoaded {
            if self.friends.demo {
                chat.history = HistoryState::Loaded;
            } else if self.with_handle(|h| h.history(id.0))
                && let Some(c) = self.state.friends.chats.get_mut(&id.0)
            {
                c.history = HistoryState::Loading;
            }
        }
        self.evict_chats();
    }

    pub(crate) fn close_chat(&mut self, id: SteamId) {
        let f = &mut self.state.friends;
        f.tabs.retain(|t| *t != id);
        if f.active == Some(id) {
            f.active = f.tabs.last().copied();
        }
    }

    /// Show the friends list again (the chat tabs stay).
    pub(crate) fn show_friend_list(&mut self) {
        self.state.friends.active = None;
        self.state.friends.active_group = None;
    }

    pub(crate) fn send_chat(&mut self, id: SteamId, text: String) {
        let text: String = text.trim().chars().take(MAX_TEXT).collect();
        if text.is_empty() {
            return;
        }
        self.friends.next_local += 1;
        let local_id = self.friends.next_local;
        let now = now_secs();
        let demo = self.friends.demo;
        let delivery = if demo { Delivery::Sent } else { Delivery::Sending };
        let msg = Message { from_me: true, text: text.clone(), time: now, ordinal: 0, local_id, delivery };
        self.push_message(id.0, msg);
        if demo {
            self.demo_echo(id, &text);
        } else if !self.with_handle(|h| h.send_message(id.0, local_id, &text)) {
            self.mark_delivery(id.0, local_id, Delivery::Failed("Not connected to Steam".into()));
        }
    }

    pub(crate) fn set_persona_state(&mut self, state: PersonaState) {
        self.state.friends.my_state = state;
        self.friends.wanted.store(state.to_wire(), Ordering::Relaxed);
        self.friends.state_loaded = true;
        if !self.friends.demo {
            save_prefs(&FriendsPrefs { persona_state: Some(state.to_wire()) });
        }
        self.with_handle(|h| h.set_state(state));
    }

    /// Ask Steam for our own persona again (after the profile name changed).
    pub(crate) fn friends_request_own(&mut self) {
        let me = self.state.friends.me.id;
        if me != 0 {
            self.with_handle(|h| h.request_data(vec![me]));
        }
    }

    /// Do something to another account (add, accept, remove, block).
    pub(crate) fn friend_action(&mut self, action: FriendAction, id: SteamId) {
        if self.friends.demo {
            self.state.notice = Some("Not available in the demo".into());
            return;
        }
        if !self.with_handle(|h| h.friend_action(action, id.0)) {
            self.state.notice = Some("Not connected to Steam".into());
        }
    }

    /// "Add a friend": a friend code, a SteamID or a profile link.
    pub(crate) fn add_friend_input(&mut self, input: &str) {
        let Some(id) = parse_friend_target(input) else {
            self.state.notice =
                Some("Enter a friend code, a SteamID or a link like steamcommunity.com/profiles/7656119...".into());
            return;
        };
        if id == self.state.friends.me.id {
            self.state.notice = Some("That is you".into());
            return;
        }
        self.friend_action(FriendAction::Add, SteamId(id));
    }

    /// Reload the list of group chats.
    pub(crate) fn load_groups(&mut self) {
        if self.friends.demo {
            return;
        }
        if self.with_handle(|h| h.load_groups()) {
            self.state.friends.groups_state = GroupsState::Loading;
        }
    }

    /// Open a channel of a group chat.
    pub(crate) fn open_group(&mut self, group: u64, room: u64) {
        let demo = self.friends.demo;
        let f = &mut self.state.friends;
        f.open = true;
        f.active = None;
        f.active_group = Some((group, room));
        let chat = f.group_chats.entry((group, room)).or_default();
        chat.unread = 0;
        chat.rev += 1;
        if chat.history == HistoryState::NotLoaded {
            if demo {
                chat.history = HistoryState::Loaded;
            } else if self.with_handle(|h| h.group_history(group, room))
                && let Some(c) = self.state.friends.group_chats.get_mut(&(group, room))
            {
                c.history = HistoryState::Loading;
            }
        }
    }

    pub(crate) fn send_group_chat(&mut self, group: u64, room: u64, text: String) {
        let text: String = text.trim().chars().take(MAX_TEXT).collect();
        if text.is_empty() {
            return;
        }
        self.friends.next_local += 1;
        let local_id = self.friends.next_local;
        let demo = self.friends.demo;
        let me = sr_steam::friends::friend_code(self.state.friends.me.id);
        let msg = GroupMessage {
            from_me: true,
            sender: me,
            text: text.clone(),
            time: now_secs(),
            ordinal: 0,
            local_id,
            delivery: if demo { Delivery::Sent } else { Delivery::Sending },
        };
        self.push_group_message(group, room, msg);
        if !demo && !self.with_handle(|h| h.group_send(group, room, local_id, &text)) {
            self.mark_group_delivery(group, room, local_id, Delivery::Failed("Not connected to Steam".into()));
        }
    }

    fn push_group_message(&mut self, group: u64, room: u64, msg: GroupMessage) {
        let chat = self.state.friends.group_chats.entry((group, room)).or_default();
        chat.rev += 1;
        chat.messages.push_back(msg);
        while chat.messages.len() > MAX_GROUP_MESSAGES {
            chat.messages.pop_front();
        }
    }

    fn mark_group_delivery(&mut self, group: u64, room: u64, local_id: u64, delivery: Delivery) {
        if let Some(chat) = self.state.friends.group_chats.get_mut(&(group, room)) {
            chat.rev += 1;
            if let Some(m) = chat.messages.iter_mut().rev().find(|m| m.local_id == local_id) {
                m.delivery = delivery;
            }
        }
    }

    /// Run `f` on the live session's handle, if there is one.
    fn with_handle(&self, f: impl FnOnce(&FriendsHandle) -> bool) -> bool {
        let guard = self.friends.handle.lock().expect("friends handle");
        guard.as_ref().is_some_and(f)
    }

    // ---- connection -----------------------------------------------------------------------------

    /// Keep a friends session running for the current account until the generation changes.
    pub(crate) fn start_friends(&mut self) {
        if self.session.is_none() {
            return;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        self.friends.rels = None;
        self.friends.people.clear();
        // Our own state: what the user chose last time, else Online. (An Offline state would make Steam send
        // no presence at all, so every friend would look offline.)
        if !self.friends.state_loaded {
            let saved = load_prefs().persona_state.map(PersonaState::from_wire);
            self.state.friends.my_state = saved.unwrap_or(PersonaState::Online);
            self.friends.state_loaded = true;
        }
        self.friends.wanted.store(self.state.friends.my_state.to_wire(), Ordering::Relaxed);
        self.state.friends.status = FriendsStatus::Connecting;
        let Some(mut hub) = self.hub.clone() else { return };
        let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
        let (slot, wanted) = (self.friends.handle.clone(), self.friends.wanted.clone());
        self.rt.spawn(async move {
            let send = |ev: FriendsEvent| {
                let (tx, wake) = (tx.clone(), wake.clone());
                async move {
                    let _ = tx.send(Event::Friends { generation, ev }).await;
                    wake();
                }
            };
            let mut last = 0;
            while let Some((id, cm)) = hub.wait_online(last).await {
                last = id;
                let initial = PersonaState::from_wire(wanted.load(Ordering::Relaxed));
                let (handle, mut rx) = FriendsSession::start(cm.conn.clone(), cm.early.clone(), initial);
                *slot.lock().expect("friends handle") = Some(handle);
                let mut ended_alone = false;
                loop {
                    tokio::select! {
                        ev = rx.recv() => match ev {
                            Some(ev) => send(ev).await,
                            None => {
                                ended_alone = true;
                                break;
                            }
                        },
                        changed = hub.rx.changed() => {
                            let same = matches!(&*hub.rx.borrow(), HubState::Online { id: now, .. } if *now == id);
                            if changed.is_err() || !same || current.load(Ordering::Relaxed) != generation {
                                break;
                            }
                        }
                    }
                }
                *slot.lock().expect("friends handle") = None;
                if ended_alone && current.load(Ordering::Relaxed) == generation {
                    // The friends session ended on a connection the hub still considers alive: try it again.
                    tokio::time::sleep(RETRY_MIN).await;
                    last = id - 1;
                }
            }
        });
    }

    /// Forget everything about friends (sign-out).
    pub(crate) fn reset_friends(&mut self) {
        *self.friends.handle.lock().expect("friends handle") = None;
        let my_state = self.state.friends.my_state;
        self.friends.rels = None;
        self.friends.people.clear();
        self.friends.demo = false;
        self.state.friends = FriendsState { my_state, ..Default::default() };
    }

    // ---- reducer --------------------------------------------------------------------------------

    pub(crate) fn apply_friends(&mut self, ev: FriendsEvent) {
        match ev {
            FriendsEvent::Connected { me } => {
                self.state.friends.status = FriendsStatus::Connected;
                self.state.friends.me.id = me;
                self.reload_open_history();
                self.load_groups();
                if let Some((g, r)) = self.state.friends.active_group {
                    self.with_handle(|h| h.group_history(g, r));
                }
            }
            FriendsEvent::Disconnected(why) => {
                self.state.friends.status = FriendsStatus::Disconnected(why);
                for c in self.state.friends.chats.values_mut() {
                    if c.history == HistoryState::Loading {
                        c.history = HistoryState::NotLoaded;
                    }
                }
            }
            FriendsEvent::List { incremental, entries } => self.apply_list(incremental, entries),
            FriendsEvent::Personas(ups) => self.apply_personas(ups),
            FriendsEvent::Message { friend, line } => self.incoming(friend, line),
            FriendsEvent::History { friend, lines } => self.apply_history(friend, lines),
            FriendsEvent::HistoryFailed { friend, error } => {
                if let Some(c) = self.state.friends.chats.get_mut(&friend) {
                    c.history = HistoryState::NotLoaded;
                }
                self.state.notice = Some(format!("Couldn't load the chat history: {error}"));
            }
            FriendsEvent::Groups(groups) => {
                let f = &mut self.state.friends;
                f.groups = groups;
                f.groups.sort_by_key(|g| g.name.to_lowercase());
                f.groups_state = GroupsState::Loaded;
                f.rev += 1;
                let keep: HashSet<(u64, u64)> =
                    f.groups.iter().flat_map(|g| g.rooms.iter().map(|r| (g.id, r.id))).collect();
                if f.active_group.is_some_and(|k| !keep.contains(&k)) {
                    f.active_group = None;
                }
            }
            FriendsEvent::GroupsFailed(e) => self.state.friends.groups_state = GroupsState::Failed(e),
            FriendsEvent::GroupMessage { group, room, line } => self.group_incoming(group, room, line),
            FriendsEvent::GroupHistory { group, room, lines } => {
                let chat = self.state.friends.group_chats.entry((group, room)).or_default();
                chat.history = HistoryState::Loaded;
                let mut have: HashSet<(u64, u32)> = chat.messages.iter().map(|m| (m.time, m.ordinal)).collect();
                let mut all: Vec<GroupMessage> = lines
                    .into_iter()
                    .filter(|l| have.insert((l.time, l.ordinal)))
                    .map(|l| group_message(l, 0, Delivery::Sent))
                    .chain(chat.messages.drain(..))
                    .collect();
                all.sort_by_key(|m| (m.time, m.ordinal));
                let skip = all.len().saturating_sub(MAX_GROUP_MESSAGES);
                chat.messages = all.into_iter().skip(skip).collect();
                chat.rev += 1;
            }
            FriendsEvent::GroupHistoryFailed { group, room, error } => {
                if let Some(c) = self.state.friends.group_chats.get_mut(&(group, room)) {
                    c.history = HistoryState::NotLoaded;
                }
                self.state.notice = Some(format!("Couldn't load the group chat: {error}"));
            }
            FriendsEvent::GroupSent { group, room, token, result } => match result {
                Ok(line) => {
                    if let Some(c) = self.state.friends.group_chats.get_mut(&(group, room)) {
                        c.rev += 1;
                        if let Some(m) = c.messages.iter_mut().rev().find(|m| m.local_id == token) {
                            (m.time, m.ordinal, m.delivery) = (line.time, line.ordinal, Delivery::Sent);
                        }
                    }
                }
                Err(e) => self.mark_group_delivery(group, room, token, Delivery::Failed(e)),
            },
            FriendsEvent::Action { what, result } => {
                self.state.notice = Some(match result {
                    Ok(()) => what,
                    Err(e) => format!("Steam couldn't do that: {e}"),
                });
            }
            FriendsEvent::Sent { friend, token, result } => match result {
                Ok(line) => {
                    if let Some(m) = self.find_message(friend, token) {
                        (m.time, m.ordinal, m.delivery) = (line.time, line.ordinal, Delivery::Sent);
                    }
                }
                Err(e) => self.mark_delivery(friend, token, Delivery::Failed(e)),
            },
        }
    }

    /// After a reconnect, the chat on screen asks Steam for what it missed.
    fn reload_open_history(&mut self) {
        if let Some(id) = self.state.friends.active
            && self.state.friends.open
            && self.with_handle(|h| h.history(id.0))
            && let Some(c) = self.state.friends.chats.get_mut(&id.0)
        {
            c.history = HistoryState::Loading;
        }
    }

    fn apply_list(&mut self, incremental: bool, entries: Vec<(u64, Relationship)>) {
        let mut rels = if incremental { self.friends.rels.take().unwrap_or_default() } else { HashMap::new() };
        for (id, rel) in entries {
            match rel {
                Relationship::None => {
                    rels.remove(&id);
                }
                _ if rels.len() < MAX_PEOPLE || rels.contains_key(&id) => {
                    rels.insert(id, rel);
                }
                _ => {}
            }
        }
        self.friends.rels = Some(rels);
        self.rebuild_lists();
    }

    fn apply_personas(&mut self, ups: Vec<PersonaUpdate>) {
        let f = &mut self.state.friends;
        let mut came_online: Vec<(String, Option<String>)> = Vec::new();
        for u in ups {
            if u.id == f.me.id && u.id != 0 {
                if let Some(n) = &u.name {
                    f.me.name = n.clone();
                }
                if u.avatar_url.is_some() {
                    f.me.avatar_url = u.avatar_url.clone();
                }
                continue;
            }
            let room = self.friends.people.len() < MAX_PEOPLE || self.friends.people.contains_key(&u.id);
            if room {
                let known = self.friends.people.contains_key(&u.id);
                let entry = self.friends.people.entry(u.id).or_insert_with(|| Friend::new(u.id));
                let was_offline = entry.state == PersonaState::Offline;
                let was_playing = entry.game.is_some();
                merge(entry, &u);
                let online = !matches!(entry.state, PersonaState::Offline | PersonaState::Invisible);
                if known && entry.has_name() && f.window_unfocused {
                    if was_offline && online {
                        came_online.push((entry.name.clone(), None));
                    } else if !was_playing && let Some(g) = &entry.game {
                        came_online.push((entry.name.clone(), Some(g.name.clone())));
                    }
                }
            }
        }
        for (name, game) in came_online {
            match game {
                None => self.notify(crate::NotifyKind::FriendOnline, &name, "is now online"),
                Some(g) => self.notify(crate::NotifyKind::FriendGame, &name, &format!("started playing {g}")),
            }
        }
        self.rebuild_lists();
    }

    /// Friends, incoming requests and pending requests, from the relationships and what we know about
    /// each account. Before the friends list has arrived nothing is shown.
    fn rebuild_lists(&mut self) {
        let Some(rels) = &self.friends.rels else { return };
        let (friends, requests, pending, blocked) = split_relationships(rels, &self.friends.people);
        let f = &mut self.state.friends;
        f.friends = friends;
        f.requests = requests;
        f.pending = pending;
        f.blocked = blocked;
        f.reindex();
    }

    fn incoming(&mut self, friend: u64, line: ChatLine) {
        let from_me = line.from_me;
        let msg = Message {
            from_me,
            text: line.text,
            time: line.time,
            ordinal: line.ordinal,
            local_id: 0,
            delivery: Delivery::Sent,
        };
        let text = msg.text.clone();
        self.push_message(friend, msg);
        let f = &mut self.state.friends;
        let visible = f.open && f.active == Some(SteamId(friend));
        if !from_me && !visible {
            if let Some(c) = f.chats.get_mut(&friend) {
                c.unread += 1;
            }
            self.friends.toast_seq += 1;
            let name = f.friend(SteamId(friend)).map_or_else(|| LOADING.to_string(), Friend::display_name);
            let unfocused = f.window_unfocused;
            f.toast = Some(Toast {
                seq: self.friends.toast_seq,
                friend: SteamId(friend),
                name: name.clone(),
                text: text.clone(),
            });
            if unfocused {
                self.notify(crate::NotifyKind::ChatMessage, &name, &text);
            }
        }
    }

    fn group_incoming(&mut self, group: u64, room: u64, line: GroupLine) {
        let from_me = line.from_me;
        self.push_group_message(group, room, group_message(line, 0, Delivery::Sent));
        let f = &mut self.state.friends;
        let visible = f.open && f.active_group == Some((group, room));
        if !from_me
            && !visible
            && let Some(c) = f.group_chats.get_mut(&(group, room))
        {
            c.unread += 1;
        }
    }

    fn apply_history(&mut self, friend: u64, lines: Vec<ChatLine>) {
        let clock = self.friends.tick();
        let chat = self.state.friends.chats.entry(friend).or_default();
        chat.touched = clock;
        chat.history = HistoryState::Loaded;
        let mut have: HashSet<(u64, u32)> =
            chat.messages.iter().filter(|m| m.ordinal != 0 || m.local_id == 0).map(|m| (m.time, m.ordinal)).collect();
        let mut older: Vec<Message> = Vec::new();
        for l in lines {
            if have.insert((l.time, l.ordinal)) {
                older.push(Message {
                    from_me: l.from_me,
                    text: l.text,
                    time: l.time,
                    ordinal: l.ordinal,
                    local_id: 0,
                    delivery: Delivery::Sent,
                });
            }
        }
        // History is merged by time with what already arrived in this session.
        let mut all: Vec<Message> = older.into_iter().chain(chat.messages.drain(..)).collect();
        all.sort_by_key(|m| (m.time, m.ordinal));
        let skip = all.len().saturating_sub(MAX_MESSAGES);
        chat.messages = all.into_iter().skip(skip).collect();
        chat.rev += 1;
        self.evict_chats();
    }

    fn push_message(&mut self, friend: u64, msg: Message) {
        let clock = self.friends.tick();
        let chat = self.state.friends.chats.entry(friend).or_default();
        chat.touched = clock;
        chat.rev += 1;
        chat.messages.push_back(msg);
        while chat.messages.len() > MAX_MESSAGES {
            chat.messages.pop_front();
        }
        self.evict_chats();
    }

    fn find_message(&mut self, friend: u64, local_id: u64) -> Option<&mut Message> {
        let chat = self.state.friends.chats.get_mut(&friend)?;
        chat.rev += 1;
        chat.messages.iter_mut().rev().find(|m| m.local_id == local_id)
    }

    fn mark_delivery(&mut self, friend: u64, local_id: u64, delivery: Delivery) {
        if let Some(m) = self.find_message(friend, local_id) {
            m.delivery = delivery;
        }
    }

    /// Keep at most [`MAX_CHATS`] chats: drop the least recently used ones that aren't open or unread.
    fn evict_chats(&mut self) {
        let f = &mut self.state.friends;
        if f.chats.len() <= MAX_CHATS {
            return;
        }
        let mut idle: Vec<(u64, u64)> = f
            .chats
            .iter()
            .filter(|(id, c)| c.unread == 0 && !f.tabs.iter().any(|t| t.0 == **id))
            .map(|(id, c)| (c.touched, *id))
            .collect();
        idle.sort_unstable();
        let over = f.chats.len() - MAX_CHATS;
        for (_, id) in idle.into_iter().take(over) {
            f.chats.remove(&id);
        }
    }

    // ---- demo -----------------------------------------------------------------------------------

    /// About 40 made-up friends and a bot that answers in chat (`KETTLE_DEMO`).
    pub(crate) fn start_friends_demo(&mut self) {
        self.friends.demo = true;
        let now = now_secs();
        let f = &mut self.state.friends;
        f.status = FriendsStatus::Connected;
        f.me = Me { id: 76_561_198_000_000_000, name: "demo".into(), avatar_url: None };
        f.my_state = PersonaState::Online;
        f.friends = demo_friends(now);
        f.requests = vec![Friend { name: "Rowan".into(), ..Friend::new(DEMO_BASE + 100) }];
        f.pending = vec![Friend { name: "Sam".into(), ..Friend::new(DEMO_BASE + 101) }];
        f.groups = vec![GroupInfo {
            id: 1,
            name: "Kettle testers".into(),
            tagline: "Demo group".into(),
            members: 12,
            default_room: 11,
            rooms: vec![RoomInfo { id: 11, name: "General".into() }, RoomInfo { id: 12, name: "Bugs".into() }],
        }];
        f.groups_state = GroupsState::Loaded;
        f.reindex();
        // A friend who writes while the panel is closed, so the toast and unread badge can be seen.
        let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
        let generation = current.load(Ordering::Relaxed);
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let line = ChatLine {
                from_me: false,
                text: "hey, are you around? want to play later?".into(),
                time: now + 4,
                ordinal: 1,
            };
            let _ =
                tx.send(Event::Friends { generation, ev: FriendsEvent::Message { friend: DEMO_BASE + 3, line } }).await;
            wake();
        });
    }

    fn demo_echo(&self, id: SteamId, text: &str) {
        let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
        let generation = current.load(Ordering::Relaxed);
        let reply = format!("Echo: {text}");
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let line = ChatLine { from_me: false, text: reply, time: now_secs(), ordinal: 0 };
            let _ = tx.send(Event::Friends { generation, ev: FriendsEvent::Message { friend: id.0, line } }).await;
            wake();
        });
    }
}

impl FriendsInner {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

const DEMO_BASE: u64 = 76_561_198_100_000_000;

fn demo_friends(now: u64) -> Vec<Friend> {
    const NAMES: [&str; 40] = [
        "Alice",
        "Bob the Builder",
        "Charlie",
        "Dana",
        "Eve",
        "Frank",
        "Grace",
        "Heidi",
        "Ivan",
        "Judy",
        "Karl",
        "Liam",
        "Mallory",
        "Nina",
        "Oscar",
        "Peggy",
        "Quentin",
        "Rita",
        "Sybil",
        "Trent",
        "Uma",
        "Victor",
        "Wendy",
        "Xavier",
        "Yara",
        "Zed",
        "Amber",
        "Boris",
        "Chloe",
        "Dmitri",
        "Elena",
        "Felix",
        "Greta",
        "Hugo",
        "Iris",
        "Jonas",
        "Kira",
        "Leo",
        "Mila",
        "Noah",
    ];
    const GAMES: [(u32, &str); 4] =
        [(570, "Dota 2"), (730, "Counter-Strike 2"), (1245620, "ELDEN RING"), (292030, "The Witcher 3: Wild Hunt")];
    let default_avatar = sr_steam::friends::avatar_url(&[]);
    NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let (state, game) = match i % 8 {
                0 | 5 => {
                    let (app_id, g) = GAMES[i % GAMES.len()];
                    (PersonaState::Online, Some(GamePlayed { app_id, name: g.to_string() }))
                }
                1 | 4 => (PersonaState::Online, None),
                2 => (PersonaState::Away, None),
                3 if i % 16 == 3 => (PersonaState::Busy, None),
                3 => (PersonaState::Snooze, None),
                _ => (PersonaState::Offline, None),
            };
            Friend {
                id: SteamId(DEMO_BASE + i as u64),
                name: (*name).to_string(),
                avatar_url: (i % 2 == 0).then(|| default_avatar.clone()),
                state,
                game,
                last_logoff: now.saturating_sub(600 + i as u64 * 5_400),
                rich: None,
            }
        })
        .collect()
}

fn merge(f: &mut Friend, u: &PersonaUpdate) {
    if let Some(n) = &u.name {
        f.name = n.clone();
    }
    if let Some(s) = u.state {
        f.state = s;
    }
    if u.avatar_url.is_some() {
        f.avatar_url = u.avatar_url.clone();
    }
    if let Some(g) = &u.game {
        f.game = g.clone();
    }
    if let Some(t) = u.last_logoff {
        f.last_logoff = t;
    }
    if u.rich.is_some() {
        f.rich = u.rich.clone();
    }
}

fn group_message(l: GroupLine, local_id: u64, delivery: Delivery) -> GroupMessage {
    GroupMessage {
        from_me: l.from_me,
        sender: l.sender,
        text: l.text,
        time: l.time,
        ordinal: l.ordinal,
        local_id,
        delivery,
    }
}

/// What we remember between runs: the persona state the user picked.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct FriendsPrefs {
    #[serde(default)]
    persona_state: Option<u32>,
}

fn prefs_path() -> std::path::PathBuf {
    sr_steam::store::data_dir().join("friends.json")
}

fn load_prefs() -> FriendsPrefs {
    sr_steam::store::load_json(&prefs_path()).unwrap_or_default()
}

fn save_prefs(p: &FriendsPrefs) {
    let _ = sr_steam::store::save_json(&prefs_path(), p);
}

/// Split the relationships into friends, incoming requests, pending (sent) requests and a count of blocked
/// accounts. Only real friends land in the first list; requests are sorted too.
fn split_relationships(
    rels: &HashMap<u64, Relationship>,
    people: &HashMap<u64, Friend>,
) -> (Vec<Friend>, Vec<Friend>, Vec<Friend>, usize) {
    let (mut friends, mut requests, mut pending, mut blocked) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for (id, rel) in rels {
        let target = match rel {
            Relationship::Friend => &mut friends,
            Relationship::RequestRecipient => &mut requests,
            Relationship::RequestInitiator => &mut pending,
            Relationship::Blocked | Relationship::Ignored | Relationship::IgnoredFriend => {
                blocked += 1;
                continue;
            }
            Relationship::None | Relationship::Suggested => continue,
        };
        if target.len() < MAX_FRIENDS {
            target.push(people.get(id).cloned().unwrap_or_else(|| Friend::new(*id)));
        }
    }
    requests.sort_by_cached_key(sort_key);
    pending.sort_by_cached_key(sort_key);
    (friends, requests, pending, blocked)
}

/// Seconds east of UTC in the local time zone at `unix` (0 where the zone can't be read).
pub fn local_utc_offset(unix: u64) -> i64 {
    #[cfg(unix)]
    {
        // SAFETY: `localtime_r` only writes into the zeroed `tm` we pass.
        unsafe {
            let t = unix as libc::time_t;
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i64 }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = unix;
        0
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: u64, name: &str, state: u32) -> PersonaUpdate {
        PersonaUpdate { id, name: Some(name.into()), state: Some(PersonaState::from_wire(state)), ..Default::default() }
    }

    #[test]
    fn friends_are_sorted_and_bounded() {
        let mut s = FriendsState {
            friends: (0..3).map(|i| Friend { name: ["b", "A", "c"][i].into(), ..Friend::new(i as u64 + 1) }).collect(),
            ..Default::default()
        };
        s.reindex();
        let names: Vec<_> = s.friends.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["A", "b", "c"]);
        assert_eq!(s.friend(SteamId(1)).unwrap().name, "b");
    }

    #[test]
    fn merge_keeps_unmentioned_fields() {
        let mut f = Friend::new(1);
        merge(&mut f, &update(1, "Ann", 1));
        merge(&mut f, &PersonaUpdate { id: 1, state: Some(PersonaState::Away), ..Default::default() });
        assert_eq!((f.name.as_str(), f.state), ("Ann", PersonaState::Away));
    }

    #[test]
    fn demo_friends_cover_every_group() {
        let d = demo_friends(1_700_000_000);
        assert_eq!(d.len(), 40);
        assert!(d.iter().any(|f| f.game.is_some()));
        assert!(d.iter().any(|f| f.state == PersonaState::Offline));
        assert!(d.iter().any(|f| f.state.is_online() && f.game.is_none()));
    }

    fn friend(id: u64, name: &str, state: PersonaState, game: bool, logoff: u64) -> Friend {
        Friend {
            name: name.into(),
            state,
            game: game.then(|| GamePlayed { app_id: 1, name: "G".into() }),
            last_logoff: logoff,
            ..Friend::new(id)
        }
    }

    #[test]
    fn sorting_is_in_game_then_online_then_offline_by_last_online() {
        let mut v = [
            friend(1, "zed", PersonaState::Offline, false, 100),
            friend(2, "bob", PersonaState::Away, false, 0),
            friend(3, "amy", PersonaState::Offline, false, 900),
            friend(4, "cat", PersonaState::Online, true, 0),
            friend(5, "ann", PersonaState::Online, false, 0),
            friend(6, "dan", PersonaState::Snooze, true, 0),
        ];
        v.sort_by_cached_key(sort_key);
        let names: Vec<_> = v.iter().map(|f| f.name.as_str()).collect();
        // In-game (cat, dan), online (ann, bob), offline: most recently online first (amy before zed).
        assert_eq!(names, ["cat", "dan", "ann", "bob", "amy", "zed"]);
    }

    #[test]
    fn a_missing_name_is_never_a_steam_id() {
        let f = Friend::new(76_561_198_314_275_551);
        assert!(!f.has_name());
        assert_eq!(f.display_name(), LOADING);
        assert!(!f.display_name().chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn only_friends_are_listed() {
        let rels: HashMap<u64, Relationship> = [
            (1, Relationship::Friend),
            (2, Relationship::RequestRecipient),
            (3, Relationship::RequestInitiator),
            (4, Relationship::Blocked),
            (5, Relationship::Ignored),
            (6, Relationship::Suggested),
            (7, Relationship::Friend),
        ]
        .into();
        let people: HashMap<u64, Friend> = [(2, friend(2, "Rowan", PersonaState::Online, false, 0))].into();
        let (friends, requests, pending, blocked) = split_relationships(&rels, &people);
        let mut ids: Vec<u64> = friends.iter().map(|f| f.id.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 7]);
        assert_eq!(requests.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Rowan"]);
        assert_eq!(pending.iter().map(|f| f.id.0).collect::<Vec<_>>(), [3]);
        assert_eq!(blocked, 2);
    }

    #[test]
    fn group_member_names_fall_back_to_member() {
        let me = Me { id: sr_steam::friends::id_from_friend_code(7), name: "Me".into(), avatar_url: None };
        let mut s = FriendsState { me, ..Default::default() };
        s.friends.push(friend(sr_steam::friends::id_from_friend_code(9), "Ann", PersonaState::Online, false, 0));
        s.reindex();
        assert_eq!(s.member_name(7), "Me");
        assert_eq!(s.member_name(9), "Ann");
        assert_eq!(s.member_name(12345), "Member");
        assert_eq!(s.persona_name(), Some("Me"));
    }
}
