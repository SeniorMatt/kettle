//! What the main app and the in-game overlay helper (`kettle --overlay`) say to each other.
//!
//! The helper is a second process because it must be an X11 window (so it can sit over games that run under
//! XWayland without the desktop treating it as a window). It has no Steam connection and no state of its own:
//! the main app sends snapshots, the helper sends back what the user did. Plain data only; the app crate does the
//! socket and the JSON.
//!
//! Wire format: one JSON object per line over a Unix socket in `$XDG_RUNTIME_DIR` (`kettle-overlay-<pid>.sock`,
//! mode 0600, created by the main app, which is the only listener). A line is at most [`MAX_LINE`] bytes.
//! Objects carry a `"t"` field with the message kind, see [`ToHelper`] and [`FromHelper`].
//!
//! ```text
//! app -> helper  {"t":"show", ...snapshot}    show the overlay (also sent first, to fill it)
//!                {"t":"update", ...snapshot}  new data while shown
//!                {"t":"hide"}                 hide it (the Shift+Tab press arrived at the app)
//!                {"t":"quit"}                 exit
//! helper -> app  {"t":"ready"}                connected and the window exists
//!                {"t":"hidden"}               the user closed it (Esc, Return, Shift+Tab)
//!                {"t":"open_chat","id":N}     the user opened a chat (the app loads the history)
//!                {"t":"send_chat","id":N,"text":"..."}
//!                {"t":"set_note","app":N,"text":"..."}
//!                {"t":"set_overlay","on":false}
//!                {"t":"set_hud", ...HudSettings}
//! ```

use crate::{AnimationMode, Delivery, FriendsStatus, HudSettings, PersonaState, State};
use serde::{Deserialize, Serialize};
use sr_model::AppId;

/// Longest line either side accepts (bytes).
pub const MAX_LINE: usize = 1 << 20;
/// Friends in a snapshot, and messages per chat.
pub const MAX_FRIENDS_SENT: usize = 300;
pub const MAX_OFFLINE_SENT: usize = 40;
pub const MAX_MESSAGES_SENT: usize = 60;
pub const MAX_CHATS_SENT: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendRow {
    pub id: u64,
    pub name: String,
    /// `EPersonaState` as Steam numbers it (0 = offline, 1 = online, ...).
    pub state: u32,
    /// The game the friend is playing.
    pub game: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsgRow {
    pub mine: bool,
    pub text: String,
    pub time: u64,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRow {
    pub id: u64,
    pub unread: u32,
    pub messages: Vec<MsgRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub app: u32,
    pub game: String,
    /// Seconds the game has been running when the snapshot was made.
    pub session_secs: u64,
    pub friends_connected: bool,
    pub me: String,
    pub friends: Vec<FriendRow>,
    /// Chats that are open in the app (at most [`MAX_CHATS_SENT`]).
    pub chats: Vec<ChatRow>,
    pub note: String,
    pub overlay_on: bool,
    pub hud: HudSettings,
    pub hud_installed: bool,
    /// False when the user turned animations off.
    pub animations: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToHelper {
    Show(Snapshot),
    Update(Snapshot),
    Hide,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FromHelper {
    Ready,
    Hidden,
    OpenChat { id: u64 },
    SendChat { id: u64, text: String },
    SetNote { app: u32, text: String },
    SetOverlay { on: bool },
    SetHud(HudSettings),
}

/// A snapshot of `state` for the game `app`; `None` when that game is not running.
/// `hud_installed` comes from the caller (it looks at the disk).
pub fn snapshot(state: &State, app: AppId, hud_installed: bool) -> Option<Snapshot> {
    let running = state.running.get(&app)?;
    let f = &state.friends;
    let mut rows: Vec<FriendRow> = Vec::new();
    let mut offline = 0;
    for fr in &f.friends {
        if fr.state == PersonaState::Offline {
            if offline >= MAX_OFFLINE_SENT {
                continue;
            }
            offline += 1;
        }
        if rows.len() >= MAX_FRIENDS_SENT {
            break;
        }
        rows.push(FriendRow {
            id: fr.id.0,
            name: fr.display_name(),
            state: fr.state.to_wire(),
            game: fr.game.as_ref().map(|g| g.name.clone()),
        });
    }
    let chats = f
        .tabs
        .iter()
        .rev()
        .take(MAX_CHATS_SENT)
        .filter_map(|id| f.chats.get(&id.0).map(|c| (id.0, c)))
        .map(|(id, c)| ChatRow {
            id,
            unread: c.unread,
            messages: c
                .messages
                .iter()
                .skip(c.messages.len().saturating_sub(MAX_MESSAGES_SENT))
                .map(|m| MsgRow {
                    mine: m.from_me,
                    text: m.text.clone(),
                    time: m.time,
                    failed: matches!(m.delivery, Delivery::Failed(_)),
                })
                .collect(),
        })
        .collect();
    Some(Snapshot {
        app: app.0,
        game: state.game(app).map_or_else(|| format!("App {}", app.0), |g| g.name.clone()),
        session_secs: running.started.elapsed().as_secs(),
        friends_connected: f.status == FriendsStatus::Connected,
        me: f.me.name.clone(),
        friends: rows,
        chats,
        note: state.note(app).to_string(),
        overlay_on: state.settings.overlay,
        hud: state.settings.hud.clone(),
        hud_installed,
        animations: state.settings.interface.animations != AnimationMode::Off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            app: 730,
            game: "Game".into(),
            session_secs: 61,
            friends_connected: true,
            me: "me".into(),
            friends: vec![FriendRow { id: 1, name: "a".into(), state: 1, game: Some("G".into()) }],
            chats: vec![ChatRow {
                id: 1,
                unread: 0,
                messages: vec![MsgRow { mine: true, text: "hi \"there\"\n".into(), time: 5, failed: false }],
            }],
            note: "n".into(),
            overlay_on: true,
            hud: HudSettings::default(),
            hud_installed: false,
            animations: true,
        }
    }

    #[test]
    fn messages_round_trip_and_have_a_kind_tag() {
        let m = ToHelper::Show(sample());
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.starts_with(r#"{"t":"show""#), "{json}");
        assert!(!json.contains('\n'), "one line");
        assert_eq!(serde_json::from_str::<ToHelper>(&json).unwrap(), m);
        for m in [
            FromHelper::Ready,
            FromHelper::Hidden,
            FromHelper::OpenChat { id: 9 },
            FromHelper::SendChat { id: 9, text: "x\ny".into() },
            FromHelper::SetNote { app: 1, text: "t".into() },
            FromHelper::SetOverlay { on: false },
            FromHelper::SetHud(HudSettings::default()),
        ] {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<FromHelper>(&json).unwrap(), m, "{json}");
        }
        assert_eq!(serde_json::to_string(&ToHelper::Hide).unwrap(), r#"{"t":"hide"}"#);
    }

    #[test]
    fn unknown_kinds_are_errors_not_panics() {
        assert!(serde_json::from_str::<FromHelper>(r#"{"t":"format_disk"}"#).is_err());
        assert!(serde_json::from_str::<FromHelper>("not json").is_err());
    }
}
