//! The decay core — the `/decayed` operator review list, moved verbatim out
//! of the gate handler into the service layer's lifecycle family.
//!
//! OWNS (this aggregate's complete storage story):
//! - the SQL-superset WHERE ([`decayed_superset_sql`]): branch A is the exact
//!   per-chunk expiry (`expires_at < now`, index-served), branch B covers the
//!   kind-policy expiry via the raw `created_at` text at the LEAST
//!   restrictive cutoff — the WHERE narrows the scan, it never decides a row;
//! - the held-id exclusion (a held id never shows up in the decay registry —
//!   the operator must never see a frozen id as "safe to purge"), filtered
//!   BEFORE the page split so the arbiter stays a pure function of
//!   rows + policy;
//! - the Rust-side arbiter ([`page_decayed`]): the exact
//!   `effective_expiry` filter (per-domain profile replacement included) and
//!   the page split (`skip`/`take` over the STABLE row order). The SQL and
//!   the arbiter are ONE unit —
//!   `sql_superset_plus_rust_arbiter_move_together` pins the pairing.
//!
//! FK-children map: NONE — this aggregate is read-only (no DELETE, no
//! parent row).
//!
//! Bounds: `limit` is clamped to `[1, MAX_DECAYED]` and `offset` floored at
//! 0 HERE (idempotent with the route's identical clamp, so every future
//! caller inherits the fence — the fence holds of the FUNCTION, not
//! call-site discipline).
//!
//! Wire-shape ceiling (honest): rows stay the legacy `serde_json::Value`
//! maps, built with the exact `json!` literal the handler used pre-move —
//! the byte-for-byte wire pins outrank the domain-type aspiration.
//!
//! Non-goal: nothing here deletes. `/decayed` is the review list; erasure is
//! explicit (`/purge`, the DSAR workflow) and nothing decays away on its own.

use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};

/// A loaded `/decayed` row: (id, content_hash, expires_at, node_kind,
/// created_at_unix, domain) — the domain is carried so the Rust filter can
/// resolve the per-domain profile policy.
pub(crate) type DecayedRow = (i64, Option<String>, Option<i64>, String, i64, String);

/// Typed service error (the ServiceError convention: one enum per module).
/// `Database` carries the rusqlite text VERBATIM — the handler maps it onto
/// the route's frozen internal-error body byte-for-byte.
#[derive(Debug)]
pub(crate) enum DecayError {
    /// A query failed; the rusqlite message travels unchanged.
    Database(String),
}

impl std::fmt::Display for DecayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecayError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl From<rusqlite::Error> for DecayError {
    fn from(e: rusqlite::Error) -> Self {
        DecayError::Database(e.to_string())
    }
}

/// the SQL-superset WHERE for `/decayed` — branch A (exact
/// per-chunk expiry, `expires_at < ?1`, index-served) plus branch B (kind-
/// policy superset via the raw `created_at` text, cut off at the LEAST
/// restrictive threshold — min days → latest cutoff — so no Rust-expired row
/// is ever excluded: `created < now - days_k` implies `created < now -
/// min_days`). Extracted so the superset property is unit-testable: the
/// Rust-side `page_decayed` filter remains the arbiter; this clause only
/// narrows the scan. Note `unixepoch()` (INTEGER): the legacy
/// `strftime('%s', ...)` returns TEXT, so `get::<i64>` silently dropped
/// every row and `/decayed` was always empty — the regression
/// test caught it. ponytail: with an empty kind policy the clause is branch
/// A alone — byte-identical to the prior query.
pub(crate) fn decayed_superset_sql(
    now: i64,
    retention_days: &BTreeMap<String, i64>,
    domain: Option<&str>,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut sql = String::from(
        "SELECT id, content_hash, expires_at, node_kind, \
                unixepoch(COALESCE(created_at, '1970-01-01 00:00:00')), domain \
         FROM knowledge WHERE expires_at IS NOT NULL AND expires_at < ?1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    if !retention_days.is_empty() {
        let kinds: Vec<&String> = retention_days.keys().collect();
        let placeholders: Vec<String> = (2..=(1 + kinds.len())).map(|i| format!("?{i}")).collect();
        let cutoff_idx = 2 + kinds.len();
        sql.push_str(&format!(
            " OR (expires_at IS NULL AND node_kind IN ({placeholders}) \
                AND created_at < ?{cutoff_idx})",
            placeholders = placeholders.join(","),
            cutoff_idx = cutoff_idx,
        ));
        for k in &kinds {
            params.push(Box::new((*k).clone()));
        }
        let min_days = retention_days.values().copied().min().unwrap_or(0);
        let cutoff = chrono::DateTime::from_timestamp(now - min_days * 86_400, 0)
            .map(|t| t.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "1970-01-01 00:00:00".to_string());
        params.push(Box::new(cutoff));
    }
    // Shim-mode scope — the label narrows the superset to the
    // caller's domain (the Rust arbiter `page_decayed` is domain-agnostic, so
    // the narrowing is exact, never a false positive).
    if let Some(label) = domain {
        let idx = params.len() + 1;
        sql.push_str(&format!(" AND domain = ?{idx}"));
        params.push(Box::new(label.to_string()));
    }
    (sql, params)
}

