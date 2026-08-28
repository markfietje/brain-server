//! The recall core (Foundation Line, Aqueduct) — the retrieval surfaces'
//! service half: the cross-domain fusion, the per-domain filter law, the
//! per-domain read shaping, and the read-event write story.
//!
//! OWNS (the recall aggregate's storage story):
//! - the per-domain filter decision (multi-db drops the in-DB domain filter;
//!   shim mode keeps it; a bound profile's retention map REPLACES the
//!   server-wide one) — the row-domain predicates themselves run inside the
//!   retriever SQL (`search::vec0_knn` / `fts_search` / `graph_ppr`), exactly
//!   as they always have; what moves here is the decision that feeds them;
//! - the per-domain post-search read shaping: snippet window, evidence
//!   enrichment, flagged-evidence suppression (in that order — suppression
//!   runs AFTER enrichment so a flagged hit's evidence is attached then
//!   stripped, never half-attached);
//! - the cross-domain RRF merge (rank-based fusion over per-domain ranked
//!   lists; raw scores are not comparable across domains);
//! - the read-event write story: the hash-chained audit row, its replayable
//!   trace artifact, the audit-retention prune across EVERY registered
//!   domain chain, and the DSAR-ledger piggyback prune — one service call on
//!   one connection, in the legacy order, best-effort (a failed audit write
//!   never fails the recall the caller asked for; the prunes still run).
//!
//! TAKES connections — never a pool, the registry, or a transport type. The
//! pool schedule (which pool serves which domain, when a connection is
//! acquired, the concurrent vec/FTS/graph legs inside
//! `perform_search_with_prf`) is TRANSPORT and stays in the handler's
//! `spawn_blocking` closure: the hybrid search needs three concurrent pooled
//! connections per domain, and the three-leg acquisition pattern is the perf
//! contract this line must not disturb. The handler keeps exactly that
//! schedule and hands THIS module the decisions, the results, and the
//! connections.
//!
//! Read seam: nothing here sanitizes. Stored forms travel up; the read seam
//! stays at the handler's emission boundary (`results_to_hits`).
//!
//! Time: wall-clock enters as `now_unix` where a decision needs it.

use rusqlite::Connection;
use std::collections::HashMap;
use std::ops::Deref;

use crate::audit::AuditKind;
use crate::search::{SearchFilters, SearchResult};

/// Merge per-domain ranked result lists via Reciprocal Rank Fusion
/// ("merge across domains with the same RRF").
///
/// Each domain's list is treated as one retriever; a chunk that appears in
/// multiple domains accumulates RRF contributions from each. RRF is rank-based,
/// so it correctly merges results whose raw scores are not comparable across
/// domains (different IDF, different embed norms after quantization).
///
/// Dedup key is `(id, domain)` — the same content legitimately stored in two
/// domains stays distinct (two memories), but a chunk can only appear once per
/// domain (the in-domain search already deduplicated).
///
/// `k` is the final cap. The RRF contribution uses the same `RRF_K = 60`
/// constant as the in-domain hybrid fusion (`search::RRF_K`).
pub fn rrf_merge_domains(
    per_domain: Vec<(String, Vec<SearchResult>)>,
    k: usize,
) -> Vec<(SearchResult, String)> {
    use std::collections::HashMap;
    let rrf_k = crate::search::RRF_K as f32;

    // First pass: collect fused scores per (domain, id).
    let mut fused: HashMap<(String, i64), (f32, &SearchResult)> = HashMap::new();
    for (domain, rs) in &per_domain {
        for (rank, r) in rs.iter().enumerate() {
            let key = (domain.clone(), r.id);
            let contribution = 1.0 / (rrf_k + rank as f32);
            fused
                .entry(key)
                .and_modify(|(score, _)| *score += contribution)
                .or_insert((contribution, r));
        }
    }

    // Sort by fused score descending; truncate to k.
    let mut entries: Vec<((String, i64), f32, &SearchResult)> = fused
        .into_iter()
        .map(|((d, id), (score, r))| ((d, id), score, r))
        .collect();
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    entries.truncate(k);

    // Clone the (cheaply cloneable) SearchResult for the caller. ponytail:
    // cloning here keeps the helper pure + testable without lifetime gymnastics.
    entries
        .into_iter()
        .map(|((d, _), _, r)| (r.clone(), d))
        .collect()
}

