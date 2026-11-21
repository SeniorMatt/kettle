//! Kettle's side of its own Steamworks service: answers the questions the game-side `steamclient.so`
//! (`crates/steamclient`) asks over a Unix socket. See `docs/steamclient-design.md`.
//!
//! The socket is `$XDG_RUNTIME_DIR/kettle-steamclient.sock` (mode 0600; only the same user may connect, checked
//! with `SO_PEERCRED`). A connection starts with `hello` naming its app; afterwards it is served for that app only,
//! and tickets and stats only for apps the signed-in account owns. Ownership comes from Steam's servers (the
//! account's own owned-games list, set with [`CmBackend::set_owned`]); nothing here can make an unowned game run.

#![allow(dead_code)] // not wired into the app yet

use crate::cm_hub::HubHandle;
use serde::{Deserialize, Serialize};
use sr_steamclient::proto::{
    AppInfo, ERESULT_ACCESS_DENIED, ERESULT_FAIL, ERESULT_NO_CONNECTION, ERESULT_OK, Identity, Reply, ReplyLine,
    Request, RequestLine, StatValue, hex_decode, hex_encode, read_line_limited,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steam_vent::ConnectionTrait;
use steam_vent_proto_common::{RpcMessage, RpcMessageWithKind, protobuf};
use steam_vent_proto_steam::enums_clientserver::EMsg;
use steam_vent_proto_steam::steammessages_clientserver::{
    CMsgClientGetAppOwnershipTicket, CMsgClientGetAppOwnershipTicketResponse, CMsgClientRequestEncryptedAppTicket,
    CMsgClientRequestEncryptedAppTicketResponse,
};

/// Most simultaneous game connections.
const MAX_CONNECTIONS: usize = 16;
/// Most stats / achievements one app may store, and the longest name.
const MAX_STATS: usize = 4096;
const MAX_NAME: usize = 256;
/// How long a ticket request to the CM may take.
const CM_TIMEOUT: Duration = Duration::from_secs(15);

/// One app's stats and achievements (name -> unlock time).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsData {
    pub stats: BTreeMap<String, StatValue>,
    pub achievements: BTreeMap<String, u32>,
}

/// What the host needs from the signed-in account. Implemented by [`CmBackend`]; tests use a fake.
pub trait Backend: Send + Sync + 'static {
    fn identity(&self) -> Identity;
    /// Does the signed-in account own `app`? Must come from Steam's data, never a guess.
    fn owns(&self, app: u32) -> bool;
    fn app_ticket(&self, app: u32) -> Result<Vec<u8>, String>;
    fn auth_ticket(&self, app: u32) -> Result<Vec<u8>, String>;
    fn encrypted_ticket(&self, app: u32, userdata: &[u8]) -> Result<Vec<u8>, String>;
    fn load_stats(&self, app: u32) -> Result<StatsData, String>;
    fn save_stats(&self, app: u32, data: &StatsData) -> Result<(), String>;
    fn app_info(&self, app: u32) -> AppInfo;
}

/// Per-connection state.
#[derive(Default)]
pub struct Session {
    app: Option<u32>,
}

fn error(message: &str) -> Reply {
    Reply::Error { message: message.into() }
}

fn ticket_reply(r: Result<Vec<u8>, String>) -> Reply {
    match r {
        Ok(t) => Reply::Ticket { eresult: ERESULT_OK, ticket: hex_encode(&t) },
        Err(_) => Reply::Ticket { eresult: ERESULT_FAIL, ticket: String::new() },
    }
}

