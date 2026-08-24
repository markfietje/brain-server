//! verified webhook ingestion queue.
//!
//! A webhook delivery is verified (HMAC-SHA256) and idempotency-checked, then
//! appended to a bounded FIFO [`WebhookQueue`] instead of mutating the index
//! directly. A separate drain worker (`spawn_drain_worker`) processes the queue
//! in `id` order. This keeps the trust boundary honest: an unverified, replayed,
//! or forgery attempt never reaches indexing logic.
//!
//! ponytail: the queue is a single table; the worker deletes rows as it
//! processes them. A true timestamp-based replay window needs the sender's
//! timestamp header — GitHub's globally-unique `x-github-delivery` id is
//! sufficient for replay protection here (a replay reuses the same
//! `delivery_hash` and becomes a `Duplicate` no-op). `WEBHOOK_REPLAY_SECS`
//! documents the intended window if a timestamp header is ever added.
//!
//! Timestamp check: GitHub does NOT send a signed timestamp header, so a
//! caller-supplied timestamp is OPTIONAL. When `enqueue_ts` receives
//! `received_at: Some(t)`, a stamp older than `WEBHOOK_REPLAY_SECS` or more
//! than 300s in the future yields `Rejected` (untrusted). When `None` (the
//! GitHub case), only the `webhook_seen` replay window protects against
//! replays. The request's HTTP `Date` header, if parseable, is the best-effort
//! caller-supplied timestamp.

use crate::Pool;
use crate::audit::{self, AuditKind, AuditStatus};
use crate::config::{WEBHOOK_QUEUE_MAX, WEBHOOK_REPLAY_SECS};
use crate::handlers::HandlerError;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use rusqlite::OptionalExtension;
use rusqlite::params;
use sha2::Sha256;
use std::sync::Arc;
use std::time::SystemTime;
use xxhash_rust::xxh3::xxh3_64;

/// Max accepted clock-skew for a future-dated timestamp (seconds). Beyond this
/// a sender's timestamp is treated as a forgery.
pub(crate) const WEBHOOK_TS_FUTURE_SKEW_SECS: u64 = 300;

type HmacSha256 = Hmac<Sha256>;

/// Outcome of attempting to enqueue a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    /// New delivery accepted into the queue.
    Enqueued,
    /// delivery_hash already present — idempotency no-op (replay or duplicate).
    Duplicate,
    /// Queue is at `WEBHOOK_QUEUE_MAX`; caller should signal 503.
    Full,
    /// Timestamp check failed: too old (> WEBHOOK_REPLAY_SECS) or too far in
    /// the future (> 300s skew). Caller should treat as untrusted (401).
    Rejected,
}

/// Bounded FIFO of verified webhook deliveries, backed by the shared rusqlite
/// pool.
pub struct WebhookQueue {
    pool: Arc<Pool>,
}

impl WebhookQueue {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    /// Verify a GitHub-style `sha256=<hex>` signature against `body` using an
    /// HMAC-SHA256 keyed by `secret`. Constant-time via the `hmac` crate's
    /// `Mac::verify_slice`.
    pub fn verify_github_signature(secret: &[u8], body: &[u8], header_sig: &str) -> bool {
        let hex = match header_sig.strip_prefix("sha256=") {
            Some(h) => h,
            None => return false,
        };
        let expected = match hex::decode(hex) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
        mac.update(body);
        mac.verify_slice(&expected).is_ok()
    }

    /// verify a Standard Webhooks `v1,<base64>` signature
    /// — HMAC-SHA256 over `{webhook-id}.{webhook-timestamp}.{raw body}` keyed by
    /// `secret`, compared in constant time. The timestamp rides inside the HMAC
    /// payload, so a replay cannot re-stamp it. Header name + scheme match the
    /// open spec (standardwebhooks.com), so any svix-style signer interoperates.
    pub fn verify_standard_signature(
        secret: &[u8],
        id: &str,
        timestamp: &str,
        payload: &[u8],
        header_sig: &str,
    ) -> bool {
        let b64 = match header_sig.strip_prefix("v1,") {
            Some(b) => b,
            None => return false,
        };
        let expected = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac.verify_slice(&expected).is_ok()
    }

