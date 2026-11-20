//! Friends list, presence and 1:1 chat over a logged-in CM connection. No GUI types here.
//!
//! [`FriendsSession::start`] spawns one task on the current tokio runtime. It listens for the friends list,
//! persona states and incoming chat messages, and turns them into [`FriendsEvent`]s. Requests (send a
//! message, change our state, load history) go through the cloneable [`FriendsHandle`]. When the connection
//! drops the task ends after sending [`FriendsEvent::Disconnected`]; reconnecting (with a fresh access token
//! from the refresh token) is the caller's job.

use futures_util::StreamExt;
use std::collections::HashSet;
use std::time::Duration;
use steam_vent::{Connection, ConnectionTrait, NetMessage, RawNetMessage};
use steam_vent_proto_steam::steammessages_clientserver_friends::{
    CMsgClientChangeStatus, CMsgClientFriendsList, CMsgClientPersonaState, CMsgClientRequestFriendData,
    cmsg_client_persona_state::Friend as PersonaFriend,
};
use steam_vent_proto_steam::steammessages_friendmessages_steamclient::{
    CFriendMessages_GetRecentMessages_Request, CFriendMessages_IncomingMessage_Notification,
    CFriendMessages_SendMessage_Request,
};
use tokio::sync::mpsc;

/// Events buffered between the session task and its consumer.
const EVENT_QUEUE: usize = 512;
/// Requests buffered between the handle and the session task.
const COMMAND_QUEUE: usize = 64;
/// Persona data is requested for this many friends per message.
const REQUEST_CHUNK: usize = 400;
/// How many recent messages a chat loads when it opens.
pub const HISTORY_COUNT: u32 = 50;
/// `CMsgClientRequestFriendData` flags: name, presence, source id, last seen, game extra info.
const PERSONA_FLAGS: u32 = 1 | 2 | 4 | 64 | 256;
/// `EFriendRelationship::Friend`.
const RELATIONSHIP_FRIEND: u32 = 3;
/// `EChatEntryType::ChatMsg` (typing notifications and others are ignored).
const CHAT_MSG: i32 = 1;
const AVATAR_HOST: &str = "https://avatars.steamstatic.com";
/// The picture Steam shows for accounts without their own avatar.
const DEFAULT_AVATAR: &str = "fef49e7fa7e1997310d705b2a6158ff8dc1cdfeb";

// ---- plain data -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PersonaState {
    #[default]
    Offline,
    Online,
    Busy,
    Away,
    Snooze,
    LookingToTrade,
    LookingToPlay,
    /// Online, but shown as offline to others (only ever our own state).
    Invisible,
}

impl PersonaState {
    /// `EPersonaState`; unknown values count as offline.
    pub fn from_wire(v: u32) -> Self {
        match v {
            1 => Self::Online,
            2 => Self::Busy,
            3 => Self::Away,
            4 => Self::Snooze,
            5 => Self::LookingToTrade,
            6 => Self::LookingToPlay,
            7 => Self::Invisible,
            _ => Self::Offline,
        }
    }

    pub fn to_wire(self) -> u32 {
        match self {
            Self::Offline => 0,
            Self::Online => 1,
            Self::Busy => 2,
            Self::Away => 3,
            Self::Snooze => 4,
            Self::LookingToTrade => 5,
            Self::LookingToPlay => 6,
            Self::Invisible => 7,
        }
    }

    /// Shown in the "Online" group (everything but offline; invisible friends look offline to us anyway).
    pub fn is_online(self) -> bool {
        !matches!(self, Self::Offline | Self::Invisible)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Offline => "Offline",
            Self::Online => "Online",
            Self::Busy => "Busy",
            Self::Away => "Away",
            Self::Snooze => "Snooze",
            Self::LookingToTrade => "Looking to trade",
            Self::LookingToPlay => "Looking to play",
            Self::Invisible => "Invisible",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePlayed {
    pub app_id: u32,
    pub name: String,
}

/// What a persona-state message says about one account. `None` = not mentioned, keep what we have.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PersonaUpdate {
    pub id: u64,
    pub name: Option<String>,
    pub state: Option<PersonaState>,
    pub avatar_url: Option<String>,
    /// `Some(None)` = stopped playing.
    pub game: Option<Option<GamePlayed>>,
    pub last_logoff: Option<u64>,
}

/// One chat line, ours or theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub from_me: bool,
    pub text: String,
    /// Unix seconds (server time).
    pub time: u64,
    /// Orders messages within the same second.
    pub ordinal: u32,
}