/// Answer one request. Pure apart from the backend, so it is tested without a socket.
pub fn handle(backend: &dyn Backend, session: &mut Session, req: Request) -> Reply {
    if let Request::Hello { app } = req {
        session.app = Some(app);
        return Reply::Hello(backend.identity());
    }
    let Some(mine) = session.app else { return error("say hello first") };
    let (Request::Owns { app }
    | Request::AppTicket { app }
    | Request::AuthTicket { app }
    | Request::EncryptedTicket { app, .. }
    | Request::GetStats { app }
    | Request::StoreStats { app, .. }
    | Request::AppInfo { app }) = &req
    else {
        return error("unknown request");
    };
    let app = *app;
    // Looking up ownership or install info of other apps (DLC) is fine; everything else is for this app only.
    if !matches!(req, Request::Owns { .. } | Request::AppInfo { .. }) && app != mine {
        return error("request for another app");
    }
    let me = backend.identity();
    match req {
        Request::Owns { .. } => Reply::Owns { owned: me.logged_on && backend.owns(app) },
        Request::AppInfo { .. } => Reply::AppInfo(backend.app_info(app)),
        _ if !me.logged_on => match req {
            Request::GetStats { .. } => {
                Reply::Stats { eresult: ERESULT_NO_CONNECTION, stats: BTreeMap::new(), achievements: BTreeMap::new() }
            }
            Request::StoreStats { .. } => Reply::Stored { eresult: ERESULT_NO_CONNECTION },
            _ => Reply::Ticket { eresult: ERESULT_NO_CONNECTION, ticket: String::new() },
        },
        _ if !backend.owns(app) => match req {
            Request::GetStats { .. } => {
                Reply::Stats { eresult: ERESULT_ACCESS_DENIED, stats: BTreeMap::new(), achievements: BTreeMap::new() }
            }
            Request::StoreStats { .. } => Reply::Stored { eresult: ERESULT_ACCESS_DENIED },
            _ => Reply::Ticket { eresult: ERESULT_ACCESS_DENIED, ticket: String::new() },
        },
        Request::AppTicket { .. } => ticket_reply(backend.app_ticket(app)),
        Request::AuthTicket { .. } => ticket_reply(backend.auth_ticket(app)),
        Request::EncryptedTicket { userdata, .. } => match hex_decode(&userdata) {
            Some(d) if d.len() <= 2048 => ticket_reply(backend.encrypted_ticket(app, &d)),
            _ => error("bad user data"),
        },
        Request::GetStats { .. } => match backend.load_stats(app) {
            Ok(d) => Reply::Stats { eresult: ERESULT_OK, stats: d.stats, achievements: d.achievements },
            Err(_) => Reply::Stats { eresult: ERESULT_FAIL, stats: BTreeMap::new(), achievements: BTreeMap::new() },
        },
        Request::StoreStats { stats, achievements, .. } => {
            let bad = stats.len() > MAX_STATS
                || achievements.len() > MAX_STATS
                || stats.keys().chain(achievements.keys()).any(|k| k.is_empty() || k.len() > MAX_NAME);
            if bad {
                return error("too many or badly named stats");
            }
            match backend.save_stats(app, &StatsData { stats, achievements }) {
                Ok(()) => Reply::Stored { eresult: ERESULT_OK },
                Err(_) => Reply::Stored { eresult: ERESULT_FAIL },
            }
        }
        Request::Hello { .. } => unreachable!("handled above"),
    }
}

// ---------------------------------------------------------------- stats on disk

/// `<root>/<steamid>/<app>.json`, written atomically. Names come from the game, but only numbers build the path.
pub struct StatsStore {
    root: PathBuf,
}