    /// outbound mirror of [`verify_standard_signature`] —
    /// produce the `v1,<base64>` HMAC-SHA256 over `{id}.{timestamp}.{raw body}`
    /// for the alert webhook sink. Interoperates with any svix-style receiver
    /// (the same scheme the server verifies inbound on `receive_standard`).
    pub fn sign_standard_signature(
        secret: &[u8],
        id: &str,
        timestamp: &str,
        payload: &[u8],
    ) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(payload);
        let sig = mac.finalize().into_bytes();
        format!(
            "v1,{}",
            base64::engine::general_purpose::STANDARD.encode(sig)
        )
    }

    /// Enqueue a delivery. `delivery_id` is the idempotency key (e.g. GitHub's
    /// `x-github-delivery`); `payload` is the raw body (for hashing only).
    /// Returns `Full` if the queue has reached `WEBHOOK_QUEUE_MAX` rows.
    pub fn enqueue(
        &self,
        kind: &str,
        event: &str,
        delivery_id: &str,
        payload: &[u8],
    ) -> Result<EnqueueOutcome, HandlerError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| HandlerError::internal(format!("webhook pool: {e}")))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM webhook_queue", [], |r| r.get(0))
            .map_err(|e| HandlerError::internal(format!("webhook count: {e}")))?;
        if count as usize >= WEBHOOK_QUEUE_MAX {
            return Ok(EnqueueOutcome::Full);
        }

        let delivery_hash = format!("{:016x}", xxh3_64(delivery_id.as_bytes()));
        let payload_hash = format!("{:016x}", xxh3_64(payload));

        // Replay window: prune seen entries older than WEBHOOK_REPLAY_SECS, then
        // reject a delivery whose hash is still present (a replay that arrived
        // after its queue row was drained). This is the real time-bounded
        // replay protection; the queue's UNIQUE(delivery_hash) catches in-window
        // duplicates that are still queued.
        conn.execute(
            "DELETE FROM webhook_seen WHERE seen_at < datetime('now', ?1)",
            params![format!("-{} seconds", WEBHOOK_REPLAY_SECS)],
        )
        .ok();
        let seen: bool = conn
            .query_row(
                "SELECT 1 FROM webhook_seen WHERE delivery_hash = ?1",
                params![delivery_hash],
                |_| Ok(true),
            )
            .optional()
            .map(|o| o.is_some())
            .unwrap_or(false);
        if seen {
            return Ok(EnqueueOutcome::Duplicate);
        }

        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO webhook_queue(kind, event, delivery_hash, payload_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                params![kind, event, delivery_hash, payload_hash],
            )
            .map_err(|e| HandlerError::internal(format!("webhook enqueue: {e}")))?;

        let outcome = if changed == 0 {
            EnqueueOutcome::Duplicate
        } else {
            // Record the delivery in the replay-window table so a post-drain
            // replay is still rejected until the window elapses.
            // was `let _ =` — a failed seen-write would
            // let a replay within the window double-deliver to the sink.
            conn.execute(
                "INSERT OR IGNORE INTO webhook_seen(delivery_hash) VALUES (?1)",
                params![delivery_hash],
            )
            .map_err(|e| HandlerError::internal(format!("webhook seen-write: {e}")))?;
            EnqueueOutcome::Enqueued
        };

        let detail = match outcome {
            EnqueueOutcome::Enqueued => "enqueued",
            EnqueueOutcome::Duplicate => "duplicate",
            EnqueueOutcome::Full => "full",
            EnqueueOutcome::Rejected => "rejected",
        };
        audit::record(
            &conn,
            AuditKind::Webhook,
            kind,
            delivery_id,
            AuditStatus::Ok,
            detail,
        );
        Ok(outcome)
    }

    /// Replay-window check WITHOUT queueing: prune expired entries, then
    /// atomically claim the delivery id (true = first sight, false = replay).
    /// The Beacon kb-feedback receiver uses this — its payload is consumed
    /// synchronously into a findings row, so there is nothing to enqueue.
    pub fn seen_claim(&self, delivery_id: &str) -> Result<bool, HandlerError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| HandlerError::internal(format!("webhook pool: {e}")))?;
        conn.execute(
            "DELETE FROM webhook_seen WHERE seen_at < datetime('now', ?1)",
            params![format!("-{} seconds", WEBHOOK_REPLAY_SECS)],
        )
        .ok();
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO webhook_seen(delivery_hash) VALUES (?1)",
                params![format!("{:016x}", xxh3_64(delivery_id.as_bytes()))],
            )
            .map_err(|e| HandlerError::internal(format!("webhook seen-write: {e}")))?;
        Ok(changed > 0)
    }

    /// Like [`enqueue`], but with an optional caller-supplied receipt time.
    /// `received_at: Some(t)` rejects the delivery (`Rejected`) if `t` is older
    /// than `WEBHOOK_REPLAY_SECS` from now (captured/stale) or more than
    /// `WEBHOOK_TS_FUTURE_SKEW_SECS` in the future (clock-skew/forgery).
    /// `received_at: None` skips the timestamp check (GitHub sends no signed
    /// timestamp); the `webhook_seen` replay window still applies.
    pub fn enqueue_ts(
        &self,
        kind: &str,
        event: &str,
        delivery_id: &str,
        payload: &[u8],
        received_at: Option<SystemTime>,
    ) -> Result<EnqueueOutcome, HandlerError> {
        if let Some(t) = received_at {
            let now = SystemTime::now();
            let age = now.duration_since(t).unwrap_or(std::time::Duration::ZERO);
            let ahead = t.duration_since(now).unwrap_or(std::time::Duration::ZERO);
            if age.as_secs() > WEBHOOK_REPLAY_SECS || ahead.as_secs() > WEBHOOK_TS_FUTURE_SKEW_SECS
            {
                return Ok(EnqueueOutcome::Rejected);
            }
        }
        self.enqueue(kind, event, delivery_id, payload)
    }
}

