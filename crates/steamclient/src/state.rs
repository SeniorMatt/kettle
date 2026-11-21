//! Process-wide state of the library: pipes, the callback queue, call results, the connection to Kettle and the
//! per-app caches. Everything is behind one mutex; the socket has its own, and the socket is never used while the
//! main one is held.

use crate::ipc::Client;
use crate::proto::{AppInfo, Identity, Reply, Request, StatValue, socket_path};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::{CString, c_char};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

// Callback ids (k_i...Callbacks base + n) of the structures this library posts.
#[allow(dead_code)] // Sent once real CM connection events are forwarded.
pub const CB_STEAM_SERVERS_CONNECTED: i32 = 101;
pub const CB_ENCRYPTED_APP_TICKET_RESPONSE: i32 = 154;
pub const CB_GET_AUTH_SESSION_TICKET_RESPONSE: i32 = 163;
pub const CB_API_CALL_COMPLETED: i32 = 703;
pub const CB_USER_STATS_RECEIVED: i32 = 1101;
pub const CB_USER_STATS_STORED: i32 = 1102;
pub const CB_USER_ACHIEVEMENT_STORED: i32 = 1103;

pub struct Callback {
    pub id: i32,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct StatsCache {
    pub loaded: bool,
    pub stats: BTreeMap<String, StatValue>,
    /// Achievement name -> unlock time (unix seconds).
    pub achievements: BTreeMap<String, u32>,
    /// Achievements unlocked since the last store.
    pub fresh: BTreeSet<String>,
}

pub struct Global {
    pub pipes: BTreeSet<i32>,
    next_pipe: i32,
    pub queue: VecDeque<Callback>,
    /// The buffer of the callback handed out last; kept alive until `Steam_FreeLastCallback`.
    pub last: Option<Vec<u8>>,
    pub results: HashMap<u64, (i32, Vec<u8>)>,
    next_call: u64,
    pub identity: Option<Identity>,
    pub owned: HashMap<u32, bool>,
    pub app_info: HashMap<u32, AppInfo>,
    pub stats: StatsCache,
    pub encrypted_ticket: Option<Vec<u8>>,
    pub auth_tickets: HashMap<u32, Vec<u8>>,
    next_ticket: u32,
    /// Interface objects handed out, by version string.
    pub interfaces: HashMap<String, usize>,
    pub strings: HashMap<String, usize>,
    pub warned: BTreeSet<String>,
    pub started: Instant,
}

static GLOBAL: OnceLock<Mutex<Global>> = OnceLock::new();
static IPC: Mutex<Option<Client>> = Mutex::new(None);

pub fn g() -> MutexGuard<'static, Global> {
    GLOBAL
        .get_or_init(|| {
            Mutex::new(Global {
                pipes: BTreeSet::new(),
                next_pipe: 1,
                queue: VecDeque::new(),
                last: None,
                results: HashMap::new(),
                next_call: 1,
                identity: None,
                owned: HashMap::new(),
                app_info: HashMap::new(),
                stats: StatsCache::default(),
                encrypted_ticket: None,
                auth_tickets: HashMap::new(),
                next_ticket: 1,
                interfaces: HashMap::new(),
                strings: HashMap::new(),
                warned: BTreeSet::new(),
                started: Instant::now(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

impl Global {
    pub fn new_pipe(&mut self) -> i32 {
        let p = self.next_pipe;
        self.next_pipe += 1;
        self.pipes.insert(p);
        p
    }

    pub fn push(&mut self, id: i32, data: Vec<u8>) {
        self.queue.push_back(Callback { id, data });
    }

    /// Store the result of an asynchronous call and announce it with `SteamAPICallCompleted_t`.
    pub fn complete_call(&mut self, callback_id: i32, data: Vec<u8>) -> u64 {
        let h = self.next_call;
        self.next_call += 1;
        let mut ev = Vec::with_capacity(16);
        ev.extend_from_slice(&h.to_le_bytes());
        ev.extend_from_slice(&callback_id.to_le_bytes());
        ev.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.results.insert(h, (callback_id, data));
        self.push(CB_API_CALL_COMPLETED, ev);
        h
    }

    pub fn new_ticket_handle(&mut self) -> u32 {
        let t = self.next_ticket;
        self.next_ticket += 1;
        t
    }

    /// A NUL-terminated string that lives forever (the C side keeps pointers to it).
    pub fn static_str(&mut self, s: &str) -> *const c_char {
        if let Some(p) = self.strings.get(s) {
            return *p as *const c_char;
        }
        let c = CString::new(s.replace('\0', "")).unwrap_or_default();
        let p = c.into_raw() as usize;
        self.strings.insert(s.to_string(), p);
        p as *const c_char
    }
}

pub fn log(msg: &str) {
    eprintln!("kettle-steamclient: {msg}");
}

pub fn verbose() -> bool {
    std::env::var_os("KETTLE_STEAMCLIENT_LOG").is_some()
}

/// The app this process is: `SteamAppId`, `SteamGameId`, or `steam_appid.txt` next to the working directory.
pub fn current_app() -> u32 {
    for k in ["SteamAppId", "SteamGameId"] {
        if let Some(n) = std::env::var(k).ok().and_then(|v| v.trim().parse::<u64>().ok()) {
            return (n & 0xFF_FFFF) as u32;
        }
    }
    std::fs::read_to_string("steam_appid.txt").ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// One request to Kettle. `None` when Kettle can't be reached (the library then behaves as "not logged on").
/// Never call this while holding the [`g`] guard.
pub fn rpc(req: Request) -> Option<Reply> {
    let mut slot = IPC.lock().unwrap_or_else(|e| e.into_inner());
    for attempt in 0..2 {
        if slot.is_none() {
            match Client::connect(&socket_path(), Duration::from_secs(20)) {
                Ok(mut c) => {
                    // Every new connection says which app it is.
                    if !matches!(req, Request::Hello { .. }) {
                        let _ = c.call(&Request::Hello { app: current_app() });
                    }
                    *slot = Some(c);
                }
                Err(e) => {
                    if attempt == 0 && verbose() {
                        log(&format!("cannot reach Kettle at {}: {e}", socket_path().display()));
                    }
                    return None;
                }
            }
        }
        match slot.as_mut().map(|c| c.call(&req)) {
            Some(Ok(rep)) => return Some(rep),
            _ => *slot = None,
        }
    }
    None
}

/// Who is signed in; asked once, and again after a failure.
pub fn identity() -> Identity {
    if let Some(i) = g().identity.clone() {
        return i;
    }
    match rpc(Request::Hello { app: current_app() }) {
        Some(Reply::Hello(i)) => {
            g().identity = Some(i.clone());
            i
        }
        _ => Identity::default(),
    }
}

pub fn owns(app: u32) -> bool {
    if let Some(v) = g().owned.get(&app) {
        return *v;
    }
    match rpc(Request::Owns { app }) {
        Some(Reply::Owns { owned }) => {
            g().owned.insert(app, owned);
            owned
        }
        _ => false,
    }
}

pub fn app_info(app: u32) -> AppInfo {
    if let Some(v) = g().app_info.get(&app) {
        return v.clone();
    }
    match rpc(Request::AppInfo { app }) {
        Some(Reply::AppInfo(i)) => {
            g().app_info.insert(app, i.clone());
            i
        }
        _ => AppInfo::default(),
    }
}
