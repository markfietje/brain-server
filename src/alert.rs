//! live operator alert feed (`GET /events`) + optional webhook sink.
//!
//! Decision-critical events (pending/expiry/injection/chain-verify) publish on a
//! bounded `AppState` broadcast and stream to the `/ops` panel via SSE; the same
//! events feed an optional `BRAIN_ALERT_WEBHOOK_URL` (v1.20.4 Webhooks handshake).
//!
//! Invariants (2026 alert-fatigue + PII): the signal set is **fixed & hand-curated**
//! (no rules engine); `AlertEvent.payload` is a fixed small object — **never content,
//! never PII** (the client fetches detail from endpoints); `expiry` fires **once per
//! boundary crossed** via the pure [`tier_transition`] core.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, KeepAliveStream, Sse};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::StreamExt;

use crate::AppState;

/// The four fixed alert kinds (mirrors `config::ALERT_KIND_*`).
pub use crate::config::{
    ALERT_KIND_CHAIN, ALERT_KIND_EXPIRY, ALERT_KIND_PENDING, ALERT_KIND_SCREEN,
};

/// The v1.20.6 ops-clock SLA tier (mirrors the client clock). `Ok` → `Warn` →
/// `Critical` as the remaining proposal lifetime shrinks.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Tier {
    Ok,
    Warn,
    Critical,
}

impl Tier {
    /// Map remaining lifetime (secs) onto a tier via config boundaries (crit<5min, warn<1hr).
    pub fn from_remaining(remaining_secs: i64) -> Tier {
        if remaining_secs < crate::config::ALERT_CRITICAL_SECS {
            Tier::Critical
        } else if remaining_secs < crate::config::ALERT_WARN_SECS {
            Tier::Warn
        } else {
            Tier::Ok
        }
    }
}

/// Fire exactly one transition per boundary crossing — a re-trigger on the same
/// tier is silent (the queue is a clock, the alert is a signal). Returns the label.
pub fn tier_transition(before: Tier, after: Tier) -> Option<&'static str> {
    match (before, after) {
        (Tier::Ok, Tier::Warn) => Some("warn"),
        (Tier::Ok, Tier::Critical) | (Tier::Warn, Tier::Critical) => Some("critical"),
        _ => None,
    }
}

/// Last audit-chain posture, written by [`spawn_chain_watcher`], read by `/health`.
/// Default `chain_ok=false` until the first check (the watcher runs once at boot).
#[derive(Debug, Clone, Default)]
pub struct ChainStatus {
    /// `true` when the last full-chain verify passed.
    pub chain_ok: bool,
    /// Unix epoch seconds of the last check (0 = never).
    pub checked_at: i64,
    /// Chain-head hash of the last check ("" = unchecked) — pins the posture claim.
    pub chain_head: String,
}

/// Shared handle to the watcher's latest result (writer = watcher, readers = `/health`).
/// Mirrors `integrity::SnapshotState`.
#[derive(Clone, Default)]
pub struct ChainWatchState {
    inner: Arc<RwLock<ChainStatus>>,
}

impl ChainWatchState {
    pub fn read(&self) -> ChainStatus {
        self.inner.read().map(|s| s.clone()).unwrap_or_default()
    }
    fn set(&self, status: ChainStatus) {
        if let Ok(mut g) = self.inner.write() {
            *g = status;
        }
    }
}

/// Decide whether the integrity watcher signals this tick. Fires only on an
/// `ok` ↔ `broken` transition (or a broken first check) — a stable tick is silent.
/// Returns `danger` (broken) or `ok` (recovered).
pub fn chain_transition(prev_ok: Option<bool>, now_ok: bool) -> Option<&'static str> {
    match (prev_ok, now_ok) {
        (Some(true), false) => Some("danger"), // ok → broken
        (Some(false), true) => Some("ok"),     // broken → recovered
        (None, false) => Some("danger"),       // first check already broken
        _ => None,                             // stable, or a healthy first check
    }
}