impl StatsStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, steam_id: u64, app: u32) -> PathBuf {
        self.root.join(steam_id.to_string()).join(format!("{app}.json"))
    }

    pub fn load(&self, steam_id: u64, app: u32) -> Result<StatsData, String> {
        match std::fs::read(self.path(steam_id, app)) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(StatsData::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self, steam_id: u64, app: u32, data: &StatsData) -> Result<(), String> {
        let path = self.path(steam_id, app);
        let dir = path.parent().ok_or("no directory")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- the CM-backed backend

macro_rules! wire {
    ($name:ident, $inner:ty, $kind:ident) => {
        /// The proto crate does not mark this message with its EMsg; this wrapper does.
        #[derive(Debug)]
        struct $name($inner);
        impl RpcMessage for $name {
            fn parse(r: &mut dyn std::io::Read) -> protobuf::Result<Self> {
                <$inner as RpcMessage>::parse(r).map($name)
            }
            fn write(&self, w: &mut dyn std::io::Write) -> protobuf::Result<()> {
                RpcMessage::write(&self.0, w)
            }
            fn encode_size(&self) -> usize {
                RpcMessage::encode_size(&self.0)
            }
        }
        impl RpcMessageWithKind for $name {
            type KindEnum = EMsg;
            const KIND: EMsg = EMsg::$kind;
        }
    };
}
wire!(OwnershipTicketReq, CMsgClientGetAppOwnershipTicket, k_EMsgClientGetAppOwnershipTicket);
wire!(OwnershipTicketRsp, CMsgClientGetAppOwnershipTicketResponse, k_EMsgClientGetAppOwnershipTicketResponse);
wire!(EncryptedTicketReq, CMsgClientRequestEncryptedAppTicket, k_EMsgClientRequestEncryptedAppTicket);
wire!(EncryptedTicketRsp, CMsgClientRequestEncryptedAppTicketResponse, k_EMsgClientRequestEncryptedAppTicketResponse);

/// Answers from the one CM session of the signed-in account.
pub struct CmBackend {
    hub: HubHandle,
    rt: tokio::runtime::Handle,
    stats: StatsStore,
    last_steam_id: AtomicU64,
    owned: Mutex<HashSet<u32>>,
    persona: Mutex<String>,
    language: Mutex<String>,
    apps: Mutex<HashMap<u32, AppInfo>>,
}

impl CmBackend {
    pub(crate) fn new(hub: HubHandle, rt: tokio::runtime::Handle, stats_root: PathBuf) -> Self {
        Self {
            hub,
            rt,
            stats: StatsStore::new(stats_root),
            last_steam_id: AtomicU64::new(0),
            owned: Mutex::new(HashSet::new()),
            persona: Mutex::new(String::new()),
            language: Mutex::new("english".into()),
            apps: Mutex::new(HashMap::new()),
        }
    }

    /// The app ids of the account's own library (from Steam's owned-games answer), replacing the previous set.
    pub fn set_owned(&self, ids: impl IntoIterator<Item = u32>) {
        *self.owned.lock().unwrap_or_else(|e| e.into_inner()) = ids.into_iter().collect();
    }

    pub fn set_persona(&self, name: &str) {
        *self.persona.lock().unwrap_or_else(|e| e.into_inner()) = name.to_string();
    }

    pub fn set_language(&self, lang: &str) {
        *self.language.lock().unwrap_or_else(|e| e.into_inner()) = lang.to_string();
    }

    pub fn set_app_info(&self, app: u32, info: AppInfo) {
        self.apps.lock().unwrap_or_else(|e| e.into_inner()).insert(app, info);
    }

    fn steam_id(&self) -> u64 {
        if let Some(cm) = self.hub.online_now() {
            let id = u64::from(cm.conn.steam_id());
            self.last_steam_id.store(id, Ordering::Relaxed);
            return id;
        }
        self.last_steam_id.load(Ordering::Relaxed)
    }

    fn cm(&self) -> Result<Arc<sr_steam::cm::Cm>, String> {
        self.hub.online_now().ok_or_else(|| "not connected to Steam".to_string())
    }
}

impl Backend for CmBackend {
    fn identity(&self) -> Identity {
        let logged_on = self.hub.online_now().is_some();
        Identity {
            steam_id: self.steam_id(),
            persona: self.persona.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            logged_on,
            language: self.language.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    fn owns(&self, app: u32) -> bool {
        self.owned.lock().unwrap_or_else(|e| e.into_inner()).contains(&app)
    }

    fn app_ticket(&self, app: u32) -> Result<Vec<u8>, String> {
        let cm = self.cm()?;
        let req = OwnershipTicketReq(CMsgClientGetAppOwnershipTicket { app_id: Some(app), ..Default::default() });
        let rsp: OwnershipTicketRsp = self
            .rt
            .block_on(async { tokio::time::timeout(CM_TIMEOUT, cm.conn.job(req)).await })
            .map_err(|_| "Steam did not answer in time".to_string())?
            .map_err(|e| e.to_string())?;
        if rsp.0.eresult() != ERESULT_OK as u32 {
            return Err(format!("Steam refused the ticket (EResult {})", rsp.0.eresult()));
        }
        rsp.0.ticket.clone().filter(|t| !t.is_empty()).ok_or_else(|| "Steam sent no ticket".to_string())
    }

    /// For now the raw ownership ticket. Steam's own client frames it with a GC token and a session header
    /// (see the design doc); servers that verify the full framing will not accept this yet.
    fn auth_ticket(&self, app: u32) -> Result<Vec<u8>, String> {
        self.app_ticket(app)
    }

    fn encrypted_ticket(&self, app: u32, userdata: &[u8]) -> Result<Vec<u8>, String> {
        let cm = self.cm()?;
        let req = EncryptedTicketReq(CMsgClientRequestEncryptedAppTicket {
            app_id: Some(app),
            userdata: Some(userdata.to_vec()),
            ..Default::default()
        });
        let rsp: EncryptedTicketRsp = self
            .rt
            .block_on(async { tokio::time::timeout(CM_TIMEOUT, cm.conn.job(req)).await })
            .map_err(|_| "Steam did not answer in time".to_string())?
            .map_err(|e| e.to_string())?;
        if rsp.0.eresult() != ERESULT_OK {
            return Err(format!("Steam refused the ticket (EResult {})", rsp.0.eresult()));
        }
        rsp.0
            .encrypted_app_ticket
            .as_ref()
            .and_then(|t| t.encrypted_ticket.clone())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "Steam sent no ticket".to_string())
    }

    fn load_stats(&self, app: u32) -> Result<StatsData, String> {
        self.stats.load(self.steam_id(), app)
    }

    fn save_stats(&self, app: u32, data: &StatsData) -> Result<(), String> {
        self.stats.save(self.steam_id(), app, data)
    }

    fn app_info(&self, app: u32) -> AppInfo {
        self.apps.lock().unwrap_or_else(|e| e.into_inner()).get(&app).cloned().unwrap_or_default()
    }
}

// ---------------------------------------------------------------- the socket

pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
}

fn same_user(s: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    // SAFETY: plain getsockopt on a valid socket into a ucred-sized buffer.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let r = libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        );
        r == 0 && cred.uid == libc::geteuid()
    }
}

impl Server {
    /// Listen on `path`. A stale socket file is replaced; a live one (another Kettle) is an error.
    pub fn start(path: &Path, backend: Arc<dyn Backend>) -> io::Result<Server> {
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(io::ErrorKind::AddrInUse, "another Kettle is already serving Steamworks"));
            }
            std::fs::remove_file(path)?;
        }
        // No umask change here: umask is process-wide, so other threads creating files at the same moment would
        // get mode 0600 directories. The socket lives in $XDG_RUNTIME_DIR (0700, only this user), and its mode is
        // narrowed right after the bind.
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, active) = (stop.clone(), Arc::new(AtomicUsize::new(0)));
        std::thread::Builder::new().name("steamworks-host".into()).spawn(move || {
            for stream in listener.incoming() {
                if stop2.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                if !same_user(&stream) {
                    continue;
                }
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let (backend, active2) = (backend.clone(), active.clone());
                let spawned = std::thread::Builder::new().name("steamworks-conn".into()).spawn(move || {
                    let _ = serve_connection(stream, backend.as_ref());
                    active2.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })?;
        Ok(Server { path: path.to_path_buf(), stop })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop so it sees the flag.
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve_connection(stream: UnixStream, backend: &dyn Backend) -> io::Result<()> {
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    let mut session = Session::default();
    while let Some(line) = read_line_limited(&mut r)? {
        let Ok(req) = serde_json::from_str::<RequestLine>(&line) else { return Ok(()) };
        let rep = handle(backend, &mut session, req.req);
        let mut out = serde_json::to_string(&ReplyLine { id: req.id, rep }).map_err(io::Error::other)?;
        out.push('\n');
        w.write_all(out.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_steamclient::ipc::Client;

    struct Fake {
        online: bool,
        stats: Mutex<StatsData>,
    }

    impl Fake {
        fn new(online: bool) -> Arc<Self> {
            Arc::new(Self { online, stats: Mutex::new(StatsData::default()) })
        }
    }

    impl Backend for Fake {
        fn identity(&self) -> Identity {
            Identity { steam_id: 42, persona: "P".into(), logged_on: self.online, language: "english".into() }
        }
        fn owns(&self, app: u32) -> bool {
            app == 480 || app == 481
        }
        fn app_ticket(&self, _: u32) -> Result<Vec<u8>, String> {
            Ok(vec![1, 2])
        }
        fn auth_ticket(&self, _: u32) -> Result<Vec<u8>, String> {
            Ok(vec![3])
        }
        fn encrypted_ticket(&self, _: u32, d: &[u8]) -> Result<Vec<u8>, String> {
            Ok(d.to_vec())
        }
        fn load_stats(&self, _: u32) -> Result<StatsData, String> {
            Ok(self.stats.lock().unwrap().clone())
        }
        fn save_stats(&self, _: u32, d: &StatsData) -> Result<(), String> {
            *self.stats.lock().unwrap() = d.clone();
            Ok(())
        }
        fn app_info(&self, _: u32) -> AppInfo {
            AppInfo { install_dir: Some("/g".into()), build_id: 7 }
        }
    }

    fn ask(b: &Fake, s: &mut Session, r: Request) -> Reply {
        handle(b, s, r)
    }

    #[test]
    fn a_session_must_say_hello_and_stays_on_its_app() {
        let b = Fake::new(true);
        let mut s = Session::default();
        assert!(matches!(ask(&b, &mut s, Request::Owns { app: 480 }), Reply::Error { .. }));
        assert!(matches!(ask(&b, &mut s, Request::Hello { app: 480 }), Reply::Hello(i) if i.steam_id == 42));
        assert_eq!(ask(&b, &mut s, Request::Owns { app: 481 }), Reply::Owns { owned: true });
        assert_eq!(ask(&b, &mut s, Request::Owns { app: 440 }), Reply::Owns { owned: false });
        assert!(matches!(ask(&b, &mut s, Request::AppTicket { app: 481 }), Reply::Error { .. }));
        assert!(matches!(ask(&b, &mut s, Request::GetStats { app: 440 }), Reply::Error { .. }));
        assert_eq!(
            ask(&b, &mut s, Request::AppTicket { app: 480 }),
            Reply::Ticket { eresult: 1, ticket: "0102".into() }
        );
    }

    #[test]
    fn unowned_apps_get_no_tickets_or_stats() {
        let b = Fake::new(true);
        let mut s = Session::default();
        ask(&b, &mut s, Request::Hello { app: 440 });
        let denied = Reply::Ticket { eresult: ERESULT_ACCESS_DENIED, ticket: String::new() };
        assert_eq!(ask(&b, &mut s, Request::AppTicket { app: 440 }), denied);
        assert_eq!(ask(&b, &mut s, Request::AuthTicket { app: 440 }), denied);
        assert_eq!(ask(&b, &mut s, Request::EncryptedTicket { app: 440, userdata: String::new() }), denied);
        assert!(matches!(
            ask(&b, &mut s, Request::GetStats { app: 440 }),
            Reply::Stats { eresult: ERESULT_ACCESS_DENIED, .. }
        ));
        assert_eq!(
            ask(&b, &mut s, Request::StoreStats { app: 440, stats: BTreeMap::new(), achievements: BTreeMap::new() }),
            Reply::Stored { eresult: ERESULT_ACCESS_DENIED }
        );
        assert!(b.stats.lock().unwrap().stats.is_empty());
    }

    #[test]
    fn offline_means_not_logged_on_and_nothing_owned() {
        let b = Fake::new(false);
        let mut s = Session::default();
        assert!(matches!(ask(&b, &mut s, Request::Hello { app: 480 }), Reply::Hello(i) if !i.logged_on));
        assert_eq!(ask(&b, &mut s, Request::Owns { app: 480 }), Reply::Owns { owned: false });
        assert_eq!(
            ask(&b, &mut s, Request::AppTicket { app: 480 }),
            Reply::Ticket { eresult: ERESULT_NO_CONNECTION, ticket: String::new() }
        );
        assert_eq!(
            ask(&b, &mut s, Request::StoreStats { app: 480, stats: BTreeMap::new(), achievements: BTreeMap::new() }),
            Reply::Stored { eresult: ERESULT_NO_CONNECTION }
        );
    }

    #[test]
    fn stats_round_trip_and_bad_names_are_refused() {
        let b = Fake::new(true);
        let mut s = Session::default();
        ask(&b, &mut s, Request::Hello { app: 480 });
        let mut stats = BTreeMap::new();
        stats.insert("kills".to_string(), StatValue::Int(3));
        let mut ach = BTreeMap::new();
        ach.insert("WIN".to_string(), 99u32);
        let store = Request::StoreStats { app: 480, stats: stats.clone(), achievements: ach.clone() };
        assert_eq!(ask(&b, &mut s, store), Reply::Stored { eresult: 1 });
        assert_eq!(
            ask(&b, &mut s, Request::GetStats { app: 480 }),
            Reply::Stats { eresult: 1, stats: stats.clone(), achievements: ach }
        );
        let mut bad = BTreeMap::new();
        bad.insert(String::new(), StatValue::Int(1));
        assert!(matches!(
            ask(&b, &mut s, Request::StoreStats { app: 480, stats: bad, achievements: BTreeMap::new() }),
            Reply::Error { .. }
        ));
        assert_eq!(b.stats.lock().unwrap().stats, stats, "a refused store changes nothing");
        let long = "x".repeat(MAX_NAME + 1);
        let mut bad = BTreeMap::new();
        bad.insert(long, 1u32);
        assert!(matches!(
            ask(&b, &mut s, Request::StoreStats { app: 480, stats: BTreeMap::new(), achievements: bad }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn encrypted_ticket_user_data_is_checked() {
        let b = Fake::new(true);
        let mut s = Session::default();
        ask(&b, &mut s, Request::Hello { app: 480 });
        assert_eq!(
            ask(&b, &mut s, Request::EncryptedTicket { app: 480, userdata: "6869".into() }),
            Reply::Ticket { eresult: 1, ticket: "6869".into() }
        );
        assert!(matches!(
            ask(&b, &mut s, Request::EncryptedTicket { app: 480, userdata: "zz".into() }),
            Reply::Error { .. }
        ));
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kettle-host-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn stats_store_persists_per_account_and_app() {
        let root = temp("stats");
        let store = StatsStore::new(root.clone());
        assert_eq!(store.load(1, 480).unwrap(), StatsData::default());
        let mut d = StatsData::default();
        d.stats.insert("a".into(), StatValue::Float(1.5));
        d.achievements.insert("B".into(), 5);
        store.save(1, 480, &d).unwrap();
        assert_eq!(store.load(1, 480).unwrap(), d);
        assert_eq!(store.load(2, 480).unwrap(), StatsData::default());
        assert_eq!(store.load(1, 481).unwrap(), StatsData::default());
        std::fs::write(root.join("1").join("7.json"), b"not json").unwrap();
        assert!(store.load(1, 7).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_socket_serves_a_game_side_client() {
        let dir = temp("sock");
        let path = dir.join("s.sock");
        let server = Server::start(&path, Fake::new(true)).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // A second server on a live socket is refused.
        assert!(Server::start(&path, Fake::new(true)).is_err());
        let mut c = Client::connect(&path, Duration::from_secs(5)).unwrap();
        assert!(matches!(c.call(&Request::Hello { app: 480 }).unwrap(), Reply::Hello(i) if i.persona == "P"));
        assert_eq!(c.call(&Request::Owns { app: 480 }).unwrap(), Reply::Owns { owned: true });
        assert_eq!(
            c.call(&Request::AppInfo { app: 480 }).unwrap(),
            Reply::AppInfo(AppInfo { install_dir: Some("/g".into()), build_id: 7 })
        );
        drop(c);
        drop(server);
        assert!(!path.exists(), "the socket file is removed on shutdown");
        let _ = std::fs::remove_dir_all(dir);
    }
}
