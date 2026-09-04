//! HTTP-edge load control: the per-IP request limiter, the
//! connection-capacity tracker (with its RAII slot guard), and the
//! connection/RSS watchdogs. No transport types — the rate-limit middleware
//! and the boot wiring call in from `main.rs`.

use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration as StdDuration, Instant};

use sysinfo::System;
use tracing::error;

use crate::config;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    id: usize,
    acquired_at: Instant,
    location: String,
}

pub struct ConnectionTracker {
    connections: Mutex<HashMap<usize, ConnectionInfo>>,
    next_id: AtomicUsize,
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionTracker {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            next_id: AtomicUsize::new(1),
        }
    }

    pub fn track(&self, location: &str) -> usize {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let info = ConnectionInfo {
            id,
            acquired_at: Instant::now(),
            location: location.to_string(),
        };
        if let Ok(mut conns) = self.connections.lock() {
            conns.insert(id, info);
        }
        id
    }

    pub fn release(&self, id: usize) {
        if let Ok(mut conns) = self.connections.lock() {
            conns.remove(&id);
        }
    }

    pub fn get_long_running(&self, threshold: std::time::Duration) -> Vec<ConnectionInfo> {
        if let Ok(conns) = self.connections.lock() {
            conns
                .values()
                .filter(|info| info.acquired_at.elapsed() > threshold)
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Slot count — test-only introspection for the RAII/watchdog pins.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        if let Ok(conns) = self.connections.lock() {
            conns.len()
        } else {
            0
        }
    }
}

/// RAII guard for a [`ConnectionTracker`] slot.
/// The release used to live at the end of each defer-less closure — a
/// short-circuit `return` inside `spawn_blocking` (or the 60 s timeout
/// dropping the task mid-flight) leaked the slot until the watchdog noticed.
/// Drop fires on EVERY exit path — early `return`, `?`, panic, timeout drop —
/// so the capacity guard the tracker implements can never be silently
/// bypassed by an in-flight closure.
pub struct TrackerEntry {
    id: usize,
    tracker: std::sync::Arc<ConnectionTracker>,
}

impl TrackerEntry {
    pub fn new(tracker: std::sync::Arc<ConnectionTracker>, location: &str) -> Self {
        let id = tracker.track(location);
        Self { id, tracker }
    }
}

impl Drop for TrackerEntry {
    fn drop(&mut self) {
        self.tracker.release(self.id);
    }
}

pub struct RateLimiter {
    requests: Mutex<HashMap<String, Vec<Instant>>>,
    max_requests: usize,
    window: StdDuration,
    /// bounded memory. When the tracked-IP set would exceed this,
    /// the oldest 25% of buckets are evicted. Defeats the spoofed-XFF memory
    /// exhaustion attack.
    max_keys: usize,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    /// The default per-window budget, exposed read-only for router-level pins
    /// that drive a limiter to exhaustion without reaching into privates.
    #[cfg(test)]
    pub(crate) const WINDOW_BUDGET_PROBE: usize = 10_000;
    /// Promoted to `pub` at the lib flip (integration fixtures construct it);
    /// the `Default` impl satisfies the clippy lint without changing behavior.
    pub fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            max_requests: 10_000,
            window: StdDuration::from_secs(60),
            max_keys: config::RATE_LIMIT_MAX_KEYS,
        }
    }

    pub(crate) fn is_allowed(&self, ip: &str) -> bool {
        let now = Instant::now();
        if let Ok(mut requests) = self.requests.lock() {
            // Bounded memory: if the bucket count is at the cap, evict the
            // oldest 25% by their newest request timestamp. We pay an O(n)
            // scan only on the rare cap-hit path, not on every request.
            if requests.len() >= self.max_keys {
                let quarter = (self.max_keys / 4).max(1);
                let mut sizes: Vec<(Instant, String)> = requests
                    .iter()
                    .filter_map(|(k, v)| v.last().map(|t| (*t, k.clone())))
                    .collect();
                sizes.sort_unstable();
                for (_, k) in sizes.into_iter().take(quarter) {
                    requests.remove(&k);
                }
            }
            let entry = requests.entry(ip.to_string()).or_insert_with(Vec::new);
            entry.retain(|t| *t > now - self.window);
            if entry.len() >= self.max_requests {
                return false;
            }
            entry.push(now);
            true
        } else {
            // Fail CLOSED on a poisoned lock — the same
            // posture applied to the token/role
            // stores. A poisoned limiter mutex means a panic raced the hot
            // path; letting everything through would silently disable the
            // only request-bound this side of authN.
            false
        }
    }
}

