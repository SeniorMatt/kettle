//! When a friend notification may be shown: nothing in the first seconds after sign-in (Steam sends every
//! friend's state then), at most a few per minute, and one per friend every few minutes.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Silence after the first friend update of a session.
pub const QUIET_AT_START: Duration = Duration::from_secs(30);
/// The same friend notifies at most this often.
pub const PER_FRIEND: Duration = Duration::from_secs(5 * 60);
/// At most [`MAX_PER_WINDOW`] notifications in this window, over all friends.
pub const WINDOW: Duration = Duration::from_secs(60);
pub const MAX_PER_WINDOW: usize = 4;
/// Friends remembered for the per-friend limit.
const MAX_TRACKED: usize = 512;

#[derive(Debug, Default)]
pub struct Gate {
    started: Option<Instant>,
    recent: VecDeque<Instant>,
    last: HashMap<u64, Instant>,
}

impl Gate {
    /// May a notification about `friend` be shown at `now`? A `true` counts it.
    pub fn allow(&mut self, now: Instant, friend: u64) -> bool {
        let started = *self.started.get_or_insert(now);
        if now.saturating_duration_since(started) < QUIET_AT_START {
            return false;
        }
        while self.recent.front().is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW) {
            self.recent.pop_front();
        }
        if self.recent.len() >= MAX_PER_WINDOW {
            return false;
        }
        if self.last.get(&friend).is_some_and(|t| now.saturating_duration_since(*t) < PER_FRIEND) {
            return false;
        }
        if self.last.len() >= MAX_TRACKED {
            self.last.retain(|_, t| now.saturating_duration_since(*t) < PER_FRIEND);
        }
        self.recent.push_back(now);
        self.last.insert(friend, now);
        true
    }

    /// Note that updates are arriving (starts the quiet time) without asking for a notification.
    pub fn touch(&mut self, now: Instant) {
        self.started.get_or_insert(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_for_the_first_thirty_seconds() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        assert!(!g.allow(t0, 1));
        assert!(!g.allow(t0 + Duration::from_secs(29), 2));
        assert!(g.allow(t0 + Duration::from_secs(31), 3));
    }

    #[test]
    fn one_per_friend_and_a_few_per_minute() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        g.touch(t0);
        let t = t0 + Duration::from_secs(40);
        assert!(g.allow(t, 1));
        assert!(!g.allow(t + Duration::from_secs(10), 1), "same friend again");
        assert!(g.allow(t + Duration::from_secs(1), 2));
        assert!(g.allow(t + Duration::from_secs(2), 3));
        assert!(g.allow(t + Duration::from_secs(3), 4));
        assert!(!g.allow(t + Duration::from_secs(4), 5), "fifth in a minute");
        assert!(g.allow(t + Duration::from_secs(70), 5), "the window moved on");
        assert!(g.allow(t + PER_FRIEND + Duration::from_secs(1), 1), "the friend's pause is over");
    }
}
