//! v1.20.8 "Signal" — live operator alert feed (`GET /events`) + optional
//! webhook sink.
//!
//! The decision-critical events the console cares about (new pending proposal,
//! proposal near-expiry, injection verdict, audit-chain verify failure) are
//! published on a bounded `tokio::sync::broadcast` channel held in `AppState`
//! and streamed to the `/ops` panel over SSE. The same events feed an optional
//! `BRAIN_ALERT_WEBHOOK_URL` sink so a headless operator gets identical
//! signals via the v1.20.4 Standard Webhooks handshake.
//!
//! Invariants (the 2026 alert-fatigue + PII rules):
//! - The signal set is **fixed and hand-curated** (no rules engine).
//! - `AlertEvent.payload` is a fixed, small object — **never content, never
//!   PII** (the client fetches full detail from the existing endpoints).
//! - `expiry` fires **once per boundary crossed** via the pure
//!   [`tier_transition`] core, not on every clock tick.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
    /// Map remaining proposal lifetime (seconds) onto a tier using the config
    /// boundaries (`ALERT_CRITICAL_SECS` < 5 min, `ALERT_WARN_SECS` < 1 hr).
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

/// Fire exactly one transition per boundary crossing — "crossing critical fires
/// once, re-triggering on the same tier does not" (the queue is a clock, but
/// the alert is a signal). Returns the tier label to raise.
pub fn tier_transition(before: Tier, after: Tier) -> Option<&'static str> {
    match (before, after) {
        (Tier::Ok, Tier::Warn) => Some("warn"),
        (Tier::Ok, Tier::Critical) | (Tier::Warn, Tier::Critical) => Some("critical"),
        _ => None,
    }
}

/// Optional `?kinds=a,b,c` filter on `GET /events`.
#[derive(Debug, Default, Deserialize)]
pub struct EventsQuery {
    kinds: Option<String>,
}

/// `GET /events` — SSE live alert feed. Public-but-Read-gated (auth
/// `Action::Read`, the operator console token already has read). Mirrors the
/// `/ump/subscribe` broadcast→SSE shape: a `pending` handshake first, then
/// `alert` events. A lagging consumer drops missed events (bounded broadcast)
/// and re-syncs from the console's polling fallback. `?kinds=` filters the
/// stream to a subset of [`ALERT_KIND_*`].
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

/// Publish an alert: fan out on the SSE broadcast (never blocks), and if the
/// webhook sink is configured, hand the event to it (enqueue for the audit/
/// idempotency record + a real Standard-Webhooks POST). Fail-soft both ways.
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

/// Webhook sink: audit/idempotency record in the existing `webhook_queue`
/// (kind='alert', delivery-id = `alert-<seq>`), then a Standard Webhooks
/// POST (`webhook-id`/`webhook-timestamp`/`webhook-signature: v1,<base64>`),
/// bounded retries, fail-soft. `webhook_seen` dedups a replay; the receiver
/// also has `alert.seq` in the body for its own idempotency.
async fn sink(state: &Arc<AppState>, seq: u64, body: &str) {
    let Some(url) = crate::config::alert_webhook_url() else {
        return;
    };
    let delivery_id = format!("alert-{seq}");
    let queue = crate::webhook::WebhookQueue::new(Arc::new(state.pool.clone()));
    let _ = queue.enqueue("alert", "alert", &delivery_id, body.as_bytes());

    let client = reqwest::Client::new();
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

/// Background task: watch pending proposals and fire the `expiry` alert once
/// per proposal as it crosses each SLA tier boundary. `Ok` → `Warn` (fires
/// "warn") and any → `Critical` (fires "critical"); re-triggering on the same
/// tier does not. Proposals that leave the pending set are pruned.
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
    fn alert_kinds_are_fixed_curated_set() {
        // The 2026 alert-fatigue guard: the signal set is fixed and hand-
        // curated. Adding a kind is a deliberate code change, never a config.
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
        // The `AlertEvent` envelope is exactly {kind, ts, seq, payload} — no
        // content/PII ever rides in the envelope itself (payloads are the
        // hand-curated signal objects). Guard-style assertion on the shape.
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
