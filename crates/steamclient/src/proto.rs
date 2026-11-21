//! The wire protocol between the game side (`steamclient.so`) and Kettle (the host).
//!
//! One JSON object per line over a Unix socket (`$XDG_RUNTIME_DIR/kettle-steamclient.sock`, mode 0600, created by
//! Kettle, which is the only listener). The game side sends `{"id":N,"req":{"op":...}}` and waits for
//! `{"id":N,"rep":{"kind":...}}`. Byte strings are lower-case hex. A line is at most [`MAX_LINE`] bytes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Read};
use std::path::PathBuf;

pub const SOCKET_NAME: &str = "kettle-steamclient.sock";
pub const MAX_LINE: usize = 1 << 20;
/// `EResult` values used on the wire.
pub const ERESULT_OK: i32 = 1;
pub const ERESULT_FAIL: i32 = 2;
pub const ERESULT_NO_CONNECTION: i32 = 3;
pub const ERESULT_ACCESS_DENIED: i32 = 15;

/// Where the socket lives: `$KETTLE_STEAMCLIENT_SOCK`, else `$XDG_RUNTIME_DIR/kettle-steamclient.sock`, else
/// `/tmp/kettle-steamclient-<uid>.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KETTLE_STEAMCLIENT_SOCK").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join(SOCKET_NAME);
    }
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
    PathBuf::from(format!("/tmp/kettle-steamclient-{uid}.sock"))
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.chunks(2).map(|p| Some(nib(p[0])? << 4 | nib(p[1])?)).collect()
}

/// A stat value. Steam stats are 32-bit integers or floats.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StatValue {
    #[serde(rename = "i")]
    Int(i32),
    #[serde(rename = "f")]
    Float(f32),
}

/// Who is signed in, as the game side needs to know.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Identity {
    pub steam_id: u64,
    pub persona: String,
    pub logged_on: bool,
    pub language: String,
}

/// What Kettle knows about an installed app.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppInfo {
    pub install_dir: Option<String>,
    pub build_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// First message of a connection: names the app this process is (the host only serves that app afterwards).
    Hello {
        app: u32,
    },
    Owns {
        app: u32,
    },
    AppTicket {
        app: u32,
    },
    AuthTicket {
        app: u32,
    },
    EncryptedTicket {
        app: u32,
        userdata: String,
    },
    GetStats {
        app: u32,
    },
    StoreStats {
        app: u32,
        stats: BTreeMap<String, StatValue>,
        achievements: BTreeMap<String, u32>,
    },
    AppInfo {
        app: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    Hello(Identity),
    Owns { owned: bool },
    Ticket { eresult: i32, ticket: String },
    Stats { eresult: i32, stats: BTreeMap<String, StatValue>, achievements: BTreeMap<String, u32> },
    Stored { eresult: i32 },
    AppInfo(AppInfo),
    Error { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestLine {
    pub id: u64,
    pub req: Request,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplyLine {
    pub id: u64,
    pub rep: Reply,
}

/// Read one line of at most [`MAX_LINE`] bytes. `Ok(None)` at end of stream; an over-long line is an error.
pub fn read_line_limited(r: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    String::from_utf8(buf).map(Some).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let b = [0u8, 1, 0xab, 0xff, 0x10];
        assert_eq!(hex_encode(&b), "0001abff10");
        assert_eq!(hex_decode("0001ABff10").unwrap(), b);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
    }

    #[test]
    fn requests_and_replies_round_trip() {
        let mut stats = BTreeMap::new();
        stats.insert("kills".to_string(), StatValue::Int(7));
        stats.insert("dist".to_string(), StatValue::Float(1.5));
        let line = RequestLine {
            id: 9,
            req: Request::StoreStats { app: 480, stats: stats.clone(), achievements: BTreeMap::new() },
        };
        let text = serde_json::to_string(&line).unwrap();
        assert!(text.contains(r#""op":"store_stats""#), "{text}");
        let back: RequestLine = serde_json::from_str(&text).unwrap();
        assert_eq!(back.req, line.req);
        for rep in [
            Reply::Hello(Identity {
                steam_id: 76561197960265729,
                persona: "A".into(),
                logged_on: true,
                language: "english".into(),
            }),
            Reply::Owns { owned: true },
            Reply::Stats { eresult: 1, stats, achievements: BTreeMap::new() },
            Reply::AppInfo(AppInfo { install_dir: Some("/g".into()), build_id: 5 }),
            Reply::Error { message: "x".into() },
        ] {
            let t = serde_json::to_string(&ReplyLine { id: 1, rep: rep.clone() }).unwrap();
            let b: ReplyLine = serde_json::from_str(&t).unwrap();
            assert_eq!(b.rep, rep, "{t}");
        }
    }

    #[test]
    fn long_lines_are_refused() {
        let mut big = vec![b'a'; MAX_LINE + 10];
        big.push(b'\n');
        assert!(read_line_limited(&mut io::BufReader::new(&big[..])).is_err());
        let mut ok = io::BufReader::new(&b"hi\r\nx"[..]);
        assert_eq!(read_line_limited(&mut ok).unwrap().as_deref(), Some("hi"));
        assert_eq!(read_line_limited(&mut ok).unwrap().as_deref(), Some("x"));
        assert_eq!(read_line_limited(&mut ok).unwrap(), None);
    }
}