/// The per-domain filter decision — the row-domain predicates' control plane.
///
/// - multi-db: the pool IS the domain, so drop the in-DB domain filter to
///   avoid double-restricting;
/// - shim mode: keep it, narrowed to the searched label, so the single shared
///   pool still scopes rows per domain;
/// - a bound profile's retention map is THE policy for this domain (replaces
///   the server-wide map; an empty map = no kind decay — the smb-simple
///   posture).
///
/// Pure; every other `SearchFilters` field is cloned through untouched.
pub fn domain_filters(
    base: &SearchFilters,
    domain: &str,
    multi_db: bool,
    profile_retention: &HashMap<String, Vec<(String, i64)>>,
) -> SearchFilters {
    let mut f = base.clone();
    if multi_db {
        f.domain = None;
    } else {
        f.domain = Some(domain.to_string());
    }
    if let Some(days) = profile_retention.get(domain) {
        f.retention_days = std::sync::Arc::new(days.clone());
    }
    f
}

/// The per-domain post-search read shaping, in the legacy order: snippet
/// window first, evidence enrichment second (best-effort — an unavailable
/// connection skips enrichment exactly as the pre-move pooled `get` did),
/// flagged-evidence suppression LAST (after enrichment, so a flagged hit the
/// caller did not opt into has its snippet + evidence attached and then
/// stripped — never half-attached).
pub fn finish_domain_results(
    conn: Option<&Connection>,
    results: &mut [SearchResult],
    snippet_query: &str,
    historical: bool,
    include_flagged: bool,
) {
    for r in results.iter_mut() {
        r.with_snippet(snippet_query);
    }
    if let Some(conn) = conn {
        let _ = SearchResult::enrich_evidence(conn, results, snippet_query, historical);
    }
    for r in results.iter_mut() {
        crate::suppress_flagged_evidence(r, include_flagged);
    }
}

/// The read-event input envelope — the audit identity plus the retention
/// cadence the write story runs at.
pub struct ReadEvent<'a> {
    pub kind: AuditKind,
    pub actor: &'a str,
    /// the raw query text — the audit row's hash-only target; the replay
    /// artifact (`trace_detail`) carries the SHA-256, never this text.
    pub query: &'a str,
    pub trace_detail: Option<&'a str>,
    pub tenant: &'a str,
    /// audit-chain retention days; `None` = the prune leg does not run (and
    /// the prune connections are never pulled — the iterator stays lazy).
    pub prune_days: Option<u32>,
    pub dsar_retention_days: u32,
}