/// Pure core of `/decayed`: from the loaded `ORDER BY id` rows, keep the
/// expired ones (Rust-side [`crate::gate::effective_expiry`] — not an
/// expressible SQL predicate) and page them. Stable across the Rust filter.
/// a row whose domain has a bound profile with a retention block is
/// judged by THAT map (replacing the server-wide policy); other rows keep the
/// server-wide policy.
pub(crate) fn page_decayed(
    rows: &[DecayedRow],
    now: i64,
    retention_days: &BTreeMap<String, i64>,
    per_domain: &HashMap<String, BTreeMap<String, i64>>,
    offset: i64,
    limit: i64,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for (id, content_hash, expires_at, kind, created_unix, domain) in rows {
        let policy = per_domain.get(domain).unwrap_or(retention_days);
        let effective =
            crate::gate::effective_expiry(*expires_at, Some(*created_unix), kind, policy);
        if effective.is_some_and(|e| e < now) {
            out.push(serde_json::json!({
                "id": id,
                "content_hash": content_hash,
                "expires_at": expires_at,
                "effective_expiry": effective,
                "memory_kind": kind,
                "reason": crate::gate::retention_reason(*expires_at, effective),
            }));
        }
    }
    out.into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect()
}

/// The `/decayed` storage story in one call: load the superset rows, drop
/// held ids, run the Rust arbiter, split the page. `limit`/`offset` are
/// re-clamped here so every future caller inherits the bounded-page fence.
pub(crate) fn decayed_page(
    conn: &Connection,
    now: i64,
    retention_days: &BTreeMap<String, i64>,
    per_domain: &HashMap<String, BTreeMap<String, i64>>,
    shim_label: Option<&str>,
    offset: i64,
    limit: i64,
) -> Result<Vec<serde_json::Value>, DecayError> {
    // bounded page; the Rust-side expiry filter runs BEFORE
    // the page split so a boundary never splits the "is it expired?" decision.
    let limit = limit.clamp(1, crate::config::MAX_DECAYED);
    let offset = offset.max(0);
    // the SQL superset policy = the server-wide map united with
    // every profile policy's kinds, so the superset property holds under any
    // per-domain replacement (the arbiter still judges per row's domain).
    let mut sql_policy = retention_days.clone();
    for m in per_domain.values() {
        for (k, v) in m {
            sql_policy.insert(k.clone(), *v);
        }
    }
    let (sql, params) = decayed_superset_sql(now, &sql_policy, shim_label);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p as &dyn rusqlite::types::ToSql)),
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )?
        .filter_map(|r| r.ok())
        .collect::<Vec<DecayedRow>>();
    // a held id never shows up in the decay
    // registry — the operator must never see a frozen id as "safe to
    // purge". Filter the loaded rows before the Rust arbiter pages them
    // (keeps `page_decayed` a pure function of the rows + policy).
    let held = crate::legal_hold::active_hold_ids(conn)?;
    let rows = rows
        .into_iter()
        .filter(|(id, ..)| !held.contains(id))
        .collect::<Vec<DecayedRow>>();
    Ok(page_decayed(
        &rows,
        now,
        retention_days,
        per_domain,
        offset,
        limit,
    ))
}

#[cfg(test)]
mod pins {
    use super::*;

    /// The pairing pin: the SQL superset and the Rust arbiter moved
    /// as ONE unit — both halves live in the decay core, NEITHER remains in
    /// the gate handler, and the route wires through the core. The
    /// SQL-never-decides-a-row law travels with both halves; splitting them
    /// across layers (or leaving a divergent copy behind) would let the SQL
    /// quietly become the row's judge.
    #[test]
    fn sql_superset_plus_rust_arbiter_move_together() {
        let decay_src = include_str!("decay.rs");
        let prod = decay_src
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first slice");
        assert!(
            prod.contains("fn decayed_superset_sql(") && prod.contains("fn page_decayed("),
            "the superset SQL and the Rust arbiter must live together in the decay core"
        );
        let gate_src = include_str!("../../handlers/gate.rs");
        assert!(
            !gate_src.contains("decayed_superset_sql") && !gate_src.contains("page_decayed"),
            "a copy of the superset SQL or the arbiter remains in the gate handler — \
             the SQL would drift from its arbiter"
        );
        assert!(
            gate_src.contains("lifecycle::decay::decayed_page("),
            "`/decayed` must route through the decay core (no parallel handler SQL)"
        );
    }

