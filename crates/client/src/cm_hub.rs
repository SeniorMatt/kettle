//! The one CM session of the signed-in account.
//!
//! Steam replaces an older session when the same account logs on again with the same address, so the
//! library, the friends list and the downloads must not each open their own. The hub connects once with the
//! refresh token, publishes the live connection (and the licenses it carries) on a `watch` channel, and
//! reconnects with a growing pause when the connection ends. It stops when the generation changes
//! (sign-out). Everything else asks the hub for the connection instead of calling `cm::connect`.

use crate::{ConnectionStatus, Event, Waker};
use sr_steam::cm::{self, Cm};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Pauses between connection attempts: 5, 10, 20, 40, then 60 seconds for every later one.
const BACKOFF_SECS: [u64; 5] = [5, 10, 20, 40, 60];

/// Pause before connection attempt number `attempt` (0 = the first retry).
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(BACKOFF_SECS[(attempt as usize).min(BACKOFF_SECS.len() - 1)])
}

/// The hub's state as consumers see it. `Online` carries a number that grows with every new connection.
#[derive(Clone)]
pub(crate) enum HubState {
    Connecting,
    Online { id: u64, cm: Arc<Cm> },
    Offline(String),
}

impl HubState {
    fn status(&self) -> ConnectionStatus {
        match self {
            Self::Connecting => ConnectionStatus::Connecting,
            Self::Online { .. } => ConnectionStatus::Online,
            Self::Offline(why) => ConnectionStatus::Failed(why.clone()),
        }
    }
}

/// A view of the hub. Cheap to clone.
#[derive(Clone)]
pub(crate) struct HubHandle {
    pub(crate) rx: watch::Receiver<HubState>,
    generation: u64,
    current: Arc<AtomicU64>,
}

impl HubHandle {
    fn stale(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    /// Wait for a connection newer than `after` (0 = any). `None` once the generation changed or the hub ended.
    pub(crate) async fn wait_online(&mut self, after: u64) -> Option<(u64, Arc<Cm>)> {
        loop {
            if self.stale() {
                return None;
            }
            if let HubState::Online { id, cm } = &*self.rx.borrow_and_update()
                && *id > after
            {
                return Some((*id, cm.clone()));
            }
            if let Ok(Err(_)) = tokio::time::timeout(Duration::from_secs(1), self.rx.changed()).await {
                return None;
            }
        }
    }

    /// The current connection, if one is up (for threads that are not async).
    pub(crate) fn online_now(&self) -> Option<Arc<Cm>> {
        match &*self.rx.borrow() {
            HubState::Online { cm, .. } => Some(cm.clone()),
            _ => None,
        }
    }
}

/// The hub's own copy of the refresh token, wiped when the task ends.
pub(crate) struct TokenCopy(pub(crate) String);

impl Drop for TokenCopy {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        unsafe { self.0.as_bytes_mut() }.fill(0);
    }
}

/// Start the hub on the current runtime. It runs until `current` no longer equals `generation`.
pub(crate) fn spawn(
    rt: &tokio::runtime::Handle,
    account: String,
    token: TokenCopy,
    generation: u64,
    current: Arc<AtomicU64>,
    events: mpsc::Sender<Event>,
    wake: Waker,
) -> HubHandle {
    let (tx, rx) = watch::channel(HubState::Connecting);
    let handle = HubHandle { rx, generation, current: current.clone() };
    rt.spawn(async move {
        let publish = |state: HubState| {
            let (events, wake, status) = (events.clone(), wake.clone(), state.status());
            let tx = &tx;
            async move {
                // The event goes first, so the UI hears "online" before any consumer reacts to it.
                let _ = events.send(Event::Hub { generation, status }).await;
                wake();
                tx.send_replace(state);
            }
        };
        let (mut attempt, mut id) = (0u32, 0u64);
        while current.load(Ordering::Relaxed) == generation {
            publish(HubState::Connecting).await;
            let tried = tokio::select! {
                r = connect_once(&account, &token.0) => Some(r),
                _ = until_stale(&current, generation) => None,
            };
            let Some(tried) = tried else { break };
            match tried {
                Ok(cm) => {
                    id += 1;
                    let conn = cm.conn.clone();
                    publish(HubState::Online { id, cm: Arc::new(cm) }).await;
                    attempt = 0;
                    let why = tokio::select! {
                        why = cm::until_closed(&conn) => why,
                        _ = until_stale(&current, generation) => break,
                    };
                    drop(conn);
                    publish(HubState::Offline(why)).await;
                }
                Err(e) => publish(HubState::Offline(format!("{e:#}"))).await,
            }
            let pause = backoff(attempt);
            attempt = attempt.saturating_add(1);
            tokio::select! {
                _ = tokio::time::sleep(pause) => {}
                _ = until_stale(&current, generation) => break,
            }
        }
    });
    handle
}

async fn connect_once(account: &str, token: &str) -> anyhow::Result<Cm> {
    // Developer aid for testing how the app behaves while Steam can't be reached (e.g. closing the window).
    if std::env::var_os("KETTLE_HUB_STALL").is_some() {
        std::future::pending::<()>().await;
    }
    cm::connect(account, token).await
}

async fn until_stale(current: &AtomicU64, generation: u64) {
    while current.load(Ordering::Relaxed) == generation {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_stops_at_a_minute() {
        let secs: Vec<u64> = (0..8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [5, 10, 20, 40, 60, 60, 60, 60]);
        assert_eq!(backoff(u32::MAX).as_secs(), 60);
    }
}