/// The recall read-event write story — the aggregate's complete write path.
///
/// Writes the hash-chained read-event audit row (with its replayable trace
/// artifact when `trace_detail` is present), prunes EVERY registered domain's
/// audit chain when retention is configured (each prune connection is pulled
/// lazily from the caller's iterator and dropped before the next is
/// acquired — the sequential one-connection-at-a-time schedule is the point),
/// and piggybacks the DSAR-ledger retention prune on the same cadence.
///
/// Best-effort BY CONTRACT: the audit row id is returned when the write
/// landed, `None` when it did not — and a failed write does NOT skip the
/// prunes (the legacy order: record attempt, then prune, then DSAR piggyback,
/// then return). Availability-first: a failure here must never fail the
/// recall the caller asked for.
///
/// `C` is the caller's connection guard (r2d2's pooled handle or a raw
/// connection — this module never names it); `I` yields one guard per
/// registered domain chain.
pub fn record_recall_read_event<C, I>(
    conn: &Connection,
    prune_conns: I,
    event: ReadEvent<'_>,
) -> Option<i64>
where
    C: Deref<Target = Connection>,
    I: IntoIterator<Item = C>,
{
    let id = crate::audit::record_read_event(
        conn,
        event.kind,
        event.actor,
        event.query,
        event.trace_detail,
        event.tenant,
    );
    if let Some(days) = event.prune_days {
        // No-op on failure — prunes are fail-safe (retention lingers; it
        // never false-deletes); the warning is logged inside the helper.
        for c in prune_conns {
            crate::audit::prune_audit_retention(&c, days);
        }
    }
    // piggyback the DSAR ledger retention on the
    // same read-event prune cadence (no dedicated timer).
    crate::service::dsar::purge_stale_dsar_ledger(conn, event.dsar_retention_days);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-domain RRF: results are ranked by their RRF contribution
    /// `1/(k+rank)` per-domain. Raw scores are NOT comparable across domains
    /// (different IDF tables, different embed norms); rank IS comparable.
    /// Each (domain, id) pair is a distinct hit tagged with its source domain.
    #[test]
    fn rrf_merge_ranks_by_per_domain_rank_not_raw_score() {
        let mk = |id: i64, score: f32, content: &str| SearchResult {
            id,
            score,
            content: content.into(),
            untrusted: true,
            ..Default::default()
        };
        // Domain A: chunk 1 rank 0 (raw score 0.10), chunk 2 rank 1 (raw 0.95).
        // Domain B: chunk 3 rank 0 (raw 0.99), chunk 4 rank 1 (raw 0.50).
        //
        // Under the OLD raw-score merge, chunk 3 (0.99) would win. Under RRF,
        // the two rank-0 hits (chunk 1 in A, chunk 3 in B) tie for first because
        // each contributes exactly `1/60`. The high raw 0.99 score must NOT
        // outweigh the low raw 0.10 score — both are rank 0 in their domain.
        let per_domain = vec![
            ("a".to_string(), vec![mk(1, 0.10, "a1"), mk(2, 0.95, "a2")]),
            ("b".to_string(), vec![mk(3, 0.99, "b3"), mk(4, 0.50, "b4")]),
        ];
        let merged = rrf_merge_domains(per_domain, 10);
        // Top two must be the two rank-0 hits (ids 1 and 3), in either order.
        let top_ids: std::collections::HashSet<i64> =
            merged.iter().take(2).map(|(r, _)| r.id).collect();
        assert_eq!(
            top_ids,
            [1, 3].into_iter().collect(),
            "rank-0 hits should win regardless of raw score"
        );
        // Every hit is tagged with its source domain.
        let tags: Vec<&str> = merged.iter().map(|(_, d)| d.as_str()).collect();
        assert!(tags.contains(&"a") && tags.contains(&"b"));
        // Cap to k.
        let capped = rrf_merge_domains(
            vec![
                ("a".to_string(), vec![mk(1, 0.1, "a1"), mk(2, 0.2, "a2")]),
                ("b".to_string(), vec![mk(3, 0.3, "b3"), mk(4, 0.4, "b4")]),
            ],
            2,
        );
        assert_eq!(capped.len(), 2, "k truncation must apply");
    }

    /// Same chunk id in the SAME domain twice (shouldn't happen, but the dedup
    /// key is (domain, id) so we must keep them distinct across domains).
    #[test]
    fn rrf_merge_keeps_same_id_in_different_domains() {
        let mk = |id: i64, content: &str| SearchResult {
            id,
            score: 0.5,
            content: content.into(),
            untrusted: true,
            ..Default::default()
        };
        let per_domain = vec![
            ("a".to_string(), vec![mk(7, "a-copy")]),
            ("b".to_string(), vec![mk(7, "b-copy")]),
        ];
        let merged = rrf_merge_domains(per_domain, 10);
        assert_eq!(
            merged.len(),
            2,
            "same id in different domains stays distinct"
        );
    }

    /// the stored recall trace records `query_hash` (SHA-256, v1.20.25),
    /// never the raw query text — a recall query typed by a user is itself
    /// personal data of that subject, and must not linger in the replay
    /// artifact (the DSAR residue sweep relies on this invariant).
    #[test]
    fn stored_trace_hashes_query_never_stores_raw_text() {
        crate::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        let secret_query = "alice@example.com's medical history";
        let trace_detail = serde_json::json!({
            "query_hash": crate::audit::hash(secret_query),
            "decision": "Ok",
            "graph_rescued": false,
            "hits": [],
        })
        .to_string();
        let id = crate::audit::record_read_event(
            &conn,
            crate::audit::AuditKind::Recall,
            "alice@example.com",
            secret_query,
            Some(&trace_detail),
            "api",
        )
        .expect("trace row");
        let replayed = crate::audit::read_trace(&conn, id).unwrap();
        assert!(
            !replayed.contains(secret_query),
            "raw query text must never be stored in the trace"
        );
        let v: serde_json::Value = serde_json::from_str(&replayed).unwrap();
        assert_eq!(v["query_hash"], crate::audit::hash(secret_query));
        // The raw query lives only on the tamper-evident audit row's target
        // (which is what record_read_event stores), never the replay artifact.
    }

    /// the row-domain predicate decision: multi-db drops the in-DB domain
    /// filter (the pool IS the domain — double-restricting would empty every
    /// result set), shim mode keeps it scoped to the searched label, and a
    /// bound profile's retention map REPLACES the server-wide map verbatim
    /// (an unbound domain keeps the server-wide map). Everything else clones
    /// through untouched.
    #[test]
    fn domain_filters_decides_row_domain_scope_and_retention_replacement() {
        let mut base = SearchFilters {
            domain: Some("global".to_string()),
            ..Default::default()
        };
        base.now_unix = 1_000;
        base.retention_days = std::sync::Arc::new(vec![("memory".to_string(), 30_i64)]);

        let mut profiles: HashMap<String, Vec<(String, i64)>> = HashMap::new();
        profiles.insert("health".to_string(), vec![("fact".to_string(), 365)]);

        // multi-db: the in-DB filter is dropped, the profile map replaced.
        let f = domain_filters(&base, "health", true, &profiles);
        assert_eq!(f.domain, None, "multi-db must not double-restrict");
        assert!(
            f.retention_days
                .iter()
                .any(|(k, v)| k == "fact" && *v == 365),
            "the bound profile's map is THE policy for the domain"
        );
        assert!(
            !f.retention_days.iter().any(|(k, _)| k == "memory"),
            "the server-wide kind is GONE once a profile binds (replace, not merge)"
        );
        assert_eq!(f.now_unix, 1_000, "everything else clones through");

        // shim mode: the filter stays, scoped to the searched label; the
        // unbound domain keeps the server-wide map.
        let f = domain_filters(&base, "global", false, &profiles);
        assert_eq!(f.domain.as_deref(), Some("global"));
        assert!(
            f.retention_days
                .iter()
                .any(|(k, v)| k == "memory" && *v == 30),
            "no bound profile → the server-wide map stands"
        );
    }

    /// the per-domain read shaping order: snippet first, evidence enrichment
    /// second, flagged suppression LAST. A flagged hit the caller did not opt
    /// into ends with NO snippet and NO evidence (attached, then stripped);
    /// an opted-in reviewer sees the enriched evidence survive.
    #[test]
    fn finish_domain_results_enriches_then_suppresses_flagged_evidence() {
        crate::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        conn.execute_batch(
            "INSERT INTO knowledge(id, content, source) VALUES (1, 'alpha beta gamma', 'structured'),
             (2, 'delta epsilon zeta', 'structured');
             INSERT INTO sources(id, uri, kind, state) VALUES (1, 'file:///a', 'file', 'active');
             UPDATE knowledge SET source_id = 1 WHERE id IN (1, 2);",
        )
        .unwrap();
        let flagged = SearchResult {
            id: 1,
            score: 0.9,
            content: "alpha beta gamma".into(),
            flagged: true,
            untrusted: true,
            ..Default::default()
        };
        let clean = SearchResult {
            id: 2,
            score: 0.8,
            content: "delta epsilon zeta".into(),
            untrusted: true,
            ..Default::default()
        };
        let mut both = vec![flagged, clean];
        finish_domain_results(Some(&conn), &mut both, "alpha", false, false);
        assert!(
            both[0].snippet.is_none() && both[0].evidence.is_none(),
            "a flagged hit the caller did not opt into ends with NO snippet and NO \
             evidence — suppression runs after enrichment, so enrichment cannot \
             re-attach what the review posture strips"
        );
        assert!(
            both[1].snippet.as_deref().is_some(),
            "the clean hit keeps its snippet"
        );

        let flagged_again = SearchResult {
            id: 1,
            score: 0.9,
            content: "alpha beta gamma".into(),
            flagged: true,
            untrusted: true,
            ..Default::default()
        };
        let mut reviewer = vec![flagged_again];
        finish_domain_results(Some(&conn), &mut reviewer, "alpha", false, true);
        assert!(
            reviewer[0].snippet.is_some(),
            "the reviewer opted in — the snippet survives suppression"
        );

        // An unavailable connection skips enrichment but still shapes.
        let mut bare = vec![SearchResult {
            id: 2,
            score: 0.8,
            content: "delta epsilon zeta".into(),
            untrusted: true,
            ..Default::default()
        }];
        finish_domain_results(None, &mut bare, "delta", false, false);
        assert!(
            bare[0].snippet.is_some(),
            "snippet runs without a connection"
        );
    }

    /// the write story covers EVERY registered domain chain: one read event
    /// write, then the retention prune touches the global chain AND each
    /// domain chain (aged rows emptied everywhere), then the DSAR piggyback
    /// empties stale completed ledger rows — in that order, on the legacy
    /// cadence.
    #[test]
    fn read_event_prunes_every_domain_chain_and_piggybacks_dsar() {
        crate::register_sqlite_vec();
        let mut global = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut global, 1).unwrap();
        let mut domain_a = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut domain_a, 1).unwrap();
        let mut domain_b = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut domain_b, 1).unwrap();

        // Aged evidence on every chain (hand-inserted legacy rows — the same
        // fixture shape the retention pins use; NULL prev_hash rows verify as
        // a leading legacy run) + one stale completed DSAR ledger row.
        for c in [&global, &domain_a, &domain_b] {
            c.execute(
                "INSERT INTO audit_events(id, ts, kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash)
                 VALUES (1, '2000-01-01 00:00:00', 'recall', 'actor', 't', 'ok', 'd', 'tenant', NULL),
                        (2, '2000-01-01 00:00:01', 'recall', 'actor', 't', 'ok', 'd', 'tenant', NULL),
                        (3, '2000-01-01 00:00:02', 'recall', 'actor', 't', 'ok', 'd', 'tenant', NULL)",
                [],
            )
            .unwrap();
        }
        global
            .execute(
                "INSERT INTO dsar_requests(subject, action, status, created_at, completed_at)
                 VALUES ('sub', 'erase', 'completed', 946684800, 946771200)",
                [],
            )
            .unwrap();

        let id = record_recall_read_event(
            &global,
            [&global, &domain_a, &domain_b],
            ReadEvent {
                kind: crate::audit::AuditKind::Recall,
                actor: "actor",
                query: "the query",
                trace_detail: None,
                tenant: "tenant",
                prune_days: Some(30),
                dsar_retention_days: 30,
            },
        )
        .expect("the read event itself lands");
        assert!(
            crate::audit::read_trace(&global, id).is_none(),
            "no trace_detail → no replay artifact, audit row only"
        );
        // Every chain lost its aged rows; the prune evidences ITSELF with one
        // AuditKind::Retention row per pruned chain. The global chain
        // additionally carries the read event itself.
        for (name, c, expect) in [
            ("global", &global, vec!["recall", "retention"]),
            ("a", &domain_a, vec!["retention"]),
            ("b", &domain_b, vec!["retention"]),
        ] {
            let kinds: Vec<String> = {
                let mut stmt = c
                    .prepare("SELECT kind FROM audit_events ORDER BY id")
                    .unwrap();
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .flatten()
                    .collect()
            };
            assert_eq!(
                kinds, expect,
                "{name} chain: aged rows pruned, survivors are the expected evidence"
            );
        }
        let stale: i64 = global
            .query_row(
                "SELECT COUNT(*) FROM dsar_requests WHERE status = 'completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "the DSAR piggyback emptied the stale ledger row");
    }

    /// best-effort BY CONTRACT, with the legacy order: a failed audit write
    /// returns None AND the prunes still run (the pre-move code had no early
    /// return between the record attempt and the prunes — pinned so a future
    /// `?` refactor cannot silently strand retention).
    #[test]
    fn read_event_failure_returns_none_and_still_prunes() {
        crate::register_sqlite_vec();
        // No migration → the audit chain cannot be written → the record fails.
        let broken = Connection::open_in_memory().unwrap();
        let mut prunable = Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut prunable, 1).unwrap();
        prunable
            .execute(
                "INSERT INTO audit_events(id, ts, kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash)
                 VALUES (1, '2000-01-01 00:00:00', 'recall', 'actor', 't', 'ok', 'd', 'tenant', NULL)",
                [],
            )
            .unwrap();

        // The DSAR piggyback tolerates the missing table (fail-safe count).
        let id = record_recall_read_event(
            &broken,
            [&prunable],
            ReadEvent {
                kind: crate::audit::AuditKind::Recall,
                actor: "actor",
                query: "q",
                trace_detail: None,
                tenant: "tenant",
                prune_days: Some(30),
                dsar_retention_days: 30,
            },
        );
        assert!(id.is_none(), "a failed write is None, never a panic");
        let (n, kind): (i64, String) = prunable
            .query_row(
                "SELECT COUNT(*), COALESCE(MIN(kind), '') FROM audit_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "the prune ran anyway — retention never rides the write's success"
        );
        assert_eq!(
            kind, "retention",
            "the survivor is the prune's own evidence row"
        );
    }
}