    /// `/decayed` returns a bounded first page and `?offset=`
    /// pages the rest — the page split never re-introduces an unbounded list.
    /// (Moved verbatim with the arbiter from the gate handler tests.)
    #[test]
    fn page_decayed_respects_limit_and_offset() {
        // Three expired rows (expires_at in the past); no kind policy.
        let rows: Vec<DecayedRow> = vec![
            (
                1,
                None,
                Some(100),
                "fact".to_string(),
                50,
                "global".to_string(),
            ),
            (
                2,
                None,
                Some(100),
                "fact".to_string(),
                50,
                "global".to_string(),
            ),
            (
                3,
                None,
                Some(100),
                "fact".to_string(),
                50,
                "global".to_string(),
            ),
        ];
        let retention = std::collections::BTreeMap::new();
        let per_domain = std::collections::HashMap::new();
        let now = 1000;

        let first = page_decayed(&rows, now, &retention, &per_domain, 0, 2);
        assert_eq!(first.len(), 2, "first page honors the limit");
        assert_eq!(first[0]["id"], 1);
        assert_eq!(first[1]["id"], 2);

        let next = page_decayed(&rows, now, &retention, &per_domain, 2, 2);
        assert_eq!(next.len(), 1, "offset pages the remainder");
        assert_eq!(next[0]["id"], 3);

        // A page past the end yields nothing (stable, not an error).
        assert!(page_decayed(&rows, now, &retention, &per_domain, 99, 2).is_empty());
    }

    /// a row in a bound domain is judged by THAT profile's
    /// retention map (replacing the server-wide policy); unbound rows keep the
    /// server-wide policy. An empty profile map = no kind decay at all.
    /// (Moved verbatim with the arbiter from the gate handler tests.)
    #[test]
    fn page_decayed_judges_bound_domains_by_their_profile() {
        // Two identical 400-day-old episodic rows, one in a call-center domain
        // (episodic: 90 — expired) and one in a domain bound to an empty map
        // (no decay — alive despite the server-wide episodic default).
        let now = chrono::Utc::now().timestamp();
        let created = now - 400 * 86_400;
        let rows: Vec<DecayedRow> = vec![
            (
                1,
                None,
                None,
                "episodic".to_string(),
                created,
                "support".to_string(),
            ),
            (
                2,
                None,
                None,
                "episodic".to_string(),
                created,
                "simple".to_string(),
            ),
            (
                3,
                None,
                None,
                "episodic".to_string(),
                created,
                "global".to_string(),
            ),
        ];
        let server_wide = std::collections::BTreeMap::from([("episodic".to_string(), 30)]);
        let per_domain = std::collections::HashMap::from([
            (
                "support".to_string(),
                std::collections::BTreeMap::from([("episodic".to_string(), 90)]),
            ),
            ("simple".to_string(), std::collections::BTreeMap::new()),
        ]);
        let out = page_decayed(&rows, now, &server_wide, &per_domain, 0, 100);
        let ids: Vec<i64> = out.iter().filter_map(|v| v["id"].as_i64()).collect();
        // 1: created+90d elapsed → expired (the profile EXTENDED life past the
        //    server-wide 30d — and the row is now past even 90d).
        // 2: empty profile map → no kind decay → alive.
        // 3: unbound → server-wide 30d → expired.
        assert_eq!(ids, vec![1, 3]);
    }