#[derive(Debug)]
pub enum FriendsEvent {
    /// Logged on; `me` is our SteamID64.
    Connected {
        me: u64,
    },
    /// The connection ended. Nothing more will arrive on this channel.
    Disconnected(String),
    /// The friends list (`incremental` = only changes; removed friends have `friend == false`).
    List {
        incremental: bool,
        entries: Vec<(u64, bool)>,
    },
    Personas(Vec<PersonaUpdate>),
    Message {
        friend: u64,
        line: ChatLine,
    },
    History {
        friend: u64,
        lines: Vec<ChatLine>,
    },
    HistoryFailed {
        friend: u64,
        error: String,
    },
    /// Result of `send_message`; `token` is the caller's id for the message.
    Sent {
        friend: u64,
        token: u64,
        result: Result<ChatLine, String>,
    },
}

// ---- pure helpers -----------------------------------------------------------------------------------

/// URL of an avatar picture (`_medium` is 64x64). An empty or all-zero hash means "no avatar set": Steam
/// shows its default picture then.
pub fn avatar_url(hash: &[u8]) -> String {
    let name = if hash.len() == 20 && hash.iter().any(|b| *b != 0) {
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    } else {
        DEFAULT_AVATAR.to_string()
    };
    format!("{AVATAR_HOST}/{name}_medium.jpg")
}