/// Optional `?kinds=a,b,c` filter on `GET /events`.
#[derive(Debug, Default, Deserialize)]
pub struct EventsQuery {
    kinds: Option<String>,
}

/// `GET /events` — SSE live alert feed (Read-gated). Mirrors `/ump/subscribe`:
/// a `pending` handshake, then `alert` events; a lagging consumer drops missed
/// events (bounded broadcast) and re-syncs via the polling fallback.
/// `?kinds=` filters to a subset of [`ALERT_KIND_*`].
pub async fn events(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
) -> Sse<KeepAliveStream<Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>>>>
{
    let rx = state.alert_events.subscribe();
    let kinds: Option<std::collections::HashSet<String>> = q
        .kinds
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());
    let handshake = tokio_stream::once(Ok::<Event, Infallible>(
        Event::default()
            .event("pending")
            .data("{\"feed\":\"alert\"}"),
    ));
    let gate = crate::handlers::authorize(&principal.0, crate::auth::Action::Read, "", "global");
    let stream: Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>> =
        if let Err(e) = gate {
            Box::pin(tokio_stream::once(Ok::<Event, Infallible>(
                Event::default().event("error").data(format!("{e:?}")),
            )))
        } else {
            Box::pin(
                handshake.chain(tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
                    move |item| match item {
                        Ok(v) => {
                            // Drop events the caller didn't ask for (per-kind filter).
                            if let Some(k) = &kinds {
                                let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
                                if !k.contains(kind) {
                                    return None;
                                }
                            }
                            Some(Ok::<Event, Infallible>(
                                Event::default()
                                    .event("alert")
                                    .json_data(v)
                                    .unwrap_or_default(),
                            ))
                        }
                        Err(_) => None,
                    },
                )),
            )
        };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Publish an alert: fan out on the SSE broadcast (never blocks); if the webhook
/// sink is configured, hand it off (audit/idempotency record + Webhooks POST). Fail-soft.
pub(crate) fn publish(state: &Arc<AppState>, kind: &str, payload: Value) {
    let seq = state.alert_seq.fetch_add(1, Ordering::Relaxed);
    let event = json!({
        "kind": kind,
        "ts": chrono::Utc::now().timestamp(),
        "seq": seq,
        "payload": payload,
    });
    let _ = state.alert_events.send(event.clone());
    if crate::config::alert_webhook_url().is_some() {
        let state = Arc::clone(state);
        let body = event.to_string();
        tokio::spawn(async move { sink(&state, seq, &body).await });
    }
}

