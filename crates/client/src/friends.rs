//! Friends list and chat: the state the UI reads, the reducer for session events, and the connection
//! supervisor that keeps one friends session alive (with backoff) for the signed-in account.
//!
//! Everything is bounded: at most [`MAX_FRIENDS`] friends, [`MAX_MESSAGES`] messages per chat, [`MAX_CHATS`]
//! chats in memory and [`MAX_TABS`] open chat tabs. Tokens never enter the state; the supervisor task holds
//! its own copy of the refresh token and wipes it when it ends.

use crate::{Client, Event};
use sr_model::SteamId;
use sr_steam::friends::{ChatLine, FriendsEvent, FriendsHandle, FriendsSession, PersonaUpdate};
pub use sr_steam::friends::{GamePlayed, PersonaState};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_FRIENDS: usize = 2000;
pub const MAX_MESSAGES: usize = 500;
pub const MAX_CHATS: usize = 64;
pub const MAX_TABS: usize = 8;
/// Longest message we send (characters).
pub const MAX_TEXT: usize = 2000;
/// First retry after a lost connection; doubles up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(5);
/// Reconnect attempts are never closer together than this once backed off.
const RETRY_MAX: Duration = Duration::from_secs(60);

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
        }
    }

    /// The name to show; the SteamID while Steam hasn't told us the name yet.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.id.0.to_string() } else { self.name.clone() }
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