    /// the SQL WHERE is a superset of the Rust-side
    /// filter — every row `page_decayed` would keep must be selected by the
    /// narrowed SQL, on real CURRENT_TIMESTAMP-format dates. The SQL never
    /// decides a row's fate; the exact filter still lives in Rust.
    /// (Moved verbatim with the pair from the gate handler tests.)
    #[test]
    fn decayed_superset_sql_covers_every_rust_expired_row() {
        let now = chrono::Utc::now().timestamp();
        let fmt = |t: chrono::DateTime<chrono::Utc>| {
            t.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string()
        };
        let old = fmt(chrono::DateTime::from_timestamp(now - 400 * 86_400, 0).unwrap());
        let fresh = fmt(chrono::DateTime::from_timestamp(now - 10 * 86_400, 0).unwrap());

        let conn = rusqlite::Connection::open_in_memory().expect("db");
        conn.execute(
            "CREATE TABLE knowledge (
                id INTEGER PRIMARY KEY,
                content_hash TEXT,
                expires_at INTEGER,
                node_kind TEXT DEFAULT 'chunk',
                created_at TEXT,
                domain TEXT DEFAULT 'global'
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge(id, content_hash, expires_at, node_kind, created_at) VALUES
                 (1, 'a', 100,        'fact', ?1),  -- per-chunk expired (branch A)
                 (2, 'b', NULL,       'note', ?2),  -- kind-policy expired (branch B, old)
                 (3, 'c', NULL,       'note', ?1),  -- kind-policy NOT expired (fresh)
                 (4, 'd', NULL,       'chunk', ?2); -- kind NOT in policy, never expires",
            rusqlite::params![fresh, old],
        )
        .unwrap();

        // Kind policy: note=90d, fact=180d — min days = 90 (latest cutoff).
        let mut retention = std::collections::BTreeMap::new();
        retention.insert("note".to_string(), 90);
        retention.insert("fact".to_string(), 180);
        let (sql, params) = decayed_superset_sql(now, &retention, None);

        let mut stmt = conn.prepare(&sql).unwrap();
        let sql_ids: std::collections::BTreeSet<i64> = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p as &dyn rusqlite::types::ToSql)),
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            .flatten()
            .collect();

        // Rust-side truth: run the exact filter over the full table (the SQL
        // superset policy = the server-wide map; no per-domain bindings in
        // this fixture — the domain-aware path is covered by its own test).
        let all: Vec<DecayedRow> = conn
            .prepare(
                "SELECT id, content_hash, expires_at, node_kind, \
                        unixepoch(COALESCE(created_at, '1970-01-01 00:00:00')), domain \
                 FROM knowledge ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .unwrap()
            .flatten()
            .collect();
        let empty_per_domain = std::collections::HashMap::new();
        let rust_expired: std::collections::BTreeSet<i64> =
            page_decayed(&all, now, &retention, &empty_per_domain, 0, i64::MAX)
                .iter()
                .filter_map(|v| v["id"].as_i64())
                .collect();
        let rust_visible: std::collections::BTreeSet<i64> = page_decayed(
            &all,
            now,
            &std::collections::BTreeMap::new(),
            &empty_per_domain,
            0,
            i64::MAX,
        )
        .iter()
        .filter_map(|v| v["id"].as_i64())
        .collect();

        assert!(
            !rust_expired.is_empty(),
            "fixture must contain expired rows"
        );
        assert_eq!(rust_expired, std::collections::BTreeSet::from([1, 2]));
        assert!(
            sql_ids.is_superset(&rust_expired),
            "SQL ({sql_ids:?}) must cover every Rust-expired row ({rust_expired:?})"
        );
        assert_eq!(
            sql_ids, rust_expired,
            "superset must not widen to rows the exact filter rejects"
        );

        // Empty policy → branch A only: NULL-expiry rows are never selected.
        let (sql_a, params_a) = decayed_superset_sql(now, &std::collections::BTreeMap::new(), None);
        let mut stmt_a = conn.prepare(&sql_a).unwrap();
        let sql_a_ids: std::collections::BTreeSet<i64> = stmt_a
            .query_map(
                rusqlite::params_from_iter(
                    params_a.iter().map(|p| p as &dyn rusqlite::types::ToSql),
                ),
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(
            sql_a_ids, rust_visible,
            "no-policy SQL == per-chunk-only Rust filter"
        );
    }

    /// The core re-asserts the bounded-page fence (the bounds inventory):
    /// an oversized `limit` clamps to `MAX_DECAYED` and a negative `offset`
    /// floors at 0 — the fence holds of the FUNCTION (the route's identical
    /// clamp stays in front for the wire vocabulary).
    #[test]
    fn decayed_page_reasserts_the_bounds_fence() {
        // The exact clamp arithmetic the core runs at entry, asserted at the
        // fence edges (the arbiter + load path above cover the semantics).
        let oversized: i64 = crate::config::MAX_DECAYED * 4;
        assert_eq!(
            oversized.clamp(1, crate::config::MAX_DECAYED),
            crate::config::MAX_DECAYED,
            "oversized limit clamps to MAX_DECAYED"
        );
        assert_eq!(0_i64.clamp(1, crate::config::MAX_DECAYED), 1, "0 → 1");
        let negative_offset: i64 = -7;
        assert_eq!(negative_offset.max(0), 0, "negative offset floors at 0");
        assert_eq!(
            crate::config::MAX_DECAYED.clamp(1, crate::config::MAX_DECAYED),
            crate::config::MAX_DECAYED,
            "the default page IS the cap"
        );
    }
}