/// Text for the wire: the official client sends BBCode, so `\` and `[` are escaped with a backslash.
pub fn escape_outgoing(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        if c == '\\' || c == '[' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Undo [`escape_outgoing`] on a BBCode message and drop simple formatting tags (`[b]`, `[/i]`,
/// `[url=...]`, ...). A `[` that was escaped stays literal.
pub fn unescape_incoming(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if matches!(chars.get(i + 1), Some('\\' | '[')) => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '[' => match tag_end(&chars, i) {
                Some(end) => i = end + 1,
                None => {
                    out.push('[');
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Index of the `]` closing a short BBCode tag that starts at `start`, if it looks like one.
fn tag_end(chars: &[char], start: usize) -> Option<usize> {
    let close = chars[start + 1..].iter().take(64).position(|c| *c == ']')? + start + 1;
    let inner: String = chars[start + 1..close].iter().collect();
    let name = inner.trim_start_matches('/').split(['=', ' ']).next().unwrap_or_default();
    let known = ["b", "i", "u", "s", "url", "code", "quote", "spoiler", "h1", "h2", "h3", "noparse", "hr"];
    (!name.is_empty() && known.contains(&name.to_ascii_lowercase().as_str())).then_some(close)
}

/// The text to show for a message: the server's plain version when it sent one, else our unescaped copy.
pub fn display_text(message: &str, plain: &str) -> String {
    if plain.is_empty() { unescape_incoming(message) } else { plain.to_string() }
}

/// Oldest first, by (time, ordinal); exact duplicates (same time, ordinal and text) are dropped.
pub fn order_history(mut lines: Vec<ChatLine>) -> Vec<ChatLine> {
    lines.sort_by_key(|a| (a.time, a.ordinal));
    lines.dedup_by(|a, b| a.time == b.time && a.ordinal == b.ordinal && a.text == b.text);
    lines
}

/// Individual accounts only (not clans, chat groups or game servers).
pub fn is_individual(id: u64) -> bool {
    (id >> 52) & 0xF == 1
}

fn account_id(id: u64) -> u32 {
    (id & 0xFFFF_FFFF) as u32
}

fn persona_update(f: &PersonaFriend) -> Option<PersonaUpdate> {
    let id = f.friendid.filter(|id| is_individual(*id))?;
    let game = f.game_played_app_id.map(|app| {
        let gid = f.gameid.unwrap_or(0);
        let app_id = if app != 0 { app } else { (gid & 0xFFFF_FFFF) as u32 };
        (app_id != 0).then(|| GamePlayed {
            app_id,
            name: f.game_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("App {app_id}")),
        })
    });
    Some(PersonaUpdate {
        id,
        name: f.player_name.clone().filter(|n| !n.is_empty()),
        state: f.persona_state.map(PersonaState::from_wire),
        avatar_url: f.avatar_hash.as_ref().map(|h| avatar_url(h)),
        game,
        last_logoff: f.last_logoff.filter(|t| *t != 0).map(u64::from),
    })
}

// ---- the session -----------------------------------------------------------------------------------

enum Cmd {
    Send { friend: u64, token: u64, text: String },
    SetState(PersonaState),
    History(u64),
    RequestData(Vec<u64>),
}

/// Requests to a running session. Cheap to clone; every method returns at once.
#[derive(Clone)]
pub struct FriendsHandle {
    tx: mpsc::Sender<Cmd>,
}

impl FriendsHandle {
    /// Send a chat message. The answer comes as [`FriendsEvent::Sent`] with the same `token`.
    pub fn send_message(&self, friend: u64, token: u64, text: &str) -> bool {
        self.tx.try_send(Cmd::Send { friend, token, text: text.to_string() }).is_ok()
    }

    pub fn set_state(&self, state: PersonaState) -> bool {
        self.tx.try_send(Cmd::SetState(state)).is_ok()
    }

    /// Load the recent messages with `friend` ([`FriendsEvent::History`]).
    pub fn history(&self, friend: u64) -> bool {
        self.tx.try_send(Cmd::History(friend)).is_ok()
    }

    /// Ask Steam for names and presence of these accounts.
    pub fn request_data(&self, ids: Vec<u64>) -> bool {
        self.tx.try_send(Cmd::RequestData(ids)).is_ok()
    }
}

pub struct FriendsSession;

impl FriendsSession {
    /// Start on the current tokio runtime. `early` are messages that arrived before anyone listened (from
    /// `cm::Cm::early`). Our persona is set to `initial` as soon as the session runs.
    pub fn start(
        conn: Connection,
        early: Vec<RawNetMessage>,
        initial: PersonaState,
    ) -> (FriendsHandle, mpsc::Receiver<FriendsEvent>) {
        let (ev_tx, ev_rx) = mpsc::channel(EVENT_QUEUE);
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_QUEUE);
        tokio::spawn(async move {
            let why = run(conn, early, initial, cmd_rx, &ev_tx).await;
            let _ = ev_tx.send(FriendsEvent::Disconnected(why)).await;
        });
        (FriendsHandle { tx: cmd_tx }, ev_rx)
    }
}

/// Runs until the connection or the consumer goes away; returns why.
async fn run(
    conn: Connection,
    early: Vec<RawNetMessage>,
    initial: PersonaState,
    mut cmds: mpsc::Receiver<Cmd>,
    out: &mpsc::Sender<FriendsEvent>,
) -> String {
    let me = u64::from(conn.steam_id());
    // Subscribe before replaying what was buffered, so nothing falls in between.
    let mut lists = Box::pin(conn.on::<CMsgClientFriendsList>());
    let mut personas = Box::pin(conn.on::<CMsgClientPersonaState>());
    let mut incoming = Box::pin(conn.on_notification::<CFriendMessages_IncomingMessage_Notification>());
    if out.send(FriendsEvent::Connected { me }).await.is_err() {
        return "closed".into();
    }
    if let Err(e) = set_state(&conn, initial).await {
        return format!("{e}");
    }
    let mut known: HashSet<u64> = HashSet::new();
    for raw in early.into_iter().chain(conn.take_unprocessed()) {
        if raw.kind == CMsgClientFriendsList::KIND {
            if let Ok(l) = raw.into_message::<CMsgClientFriendsList>()
                && on_list(&conn, &l, &mut known, out).await.is_err()
            {
                return "closed".into();
            }
        } else if raw.kind == CMsgClientPersonaState::KIND
            && let Ok(p) = raw.into_message::<CMsgClientPersonaState>()
            && out.send(FriendsEvent::Personas(p.friends.iter().filter_map(persona_update).collect())).await.is_err()
        {
            return "closed".into();
        }
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return "closed".into() };
                run_cmd(&conn, me, cmd, out).await;
            }
            Some(l) = lists.next() => {
                if let Ok(l) = l && on_list(&conn, &l, &mut known, out).await.is_err() {
                    return "closed".into();
                }
            }
            Some(p) = personas.next() => {
                if let Ok(p) = p {
                    let ups: Vec<_> = p.friends.iter().filter_map(persona_update).collect();
                    if !ups.is_empty() && out.send(FriendsEvent::Personas(ups)).await.is_err() {
                        return "closed".into();
                    }
                }
            }
            Some(m) = incoming.next() => {
                if let Ok(m) = m && let Some(ev) = incoming_event(&m)
                    && out.send(ev).await.is_err()
                {
                    return "closed".into();
                }
            }
            else => return "Connection to Steam closed".into(),
        }
    }
}