/// the one outbound HTTP client used by both webhook
/// sinks (alert + Art-19 DSAR). Redirects are refused (a 3xx is surfaced to
/// the caller, not fetched), so an operator URL that bounces to cloud metadata
/// or loopback cannot be followed. Defense-in-depth, not a replacement for
/// operator care.
///
/// ponytail: this does NOT resolve+validate the host's IPs against RFC1918 /
/// loopback / link-local / 169.254.x before the *first* request — that is the
/// v2.x per-request resolver upgrade. The redirect refusal closes the cheap,
/// high-probability SSRF class (302→metadata) today; DNS-rebinding across the
/// connection-pool TTL remains the documented ceiling. It also does NOT touch
/// body handling, request signing, retry policy, or any URL allowlist.
pub fn egress_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // A stalled sink must not wedge the drain worker or accumulate
        // ignored hangs. Connect is bounded (5 s) and the whole request
        // is capped at (15 s).
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("hardened egress client has no invalid defaults")
}

/// Pop and return the oldest queued delivery (by `id`), deleting it. Returns
/// `None` when the queue is empty. Used by the drain worker.
fn drain_one(conn: &rusqlite::Connection) -> rusqlite::Result<Option<(String, String, String)>> {
    let tx = conn.unchecked_transaction()?;
    let row: Option<(i64, String, String, String)> = tx
        .query_row(
            "SELECT id, kind, event, payload_hash FROM webhook_queue ORDER BY id ASC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((id, kind, event, payload_hash)) = row {
        tx.execute("DELETE FROM webhook_queue WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(Some((kind, event, payload_hash)))
    } else {
        tx.commit()?;
        Ok(None)
    }
}

/// Spawn the background drain worker: every 2 s it pops the oldest verified
/// delivery and records its drain as an audit row. The HTTP handler never
/// mutates the index directly — it verifies + enqueues; this worker is the
/// only queue consumer.
///
/// ponytail: `process_webhook_event` is not wired — the GitHub issue index is
/// kept fresh by the `brain-connector-gh` binary pulling; webhooks are a
/// freshness signal, and the security property (no direct index mutation from
/// an unverified/unbounded source) holds because the handler only enqueues.
pub fn spawn_drain_worker(pool: Pool) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => continue,
            };
            while let Ok(Some((kind, _event, payload_hash))) = drain_one(&conn) {
                // real ingestion. Today we only record that a verified,
                // idempotent delivery was drained; the event payload itself is
                // intentionally not consumed (the index is kept fresh by the
                // connector binaries pulling — the webhook is a freshness
                // signal, never a write source).
                audit::record(
                    &conn,
                    AuditKind::Webhook,
                    &kind,
                    &payload_hash,
                    AuditStatus::Ok,
                    "drained",
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pool;
    use r2d2_sqlite::SqliteConnectionManager;

    fn db() -> Arc<Pool> {
        let mgr = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(mgr).unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_events(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT DEFAULT CURRENT_TIMESTAMP,
                kind TEXT NOT NULL, actor TEXT, target_hash TEXT,
                status TEXT, detail_hash TEXT);
             CREATE TABLE webhook_queue(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL, event TEXT NOT NULL,
                delivery_hash TEXT NOT NULL UNIQUE, payload_hash TEXT NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE webhook_seen(
                delivery_hash TEXT PRIMARY KEY,
                seen_at TEXT DEFAULT CURRENT_TIMESTAMP);",
        )
        .unwrap();
        Arc::new(pool)
    }

    #[test]
    fn verify_github_signature_accepts_valid() {
        let secret = b"topsecret";
        let body = b"body";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(WebhookQueue::verify_github_signature(secret, body, &sig));
    }

    #[test]
    fn verify_github_signature_rejects_wrong() {
        let secret = b"topsecret";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(b"body");
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        // Tampered body.
        assert!(!WebhookQueue::verify_github_signature(
            secret, b"bodyx", &sig
        ));
    }

    #[test]
    fn verify_github_signature_rejects_bad_header_format() {
        assert!(!WebhookQueue::verify_github_signature(
            b"topsecret",
            b"body",
            "not-a-sig"
        ));
        assert!(!WebhookQueue::verify_github_signature(
            b"topsecret",
            b"body",
            "sha1=deadbeef"
        ));
    }

    fn std_signature(secret: &[u8], id: &str, ts: &str, payload: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(ts.as_bytes());
        mac.update(b".");
        mac.update(payload);
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            mac.finalize().into_bytes(),
        );
        format!("v1,{b64}")
    }

    #[test]
    fn standard_signature_covers_id_timestamp_payload() {
        // the spec's canonical `v1,` scheme signs
        // `{id}.{timestamp}.{raw body}` — a tamper to ANY of the three fails the
        // constant-time compare (so a replay cannot re-stamp the timestamp).
        let secret = b"topsecret";
        let id = "msg_123";
        let ts = "1700000000";
        let body = b"payload";
        let good = std_signature(secret, id, ts, body);
        assert!(WebhookQueue::verify_standard_signature(
            secret, id, ts, body, &good
        ));
        assert!(!WebhookQueue::verify_standard_signature(
            secret,
            id,
            ts,
            b"payloadx",
            &good
        ));
        assert!(!WebhookQueue::verify_standard_signature(
            secret,
            id,
            "1700000001",
            body,
            &good
        ));
        assert!(!WebhookQueue::verify_standard_signature(
            secret, "msg_999", ts, body, &good
        ));
    }

    #[test]
    fn standard_signature_rejects_bad_header_format() {
        assert!(!WebhookQueue::verify_standard_signature(
            b"topsecret",
            "msg_1",
            "1700000000",
            b"payload",
            "not-a-sig"
        ));
        // Legacy `sha256=` scheme must NOT pass the standard check.
        let mut mac = HmacSha256::new_from_slice(b"topsecret").unwrap();
        mac.update(b"payload");
        let hex_sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(!WebhookQueue::verify_standard_signature(
            b"topsecret",
            "msg_1",
            "1700000000",
            b"payload",
            &hex_sig
        ));
    }

    #[test]
    fn enqueue_then_duplicate_is_idempotent() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let first = q.enqueue("github", "push", "deliv-1", b"payload").unwrap();
        assert_eq!(first, EnqueueOutcome::Enqueued);
        let second = q.enqueue("github", "push", "deliv-1", b"payload").unwrap();
        assert_eq!(second, EnqueueOutcome::Duplicate);
    }

    #[test]
    fn alert_kind_roundtrips_through_verified_queue() {
        // the outbound `sign_standard_signature` round-trips
        // through the verify side, and a `kind='alert'` enqueue is idempotent
        // via `webhook_seen` (the delivery-id dedup).
        let secret = b"alertsecret";
        let id = "alert-3";
        let ts = "1700000000";
        let body = br#"{"kind":"pending","seq":3}"#;
        let sig = WebhookQueue::sign_standard_signature(secret, id, ts, body);
        assert!(WebhookQueue::verify_standard_signature(
            secret, id, ts, body, &sig
        ));

        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let first = q.enqueue("alert", "alert", id, body).unwrap();
        assert_eq!(first, EnqueueOutcome::Enqueued);
        let second = q.enqueue("alert", "alert", id, body).unwrap();
        assert_eq!(second, EnqueueOutcome::Duplicate);
    }

    #[test]
    fn enqueue_ts_rejects_future_timestamp() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let future = SystemTime::now() + std::time::Duration::from_secs(400);
        let out = q
            .enqueue_ts("github", "push", "deliv-future", b"payload", Some(future))
            .unwrap();
        assert_eq!(out, EnqueueOutcome::Rejected);
    }

    #[test]
    fn enqueue_ts_rejects_stale_timestamp() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let stale = SystemTime::now() - std::time::Duration::from_secs(400);
        let out = q
            .enqueue_ts("github", "push", "deliv-stale", b"payload", Some(stale))
            .unwrap();
        assert_eq!(out, EnqueueOutcome::Rejected);
    }

    #[test]
    fn enqueue_ts_accepts_recent_timestamp() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let recent = SystemTime::now() - std::time::Duration::from_secs(10);
        let out = q
            .enqueue_ts("github", "push", "deliv-recent", b"payload", Some(recent))
            .unwrap();
        assert_eq!(out, EnqueueOutcome::Enqueued);
    }

    #[test]
    fn enqueue_ts_none_accepted() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        let out = q
            .enqueue_ts("github", "push", "deliv-none", b"payload", None)
            .unwrap();
        assert_eq!(out, EnqueueOutcome::Enqueued);
    }

    #[test]
    fn enqueue_is_full_when_capped() {
        let pool = db();
        let q = WebhookQueue::new(pool.clone());
        for i in 0..WEBHOOK_QUEUE_MAX {
            let out = q
                .enqueue("github", "push", &format!("deliv-{i}"), b"payload")
                .unwrap();
            assert_eq!(out, EnqueueOutcome::Enqueued);
        }
        let full = q
            .enqueue("github", "push", "deliv-overflow", b"payload")
            .unwrap();
        assert_eq!(full, EnqueueOutcome::Full);
    }

    // ── SSRF egress tests ──────────────────────────────
    //
    // Reuses the raw-HTTP `TcpListener` responder idiom from
    // `main.rs::test_observe_art19_webhook_posts_on_purge` — no new dev-dep.

    /// the hardened egress client MUST NOT follow a 302 redirect.
    /// The responder returns `302 → http://{same-listener}/followed`; if the
    /// client followed it, a second connection would land on the same listener
    /// and bump the accept counter. We assert the returned status is the 302
    /// itself AND that exactly one connection was accepted (no follow).
    #[test]
    fn egress_client_refuses_redirect_to_loopback() {
        use std::io::{Read, Write};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/sink");
        let connects = Arc::new(AtomicU32::new(0));
        let connects_cloned = Arc::clone(&connects);
        // Non-blocking + deadline so the responder self-terminates whether or
        // not a buggy follow arrives (no hang on the second accept).
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
            while std::time::Instant::now() < deadline {
                let (mut sock, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                // Read the request as blocking (accepted sockets inherit
                // non-blocking from the listener).
                let _ = sock.set_nonblocking(false);
                connects_cloned.fetch_add(1, Ordering::SeqCst);
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let n = sock.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                // First connection → 302 to the same listener; any stray
                // second connection (a buggy follow) → 200.
                if connects_cloned.load(Ordering::SeqCst) == 1 {
                    let loc = format!("http://{addr}/followed");
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n"
                    );
                    let _ = sock.write_all(resp.as_bytes());
                } else {
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                }
            }
        });

        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(async {
            let client = egress_client();
            let resp = client
                .post(&url)
                .body(b"x".to_vec())
                .send()
                .await
                .expect("send");
            resp.status()
        });

        // Wait for the responder's deadline so its accept count is final.
        thread.join().expect("responder");

        assert_eq!(
            status.as_u16(),
            302,
            "Policy::none surfaces the redirect instead of fetching it"
        );
        assert_eq!(
            connects.load(Ordering::SeqCst),
            1,
            "client must NOT issue a second request to the redirect target"
        );
    }

    /// happy path — the hardened client still delivers a legitimate
    /// 200 response (regression: redirect refusal did not break normal egress).
    #[test]
    fn egress_client_sends_to_allowed_host() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/sink");
        let (sent_tx, sent_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            let _ = sent_tx.send(buf);
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (status, body) = rt.block_on(async {
            let client = egress_client();
            let resp = client.post(&url).body("hello").send().await.expect("send");
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            (status, body)
        });
        let _ = sent_rx.recv_timeout(std::time::Duration::from_secs(2));
        let _ = thread.join();
        assert!(status.is_success(), "legitimate URL is delivered: {status}");
        assert_eq!(body, "ok", "response body is read intact");
    }
}