pub fn spawn_connection_watchdog(tracker: std::sync::Arc<ConnectionTracker>) {
    use config::{CONNECTION_WATCHDOG_INTERVAL_SECS, CONNECTION_WATCHDOG_THRESHOLD_SECS};
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
            CONNECTION_WATCHDOG_INTERVAL_SECS,
        ));
        loop {
            interval.tick().await;
            let long_running = tracker.get_long_running(std::time::Duration::from_secs(
                CONNECTION_WATCHDOG_THRESHOLD_SECS,
            ));
            if !long_running.is_empty() {
                eprintln!(
                    "⚠️ WARNING: {} connection(s) held for >{}s:",
                    long_running.len(),
                    CONNECTION_WATCHDOG_THRESHOLD_SECS
                );
                for info in long_running {
                    eprintln!(
                        " - Connection {} at {}: {:?}",
                        info.id,
                        info.location,
                        info.acquired_at.elapsed()
                    );
                }
            }
        }
    });
}

/// RSS watchdog. Polls every `CONNECTION_WATCHDOG_INTERVAL_SECS`
/// (reuses the leak-detector cadence — both are "is something stuck" checks).
/// When process RSS exceeds the active envelope's `max_rss_mib` for two
/// consecutive samples, logs `error!`. If `BRAIN_RSS_RESTART=1` is set, exits
/// with code 1 so systemd `Restart=on-failure` recycles the process; default
/// is log-only — a tight restart loop is worse than a slow leak (plan risk note).
pub fn spawn_rss_watchdog() {
    use config::CONNECTION_WATCHDOG_INTERVAL_SECS;
    let restart_on_breach = std::env::var("BRAIN_RSS_RESTART")
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
            CONNECTION_WATCHDOG_INTERVAL_SECS,
        ));
        let mut prev_over = false;
        let envelope =
            crate::capacity::CapacityEnvelope::for_target(crate::capacity::capacity_target());
        loop {
            interval.tick().await;
            let rss = process_rss_mib();
            let over = rss > envelope.max_rss_mib;
            if over && prev_over {
                error!(
                    target: "brain::rss",
                    "RSS sustained at {rss} MiB across two samples (ceiling {} MiB)",
                    envelope.max_rss_mib
                );
                if restart_on_breach {
                    error!(target: "brain::rss", "BRAIN_RSS_RESTART=1 → exiting for supervisor restart");
                    std::process::exit(1);
                }
            }
            prev_over = over;
        }
    });
}

