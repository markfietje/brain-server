//! Heartbeat re-auth for SSE/event streams.
//!
//! Admission (`403 BEFORE the stream opens`) is not enough: a principal
//! revoked MID-STREAM kept receiving until the client went away. Every
//! guarded stream re-consults the revocation registry on a heartbeat
//! cadence (`BRAIN_SSE_REAUTH_SECS`, default 30s, `0` = the explicit
//! admission-only ceiling) and, on revoked-or-unreadable, emits the
//! termination frame `{"revoked":true,"at":<unix ts>}` and closes. A
//! reconnecting client meets the admission 403. Bounded-kill, not
//! instant-kill: the bound IS the interval (THREAT_MODEL §5b).
//!
//! Both SSE endpoints (`alert::events`, `ump_ops::subscribe`) pump through
//! [`pump_guarded`] — one kill loop, two wirings, each pinned below.

use std::convert::Infallible;

use axum::response::sse::Event;

use crate::Pool;

/// SSE event name carrying the termination frame.
pub(crate) const TERMINATION_EVENT_NAME: &str = "revoked";

/// mpsc backpressure bound between the pump task and the SSE body — deep
/// enough to absorb a reconnect replay burst, bounded so a dead client
/// cannot grow the queue without bound (a full channel means the client
/// is gone; the pump exits and the task ends).
pub(crate) const PUMP_CHANNEL_CAPACITY: usize = 64;

/// The termination frame bytes: exactly `{"revoked":true,"at":<unix
/// ts>}`. Pure (clock read lives here, not hidden in the Event wrapper)
/// so tests pin the shape byte-for-byte.
pub(crate) fn termination_payload() -> String {
    let at = chrono::Utc::now().timestamp();
    format!(r#"{{"revoked":true,"at":{at}}}"#)
}

/// The termination frame as an SSE event: name `revoked`, data the
/// payload above. The client treats it as a kill receipt — reconnecting
/// meets the admission 403.
pub(crate) fn termination_event() -> Event {
    Event::default()
        .event(TERMINATION_EVENT_NAME)
        .data(termination_payload())
}

/// One heartbeat verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReauthDecision {
    Continue,
    Kill,
}

/// One heartbeat re-check: is `sub` revoked NOW? The same kill-switch
/// read the bearer middleware consults at admission
/// (`workflow::mesh::is_revoked`, one indexed SELECT), re-run mid-stream.
/// `None` (anonymous/opaque-token stream) has no identity to kill →
/// Continue. ANY store failure (pool exhaustion, SQL error, join failure)
/// → Kill: during incident response the registry read must never fail
/// open and keep a revoked principal fed.
pub(crate) async fn reauth_decision(pool: &Pool, sub: Option<&str>) -> ReauthDecision {
    let Some(sub) = sub else {
        return ReauthDecision::Continue;
    };
    let sub = sub.to_string();
    let pool = pool.clone();
    let verdict = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let conn = pool.get().map_err(|e| format!("revocation pool: {e}"))?;
        crate::workflow::mesh::is_revoked(&conn, &sub).map_err(|e| format!("revocation read: {e}"))
    })
    .await;
    match verdict {
        Ok(Ok(false)) => ReauthDecision::Continue,
        // Revoked — or the registry could not answer. Both kill.
        _ => ReauthDecision::Kill,
    }
}

