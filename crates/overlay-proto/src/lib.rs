//! The overlay socket protocol between Kettle and `libkettle_overlay.so`, the library that draws the overlay
//! inside the game's own frames (see `crates/overlay-hook`).
//!
//! Wire format: one JSON object per line over a Unix socket (`$XDG_RUNTIME_DIR/kettle-overlay-<pid>.sock`, mode
//! 0600, created by Kettle; the game's processes find it through `KETTLE_OVERLAY_SOCKET`). Objects carry a `"t"`
//! field with the message kind. A line is at most [`MAX_LINE`] bytes. Several game processes (a launcher, Wine
//! services, the game) may connect at once; Kettle sends to all of them and listens to all of them.
//!
//! ```text
//! kettle -> game  {"t":"data", ...OverlayData}   new data (sent when connected, then when it changes)
//!                 {"t":"toggle"}                 show or hide (menu "Show overlay")
//! game -> kettle  {"t":"hello","pid":N,"exe":"..."}
//!                 {"t":"visible","open":true}    the user opened or closed it
//!                 {"t":"action","a":{...}}       something the user did
//! ```

use serde::{Deserialize, Serialize};

/// Longest line either side accepts (bytes).
pub const MAX_LINE: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FriendRow {
    pub id: u64,
    pub name: String,
    /// "Online", "Away", or the game they are playing.
    pub status: String,
    pub online: bool,
    pub in_game: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AchievementRow {
    pub name: String,
    pub description: String,
    /// Unlock time, unix seconds.
    pub time: u64,
}

/// Everything the overlay shows, as a snapshot. No state of its own is kept on Kettle's side.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayData {
    pub app: u32,
    pub game: String,
    /// Seconds the game has been running when the snapshot was made.
    pub session_secs: u64,
    /// The signed-in account's name.
    pub me: String,
    pub friends_connected: bool,
    pub friends: Vec<FriendRow>,
    pub total_minutes: u32,
    /// The account's Steam id (its picture arrives as an `avatar` message with this id).
    pub me_id: u64,
    /// The folder screenshots of this game go to (the library creates it).
    pub screenshot_dir: String,
    pub achievements_unlocked: u32,
    pub achievements_total: u32,
    /// Most recent first.
    pub achievements_recent: Vec<AchievementRow>,
    /// Draw the performance HUD while the overlay is closed.
    pub hud: bool,
    /// Kettle's setting: the overlay is on. Off = the library never opens it.
    pub enabled: bool,
    /// False when the user turned animations off.
    pub animations: bool,
}

/// What the user did in the overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum OverlayAction {
    OpenUrl {
        url: String,
    },
    OpenChat {
        id: u64,
    },
    SendChat {
        id: u64,
        text: String,
    },
    Screenshot,
    /// The performance monitor was switched on or off in the overlay.
    PerfHud {
        on: bool,
    },
    /// The user asked to close the overlay (the library closes it).
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToHook {
    Data(OverlayData),
    Toggle,
    /// A square picture (a friend's, or the account's), `size` x `size` RGBA pixels as base64.
    Avatar {
        id: u64,
        size: u32,
        rgba: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FromHook {
    Hello {
        pid: u32,
        exe: String,
    },
    Visible {
        open: bool,
    },
    Action {
        a: OverlayAction,
    },
    /// A screenshot of the game was written to `path`.
    Screenshot {
        path: String,
    },
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The bytes of standard base64; `None` for anything else.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let t = text.trim_end_matches('=').as_bytes();
    if t.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(t.len() * 3 / 4);
    for c in t.chunks(4) {
        let mut n = 0u32;
        for (i, b) in c.iter().enumerate() {
            let v = B64.iter().position(|x| x == b)? as u32;
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// One message as one line (with the newline).
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let mut line = serde_json::to_vec(msg).unwrap_or_default();
    line.push(b'\n');
    line
}

/// A message from a line; anything that does not parse is dropped by the caller.
pub fn decode<T: for<'a> Deserialize<'a>>(line: &[u8]) -> Option<T> {
    serde_json::from_slice(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_and_rejects_garbage() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&data)).as_deref(), Some(&data[..]), "{n} bytes");
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        assert_eq!(b64_decode("TW@u"), None);
        assert_eq!(b64_decode("T"), None);
    }

    #[test]
    fn avatar_and_screenshot_messages_round_trip() {
        let a = ToHook::Avatar { id: 7, size: 2, rgba: b64_encode(&[1; 16]) };
        assert_eq!(decode::<ToHook>(&encode(&a)), Some(a));
        let s = FromHook::Screenshot { path: "/p/a.png".into() };
        assert_eq!(decode::<FromHook>(&encode(&s)), Some(s));
    }

    #[test]
    fn round_trip() {
        let d = OverlayData {
            game: "G".into(),
            friends: vec![FriendRow { id: 1, ..Default::default() }],
            ..Default::default()
        };
        for m in [ToHook::Data(d), ToHook::Toggle] {
            let line = encode(&m);
            assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
            assert_eq!(decode::<ToHook>(&line), Some(m));
        }
        let a = FromHook::Action { a: OverlayAction::SendChat { id: 2, text: "a\nb".into() } };
        assert_eq!(decode::<FromHook>(&encode(&a)), Some(a));
        assert_eq!(decode::<FromHook>(b"{\"t\":\"format_disk\"}"), None);
        assert_eq!(decode::<ToHook>(b"{oops"), None);
    }
}