#[derive(Debug, Clone, Default)]
pub struct FriendsState {
    /// The panel is shown.
    pub open: bool,
    pub status: FriendsStatus,
    pub me: Me,
    /// The state we asked for (shown in the picker).
    pub my_state: PersonaState,
    /// Sorted by name (case-insensitive).
    pub friends: Vec<Friend>,
    /// Bumped whenever `friends` changes.
    pub rev: u64,
    pub chats: HashMap<u64, Chat>,
    /// Open chat tabs, oldest first.
    pub tabs: Vec<SteamId>,
    /// The chat shown in the panel (`None` = the friends list).
    pub active: Option<SteamId>,
    pub toast: Option<Toast>,
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
        self.chats.values().map(|c| c.unread).sum()
    }

    fn reindex(&mut self) {
        self.friends.sort_by_cached_key(|f| (f.display_name().to_lowercase(), f.id.0));
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
    /// Ids from the last full friends list (`None` until it arrives).
    ids: Option<HashSet<u64>>,
    /// Persona data that arrived before the list.
    early: HashMap<u64, PersonaUpdate>,
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
        self.with_handle(|h| h.set_state(state));
    }

    /// Run `f` on the live session's handle, if there is one.
    fn with_handle(&self, f: impl FnOnce(&FriendsHandle) -> bool) -> bool {
        let guard = self.friends.handle.lock().expect("friends handle");
        guard.as_ref().is_some_and(f)
    }

    // ---- connection -----------------------------------------------------------------------------

    /// Keep a friends session running for the current account until the generation changes.
    pub(crate) fn start_friends(&mut self) {
        let Some(s) = &self.session else { return };
        let generation = self.generation.load(Ordering::Relaxed);
        self.friends.ids = None;
        self.friends.early.clear();
        self.friends.wanted.store(self.state.friends.my_state.to_wire(), Ordering::Relaxed);
        self.state.friends.status = FriendsStatus::Connecting;
        let name = s.account.account_name.clone();
        let token = TokenCopy(s.refresh_token.clone());
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
            let mut delay = RETRY_MIN;
            while current.load(Ordering::Relaxed) == generation {
                match sr_steam::cm::connect(&name, &token.0).await {
                    Ok(cm) => {
                        let initial = PersonaState::from_wire(wanted.load(Ordering::Relaxed));
                        let (handle, mut rx) = FriendsSession::start(cm.conn, cm.early, initial);
                        *slot.lock().expect("friends handle") = Some(handle);
                        let mut watch = tokio::time::interval(Duration::from_secs(2));
                        loop {
                            tokio::select! {
                                ev = rx.recv() => match ev {
                                    Some(ev) => {
                                        if matches!(ev, FriendsEvent::Connected { .. }) {
                                            delay = RETRY_MIN;
                                        }
                                        send(ev).await;
                                    }
                                    None => break,
                                },
                                _ = watch.tick() => {
                                    if current.load(Ordering::Relaxed) != generation {
                                        break;
                                    }
                                }
                            }
                        }
                        *slot.lock().expect("friends handle") = None;
                    }
                    Err(e) => send(FriendsEvent::Disconnected(format!("{e:#}"))).await,
                }
                // Wait before the next attempt, but stop soon after a sign-out.
                let until = tokio::time::Instant::now() + delay;
                while tokio::time::Instant::now() < until && current.load(Ordering::Relaxed) == generation {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                delay = (delay * 2).min(RETRY_MAX);
            }
        });
    }

    /// Forget everything about friends (sign-out).
    pub(crate) fn reset_friends(&mut self) {
        *self.friends.handle.lock().expect("friends handle") = None;
        let my_state = self.state.friends.my_state;
        self.friends.ids = None;
        self.friends.early.clear();
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

    fn apply_list(&mut self, incremental: bool, entries: Vec<(u64, bool)>) {
        let f = &mut self.state.friends;
        let mut ids = if incremental { self.friends.ids.take().unwrap_or_default() } else { HashSet::new() };
        for (id, is_friend) in entries {
            if !is_friend {
                ids.remove(&id);
            } else if ids.len() < MAX_FRIENDS {
                ids.insert(id);
            }
        }
        f.friends.retain(|x| ids.contains(&x.id.0));
        let have: HashSet<u64> = f.friends.iter().map(|x| x.id.0).collect();
        for id in ids.iter().filter(|id| !have.contains(id)) {
            let mut fr = Friend::new(*id);
            if let Some(u) = self.friends.early.remove(id) {
                merge(&mut fr, &u);
            }
            f.friends.push(fr);
        }
        self.friends.early.clear();
        self.friends.ids = Some(ids);
        f.reindex();
    }

    fn apply_personas(&mut self, ups: Vec<PersonaUpdate>) {
        let f = &mut self.state.friends;
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
            match &self.friends.ids {
                // The list is known: only friends matter.
                Some(ids) if !ids.contains(&u.id) => {}
                Some(_) => {
                    if let Some(i) = f.index.get(&u.id).copied() {
                        merge(&mut f.friends[i], &u);
                    } else {
                        let mut fr = Friend::new(u.id);
                        merge(&mut fr, &u);
                        f.friends.push(fr);
                    }
                }
                // Before the list: keep the newest data per account, bounded.
                None => {
                    if self.friends.early.len() < MAX_FRIENDS {
                        let e = self
                            .friends
                            .early
                            .entry(u.id)
                            .or_insert_with(|| PersonaUpdate { id: u.id, ..Default::default() });
                        merge_update(e, u);
                    }
                }
            }
        }
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
            let name = f.friend(SteamId(friend)).map_or_else(|| friend.to_string(), Friend::display_name);
            f.toast = Some(Toast { seq: self.friends.toast_seq, friend: SteamId(friend), name, text });
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

    /// About 40 made-up friends and a bot that answers in chat (`STEAM_RUST_DEMO`).
    pub(crate) fn start_friends_demo(&mut self) {
        self.friends.demo = true;
        let now = now_secs();
        let f = &mut self.state.friends;
        f.status = FriendsStatus::Connected;
        f.me = Me { id: 76_561_198_000_000_000, name: "demo".into(), avatar_url: None };
        f.my_state = PersonaState::Online;
        f.friends = demo_friends(now);
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
}

/// Fold a newer update into an older one (fields the newer one doesn't mention stay).
fn merge_update(into: &mut PersonaUpdate, new: PersonaUpdate) {
    into.name = new.name.or(into.name.take());
    into.state = new.state.or(into.state);
    into.avatar_url = new.avatar_url.or(into.avatar_url.take());
    into.game = new.game.or(into.game.take());
    into.last_logoff = new.last_logoff.or(into.last_logoff);
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

/// The supervisor's own copy of the refresh token, wiped when the task ends.
struct TokenCopy(String);

impl Drop for TokenCopy {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        unsafe { self.0.as_bytes_mut() }.fill(0);
    }
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
        let mut early = PersonaUpdate { id: 1, ..Default::default() };
        merge_update(&mut early, update(1, "Ann", 1));
        merge_update(&mut early, PersonaUpdate { id: 1, state: Some(PersonaState::Busy), ..Default::default() });
        assert_eq!((early.name.as_deref(), early.state), (Some("Ann"), Some(PersonaState::Busy)));
    }

    #[test]
    fn demo_friends_cover_every_group() {
        let d = demo_friends(1_700_000_000);
        assert_eq!(d.len(), 40);
        assert!(d.iter().any(|f| f.game.is_some()));
        assert!(d.iter().any(|f| f.state == PersonaState::Offline));
        assert!(d.iter().any(|f| f.state.is_online() && f.game.is_none()));
    }
}