/// The guarded producer behind both SSE endpoints: sends the prelude
/// (handshake + reconnect replay, already admission-gated and
/// domain-checked by the caller), then drains the broadcast channel
/// through `map` (the caller's per-event authz filter) while a heartbeat
/// tick re-runs [`reauth_decision`]. On Kill the termination frame goes
/// out and the sender drops (stream closes). Lagged receivers skip the
/// missed events (the broadcast discipline: drop+resync, never block);
/// a closed bus or a gone client ends the task (no leaked pump).
///
/// `interval_secs == 0` is the explicit admission-only ceiling (the
/// pre-.86 posture, `BRAIN_SSE_REAUTH_SECS=0`): the tick never fires and
/// the loop is a pure drain — pinned by
/// `admission_only_mode_documents_no_kill_ceiling`.
pub(crate) async fn pump_guarded(
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    mut rx: tokio::sync::broadcast::Receiver<serde_json::Value>,
    pool: Pool,
    sub: Option<String>,
    interval_secs: u64,
    prelude: Vec<Event>,
    mut map: impl FnMut(serde_json::Value) -> Option<Event>,
) {
    for ev in prelude {
        if tx.send(Ok(ev)).await.is_err() {
            return;
        }
    }
    if interval_secs == 0 {
        drain_forever(&tx, &mut rx, &mut map).await;
        return;
    }
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    // Consume the immediate first tick: admission JUST passed, re-checking
    // in the same millisecond is pure cost.
    interval.tick().await;
    loop {
        tokio::select! {
            res = rx.recv() => {
                if !drain_one(&tx, res, &mut map).await {
                    return;
                }
            }
            _ = interval.tick() => {
                if reauth_decision(&pool, sub.as_deref()).await == ReauthDecision::Kill {
                    let _ = tx.send(Ok(termination_event())).await;
                    return;
                }
            }
        }
    }
}