async fn set_state(conn: &Connection, state: PersonaState) -> anyhow::Result<()> {
    let msg = CMsgClientChangeStatus {
        persona_state: Some(state.to_wire()),
        persona_set_by_user: Some(true),
        ..Default::default()
    };
    conn.send(msg).await?;
    Ok(())
}

/// Forward the friends list and ask for persona data of every friend.
async fn on_list(
    conn: &Connection,
    l: &CMsgClientFriendsList,
    known: &mut HashSet<u64>,
    out: &mpsc::Sender<FriendsEvent>,
) -> Result<(), ()> {
    let entries: Vec<(u64, bool)> = l
        .friends
        .iter()
        .filter_map(|f| Some((f.ulfriendid?, f.efriendrelationship? == RELATIONSHIP_FRIEND)))
        .filter(|(id, _)| is_individual(*id))
        .collect();
    let new: Vec<u64> = entries.iter().filter(|(id, friend)| *friend && known.insert(*id)).map(|(id, _)| *id).collect();
    out.send(FriendsEvent::List { incremental: l.bincremental(), entries }).await.map_err(|_| ())?;
    request_data(conn, &new).await;
    Ok(())
}

async fn request_data(conn: &Connection, ids: &[u64]) {
    for chunk in ids.chunks(REQUEST_CHUNK) {
        let msg = CMsgClientRequestFriendData {
            persona_state_requested: Some(PERSONA_FLAGS),
            friends: chunk.to_vec(),
            ..Default::default()
        };
        let _ = conn.send(msg).await;
    }
}

fn incoming_event(m: &CFriendMessages_IncomingMessage_Notification) -> Option<FriendsEvent> {
    if m.chat_entry_type() != CHAT_MSG {
        return None; // typing notifications etc.
    }
    let text = display_text(m.message(), m.message_no_bbcode());
    if text.is_empty() {
        return None;
    }
    let line = ChatLine {
        // A copy of what we sent from another device.
        from_me: m.local_echo(),
        text,
        time: u64::from(m.rtime32_server_timestamp()),
        ordinal: m.ordinal(),
    };
    Some(FriendsEvent::Message { friend: m.steamid_friend(), line })
}