/// Resident memory (MB) of *this* process — not system-wide. Used by the
/// capacity envelope check so a 320 MB per-process ceiling is measured against
/// the process's actual footprint, not whatever else the host is running.
/// Returns 0 if the lookup fails (fail-open: don't block writes on a
/// measurement error).
pub(crate) fn process_rss_mib() -> u64 {
    let mut sys = System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        false,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    sys.process(pid)
        .map(|p| p.memory() / 1_000_000)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_rss_mib_reports_plausible_process_footprint() {
        // the /metrics gauge must reflect THIS
        // process's RSS, not system-wide used memory (which is ~50x larger on
        // a busy host and would silently mislead Prometheus consumers).
        let rss = process_rss_mib();
        // Fail-open is 0; a healthy process here is tens to a few hundred MB.
        assert!(rss > 0, "process_rss_mib returned 0 (lookup failed)");
        assert!(
            rss < 4096,
            "process_rss_mib {rss} MiB looks like host memory, not process RSS"
        );
    }

    #[test]
    fn test_connection_tracker_track() {
        let tracker = ConnectionTracker::new();
        let id1 = tracker.track("/test1");
        let id2 = tracker.track("/test2");

        assert_ne!(id1, id2);
        assert_eq!(tracker.count(), 2);
    }

    #[test]
    fn test_connection_tracker_release() {
        let tracker = ConnectionTracker::new();
        let id = tracker.track("/test");

        tracker.release(id);

        assert_eq!(tracker.count(), 0);
    }

    /// the rate limiter's HashMap is bounded so an attacker
    /// cycling spoofed `X-Forwarded-For` values can't grow memory unboundedly.
    /// At the cap the oldest 25% of buckets are evicted; the limiter keeps
    /// working (new IPs get tracked) instead of OOMing.
    #[test]
    fn rate_limiter_caps_tracked_ips_and_evicts_oldest() {
        let rl = RateLimiter::new();
        // Drive the cap to exactly max_keys by simulating distinct IPs.
        for i in 0..rl.max_keys {
            let ip = format!("10.0.{}.{}", i / 256, i % 256);
            let _ = rl.is_allowed(&ip);
        }
        let before = rl.requests.lock().map(|g| g.len()).unwrap_or(0);
        assert_eq!(before, rl.max_keys, "filled to the cap");
        // One more distinct IP triggers eviction (oldest 25% dropped).
        let _ = rl.is_allowed("192.168.1.1");
        let after = rl.requests.lock().map(|g| g.len()).unwrap_or(0);
        // After eviction + 1 insert the count is well under the cap.
        assert!(
            after < rl.max_keys,
            "eviction freed space: before={}, after={}",
            before,
            after
        );
        // The limiter still allows a fresh IP.
        assert!(rl.is_allowed("172.16.0.1"));
    }

    #[test]
    fn test_connection_tracker_long_running() {
        let tracker = ConnectionTracker::new();
        tracker.track("/test");

        let long_running = tracker.get_long_running(std::time::Duration::from_secs(0));
        assert_eq!(long_running.len(), 1);

        let none = tracker.get_long_running(std::time::Duration::from_secs(3600));
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn test_rate_limiter() {
        let limiter = RateLimiter::new();

        for _ in 0..10_000 {
            assert!(limiter.is_allowed("127.0.0.1"));
        }

        assert!(!limiter.is_allowed("127.0.0.1"));

        assert!(limiter.is_allowed("192.168.1.1"));
    }

    /// with `RATE_LIMIT_MAX_KEYS` buckets the
    /// tracked set is bounded — a flurry of distinct (spoofed) IPs evicts the
    /// oldest 25%, and one user's exhaustion never denies another.
    #[test]
    fn rate_limiter_evicts_oldest_quarter_and_stays_bounded() {
        let l = RateLimiter::new();
        let max = config::RATE_LIMIT_MAX_KEYS;
        for i in 0..(max + 1) {
            assert!(l.is_allowed(&format!("10.9.9.{i}")), "fresh bucket allowed");
        }
        let n = l.requests.lock().unwrap().len();
        assert!(n <= max, "tracked set stays bounded ({n} > {max})");

        let l2 = RateLimiter::new();
        for _ in 0..l2.max_requests {
            assert!(l2.is_allowed("10.1.1.1"));
        }
        assert!(!l2.is_allowed("10.1.1.1"), "same user exhausted → denied");
        assert!(l2.is_allowed("10.1.1.2"), "other user untouched");
    }

    // ── F-53: the connection-tracker slot is RAII — released on Drop, on
    // panic unwind, and when the ingest timeout drops the worker task ─────

    #[test]
    fn tracker_entry_releases_on_drop_and_panic() {
        let t: std::sync::Arc<ConnectionTracker> = std::sync::Arc::new(ConnectionTracker::new());
        assert_eq!(t.count(), 0);
        {
            let _e = TrackerEntry::new(t.clone(), "test-drop");
            assert_eq!(t.count(), 1, "entry holds a slot while alive");
        }
        assert_eq!(t.count(), 0, "Drop releases the slot");

        let tp = t.clone();
        let h = std::thread::spawn(move || {
            let _e = TrackerEntry::new(tp, "test-panic");
            panic!("boom");
        });
        let _ = h.join();
        assert_eq!(t.count(), 0, "panic unwind releases the slot");
    }

    #[tokio::test]
    async fn ingest_timeout_releases_tracker_slot() {
        let t: std::sync::Arc<ConnectionTracker> = std::sync::Arc::new(ConnectionTracker::new());
        let t2 = t.clone();
        let fut = tokio::task::spawn_blocking(move || {
            let _e = TrackerEntry::new(t2, "ingest-timeout");
            std::thread::sleep(std::time::Duration::from_millis(80));
            42u8
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), fut)
                .await
                .is_err(),
            "timed out while the worker is still in flight"
        );
        // spawn_blocking cannot be cancelled mid-flight — the task runs to
        // completion — but the slot must be released at ITS exit, not leaked
        // until a watchdog sweep (the pre-F-53 behavior).
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert_eq!(
            t.count(),
            0,
            "the timed-out worker's slot is released at exit, not leaked"
        );
    }
}