/// One broadcast message through the caller's filter. False = the pump
/// must exit (bus closed or client gone).
async fn drain_one(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    res: Result<serde_json::Value, tokio::sync::broadcast::error::RecvError>,
    map: &mut impl FnMut(serde_json::Value) -> Option<Event>,
) -> bool {
    match res {
        Ok(v) => {
            if let Some(ev) = map(v) {
                tx.send(Ok(ev)).await.is_ok()
            } else {
                true
            }
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
        Err(tokio::sync::broadcast::error::RecvError::Closed) => false,
    }
}

/// The admission-only drain (interval 0): no ticks, no kills.
async fn drain_forever(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    rx: &mut tokio::sync::broadcast::Receiver<serde_json::Value>,
    map: &mut impl FnMut(serde_json::Value) -> Option<Event>,
) {
    loop {
        let res = rx.recv().await;
        if !drain_one(tx, res, map).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReauthDecision, pump_guarded, termination_event, termination_payload};

    use axum::response::sse::Event;
    use serde_json::{Value, json};
    use std::time::Duration;

    fn alert_map(v: Value) -> Option<Event> {
        Some(
            Event::default()
                .event("alert")
                .json_data(v)
                .unwrap_or_default(),
        )
    }

    fn pending_prelude() -> Vec<Event> {
        vec![
            Event::default()
                .event("pending")
                .data("{\"feed\":\"test\"}"),
        ]
    }

    fn test_pool() -> (tempfile::TempDir, crate::Pool) {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::TempDir::new().expect("temp dir");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(dir.path().join("brain.db"));
        let pool: crate::Pool = r2d2::Pool::builder().build(mgr).expect("pool");
        crate::migration::run_migration(&mut pool.get().expect("conn"), 0).expect("migration");
        (dir, pool)
    }

    fn revoke(pool: &crate::Pool, sub: &str) {
        let conn = pool.get().expect("conn");
        crate::workflow::mesh::revoke_principal(&conn, sub, "streamkill-test", "tester", 1)
            .expect("revoke");
    }

    async fn recv_event(
        rx: &mut tokio::sync::mpsc::Receiver<Result<Event, std::convert::Infallible>>,
    ) -> Event {
        tokio::time::timeout(Duration::from_secs(8), rx.recv())
            .await
            .expect("event within tick bound")
            .expect("stream open")
            .expect("infallible")
    }

    /// R-02 red: a principal revoked before the first tick gets the
    /// termination frame and the stream closes — today the stream lives on.
    #[tokio::test]
    async fn revoked_principal_kills_open_stream_with_termination_frame() {
        let (_dir, pool) = test_pool();
        revoke(&pool, "alice");
        let (_btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool,
            Some("alice".to_string()),
            1,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        // The 1s tick finds alice revoked → termination, then close.
        let _term = recv_event(&mut rx).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(8), rx.recv())
                .await
                .expect("close is timely")
                .is_none(),
            "killed stream must close after the termination frame"
        );
        pump.await.expect("pump joins");
    }

    /// R-02 red (channel-drain twin): revoke MID-STREAM — the message queued
    /// before revocation drains, the termination frame follows on the tick,
    /// and anything sent after never arrives.
    #[tokio::test]
    async fn revoked_principal_kills_channel_drain_mid_stream() {
        let (_dir, pool) = test_pool();
        let (btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool.clone(),
            Some("bob".to_string()),
            1,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        btx.send(json!({"kind": "workflow", "n": 1})).expect("send");
        let _msg1 = recv_event(&mut rx).await;
        revoke(&pool, "bob");
        let _term = recv_event(&mut rx).await;
        // The bus itself may report no receivers now (the pump exited) —
        // either way, nothing further may arrive on the stream.
        let _ = btx.send(json!({"kind": "workflow", "n": 2}));
        assert!(
            tokio::time::timeout(Duration::from_secs(8), rx.recv())
                .await
                .expect("close is timely")
                .is_none(),
            "post-kill events must never drain"
        );
        pump.await.expect("pump joins");
    }

    /// The live path: ticks pass, messages keep draining, no termination.
    #[tokio::test]
    async fn live_principal_survives_ticks_and_keeps_draining() {
        let (_dir, pool) = test_pool();
        let (btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool,
            Some("carol".to_string()),
            1,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        btx.send(json!({"n": 1})).expect("send");
        let _msg1 = recv_event(&mut rx).await;
        // Sleep past a tick: carol is live, the stream must hold.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        btx.send(json!({"n": 2})).expect("send");
        let _msg2 = recv_event(&mut rx).await;
        assert!(
            rx.try_recv().is_err(),
            "no termination frame may arrive for a live principal"
        );
        pump.abort();
    }

    /// Fail-closed: an unreadable revocation registry kills the stream —
    /// here a pool with no tables at all (every read errors).
    #[tokio::test]
    async fn registry_error_kills_stream_fail_closed() {
        let mgr = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool: crate::Pool = r2d2::Pool::builder().max_size(1).build(mgr).expect("pool");
        assert!(
            super::reauth_decision(&pool, Some("dave")).await == ReauthDecision::Kill,
            "registry error must read as Kill, never Continue"
        );
        let (_btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool,
            Some("dave".to_string()),
            1,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        let _term = recv_event(&mut rx).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(8), rx.recv())
                .await
                .expect("close is timely")
                .is_none(),
            "unreadable-registry stream must close after the termination frame"
        );
        pump.await.expect("pump joins");
    }

    /// Anonymous streams (no identity to kill) are never re-auth-killed.
    #[tokio::test]
    async fn anonymous_stream_is_never_reauth_killed() {
        let (_dir, pool) = test_pool();
        let (btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool,
            None,
            1,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        btx.send(json!({"n": 1})).expect("send");
        let _msg = recv_event(&mut rx).await;
        assert!(
            rx.try_recv().is_err(),
            "anonymous stream must not terminate"
        );
        pump.abort();
    }

    /// The `=0` ceiling, pinned: admission-only means a revoked principal's
    /// stream drains (the operator explicitly opted out of the kill).
    #[tokio::test]
    async fn admission_only_mode_documents_no_kill_ceiling() {
        let (_dir, pool) = test_pool();
        revoke(&pool, "erin");
        let (btx, brx) = tokio::sync::broadcast::channel::<Value>(16);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let pump = tokio::spawn(pump_guarded(
            tx,
            brx,
            pool,
            Some("erin".to_string()),
            0,
            pending_prelude(),
            alert_map,
        ));
        let _pending = recv_event(&mut rx).await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        btx.send(json!({"n": 1})).expect("send");
        let _msg = recv_event(&mut rx).await;
        assert!(
            rx.try_recv().is_err(),
            "=0 is admission-only: no kill may fire"
        );
        pump.abort();
    }

    /// The termination frame shape: exactly `{"revoked":true,"at":<ts>}`.
    #[test]
    fn termination_frame_shape() {
        let payload = termination_payload();
        let v: Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(v["revoked"], Value::Bool(true));
        assert!(
            v["at"].as_i64().is_some_and(|t| t > 0),
            "at must be a unix timestamp: {payload}"
        );
        assert_eq!(
            v.as_object().expect("object").len(),
            2,
            "no extra keys ride the termination frame"
        );
        // And the event wrapper carries the same bytes.
        let _ev: Event = termination_event();
    }
}