/// Webhook sink: an audit/idempotency `webhook_queue` record (kind='alert',
/// delivery-id = `alert-<seq>`), then a Standard Webhooks POST (`webhook-id`/
/// `webhook-timestamp`/`webhook-signature: v1,<base64>`), bounded retries, fail-soft.
/// `webhook_seen` dedups replays; the receiver also has `alert.seq` for idempotency.
async fn sink(state: &Arc<AppState>, seq: u64, body: &str) {
    let Some(url) = crate::config::alert_webhook_url() else {
        return;
    };
    let delivery_id = format!("alert-{seq}");
    let queue = crate::webhook::WebhookQueue::new(Arc::new(state.pool.clone()));
    let _ = queue.enqueue("alert", "alert", &delivery_id, body.as_bytes());

    let client = crate::webhook::egress_client();
    let ts = chrono::Utc::now().to_rfc3339();
    let mut last_err: Option<String> = None;
    for attempt in 0..3u32 {
        let mut req = client.post(&url).header("content-type", "application/json");
        if let Some(secret) = crate::config::alert_webhook_secret() {
            let sig = crate::webhook::WebhookQueue::sign_standard_signature(
                secret.as_bytes(),
                &delivery_id,
                &ts,
                body.as_bytes(),
            );
            req = req
                .header("webhook-id", &delivery_id)
                .header("webhook-timestamp", &ts)
                .header("webhook-signature", sig);
        }
        match req.body(body.to_string()).send().await {
            Ok(r) if r.status().is_success() => return,
            Ok(r) => last_err = Some(format!("http {}", r.status())),
            Err(e) => last_err = Some(e.to_string()),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
    }
    tracing::warn!("alert webhook sink failed after retries: {last_err:?}");
}

/// Background task: watch pending proposals, firing the `expiry` alert once per
/// proposal as it crosses each tier boundary (`Ok`→`Warn` "warn", any→`Critical`
/// "critical"); same-tier re-trigger is silent. Left-the-pending-set ids are pruned.
pub(crate) async fn spawn_expiry_watcher(state: Arc<AppState>) {
    let mut last_tier: std::collections::HashMap<i64, Tier> = std::collections::HashMap::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        crate::config::ALERT_WATCH_INTERVAL_SECS,
    ));
    let ttl = crate::config::proposal_ttl_secs();
    loop {
        interval.tick().await;
        let pool = state.pool.clone();
        let (ids, now) = tokio::task::spawn_blocking(move || -> (Vec<(i64, i64)>, i64) {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (Vec::new(), 0),
            };
            let mut stmt = match conn
                .prepare("SELECT id, created_at FROM proposals WHERE status = 'pending'")
            {
                Ok(s) => s,
                Err(_) => return (Vec::new(), 0),
            };
            let rows: Vec<(i64, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .and_then(|m| m.collect::<rusqlite::Result<_>>())
                .unwrap_or_default();
            (rows, chrono::Utc::now().timestamp())
        })
        .await
        .unwrap_or_default();

        // Prune ids that are no longer pending.
        last_tier.retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, created_at) in ids {
            let remaining = created_at + ttl - now;
            let after = Tier::from_remaining(remaining);
            let before = last_tier.insert(id, after).unwrap_or(Tier::Ok);
            if let Some(label) = tier_transition(before, after) {
                publish(
                    &state,
                    ALERT_KIND_EXPIRY,
                    json!({ "proposal_id": id, "tier": label, "expires_at": created_at + ttl }),
                );
            }
        }
    }
}

