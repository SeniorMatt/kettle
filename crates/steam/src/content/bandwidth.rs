//! Download speed limit, pause switch and byte counters shared by every download worker.
//!
//! The limit is a token bucket: each finished chunk "pays" its size, and when the bucket is in debt the
//! worker sleeps until the debt is repaid at the configured rate. All workers share one bucket, so the
//! limit holds for the whole download, not per connection.

use super::CancelToken;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A token bucket with a debt allowance. Time is passed in, so tests need no clock.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    /// Bytes per second; 0 = unlimited.
    rate: u64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate: u64, now: Instant) -> Self {
        TokenBucket { rate, tokens: rate as f64, last: now }
    }

    pub fn rate(&self) -> u64 {
        self.rate
    }

    pub fn set_rate(&mut self, rate: u64) {
        self.rate = rate;
        self.tokens = self.tokens.min(rate as f64);
    }

    /// Pay for `bytes` and say how long the caller must wait before going on (zero when within the limit).
    /// One second of burst is allowed; a chunk bigger than that puts the bucket in debt, repaid by waiting.
    pub fn take(&mut self, bytes: u64, now: Instant) -> Duration {
        if self.rate == 0 {
            self.last = now;
            return Duration::ZERO;
        }
        let rate = self.rate as f64;
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * rate).min(rate);
        self.tokens -= bytes as f64;
        if self.tokens >= 0.0 { Duration::ZERO } else { Duration::from_secs_f64(-self.tokens / rate) }
    }
}

static RATE: AtomicU64 = AtomicU64::new(0);
static BUCKET: Mutex<Option<TokenBucket>> = Mutex::new(None);
static PAUSED: AtomicBool = AtomicBool::new(false);
static NET_BYTES: AtomicU64 = AtomicU64::new(0);

/// Limit all downloads to `bytes_per_sec` (0 = no limit).
pub fn set_limit(bytes_per_sec: u64) {
    RATE.store(bytes_per_sec, Ordering::Relaxed);
}

pub fn limit() -> u64 {
    RATE.load(Ordering::Relaxed)
}

/// Hold every download worker (true) or let them go on (false). Workers wait between chunks.
pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

pub fn is_paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

/// Bytes received from the network since the program started (compressed, as sent).
pub fn net_bytes() -> u64 {
    NET_BYTES.load(Ordering::Relaxed)
}

/// Count `n` received bytes and sleep as long as the speed limit asks.
pub(crate) fn received(n: u64, cancel: &CancelToken) {
    NET_BYTES.fetch_add(n, Ordering::Relaxed);
    let rate = limit();
    if rate == 0 {
        return;
    }
    let wait = {
        let mut guard = BUCKET.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let bucket = guard.get_or_insert_with(|| TokenBucket::new(rate, now));
        if bucket.rate() != rate {
            bucket.set_rate(rate);
        }
        bucket.take(n, now)
    };
    sleep_unless_cancelled(wait, cancel);
}

/// Block while downloads are paused (returns early when cancelled).
pub(crate) fn wait_while_paused(cancel: &CancelToken) {
    while is_paused() && !cancel.is_cancelled() {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn sleep_unless_cancelled(total: Duration, cancel: &CancelToken) {
    let end = Instant::now() + total;
    while !cancel.is_cancelled() {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(200)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(0, t0);
        assert_eq!(b.take(10_000_000, t0), Duration::ZERO);
    }

    #[test]
    fn burst_then_debt_is_repaid_at_the_rate() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1000, t0);
        // One second of burst is free.
        assert_eq!(b.take(1000, t0), Duration::ZERO);
        // The next 500 bytes put us 500 in debt: half a second of waiting.
        let w = b.take(500, t0);
        assert!((w.as_secs_f64() - 0.5).abs() < 1e-6, "{w:?}");
        // After a full second the debt is paid and a new burst is available (capped at one second).
        assert_eq!(b.take(1000, t0 + Duration::from_secs(5)), Duration::ZERO);
    }

    #[test]
    fn long_run_average_matches_the_limit() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1_000_000, t0);
        let mut now = t0;
        let mut sent = 0u64;
        for _ in 0..100 {
            // 256 KB chunks; wait as told.
            now += b.take(262_144, now);
            sent += 262_144;
        }
        let secs = now.duration_since(t0).as_secs_f64();
        let avg = sent as f64 / secs;
        assert!(avg < 1_150_000.0 && avg > 950_000.0, "average {avg}");
    }

    #[test]
    fn lowering_the_rate_caps_saved_tokens() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(10_000, t0);
        b.set_rate(100);
        assert_eq!(b.take(100, t0), Duration::ZERO);
        assert!(b.take(100, t0) > Duration::ZERO);
    }
}