/// Requests that wait for an answer run on their own task, so the loop keeps reading.
async fn run_cmd(conn: &Connection, me: u64, cmd: Cmd, out: &mpsc::Sender<FriendsEvent>) {
    match cmd {
        Cmd::SetState(s) => {
            let _ = set_state(conn, s).await;
        }
        Cmd::RequestData(ids) => request_data(conn, &ids).await,
        Cmd::Send { friend, token, text } => {
            let (conn, out) = (conn.clone(), out.clone());
            tokio::spawn(async move {
                let req = CFriendMessages_SendMessage_Request {
                    steamid: Some(friend),
                    chat_entry_type: Some(CHAT_MSG),
                    message: Some(escape_outgoing(&text)),
                    contains_bbcode: Some(true),
                    ..Default::default()
                };
                let result = match tokio::time::timeout(Duration::from_secs(20), conn.service_method(req)).await {
                    Ok(Ok(r)) => Ok(ChatLine {
                        from_me: true,
                        text,
                        time: u64::from(r.server_timestamp()),
                        ordinal: r.ordinal(),
                    }),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("Steam did not answer in time".to_string()),
                };
                let _ = out.send(FriendsEvent::Sent { friend, token, result }).await;
            });
        }
        Cmd::History(friend) => {
            let (conn, out) = (conn.clone(), out.clone());
            tokio::spawn(async move {
                let req = CFriendMessages_GetRecentMessages_Request {
                    steamid1: Some(me),
                    steamid2: Some(friend),
                    count: Some(HISTORY_COUNT),
                    most_recent_conversation: Some(false),
                    bbcode_format: Some(true),
                    ..Default::default()
                };
                let ev = match tokio::time::timeout(Duration::from_secs(20), conn.service_method(req)).await {
                    Ok(Ok(r)) => {
                        let mine = account_id(me);
                        let lines = r
                            .messages
                            .iter()
                            .map(|m| ChatLine {
                                from_me: m.accountid() == mine,
                                text: unescape_incoming(m.message()),
                                time: u64::from(m.timestamp()),
                                ordinal: m.ordinal(),
                            })
                            .collect();
                        FriendsEvent::History { friend, lines: order_history(lines) }
                    }
                    Ok(Err(e)) => FriendsEvent::HistoryFailed { friend, error: e.to_string() },
                    Err(_) => FriendsEvent::HistoryFailed { friend, error: "Steam did not answer in time".into() },
                };
                let _ = out.send(ev).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_state_mapping() {
        for (v, s) in [
            (0, PersonaState::Offline),
            (1, PersonaState::Online),
            (2, PersonaState::Busy),
            (3, PersonaState::Away),
            (4, PersonaState::Snooze),
            (5, PersonaState::LookingToTrade),
            (6, PersonaState::LookingToPlay),
            (7, PersonaState::Invisible),
        ] {
            assert_eq!(PersonaState::from_wire(v), s);
            assert_eq!(s.to_wire(), v);
        }
        assert_eq!(PersonaState::from_wire(99), PersonaState::Offline);
        assert!(PersonaState::Away.is_online());
        assert!(!PersonaState::Offline.is_online());
        assert!(!PersonaState::Invisible.is_online());
    }

    #[test]
    fn avatar_urls() {
        let mut h = [0u8; 20];
        h[0] = 0xab;
        h[19] = 0x01;
        assert_eq!(avatar_url(&h), format!("https://avatars.steamstatic.com/ab{}01_medium.jpg", "00".repeat(18)));
        let default = format!("{AVATAR_HOST}/{DEFAULT_AVATAR}_medium.jpg");
        assert_eq!(avatar_url(&[0u8; 20]), default);
        assert_eq!(avatar_url(&[]), default);
        assert_eq!(avatar_url(&[1, 2, 3]), default);
    }

    #[test]
    fn escaping_round_trips() {
        assert_eq!(escape_outgoing("hi [b]x[/b] \\o/"), "hi \\[b]x\\[/b] \\\\o/");
        for s in ["plain", "[b]not bold[/b]", "a \\ b", "[[", "\\[b]", "ends with \\", "[url=x]y[/url] [not a tag]"] {
            assert_eq!(unescape_incoming(&escape_outgoing(s)), s, "{s}");
        }
    }

    #[test]
    fn unescape_strips_formatting_tags() {
        assert_eq!(unescape_incoming("[b]bold[/b] and [url=https://x.y]link[/url]"), "bold and link");
        assert_eq!(unescape_incoming("array[0] is [fine"), "array[0] is [fine");
        assert_eq!(display_text("a \\[b]", ""), "a [b]");
        assert_eq!(display_text("ignored", "plain text"), "plain text");
    }

    #[test]
    fn history_is_oldest_first_and_deduplicated() {
        let l = |time, ordinal, text: &str| ChatLine { from_me: false, text: text.into(), time, ordinal };
        let got = order_history(vec![l(20, 0, "c"), l(10, 1, "b"), l(10, 0, "a"), l(10, 1, "b"), l(20, 1, "d")]);
        let texts: Vec<_> = got.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c", "d"]);
    }

    #[test]
    fn individual_ids_only() {
        assert!(is_individual(76_561_198_000_000_000));
        assert!(!is_individual(103_582_791_429_521_408)); // a clan
    }

    #[test]
    fn persona_updates_keep_unmentioned_fields_unset() {
        let f = PersonaFriend {
            friendid: Some(76_561_198_000_000_001),
            persona_state: Some(1),
            player_name: Some("Ann".into()),
            game_played_app_id: Some(440),
            game_name: Some("Team Fortress 2".into()),
            avatar_hash: Some(vec![0; 20]),
            ..Default::default()
        };
        let u = persona_update(&f).unwrap();
        assert_eq!(u.name.as_deref(), Some("Ann"));
        assert_eq!(u.state, Some(PersonaState::Online));
        assert_eq!(u.game, Some(Some(GamePlayed { app_id: 440, name: "Team Fortress 2".into() })));
        assert!(u.avatar_url.unwrap().contains(DEFAULT_AVATAR));
        let only_state = PersonaFriend {
            friendid: Some(76_561_198_000_000_001),
            persona_state: Some(0),
            game_played_app_id: Some(0),
            ..Default::default()
        };
        let u = persona_update(&only_state).unwrap();
        assert_eq!((u.name, u.avatar_url, u.last_logoff, u.game), (None, None, None, Some(None)));
        assert!(persona_update(&PersonaFriend { friendid: Some(1), ..Default::default() }).is_none());
    }
}