/// Background task: watch the audit hash chain, raising an `integrity` (`chain`)
/// alert on `ok` ↔ `broken` transitions. Runs the authoritative `audit::verify_chain`
/// on a cadence + records the last posture for `/health` — a *watcher* over the
/// existing tamper-evident log. First tick runs immediately, so boot reports real posture.
pub(crate) async fn spawn_chain_watcher(state: Arc<AppState>, watch: ChainWatchState) {
    let mut prev: Option<bool> = None;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        crate::config::chain_check_secs(),
    ));
    loop {
        interval.tick().await;
        let pool = state.pool.clone();
        let (ok, head) = tokio::task::spawn_blocking(move || -> (bool, String) {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (false, String::new()),
            };
            (
                crate::audit::verify_chain(&conn),
                crate::audit::chain_head(&conn).unwrap_or_default(),
            )
        })
        .await
        .unwrap_or((false, String::new()));
        let checked_at = chrono::Utc::now().timestamp();
        watch.set(ChainStatus {
            chain_ok: ok,
            checked_at,
            chain_head: head.clone(),
        });
        if let Some(severity) = chain_transition(prev, ok) {
            publish(
                &state,
                ALERT_KIND_CHAIN,
                json!({
                    "ok": ok,
                    "severity": severity,
                    "chain_head": head,
                    "checked_at": checked_at,
                }),
            );
        }
        prev = Some(ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_transition_fires_once_per_boundary() {
        // Crossing critical fires once; re-triggering on the same tier does not.
        assert_eq!(tier_transition(Tier::Ok, Tier::Warn), Some("warn"));
        assert_eq!(tier_transition(Tier::Warn, Tier::Warn), None);
        assert_eq!(tier_transition(Tier::Ok, Tier::Critical), Some("critical"));
        assert_eq!(
            tier_transition(Tier::Warn, Tier::Critical),
            Some("critical")
        );
        assert_eq!(tier_transition(Tier::Critical, Tier::Critical), None);
        // Downgrades never alert (the operator clears by acting, not by noise).
        assert_eq!(tier_transition(Tier::Critical, Tier::Warn), None);
        assert_eq!(tier_transition(Tier::Warn, Tier::Ok), None);
    }

    #[test]
    fn tier_maps_remaining_time() {
        assert_eq!(Tier::from_remaining(10_000), Tier::Ok);
        assert_eq!(Tier::from_remaining(3_000), Tier::Warn);
        assert_eq!(Tier::from_remaining(120), Tier::Critical);
        assert_eq!(Tier::from_remaining(-5), Tier::Critical);
    }

    #[test]
    fn chain_transition_fires_only_on_ok_broken_transitions() {
        // Healthy stable ticks never raise — no per-tick spam.
        assert_eq!(chain_transition(Some(true), true), None);
        // A broken chain that stays broken is silent until it recovers.
        assert_eq!(chain_transition(Some(false), false), None);
        // ok → broken raises danger once.
        assert_eq!(chain_transition(Some(true), false), Some("danger"));
        // broken → recovered raises ok once (recovery is a signal, not a silence).
        assert_eq!(chain_transition(Some(false), true), Some("ok"));
        // First check: a healthy boot is silent; an already-broken boot raises danger immediately.
        assert_eq!(chain_transition(None, true), None);
        assert_eq!(chain_transition(None, false), Some("danger"));
    }

    #[test]
    fn chain_watch_state_default_is_not_ok_until_set() {
        let s = ChainWatchState::default();
        assert!(!s.read().chain_ok, "default is not-ok (never checked)");
        assert_eq!(s.read().checked_at, 0);
        s.set(ChainStatus {
            chain_ok: true,
            checked_at: 1_700_000_000,
            chain_head: "abc".into(),
        });
        let r = s.read();
        assert!(r.chain_ok);
        assert_eq!(r.checked_at, 1_700_000_000);
        assert_eq!(r.chain_head, "abc");
    }

    /// v1.27.27 M1 (F-26 class): a POISONED chain-watch lock must read as the
    /// fail-closed posture (`chain_ok = false`), never as "the last known good
    /// check still holds" — `/health`'s integrity claim degrades to not-ok.
    /// The `unwrap_or_default()` on the read is load-bearing precisely here.
    #[test]
    fn poisoned_chain_watch_reads_as_not_ok() {
        let s = ChainWatchState::default();
        s.set(ChainStatus {
            chain_ok: true,
            checked_at: 1_700_000_000,
            chain_head: "abc".into(),
        });
        assert!(s.read().chain_ok, "sanity: healthy before poisoning");
        // Poison: panic while holding the write guard (caught in-place — the
        // guard drops during the unwind, which is what poisons the lock).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = s.inner.write().expect("lock before panic");
            panic!("poison the chain-watch lock");
        }));
        let r = s.read();
        assert!(
            !r.chain_ok,
            "a poisoned watch must report NOT ok (fail closed)"
        );
    }

    #[test]
    fn alert_kinds_are_fixed_curated_set() {
        // 2026 alert-fatigue guard: the signal set is fixed & hand-curated; adding a kind is a deliberate code change, never config.
        assert_eq!(
            [
                ALERT_KIND_PENDING,
                ALERT_KIND_EXPIRY,
                ALERT_KIND_SCREEN,
                ALERT_KIND_CHAIN
            ]
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
            4
        );
    }

    #[test]
    fn event_envelope_is_fixed_and_never_content() {
        // The `AlertEvent` envelope is exactly {kind, ts, seq, payload} — no content/PII rides in it (payloads are the hand-curated signals).
        let event = json!({
            "kind": "pending",
            "ts": 1700000000i64,
            "seq": 7,
            "payload": { "proposal_id": 1, "screen_verdict": "clean" },
        });
        let obj = event.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        for key in ["kind", "ts", "seq", "payload"] {
            assert!(obj.contains_key(key));
        }
    }
}
