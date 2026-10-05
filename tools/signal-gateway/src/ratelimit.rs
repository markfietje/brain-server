//! In-process sliding-window rate limiting for the HTTP surface.
//!
//! This module used to be a dead file in the binary-only tree: a
//! `#![allow(dead_code)]` blanket, four tests that called its own functions,
//! and not one production caller. `POST /v2/send` therefore had no
//! request-rate control at all — the `Arc<Semaphore>` in the Signal worker
//! bounds *in-flight* sends (5 at a time), which is a different question from
//! "how many requests per minute does this endpoint accept".
//!
//! It now lives in the library target so that `main.rs` and the integration
//! tests consume ONE definition, and `apply_rate_limit` puts it on the real
//! request path.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Requests admitted per window. Not a new number: these are the values
/// `create_rate_limiter()` hardcoded before this limiter was wired, named so
/// that production and tests cannot drift apart.
pub const API_RATE_LIMIT_MAX_REQUESTS: usize = 100;

/// Window length in seconds. See [`API_RATE_LIMIT_MAX_REQUESTS`].
pub const API_RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// The key every request is counted against.
///
/// Deliberately GLOBAL, not per-IP. The router is served via
/// `axum::serve(listener, app)` with no `into_make_service_with_connect_info`,
/// so there is no `ConnectInfo` to key on without new plumbing; and under this
/// crate's posture — loopback unless `SIGNAL_GATEWAY_ALLOW_REMOTE=1` — every
/// client is `127.0.0.1` anyway, so per-IP discrimination would be an illusion
/// that reads as control without being any. Behind a proxy it collapses to one
/// shared address regardless. A global budget is the bound that is actually
/// real, so it is the one that is enforced.
///
/// The limiter itself stays generic over its key: the tests isolate per-key
/// behaviour, so a future per-IP scheme is a call-site change, not a rewrite.
pub const API_RATE_LIMIT_KEY: &str = "api";

/// Rate limiter for API requests
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<RwLock<RateLimiterInner>>,
    max_requests: usize,
    window_secs: u64,
}

struct RateLimiterInner {
    keys: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    /// Create new rate limiter
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RateLimiterInner {
                keys: HashMap::new(),
            })),
            max_requests,
            window_secs,
        }
    }

    /// The window length, for a `RETRY-AFTER` header.
    pub fn window_secs(&self) -> u64 {
        self.window_secs
    }

    /// How many keys are currently tracked.
    ///
    /// Entries are evicted as their window drains, so this counts *live* keys
    /// and not every key ever seen. It is what makes the eviction above an
    /// observable property rather than an asserted one.
    pub fn tracked_keys(&self) -> usize {
        self.inner.read().keys.len()
    }

    /// The whole decision, taken at an INJECTED instant: prune, decide, record
    /// — one write-lock critical section, so two concurrent requests can never
    /// both observe the same pre-record state and both be admitted.
    ///
    /// Taking `now` as a parameter is what makes window expiry testable at all.
    /// The previous single `Instant::now()` call site had no clock seam, so no
    /// test could express "the window passed" — the module's own tests could
    /// only ever prove that a budget fills up, never that it drains.
    pub fn admit_at(&self, key: &str, now: Instant) -> bool {
        let mut inner = self.inner.write();
        // checked_sub: a window longer than time-since-boot saturates to
        // "nothing expires" instead of panicking on Instant underflow.
        let cutoff = now
            .checked_sub(Duration::from_secs(self.window_secs))
            .unwrap_or(now);

        // Prune the touched key before deciding, so the verdict reflects the
        // window as it stands at `now` rather than as it stood when the entries
        // were recorded.
        let mut live = 0usize;
        if let Some(recorded) = inner.keys.get_mut(key) {
            recorded.retain(|t| *t > cutoff);
            live = recorded.len();
        }

        // Reclaim any key whose whole window has drained. Entries are appended
        // in chronological order, so the last one is the newest.
        //
        // Under the single global production key this is one comparison against
        // one entry and changes nothing. It exists because pruning on access
        // alone cannot bound the map: a key that stops being used would never
        // be visited again, so a per-IP keying scheme would grow the map with
        // every address that ever connected. The bound this actually gives is
        // "keys seen within the last window", and the next request is what
        // collects them.
        inner
            .keys
            .retain(|_, recorded| recorded.last().is_some_and(|newest| *newest > cutoff));

        let admitted = live < self.max_requests;
        if admitted {
            inner.keys.entry(key.to_owned()).or_default().push(now);
        }
        admitted
    }

    /// Check if a request under `key` is allowed against the real clock.
    pub fn is_allowed(&self, key: &str) -> bool {
        self.admit_at(key, Instant::now())
    }
}

/// The production limiter, built from the named constants above.
pub fn create_rate_limiter() -> RateLimiter {
    RateLimiter::new(API_RATE_LIMIT_MAX_REQUESTS, API_RATE_LIMIT_WINDOW_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_allows() {
        let limiter = RateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(limiter.is_allowed("127.0.0.1"));
        }
    }

    #[test]
    fn test_rate_limit_blocks() {
        let limiter = RateLimiter::new(2, 60);
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(!limiter.is_allowed("127.0.0.1"));
    }

    #[test]
    fn test_rate_limit_per_key() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(!limiter.is_allowed("127.0.0.1"));
        assert!(limiter.is_allowed("127.0.0.2")); // a different key is unaffected
    }
}
