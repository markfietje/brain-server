//! Retrieval engine: hybrid vector (sqlite-vec vec0) + lexical (FTS5/BM25)
//! search, Reciprocal Rank Fusion, and pseudo-relevance-feedback query
//! expansion.
//!
//! This module is deliberately `#![deny(unsafe_code)]`: all FFI (the sqlite-vec
//! extension registration) lives in the crate root. Everything here is safe
//! Rust operating over already-registered SQLite connections.
#![deny(unsafe_code)]

use crate::config::{QualityConfig, MAX_SNIPPET_CHARS, SNIPPET_CONTEXT_CHARS};
use crate::search::graph_ppr::graph_retrieve;
use crate::search::quality::{HeuristicEstimator, Recommendation, RetrievalQualityEstimator};
use crate::search::query::LegFilter;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use model2vec_rs::model::StaticModel;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::time::Instant;
use zerocopy::IntoBytes;

use crate::Pool;

/// Batch size for the legacy brute-force cosine scan fallback.
const SEARCH_BATCH_SIZE: usize = 500;

/// RRF (Reciprocal Rank Fusion) constant. Standard value k=60; robust across
/// corpora without learned weights.
pub const RRF_K: usize = 60;
/// Over-fetch depth for hybrid fusion: each retriever (vec + FTS5) returns up
/// to this many candidates, then RRF merges and caps at the requested `k`.
pub const RRF_OVERFETCH: usize = 20;

// ── Search result types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchSource {
    Vector,
    Fts,
    Graph,
    Both,
}

/// Overall retrieval strategy employed for this query.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    Hybrid,
    HybridPrf,
    /// v1.12.0 "Discern": the estimator classified the query as low-confidence
    /// and the graph leg was engaged as a second pass before abstention.
    HybridGraph,
}

/// PRF execution decision for observability.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrfDecision {
    Disabled,
    LowConfidence { confidence: f32 },
    WeakAgreement { agreement_at_k: usize },
    SmallGap { score_gap: f32 },
    Expanded { confidence: f32, terms: usize },
}

/// Per-retriever provenance: the rank each retriever assigned (0 = best),
/// the fused RRF score, and the (optional, reserved) rerank score. `None`
/// means the document did not appear in that retriever's list.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fts_rank: Option<usize>,
    /// v1.11.0 "Associate": rank assigned by the graph-PPR retriever
    /// (0 = best). `None` when the graph leg didn't retrieve the document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fused_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank_score: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub rerank_truncated: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub prf_expanded: bool,
    /// Which retriever(s) contributed the top result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_retrieval_mode: Option<SearchSource>,
    /// Overall retrieval strategy used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_strategy: Option<RetrievalStrategy>,
    /// Quality assessment from the estimator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_assessment: Option<quality::RetrievalAssessment>,
    /// PRF decision for observability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prf_decision: Option<PrfDecision>,
}

/// v0.9.8 "Evidence" M1.3: lifecycle state of a returned chunk wrt. time.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// The chunk is from the current (latest) revision of its source.
    Current,
    /// The chunk is from a revision that was superseded *after* `as_of` (only
    /// returned in historical point-in-time recall). Never silently presented
    /// as current.
    Historical,
    /// The chunk's source/revision has been deleted/tombstoned (retrievable
    /// only via review paths). Shown so a caller can see what was forgotten.
    Superseded,
}

/// A typed link to another chunk (v0.9.8 M2.2). Rendered on `Evidence` when the
/// chunk participates in a provenance relationship.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceLinkRef {
    /// Target chunk id the relationship points at.
    pub to_chunk: i64,
    /// Relationship kind: supports | supersedes | contradicts | references |
    /// derived_from.
    pub kind: String,
}

/// Structured, faithful evidence for a retrieved chunk (M2.1). `text` is always
/// a verbatim substring of the chunk `content`; `highlights` are byte-offset
/// ranges *within* `text` (never the full content) so a client can render its
/// own markers without the server injecting HTML. `source_uri` + `revision_id`
/// form a stable, dereferenceable link to the exact source revision.
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    /// Verbatim window of the chunk content around the first query-term match.
    pub text: String,
    /// 1-indexed source line span of the chunk (from the v0.9.x chunker).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<i64>,
    /// Heading breadcrumb the chunk sits under (e.g. `["Setup", "Install"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    /// Canonical source URI (`sources.uri`), e.g. `manual://{hash}` or a
    /// vault file path. `None` for pre-v0.9.4 chunks with no source linkage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    /// Immutable source revision id (`source_revisions.id`). `None` if unlinked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<i64>,
    /// `[start, end)` byte ranges within `text` to highlight (query matches).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<[usize; 2]>,
    /// v0.9.8 M1.3: when the fact became true in the world (file mtime, issue
    /// created_at, …). RFC3339; `None` for pre-v0.9.8 rows (treated as observed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    /// v0.9.8 M1.3: when the fact ceased to be true. `None` ⇒ still current.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    /// v0.9.8 M1.3: when brain-server learned the fact (ingest/sync time).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    /// v0.9.8 M2.4: source-authority tie-breaker (0..1). `None` for legacy rows
    /// (treated as `AUTHORITY_VAULT` at read time). Never in `fused_score`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<f32>,
    /// v0.9.8 M1.3: current vs historical vs superseded. `Current` is the
    /// common case and is skipped on the wire to keep it small.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Lifecycle>,
    /// v0.9.8 M2.2: typed provenance links this chunk participates in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<EvidenceLinkRef>,
    /// All retrieved content is untrusted evidence (OWASP LLM01:2025 — the
    /// control point is at the API boundary, not inside the model). This flag
    /// tells the consuming agent to treat `text` as data, never as instructions.
    pub untrusted: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchResult {
    pub id: i64,
    #[serde(rename = "similarity")]
    pub score: f32,
    pub title: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SearchSource>,
    #[serde(skip_serializing_if = "provenance_is_empty")]
    pub provenance: Provenance,
    /// True when the source row was quarantined (e.g. prompt-injection screen
    /// tripped). Internal safety flag; never serialized to clients.
    #[serde(skip)]
    pub flagged: bool,
    /// All retrieved content is untrusted evidence (OWASP LLM01:2025). Serialized
    /// so the consuming agent can enforce the instruction/data boundary.
    pub untrusted: bool,
    /// Bounded, faithful snippet of the chunk (a window around the first query
    /// term match, or a leading window). Never claims text not present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// v0.9.8 M2.4: when brain-server learned this fact (ingest/sync time).
    /// Populated by the retrievers from `knowledge.observed_at`; used only for
    /// the deterministic freshness tie-break, never blended into `score`.
    #[serde(skip)]
    pub observed_at: Option<String>,
    /// v0.9.8 M2.4: source-authority tie-breaker (0..1). `#[serde(skip)]` —
    /// it is a ranking hint, not part of the wire contract.
    #[serde(skip)]
    pub authority: Option<f32>,
    /// Structured evidence (span + source link + highlight ranges). Populated
    /// by `enrich_evidence` after retrieval; absent if enrichment is skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    /// v1.14.0 "Gate" M3: `assertion_kind` (stated|observed|inferred) read from
    /// `knowledge.assertion_kind`. Ranking-neutral; surfaced so the caller can
    /// weigh a fact. `None` for legacy rows.
    #[serde(skip)]
    pub assertion_kind: Option<String>,
    /// v1.14.0 "Gate" M3: deterministic stored confidence (0..1). Ranking-
    /// neutral; `None` for legacy rows.
    #[serde(skip)]
    pub confidence: Option<f32>,
    /// v1.14.0 "Gate" M2: the chunk's `expires_at` (unix ts), used to compute
    /// the `decayed` flag at query time. `None` = no decay.
    #[serde(skip)]
    pub expires_at: Option<i64>,
    /// v1.14.0 "Gate" M4: whether the chunk was PII-flagged at ingest. Never
    /// serialized (drives read-path redaction only).
    #[serde(skip)]
    pub pii: bool,
}

fn provenance_is_empty(p: &Provenance) -> bool {
    p.vector_rank.is_none()
        && p.fts_rank.is_none()
        && p.graph_rank.is_none()
        && p.fused_score.is_none()
        && p.rerank_score.is_none()
        && !p.rerank_truncated
        && !p.prf_expanded
}

impl SearchResult {
    /// Minimal constructor for a raw retriever hit (no provenance yet).
    pub(crate) fn raw(id: i64, score: f32, title: Option<String>, content: String) -> Self {
        Self {
            id,
            score,
            title,
            content,
            source: None,
            provenance: Provenance::default(),
            flagged: false,
            untrusted: true,
            snippet: None,
            evidence: None,
            observed_at: None,
            authority: None,
            assertion_kind: None,
            confidence: None,
            expires_at: None,
            pii: false,
        }
    }

    /// Attach a bounded, faithful snippet derived from `query` terms. The
    /// snippet is always a verbatim substring of `self.content` (never
    /// synthesized), so it can never misrepresent the source.
    pub fn with_snippet(&mut self, query: &str) {
        let q = query.trim();
        if q.is_empty() {
            self.snippet = Some(self.content.chars().take(MAX_SNIPPET_CHARS).collect());
            return;
        }
        let lower = self.content.to_lowercase();
        // Pick the first query token that appears in the content.
        let term = q
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 3)
            .find(|t| lower.contains(&t.to_lowercase()));
        let idx = term
            .and_then(|t| lower.find(&t.to_lowercase()))
            .unwrap_or(0);
        let start = idx.saturating_sub(SNIPPET_CONTEXT_CHARS);
        let end = (idx + MAX_SNIPPET_CHARS).min(self.content.chars().count());
        let snippet: String = self.content.chars().skip(start).take(end - start).collect();
        self.snippet = Some(snippet);
    }

    /// Attaches a [`Evidence`] (verbatim window + line/heading span + source
    /// link + highlight ranges) by joining the hit ids to their `knowledge`
    /// span columns and `sources`/`source_revisions`. One batched LEFT JOIN,
    /// not N queries. `snippet_q` is the term used to center the window and to
    /// compute highlight byte-ranges within the window.
    ///
    /// ponytail: highlight ranges are computed on `text` (the snippet window),
    /// not the full content, so they can never point past the revealed text;
    /// this is the redaction guarantee — `Explain`/`Evidence` never serializes
    /// content beyond the window.
    pub fn enrich_evidence(
        conn: &Connection,
        results: &mut [SearchResult],
        snippet_q: &str,
        historical: bool,
    ) -> Result<()> {
        if results.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = results.iter().map(|r| r.id).collect();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT k.id, k.line_start, k.line_end, k.heading_path, s.uri, sr.id,
                    k.valid_from, k.valid_to, k.observed_at, k.authority,
                    sr.state, s.state
             FROM knowledge k
             LEFT JOIN sources s ON k.source_id = s.id
             LEFT JOIN source_revisions sr ON k.revision_id = sr.id
             WHERE k.id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })?;
        use std::collections::HashMap;
        #[derive(Default)]
        struct Span {
            line_start: Option<i64>,
            line_end: Option<i64>,
            heading_path: Option<String>,
            source_uri: Option<String>,
            revision_id: Option<i64>,
            valid_from: Option<String>,
            valid_to: Option<String>,
            observed_at: Option<String>,
            authority: Option<f32>,
            rev_state: Option<String>,
            src_state: Option<String>,
        }
        let mut meta: HashMap<i64, Span> = HashMap::new();
        for r in rows {
            let (id, ls, le, hp, uri, rev, vf, vt, obs, auth, rs, ss) = r?;
            meta.insert(
                id,
                Span {
                    line_start: ls,
                    line_end: le,
                    heading_path: hp,
                    source_uri: uri,
                    revision_id: rev,
                    valid_from: vf,
                    valid_to: vt,
                    observed_at: obs,
                    authority: auth.map(|a| a as f32),
                    rev_state: rs,
                    src_state: ss,
                },
            );
        }
        for res in results.iter_mut() {
            // Recompute the verbatim window at the bounded size for a consistent
            // `text`, then derive highlight ranges from snippet_q within it.
            let mut snap = SearchResult {
                id: res.id,
                score: 0.0,
                title: None,
                content: std::mem::take(&mut res.content),
                source: None,
                provenance: Provenance::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                observed_at: None,
                authority: None,
                assertion_kind: None,
                confidence: None,
                expires_at: None,
                pii: false,
            };
            snap.with_snippet(snippet_q);
            let text = snap.snippet.clone().unwrap_or_default();
            // Restore content (enrich must not consume it from the caller).
            res.content = snap.content;
            let highlights = highlight_ranges(&text, snippet_q);
            if let Some(m) = meta.get(&res.id) {
                // Derive lifecycle: a chunk whose source or revision has been
                // deleted/tombstoned is `Superseded`; otherwise `Current` (the
                // historical flag is set by the recall path when as_of mode is
                // active). `Current` is the default and skipped on the wire.
                let lifecycle = if m.src_state.as_deref() == Some("deleted")
                    || m.rev_state.as_deref() == Some("tombstoned")
                    || m.rev_state.as_deref() == Some("superseded")
                {
                    Some(Lifecycle::Superseded)
                } else if historical {
                    Some(Lifecycle::Historical)
                } else {
                    None
                };
                // v0.9.8 M2.2: load typed links this chunk participates in.
                let links = evidence_links_for(conn, res.id).unwrap_or_default();
                res.evidence = Some(Evidence {
                    text: text.clone(),
                    line_start: m.line_start,
                    line_end: m.line_end,
                    heading_path: m.heading_path.clone(),
                    source_uri: m.source_uri.clone(),
                    revision_id: m.revision_id,
                    highlights,
                    valid_from: m.valid_from.clone(),
                    valid_to: m.valid_to.clone(),
                    observed_at: m.observed_at.clone(),
                    authority: m.authority,
                    lifecycle,
                    links,
                    untrusted: true,
                });
                if res.snippet.is_none() {
                    res.snippet = Some(text);
                }
            }
        }
        Ok(())
    }

    /// Attach the quarantined flag read from the `knowledge` row.
    fn with_flagged(mut self, flagged: bool) -> Self {
        self.flagged = flagged;
        self
    }
}

/// v0.9.8 M2.2: load the typed evidence links a chunk participates in, in
/// either direction (`from_chunk = id` OR `to_chunk = id`). One indexed query;
/// empty vec when none or when the `evidence_links` table does not yet exist
/// (graceful on pre-v0.9.8 DBs). `to_chunk` is always the *other* endpoint.
fn evidence_links_for(conn: &Connection, chunk_id: i64) -> Result<Vec<EvidenceLinkRef>> {
    // Guard: the table may be absent on a DB that hasn't run the v0.9.8
    // migration yet (older live DBs). `sqlite_master` check is cheap and keeps
    // enrichment non-fatal.
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='evidence_links'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT CASE WHEN from_chunk = ?1 THEN to_chunk ELSE from_chunk END, kind \
         FROM evidence_links WHERE from_chunk = ?1 OR to_chunk = ?1",
    )?;
    let rows = stmt.query_map(params![chunk_id], |row| {
        Ok(EvidenceLinkRef {
            to_chunk: row.get(0)?,
            kind: row.get(1)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Byte-offset `[start, end)` ranges within `text` of query-term matches.
/// Only alphanumeric tokens length ≥ 3 are matched (mirrors `with_snippet`),
/// and ranges are clamped to `text` so they can never escape the window.
fn highlight_ranges(text: &str, query: &str) -> Vec<[usize; 2]> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for term in q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
    {
        let tl = term.to_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..].find(&tl) {
            let start = from + pos;
            let end = start + tl.len();
            out.push([start, end]);
            from = end;
        }
    }
    out
}

/// Optional metadata filters pushed into the vec0 KNN and FTS5 WHERE clauses.
#[derive(Debug, Clone)]
pub struct SearchFilters {
    /// Deprecated single-source equality. Retained for the legacy
    /// `GET /search?source=` path; new callers use `sources` (OR scope).
    pub source: Option<String>,
    /// v1.13.3 "SourceFix": parsed retrieval-leg restriction (`vector` | `fts` |
    /// `graph`). `None` for ingest-kind filters and unrestricted queries. Set by
    /// the handler from [`crate::search::query::split_source_filter`]; applied
    /// post-fusion on the `SearchSource` tag, never as SQL.
    pub source_leg: Option<LegFilter>,
    /// Multi-source OR scope (v0.9.5 M1). Empty = unrestricted.
    pub sources: Vec<String>,
    /// Validated ISO-8601 timestamp (RFC3339 or `YYYY-MM-DD HH:MM:SS`); rows
    /// with `created_at > since` are returned. Invalid values are rejected by
    /// [`normalize_since`] rather than silently compared as opaque strings.
    pub since: Option<String>,
    /// Domain isolation filter (per-domain routing; single-DB tagged model).
    pub domain: Option<String>,
    /// Lexical query override for FTS5 (exact terms, code, phrases, `-exclusions`).
    /// When set, the FTS retriever uses this instead of the bare query string,
    /// giving lexical precision independent of the semantic `vec`/`hyde` path.
    pub lex: Option<String>,
    /// Embedding query override. `hyde` (caller-supplied hypothetical answer)
    /// takes priority, then `vec` (semantic intent); falls back to the bare `q`.
    pub embedding_query: Option<String>,
    /// Free-form intent label, recorded for provenance/explainability. Full
    /// intent-aware reranking/expansion is a later phase.
    pub intent: Option<String>,
    /// Retrieval profile hint (v0.9.5 M1). Passthrough only — no rerank
    /// plumbing consumes it yet.
    pub profile: Option<String>,
    /// v0.9.7 "Guard": when `false` (default), retrieval excludes quarantined
    /// (`flagged=1`) chunks so untrusted/prompt-injection content never reaches
    /// the agent. Set `true` only for operator review paths (`?include_flagged=1`).
    pub include_flagged: bool,
    /// v0.9.8 "Evidence": point-in-time recall. When `Some`, retrieval returns
    /// the revision of each source that was current *at* this RFC3339 instant
    /// (historical mode) — chunks whose revision was active at `as_of` and not
    /// yet superseded by a revision fetched strictly after `as_of` become
    /// visible. `None` (default) ⇒ only current evidence (existing behavior).
    pub as_of: Option<String>,
    /// v0.9.8 "Evidence": include the full `Evidence` (time + lifecycle + links)
    /// on every hit even when the caller did not request `provenance`. Purely a
    /// serialization switch on top of `enrich_evidence`.
    pub evidence: bool,
    /// v0.9.8 "Evidence": deterministic freshness tie-break (M2.4). Within equal
    /// RRF-score buckets, prefer newer `observed_at` then higher `authority`.
    /// Never blended into `fused_score` (would distort lexical/vector semantics).
    pub freshness_tiebreak: bool,
    /// v1.4.0 "Calibrate" M1: bi-temporal point-in-time filter (valid time, NOT
    /// transaction time). When `Some`, a chunk/edge is visible iff its
    /// valid-interval contains this instant:
    ///   (valid_from IS NULL OR valid_from <= at) AND (valid_to IS NULL OR valid_to > at)
    /// Distinct from `as_of` (revision/transaction-time point-in-time recall).
    /// Graphiti valid_at/invalid_at semantics (Context7-verified 2026-07-30).
    pub at: Option<String>,
    /// v1.11.0 "Associate": enable the graph-PPR retriever as a third RRF leg
    /// behind the existing vector + lexical fusion. Opt-in (the roadmap's
    /// no-default-cost guardrail): `false` = the graph leg never runs.
    pub graph: bool,
    /// v1.14.0 "Gate" M2: when `false` (default), decayed chunks
    /// (`expires_at < now`) are excluded from retrieval; `true` returns them
    /// (operator review). Historical `?at=` is unaffected — decay and
    /// supersession compose. Applied as SQL, like `valid_to`.
    pub include_decayed: bool,
    /// v1.14.0 "Gate" M2: the query-time instant (unix ts) used to evaluate
    /// `expires_at`. Defaults to now; historical recall could pin it. Kept as a
    /// field so the retriever SQL is a pure function of the filters.
    pub now_unix: i64,
    /// v1.14.0 "Gate" M3: `memory_kind` (fact/procedure/step/decision/episodic)
    /// filter. `Some` restricts retrieval to that `knowledge.node_kind`.
    pub memory_kind: Option<String>,
    /// v1.14.0 "Gate" M3: minimum relevance tier (`high`|`medium`|`low`).
    /// `Some` drops lower-tier hits after fusion (the "stop poisoning the
    /// context window" filter), evaluated on the fused RRF score.
    pub min_relevance: Option<String>,
    /// v1.14.0 "Gate" M4: record-level access-scope filter, JWT mode only.
    /// `Some` (list of allowed scopes from the principal) is applied as a
    /// deny-by-default `WHERE access_scope ∈ allowed`. `None` (loopback/
    /// opaque) = no scope restriction (trusts localhost).
    pub access_scopes: Option<Vec<String>>,
    /// v1.17.1 "Govern" M2: per-kind retention policy (`kind -> days`) used to
    /// derive a kind-default `expires_at` for chunks with none. Applied at query
    /// time: when `include_decayed` is false, a chunk whose *effective* expiry
    /// (own `expires_at`, else kind-default from `created_at`) is in the past is
    /// excluded, exactly like a per-chunk `expires_at`. Empty = no kind policy
    /// (the v1.14-only behavior).
    pub retention_days: Vec<(String, i64)>,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            source: None,
            source_leg: None,
            sources: Vec::new(),
            since: None,
            domain: None,
            lex: None,
            embedding_query: None,
            intent: None,
            profile: None,
            include_flagged: false,
            as_of: None,
            evidence: false,
            freshness_tiebreak: true,
            at: None,
            graph: false,
            include_decayed: false,
            now_unix: 0,
            memory_kind: None,
            min_relevance: None,
            access_scopes: None,
            retention_days: Vec::new(),
        }
    }
}

/// Validate and normalize a `since` filter to the database's `created_at`
/// format (`YYYY-MM-DD HH:MM:SS`, UTC). Rejects malformed timestamps so a bad
/// filter cannot degrade into a silent lexical string comparison.
///
/// Accepts: RFC3339 (`2024-03-01T12:00:00Z`), `YYYY-MM-DD HH:MM:SS`, and bare
/// `YYYY-MM-DD` (v1.4.0 Calibrate: the bi-temporal `at` filter commonly uses
/// date-only form, e.g. `?at=2015-06-01`). A bare date is padded to
/// `00:00:00` so SQLite's lexicographic comparison is well-defined.
///
/// ponytail: we normalize to UTC naive time and rely on SQLite's lexicographic
/// comparison of the fixed-width format; we do not honor arbitrary timezone
/// offsets beyond RFC3339 parsing (offsets are converted to UTC).
pub fn normalize_since(since: &str) -> Result<String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(since) {
        return Ok(dt
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(since, "%Y-%m-%d %H:%M:%S") {
        return Ok(dt.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    // v1.4.0: bare date `YYYY-MM-DD` → midnight. Valid ISO-8601 date form.
    if let Ok(d) = NaiveDate::parse_from_str(since, "%Y-%m-%d") {
        return Ok(d
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string());
    }
    anyhow::bail!(
        "invalid 'since' timestamp {:?}; expected ISO-8601 (RFC3339), 'YYYY-MM-DD HH:MM:SS', or 'YYYY-MM-DD'",
        since
    )
}

/// Per-stage latency telemetry (milliseconds), emitted at debug level.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SearchTelemetry {
    pub embed_ms: f32,
    pub vector_ms: f32,
    pub fts_ms: f32,
    /// v1.11.0 "Associate": graph-PPR retrieval latency. `0` when the graph
    /// leg was disabled (`graph=false`).
    pub graph_ms: f32,
    pub fusion_ms: f32,
    pub prf_ms: f32,
    pub rerank_ms: f32,
    /// Vector retrieval candidates before RRF.
    pub vec_candidates: usize,
    /// FTS retrieval candidates before RRF.
    pub fts_candidates: usize,
    /// v1.11.0: graph-PPR candidates before RRF.
    pub graph_candidates: usize,
    /// v1.12.0 "Discern": the graph leg was auto-engaged as a complexity-gated
    /// rescue pass (the estimator said `ClarifyQuery` and the caller had not
    /// enabled `graph`). `false` on the normal path.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub graph_rescued: bool,
    /// Results after RRF fusion.
    pub fused_count: usize,
    /// RRF k parameter used.
    pub rrf_k: u32,
    /// Intent label from a structured query, if supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// The effective embedding query actually used (hyde/vec/q).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_query: Option<String>,
    /// Vector retrieval latency (excludes embedding).
    pub retrieval_ms_vec: f32,
    /// FTS retrieval latency.
    pub retrieval_ms_fts: f32,
    /// Quality estimator confidence score.
    pub confidence: f32,
    /// Quality estimator recommendation.
    pub recommendation: Option<Recommendation>,
    /// v1.4.0 "Calibrate" M2: tokens consumed by submodular packing. `None`
    /// when packing was not requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_tokens: Option<usize>,
    /// v1.4.0 "Calibrate" M2: candidate pool size considered by packing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packing_candidates: Option<usize>,
    /// v1.4.0 "Calibrate" M2: the paper's `answer_in_context` diagnostic — did
    /// the gold answer's tokens survive into the packed context? `None` when no
    /// gold answer was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_in_context: Option<bool>,
}

// ── Similarity ────────────────────────────────────────────────────────────

pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ── Reciprocal Rank Fusion ─────────────────────────────────────────────────

/// Reciprocal Rank Fusion (RRF). Merges the ranked result lists into one
/// without learned weights: `score(d) = Σ 1/(k + rank_i(d))`. Pure and
/// testable — operates only on the rank positions, never the raw scores
/// (which are incomparable between cosine similarity and BM25).
///
/// v1.11.0 "Associate": a third, optional graph-PPR list folds in with the
/// same formula. Pass `&[]` (or a graph-disabled empty list) for the
/// legacy two-retriever behavior.
///
/// Each input list MUST already be sorted best-first (rank 0 = strongest).
/// Records per-retriever ranks and the fused score in each result's provenance.
///
/// v1.13.3 "SourceFix": `leg` optionally restricts the fused output to one
/// retrieval leg (`vector` | `fts` | `graph`), keyed on the `SearchSource` tag
/// computed during fusion. `Both`-tagged hits survive every leg filter. Applied
/// BEFORE truncation so the leg-restricted top-k is honest. `None` = unrestricted.
pub fn rrf_fuse(
    vec_results: &[SearchResult],
    fts_results: &[SearchResult],
    graph_results: &[SearchResult],
    k: usize,
    limit: usize,
    leg: Option<LegFilter>,
) -> Vec<SearchResult> {
    use std::collections::{HashMap, HashSet};

    let vec_rank: HashMap<i64, usize> = vec_results
        .iter()
        .enumerate()
        .map(|(rank, r)| (r.id, rank))
        .collect();
    let fts_rank: HashMap<i64, usize> = fts_results
        .iter()
        .enumerate()
        .map(|(rank, r)| (r.id, rank))
        .collect();
    let graph_rank: HashMap<i64, usize> = graph_results
        .iter()
        .enumerate()
        .map(|(rank, r)| (r.id, rank))
        .collect();

    let mut seen: HashSet<i64> = HashSet::new();
    let mut fused: Vec<SearchResult> =
        Vec::with_capacity(vec_results.len() + fts_results.len() + graph_results.len());
    for r in vec_results
        .iter()
        .chain(fts_results.iter())
        .chain(graph_results.iter())
    {
        if !seen.insert(r.id) {
            continue;
        }
        let vr = vec_rank.get(&r.id).copied();
        let fr = fts_rank.get(&r.id).copied();
        let gr = graph_rank.get(&r.id).copied();
        let vc = vr.map(|rank| 1.0 / (k as f32 + rank as f32)).unwrap_or(0.0);
        let fc = fr.map(|rank| 1.0 / (k as f32 + rank as f32)).unwrap_or(0.0);
        let gc = gr.map(|rank| 1.0 / (k as f32 + rank as f32)).unwrap_or(0.0);
        let source = match (vc > 0.0, fc > 0.0, gc > 0.0) {
            (true, true, true) => SearchSource::Both,
            (true, true, false) => SearchSource::Both,
            (true, false, true) => SearchSource::Both,
            (false, true, true) => SearchSource::Both,
            (true, false, false) => SearchSource::Vector,
            (false, true, false) => SearchSource::Fts,
            (false, false, true) => SearchSource::Graph,
            (false, false, false) => continue,
        };
        let fused_score = vc + fc + gc;
        fused.push(SearchResult {
            id: r.id,
            score: fused_score,
            title: r.title.clone(),
            content: r.content.clone(),
            source: Some(source),
            flagged: r.flagged,
            untrusted: true,
            provenance: Provenance {
                vector_rank: vr,
                fts_rank: fr,
                graph_rank: gr,
                fused_score: Some(fused_score),
                rerank_score: None,
                rerank_truncated: false,
                prf_expanded: false,
                top_retrieval_mode: None,
                quality_assessment: None,
                prf_decision: None,
                retrieval_strategy: None,
            },
            snippet: None,
            evidence: None,
            observed_at: r.observed_at.clone(),
            authority: r.authority,
            assertion_kind: r.assertion_kind.clone(),
            confidence: r.confidence,
            expires_at: r.expires_at,
            pii: r.pii,
        });
    }

    // v1.13.3 "SourceFix": the retrieval-leg filter is a fusion concept. Apply
    // it on the fused candidate list — keyed on the `SearchSource` tag just
    // computed — BEFORE truncation, so `source:"vector"` returns the top-k
    // vector hits, not "the vector hits that happened to survive into the top-k
    // mixed set". Candidate lists are small (≤ k per leg + rescues), so this
    // retain is O(candidates) with no extra SQL.
    if let Some(leg) = leg {
        fused.retain(|r| leg_keeps(leg, r.source));
    }

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

/// v1.13.3 "SourceFix": does a fused hit tagged `source` survive the requested
/// `leg`? `Both` (appeared in ≥2 legs) survives every leg filter; a single-leg
/// tag survives only its own leg. Untagged hits (none post-fusion) are kept.
fn leg_keeps(leg: LegFilter, source: Option<SearchSource>) -> bool {
    match source {
        Some(SearchSource::Both) | None => true,
        Some(SearchSource::Vector) => leg == LegFilter::Vector,
        Some(SearchSource::Fts) => leg == LegFilter::Fts,
        Some(SearchSource::Graph) => leg == LegFilter::Graph,
    }
}

// ── Pseudo-relevance feedback (PRF) ─────────────────────────────────────────

/// A small stopword set for PRF term extraction.
const PRF_STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "must", "can", "this",
    "that", "these", "those", "i", "you", "he", "she", "it", "we", "they", "and", "or", "but",
    "in", "on", "at", "to", "for", "of", "with", "by", "from", "up", "about", "into", "through",
    "during", "before", "after", "above", "below", "not", "no", "as", "if", "than", "then", "so",
    "such", "also", "just", "very", "too", "more", "most",
];

/// Extract high-signal expansion terms from the top-K search results (PRF).
///
/// Deterministic, corpus-statistics-free expansion (a documented deliberate
/// simplification): document frequency across the top-K hits is the signal —
/// terms appearing in multiple relevant documents are likely topical.
///
/// ponytail: naive DF ranking, no IDF/corpus weighting (ceiling: a common word
/// that happens to appear across hits can slip in). Upgrade path: fold BM25
/// term weights from FTS5 once an inverted-doc-frequency source is wired in.
///
/// Negative-feedback guardrail: hits whose content trips the prompt-injection
/// screen are excluded from term extraction so untrusted text cannot steer the
/// expansion (anti-injection defense for the query-expansion path).
///
/// NOTE: this is the pure (DB-free) fallback used by unit tests and when no
/// connection is available. The live path prefers [`prf_extract_terms_fts`],
/// which weights terms via the FTS5 vocabulary table (v0.9.1 M4.1).
pub fn prf_extract_terms(
    hits: &[SearchResult],
    original_query: &str,
    max_terms: usize,
) -> Vec<String> {
    use std::collections::{HashMap, HashSet};

    let query_terms: HashSet<String> = original_query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    let stopwords: HashSet<&str> = PRF_STOPWORDS.iter().copied().collect();

    let mut tf: HashMap<String, usize> = HashMap::new();
    for hit in hits {
        // Negative-feedback: never mine expansion terms from quarantined
        // (`flagged`) or prompt-injection-like content.
        if hit.flagged || crate::contains_suspicious_pattern(&hit.content) {
            continue;
        }
        let mut seen_in_doc = HashSet::new();
        for word in hit.content.split(|c: char| !c.is_alphanumeric()) {
            let w = word.to_lowercase();
            if w.len() < 3 || w.len() > 30 {
                continue;
            }
            if stopwords.contains(w.as_str()) {
                continue;
            }
            if query_terms.contains(&w) {
                continue;
            }
            if !seen_in_doc.insert(w.clone()) {
                continue;
            }
            *tf.entry(w).or_default() += 1;
        }
    }

    let mut ranked: Vec<(String, usize)> = tf.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(max_terms).map(|(w, _)| w).collect()
}

/// Extract PRF expansion terms weighted by the FTS5 vocabulary index (v0.9.1
/// M4.1). This is the preferred live path: it ranks candidate terms by a
/// BM25-style score derived from the FTS5 `instance` vocab table (term × doc
/// × col × cnt), giving corpus-statistics-aware weighting instead of the naive
/// in-memory document frequency used by the DB-free [`prf_extract_terms`].
///
/// Signal per term: `sum over selected docs of cnt(term, doc) * idf(term)`,
/// where `idf(term) = ln(1 + total_docs / doc_freq(term))`. This favours terms
/// that (a) appear across multiple of the top-K hits (cross-document topicality)
/// and (b) are rare in the corpus as a whole (discriminative power). Query
/// terms, stopwords, and terms mined from quarantined/injection-flagged hits are
/// excluded — same guardrails as the pure variant.
///
/// Falls back to [`prf_extract_terms`] if the vocab table is missing or the
/// query errors (e.g. fresh DB where FTS5 hasn't been populated yet).
pub fn prf_extract_terms_fts(
    conn: &Connection,
    hits: &[SearchResult],
    original_query: &str,
    max_terms: usize,
) -> Vec<String> {
    use std::collections::HashSet;

    // Collect rowids of safe (non-flagged, non-injection) hits only.
    let safe_ids: Vec<i64> = hits
        .iter()
        .filter(|h| !h.flagged && !crate::contains_suspicious_pattern(&h.content))
        .map(|h| h.id)
        .collect();
    if safe_ids.is_empty() {
        return prf_extract_terms(hits, original_query, max_terms);
    }

    let query_terms: HashSet<String> = original_query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    let stopwords: HashSet<&str> = PRF_STOPWORDS.iter().copied().collect();

    // Build an `IN (...)` placeholder list for the selected rowids.
    let placeholders: String = (0..safe_ids.len())
        .map(|i| {
            if i + 1 == safe_ids.len() {
                format!("?{}", i + 1)
            } else {
                format!("?{}, ", i + 1)
            }
        })
        .collect();

    // Per-term: sum of in-doc counts (within the selected hits) and corpus doc
    // frequency (from the vocab table aggregated across ALL docs).
    let sql = format!(
        "WITH selected AS (
             SELECT term, SUM(cnt) AS local_cnt
             FROM knowledge_fts_vocab
             WHERE col = 'content'
               AND rowid IN ({placeholders})
             GROUP BY term
         ),
         corpus AS (
             SELECT term, COUNT(DISTINCT rowid) AS df
             FROM knowledge_fts_vocab
             WHERE col = 'content'
             GROUP BY term
         )
         SELECT s.term, s.local_cnt, c.df
         FROM selected s
         JOIN corpus c ON c.term = s.term"
    );

    let total_docs: f64 = conn
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap_or(1) as f64;

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("PRF FTS5 vocab query failed ({e}); falling back to DF");
            return prf_extract_terms(hits, original_query, max_terms);
        }
    };
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(safe_ids.len());
    for id in &safe_ids {
        params_vec.push(Box::new(*id));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        let term: String = row.get(0)?;
        let local_cnt: i64 = row.get(1)?;
        let df: i64 = row.get(2)?;
        Ok((term, local_cnt, df))
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("PRF FTS5 vocab read failed ({e}); falling back to DF");
            return prf_extract_terms(hits, original_query, max_terms);
        }
    };

    let mut weighted: Vec<(String, f64)> = Vec::new();
    for row in rows.flatten() {
        let (term, local_cnt, df) = row;
        let t = term.to_lowercase();
        if t.len() < 3 || t.len() > 30 {
            continue;
        }
        if stopwords.contains(t.as_str()) || query_terms.contains(&t) {
            continue;
        }
        let idf = (1.0 + total_docs / df.max(1) as f64).ln();
        weighted.push((t, local_cnt as f64 * idf));
    }

    if weighted.is_empty() {
        return prf_extract_terms(hits, original_query, max_terms);
    }
    weighted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    weighted
        .into_iter()
        .take(max_terms)
        .map(|(w, _)| w)
        .collect()
}

/// PRF confidence policy (deterministic, calibrated on rank agreement rather
/// than on the raw RRF magnitude, which is not a relevance probability).
///
/// Expansion is entered only when the top pass-1 result appears in BOTH the
/// dense and lexical lists (`SearchSource::Both`) within a bounded rank — i.e.
/// the two independent retrievers agree — and there are at least two results to
/// mine. This replaces the old `PRF_MIN_SCORE` comparison against an RRF score,
/// which was unreachable (top RRF ≈ 2/60 ≈ 0.033 ≪ 0.3).
pub fn prf_should_expand(pass1: &[SearchResult], cfg: &PrfConfig) -> bool {
    if !cfg.enabled || pass1.len() < 2 {
        return false;
    }
    let Some(top) = pass1.first() else {
        return false;
    };
    let agreed = top.source == Some(SearchSource::Both);
    let vr_ok = top
        .provenance
        .vector_rank
        .map(|r| r <= cfg.max_rank)
        .unwrap_or(false);
    let fr_ok = top
        .provenance
        .fts_rank
        .map(|r| r <= cfg.max_rank)
        .unwrap_or(false);
    agreed && vr_ok && fr_ok
}

/// Runtime PRF configuration, read from env with documented fallbacks.
#[derive(Debug, Clone)]
pub struct PrfConfig {
    pub enabled: bool,
    pub depth: usize,
    pub terms: usize,
    pub max_rank: usize,
}

impl Default for PrfConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            depth: 10,
            terms: 5,
            max_rank: 5,
        }
    }
}

impl PrfConfig {
    /// Read PRF config from env: `PRF_ENABLED`, `PRF_DEPTH`, `PRF_TERMS`,
    /// `PRF_MAX_RANK`. Invalid values fall back to the defaults.
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            enabled: env_bool("PRF_ENABLED", d.enabled),
            depth: env_usize("PRF_DEPTH", d.depth).clamp(1, 100),
            terms: env_usize("PRF_TERMS", d.terms).clamp(1, 50),
            max_rank: env_usize("PRF_MAX_RANK", d.max_rank).clamp(0, 100),
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Fuse the original-query (pass-1) and expanded-query (pass-2) ranked lists
/// with RRF over their rank positions. This protects original-query matches:
/// a document strong for the unexpanded query keeps its pass-1 rank
/// contribution even if query drift pushed it down in pass-2. Deterministic;
/// no comparison of incomparable raw scores across passes.
/// Shared two-pass RRF fuse (v1.12.0 "Discern": also used by the graph
/// rescue, so the `prf_expanded` flag is NOT set here — see
/// [`fuse_prf_passes`]). Deterministic: dedup by id, per-pass RRF
/// contributions `1/(k+rank)` summed, original-pass order wins ties.
fn fuse_pass_lists(
    pass1: &[SearchResult],
    pass2: &[SearchResult],
    k: usize,
    limit: usize,
) -> Vec<SearchResult> {
    use std::collections::{HashMap, HashSet};

    let p1_rank: HashMap<i64, usize> = pass1.iter().enumerate().map(|(r, x)| (x.id, r)).collect();
    let p2_rank: HashMap<i64, usize> = pass2.iter().enumerate().map(|(r, x)| (x.id, r)).collect();

    let mut seen: HashSet<i64> = HashSet::new();
    let mut fused: Vec<SearchResult> = Vec::with_capacity(pass1.len() + pass2.len());
    // Original-query pass listed first so its metadata/order wins ties.
    for r in pass1.iter().chain(pass2.iter()) {
        if !seen.insert(r.id) {
            continue;
        }
        let c1 = p1_rank
            .get(&r.id)
            .map(|rank| 1.0 / (k as f32 + *rank as f32))
            .unwrap_or(0.0);
        let c2 = p2_rank
            .get(&r.id)
            .map(|rank| 1.0 / (k as f32 + *rank as f32))
            .unwrap_or(0.0);
        let mut item = r.clone();
        item.score = c1 + c2;
        item.provenance.fused_score = Some(c1 + c2);
        fused.push(item);
    }
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

/// Two-pass RRF fuse for PRF expansion: identical to [`fuse_pass_lists`] plus
/// the `prf_expanded` provenance flag (the second pass WAS query-expanded).
pub fn fuse_prf_passes(
    pass1: &[SearchResult],
    pass2: &[SearchResult],
    k: usize,
    limit: usize,
) -> Vec<SearchResult> {
    let mut fused = fuse_pass_lists(pass1, pass2, k, limit);
    for r in &mut fused {
        r.provenance.prf_expanded = true;
    }
    fused
}

/// v1.12.0 "Discern": complexity-gated graph activation (arXiv:2602.03578).
/// The graph leg is auto-engaged ONLY when the calibrated estimator says the
/// query is too weak to answer (`ClarifyQuery`) and the graph leg did not
/// already run in pass 1 (`graph_enabled`). `enabled` is the
/// `BRAIN_GRAPH_RESCUE_ENABLED` kill switch. Pure — the caller decides.
pub fn should_attempt_graph_rescue(
    recommendation: Option<Recommendation>,
    graph_enabled: bool,
    enabled: bool,
) -> bool {
    enabled && !graph_enabled && matches!(recommendation, Some(Recommendation::ClarifyQuery))
}

// ── Vector + lexical retrieval ──────────────────────────────────────────────

/// v1.14.0 "Gate": append the SQL for decay, memory_kind, and access-scope
/// filters to a retriever query, pushing any bound params in order. Shared by
/// both retrievers so the filters can never drift between vec0 and FTS. Pure
/// SQL construction; params are appended in SQL-parameter order.
fn push_gate_filters(
    sql: &mut String,
    params_vec: &mut Vec<Box<dyn rusqlite::ToSql>>,
    filters: &SearchFilters,
) {
    if !filters.include_decayed {
        if filters.retention_days.is_empty() {
            sql.push_str(" AND (k.expires_at IS NULL OR k.expires_at >= ?)");
            params_vec.push(Box::new(filters.now_unix));
        } else {
            // v1.17.1 M2: exclude chunks whose *effective* expiry (own
            // `expires_at`, else the kind-default derived from created_at) is
            // in the past. Dynamic per-kind disjunction — each kind with a
            // policy contributes `(node_kind = ?kind AND created_unix + days*86400 < ?now)`.
            let kinds: Vec<String> = filters
                .retention_days
                .iter()
                .map(|_| "k.node_kind = ? AND strftime('%s', COALESCE(k.created_at, '1970-01-01 00:00:00')) + ? * 86400 < ?".to_string())
                .collect();
            sql.push_str(" AND (");
            sql.push_str("k.expires_at IS NOT NULL AND k.expires_at >= ?");
            sql.push_str(" OR (k.expires_at IS NULL AND NOT (");
            sql.push_str(&kinds.join(" OR "));
            sql.push_str(")) )");
            params_vec.push(Box::new(filters.now_unix));
            for (kind, days) in &filters.retention_days {
                params_vec.push(Box::new(kind.clone()));
                params_vec.push(Box::new(*days));
                params_vec.push(Box::new(filters.now_unix));
            }
        }
    }
    if let Some(kind) = &filters.memory_kind {
        sql.push_str(" AND k.node_kind = ?");
        params_vec.push(Box::new(kind.clone()));
    }
    if let Some(scopes) = &filters.access_scopes {
        let ph = std::iter::repeat_n("?", scopes.len())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND k.access_scope IN ({ph})"));
        for s in scopes {
            params_vec.push(Box::new(s.clone()));
        }
    }
}

pub fn vec0_knn(
    conn: &Connection,
    query_vec: &[f32],
    k: usize,
    filters: &SearchFilters,
) -> Result<Vec<SearchResult>> {
    let mut sql = String::from(
        "SELECT k.id, k.title, k.content, v.distance, k.flagged,
                k.observed_at, k.authority, k.assertion_kind, k.confidence,
                k.expires_at, k.pii
         FROM vec_knowledge v
         JOIN knowledge k ON k.id = v.knowledge_id
         LEFT JOIN source_revisions sr ON k.revision_id = sr.id
         WHERE v.embedding_int8 MATCH vec_quantize_int8(?1, 'unit')
           AND v.k = ?2",
    );
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(query_vec.as_bytes().to_vec()), Box::new(k as i64)];
    if !filters.sources.is_empty() {
        let ph = std::iter::repeat_n("?", filters.sources.len())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND v.source IN ({ph})"));
        for s in &filters.sources {
            params_vec.push(Box::new(s.clone()));
        }
    } else if let Some(src) = &filters.source {
        sql.push_str(" AND v.source = ?");
        params_vec.push(Box::new(src.clone()));
    }
    if let Some(domain) = &filters.domain {
        sql.push_str(" AND k.domain = ?");
        params_vec.push(Box::new(domain.clone()));
    }
    if let Some(since) = &filters.since {
        sql.push_str(" AND v.created_at > ?");
        params_vec.push(Box::new(since.clone()));
    }
    // v0.9.7 Guard: exclude quarantined rows from retrieval by default so
    // flagged/prompt-injection content never reaches the agent. Review paths
    // set `include_flagged = true`. Historical mode (as_of) still honors the
    // quarantine boundary unless the caller explicitly opts into flagged rows.
    if !filters.include_flagged {
        sql.push_str(" AND k.flagged = 0");
    }
    // v0.9.8 "Evidence" M1.2: historical point-in-time recall. In historical
    // mode we return chunks whose revision was active at `as_of` — i.e. the
    // chunk's revision_id points at a revision fetched at/before as_of AND not
    // yet superseded by a revision fetched strictly after as_of. The join to
    // source_revisions makes that derivable from `fetched_at`.
    if let Some(as_of) = &filters.as_of {
        sql.push_str(
            " AND sr.fetched_at <= ? AND NOT EXISTS (\
                SELECT 1 FROM source_revisions sr2 \
                WHERE sr2.source_id = sr.source_id \
                  AND sr2.state = 'active' \
                  AND sr2.fetched_at > sr.fetched_at \
                  AND sr2.fetched_at <= ?)",
        );
        params_vec.push(Box::new(as_of.clone()));
        params_vec.push(Box::new(as_of.clone()));
    }
    // v1.4.0 "Calibrate" M1 + v1.6.0 "Reconcile" fix: bi-temporal valid-time
    // filter. Two modes:
    //   - `at` = None (default recall): exclude EXPIRED chunks only
    //     (valid_to IS NULL). v1.6.0 fix — before this, superseded chunks
    //     were still returned by default recall because the filter only fired
    //     when `at` was set. That broke the Reconcile exit criterion.
    //   - `at` = Some(t): full bi-temporal window
    //     (valid_from <= t AND (valid_to IS NULL OR valid_to > t)).
    //     Graphiti valid_at/invalid_at semantics (Context7 2026-08-01).
    match &filters.at {
        Some(at) => {
            sql.push_str(
                " AND (k.valid_from IS NULL OR k.valid_from <= ?) \
                   AND (k.valid_to IS NULL OR k.valid_to > ?)",
            );
            params_vec.push(Box::new(at.clone()));
            params_vec.push(Box::new(at.clone()));
        }
        None => {
            // Default recall: only current facts. A chunk with valid_to set
            // has been superseded (v1.6.0 resolve_supersession) and must not
            // appear unless the caller asks for historical `?at=` recall.
            sql.push_str(" AND k.valid_to IS NULL");
        }
    }
    push_gate_filters(&mut sql, &mut params_vec, filters);
    sql.push_str(" ORDER BY v.distance");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let results = stmt
        .query_map(param_refs.as_slice(), |row| {
            let distance: f32 = row.get(3)?;
            let score = (1.0 - distance).clamp(0.0, 1.0);
            let mut r = SearchResult::raw(row.get(0)?, score, row.get(1)?, row.get(2)?)
                .with_flagged(row.get(4)?);
            r.observed_at = row.get(5)?;
            r.authority = row.get::<_, Option<f64>>(6)?.map(|a| a as f32);
            r.assertion_kind = row.get(7)?;
            r.confidence = row.get::<_, Option<f64>>(8)?.map(|c| c as f32);
            r.expires_at = row.get(9)?;
            r.pii = row.get::<_, i64>(10)? != 0;
            Ok(r)
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

/// FTS5 BM25 lexical search.
fn fts_search(
    conn: &Connection,
    q: &str,
    k: usize,
    filters: &SearchFilters,
) -> Result<Vec<SearchResult>> {
    let mut sql = String::from(
        "SELECT k.id, k.title, k.content, bm25(knowledge_fts) AS score, k.flagged,
                k.observed_at, k.authority, k.assertion_kind, k.confidence,
                k.expires_at, k.pii
         FROM knowledge_fts
         JOIN knowledge k ON k.id = knowledge_fts.rowid
         LEFT JOIN source_revisions sr ON k.revision_id = sr.id
         WHERE knowledge_fts MATCH ?1",
    );
    // Lexical precision: when a structured `lex` query is supplied, the FTS
    // retriever matches on it instead of the bare (often semantic) query.
    let fts_q = filters.lex.as_deref().unwrap_or(q);
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(fts_q.to_string())];
    if !filters.sources.is_empty() {
        let ph = std::iter::repeat_n("?", filters.sources.len())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND k.source IN ({ph})"));
        for s in &filters.sources {
            params_vec.push(Box::new(s.clone()));
        }
    } else if let Some(src) = &filters.source {
        sql.push_str(" AND k.source = ?");
        params_vec.push(Box::new(src.clone()));
    }
    if let Some(domain) = &filters.domain {
        sql.push_str(" AND k.domain = ?");
        params_vec.push(Box::new(domain.clone()));
    }
    if let Some(since) = &filters.since {
        sql.push_str(" AND k.created_at > ?");
        params_vec.push(Box::new(since.clone()));
    }
    // v0.9.7 Guard: exclude quarantined rows from lexical retrieval too.
    if !filters.include_flagged {
        sql.push_str(" AND k.flagged = 0");
    }
    // v0.9.8 "Evidence" M1.2: historical point-in-time recall (see vec0_knn).
    if let Some(as_of) = &filters.as_of {
        sql.push_str(
            " AND sr.fetched_at <= ? AND NOT EXISTS (\
                SELECT 1 FROM source_revisions sr2 \
                WHERE sr2.source_id = sr.source_id \
                  AND sr2.state = 'active' \
                  AND sr2.fetched_at > sr.fetched_at \
                  AND sr2.fetched_at <= ?)",
        );
        params_vec.push(Box::new(as_of.clone()));
        params_vec.push(Box::new(as_of.clone()));
    }
    // v1.4.0 + v1.6.0 fix: bi-temporal valid-time filter (see vec0_knn).
    match &filters.at {
        Some(at) => {
            sql.push_str(
                " AND (k.valid_from IS NULL OR k.valid_from <= ?) \
                   AND (k.valid_to IS NULL OR k.valid_to > ?)",
            );
            params_vec.push(Box::new(at.clone()));
            params_vec.push(Box::new(at.clone()));
        }
        None => {
            sql.push_str(" AND k.valid_to IS NULL");
        }
    }
    push_gate_filters(&mut sql, &mut params_vec, filters);
    sql.push_str(" ORDER BY score LIMIT ?");
    params_vec.push(Box::new(k as i64));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let results: Vec<SearchResult> = stmt
        .query_map(param_refs.as_slice(), |row| {
            let bm25_score: f64 = row.get(3)?;
            let mut r = SearchResult::raw(
                row.get(0)?,
                (-bm25_score as f32).max(0.0),
                row.get(1)?,
                row.get(2)?,
            )
            .with_flagged(row.get(4)?);
            r.observed_at = row.get(5)?;
            r.authority = row.get::<_, Option<f64>>(6)?.map(|a| a as f32);
            r.assertion_kind = row.get(7)?;
            r.confidence = row.get::<_, Option<f64>>(8)?.map(|c| c as f32);
            r.expires_at = row.get(9)?;
            r.pii = row.get::<_, i64>(10)? != 0;
            Ok(r)
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

/// Legacy brute-force cosine scan (fallback when vec0 is empty).
fn perform_search_legacy(
    conn: &Connection,
    query_vec: &[f32],
    k: usize,
) -> Result<Vec<SearchResult>> {
    let total_count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))?;
    let mut results: Vec<SearchResult> = Vec::with_capacity(k * 2);
    let mut offset = 0;
    while offset < total_count as usize {
        let mut stmt = conn.prepare(
            "SELECT k.id, k.title, k.content, e.vector
             FROM knowledge k JOIN embeddings e ON k.id = e.knowledge_id
             LIMIT ? OFFSET ?",
        )?;
        let batch: Vec<SearchResult> = stmt
            .query_map(params![SEARCH_BATCH_SIZE as i64, offset as i64], |row| {
                let vec_str: String = row.get(3)?;
                let db_vec: Vec<f32> = serde_json::from_str(&vec_str).unwrap_or_default();
                Ok(SearchResult::raw(
                    row.get(0)?,
                    cosine_sim(query_vec, &db_vec),
                    row.get(1)?,
                    row.get(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        results.extend(batch);
        if results.len() >= k * 10 {
            break;
        }
        offset += SEARCH_BATCH_SIZE;
    }
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(k);
    Ok(results)
}

/// Result of the concurrent vector/FTS retrieval scope: each stage yields its
/// results plus the stage's own latency in milliseconds.
type ScopedRetrieval = Result<
    (
        (Vec<SearchResult>, f32),
        (Vec<SearchResult>, f32),
        (Vec<SearchResult>, f32),
    ),
    anyhow::Error,
>;

/// Hybrid search: run vector (vec0 KNN) and lexical (FTS5 BM25) retrieval
/// concurrently on independent read connections, then fuse with RRF.
pub fn perform_search(
    pool: &Pool,
    model: &StaticModel,
    q: String,
    k: usize,
    filters: &SearchFilters,
) -> Result<Vec<SearchResult>> {
    Ok(perform_search_traced(pool, model, q, k, filters, &mut SearchTelemetry::default())?.0)
}

/// Same as [`perform_search`] but records per-stage latency into `tel` and
/// returns it alongside the results.
pub fn perform_search_traced(
    pool: &Pool,
    model: &StaticModel,
    q: String,
    k: usize,
    filters: &SearchFilters,
    tel: &mut SearchTelemetry,
) -> Result<(Vec<SearchResult>, SearchTelemetry)> {
    let t_embed = Instant::now();
    // Structured query: prefer the `hyde`/`vec` embedding query so a caller can
    // supply a hypothetical answer or semantic intent; fall back to the bare q.
    let embed_q = filters.embedding_query.clone().unwrap_or_else(|| q.clone());
    let v = model
        .encode(std::slice::from_ref(&embed_q))
        .into_iter()
        .next()
        .context("Query encoding failed")?;
    tel.embed_ms = t_embed.elapsed().as_secs_f32() * 1000.0;
    tel.intent = filters.intent.clone();

    // Validate/normalize the temporal filters up front so an invalid `since`
    // or `at` fails fast instead of producing a silently wrong lexical
    // comparison against SQLite's fixed-width `YYYY-MM-DD HH:MM:SS` format.
    // Both are normalized independently: a caller may set either, both, or
    // neither. v1.4.0 "Calibrate" M1: `at` is the bi-temporal valid-time
    // filter (distinct from `as_of` transaction-time recall).
    let normalized_since = filters.since.as_deref().map(normalize_since).transpose()?;
    let normalized_at = filters.at.as_deref().map(normalize_since).transpose()?;
    let filters = if normalized_since.is_some() || normalized_at.is_some() {
        SearchFilters {
            source: filters.source.clone(),
            source_leg: filters.source_leg,
            sources: filters.sources.clone(),
            since: normalized_since,
            domain: filters.domain.clone(),
            lex: filters.lex.clone(),
            embedding_query: filters.embedding_query.clone(),
            intent: filters.intent.clone(),
            profile: filters.profile.clone(),
            include_flagged: false,
            as_of: filters.as_of.clone(),
            evidence: filters.evidence,
            freshness_tiebreak: filters.freshness_tiebreak,
            at: normalized_at,
            graph: filters.graph,
            include_decayed: filters.include_decayed,
            now_unix: filters.now_unix,
            memory_kind: filters.memory_kind.clone(),
            min_relevance: filters.min_relevance.clone(),
            access_scopes: filters.access_scopes.clone(),
            retention_days: filters.retention_days.clone(),
        }
    } else {
        filters.clone()
    };

    let overfetch = k.max(RRF_OVERFETCH);

    // Vector and FTS retrieval run concurrently on independent pooled read
    // connections (rusqlite Connection is not Sync, so each stage owns its own).
    let vec_pool = pool.clone();
    let fts_pool = pool.clone();
    let vfilters = filters.clone();
    let ffilters = filters.clone();
    let vq = v.clone();
    let fq = q.clone();

    let t_par = Instant::now();
    let graph_q = q.clone();
    let graph_filters = filters.clone();
    let graph_enabled = filters.graph;
    let graph_pool = pool.clone();
    let ((vec_res, _), (fts_res, _), (graph_res, graph_ms)) =
        std::thread::scope(|scope| -> ScopedRetrieval {
            let vh = scope.spawn(move || -> Result<(Vec<SearchResult>, f32)> {
                let conn = vec_pool.get().context("DB connection failed (vector)")?;
                let t_vec = Instant::now();
                let vec_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
                    .unwrap_or(0);
                let res = if vec_count == 0 {
                    perform_search_legacy(&conn, &vq, overfetch)
                } else {
                    vec0_knn(&conn, &vq, overfetch, &vfilters)
                }?;
                Ok((res, t_vec.elapsed().as_secs_f32() * 1000.0))
            });
            let fh = scope.spawn(move || -> (Vec<SearchResult>, f32) {
                let t_fts = Instant::now();
                let res = match fts_pool.get() {
                    Ok(conn) => fts_search(&conn, &fq, overfetch, &ffilters).unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                (res, t_fts.elapsed().as_secs_f32() * 1000.0)
            });
            // v1.11.0 "Associate": graph-PPR retriever as a third RRF leg. Runs
            // concurrently on its own pooled read connection (same pattern as the
            // vector/FTS stages) so the disabled path never pays for the graph.
            // Deterministic, zero-token, no embeddings.
            let gh = if graph_enabled {
                let gpool = graph_pool.clone();
                let gfilters = graph_filters.clone();
                let gq = graph_q.clone();
                Some(scope.spawn(move || -> (Vec<SearchResult>, f32) {
                    let t_g = Instant::now();
                    let res = match gpool.get() {
                        Ok(conn) => graph_retrieve(&conn, &gq, overfetch, gfilters.include_flagged)
                            .unwrap_or_default(),
                        Err(_) => Vec::new(),
                    };
                    (res, t_g.elapsed().as_secs_f32() * 1000.0)
                }))
            } else {
                None
            };
            let (vec_res, vec_ms) = vh.join().unwrap_or_else(|_| Ok((Vec::new(), 0.0)))?;
            let (fts_res, fts_ms) = fh.join().unwrap_or((Vec::new(), 0.0));
            let (graph_res, graph_ms) = match gh {
                Some(h) => h.join().unwrap_or((Vec::new(), 0.0)),
                None => (Vec::new(), 0.0),
            };
            tel.retrieval_ms_vec = vec_ms;
            tel.retrieval_ms_fts = fts_ms;
            Ok(((vec_res, vec_ms), (fts_res, fts_ms), (graph_res, graph_ms)))
        })?;
    let mut vec_results = vec_res;
    let mut fts_results = fts_res;
    let mut graph_results = graph_res;
    let par_ms = t_par.elapsed().as_secs_f32() * 1000.0;
    // Concurrent: attribute wall time to all enabled stages.
    tel.vector_ms = par_ms;
    tel.fts_ms = par_ms;
    tel.graph_ms = if graph_enabled { graph_ms } else { 0.0 };
    tel.vec_candidates = vec_results.len();
    tel.fts_candidates = fts_results.len();
    tel.graph_candidates = graph_results.len();

    let t_fuse = Instant::now();
    let mut fused = if fts_results.is_empty() && graph_results.is_empty() {
        for (rank, r) in vec_results.iter_mut().enumerate() {
            r.source = Some(SearchSource::Vector);
            r.provenance.vector_rank = Some(rank);
        }
        vec_results.truncate(k);
        vec_results
    } else if vec_results.is_empty() && graph_results.is_empty() {
        for (rank, r) in fts_results.iter_mut().enumerate() {
            r.source = Some(SearchSource::Fts);
            r.provenance.fts_rank = Some(rank);
        }
        fts_results.truncate(k);
        fts_results
    } else if vec_results.is_empty() && fts_results.is_empty() {
        // Graph-only retrieval (sparse/unlinked corpus): tag and truncate.
        for (rank, r) in graph_results.iter_mut().enumerate() {
            r.source = Some(SearchSource::Graph);
            r.provenance.graph_rank = Some(rank);
        }
        graph_results.truncate(k);
        graph_results
    } else {
        rrf_fuse(
            &vec_results,
            &fts_results,
            &graph_results,
            RRF_K,
            k,
            filters.source_leg,
        )
    };
    // v1.13.3 "SourceFix": the mixed-leg path is filtered inside `rrf_fuse`
    // (before truncation). The single-leg fast paths above are tagged uniformly,
    // so this retain is a no-op there when the leg matches and empties it
    // otherwise — e.g. `source:"fts"` on a query where only the vector leg ran.
    if let Some(leg) = filters.source_leg {
        fused.retain(|r| leg_keeps(leg, r.source));
    }
    tel.fusion_ms = t_fuse.elapsed().as_secs_f32() * 1000.0;
    tel.fused_count = fused.len();

    // Set top_retrieval_mode on the best result for provenance.
    if let Some(top) = fused.first_mut() {
        top.provenance.top_retrieval_mode = top.source;
    }

    // v0.9.8 M2.4: deterministic freshness tie-break. Within equal fused-score
    // buckets, prefer newer `observed_at` then higher `authority`. This is a
    // post-fusion stable sort that NEVER reorders across distinct scores, so it
    // cannot distort lexical/vector retrieval semantics (plan forbids that).
    // ponytail: stable sort over ≤ k.max(RRF_OVERFETCH) items — O(n log n),
    // bounded and cheap.
    if filters.freshness_tiebreak {
        use std::cmp::Ordering;
        fused.sort_by(|a, b| {
            // Equal score → apply tie-break; otherwise keep RRF order.
            if (a.score - b.score).abs() < 1e-6 {
                let by_obs = cmp_observed(b.observed_at.as_deref(), a.observed_at.as_deref());
                if by_obs != Ordering::Equal {
                    return by_obs;
                }
                let ba = b.authority.unwrap_or(0.8);
                let aa = a.authority.unwrap_or(0.8);
                return ba.partial_cmp(&aa).unwrap_or(Ordering::Equal);
            }
            Ordering::Equal
        });
    }

    Ok((fused, tel.clone()))
}

/// Compare two RFC3339/DB timestamps for freshness ordering. Unknown/absent
/// timestamps sort last (older) so explicitly-stamped evidence wins.
fn cmp_observed(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    let pa = parse_obs(a);
    let pb = parse_obs(b);
    pb.cmp(&pa) // descending: newer first
}

fn parse_obs(s: Option<&str>) -> i64 {
    let Some(s) = s else { return 0 };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().timestamp();
    }
    0
}

/// Two-pass hybrid search with PRF query expansion.
/// Returns the results plus per-stage [`SearchTelemetry`] (for `explain`).
pub fn perform_search_with_prf(
    pool: &Pool,
    model: &StaticModel,
    q: String,
    k: usize,
    filters: &SearchFilters,
) -> Result<(Vec<SearchResult>, SearchTelemetry)> {
    let cfg = PrfConfig::from_env();
    let quality_cfg = QualityConfig::from_env();
    let estimator = HeuristicEstimator::new(quality_cfg);
    let mut tel = SearchTelemetry {
        embedding_query: filters.embedding_query.clone().or_else(|| Some(q.clone())),
        ..Default::default()
    };

    // Candidate window for PRF expansion (equals k when no rerank tier).
    let window = k;
    let prf_depth = window.max(cfg.depth);
    let (mut pass1, _) =
        perform_search_traced(pool, model, q.clone(), prf_depth, filters, &mut tel)?;

    // Assess retrieval quality after initial hybrid retrieval.
    let assessment = estimator.assess(&q, &pass1, &tel);
    tel.confidence = assessment.confidence.score;
    tel.recommendation = Some(assessment.recommendation);

    // Attach quality assessment to top result for provenance.
    if let Some(top) = pass1.first_mut() {
        top.provenance.quality_assessment = Some(assessment.clone());
        top.provenance.retrieval_strategy = Some(match assessment.recommendation {
            Recommendation::Return => RetrievalStrategy::Hybrid,
            Recommendation::RunPrf => RetrievalStrategy::HybridPrf,
            Recommendation::RunReranker => RetrievalStrategy::Hybrid,
            Recommendation::IncreaseTopK => RetrievalStrategy::Hybrid,
            Recommendation::ClarifyQuery => RetrievalStrategy::Hybrid,
        });
    }

    // Record PRF decision for observability.
    let prf_decision = match assessment.recommendation {
        Recommendation::Return => PrfDecision::Disabled,
        Recommendation::RunPrf => {
            let gap = assessment.confidence.gap;
            if gap < 0.023 {
                PrfDecision::SmallGap { score_gap: gap }
            } else {
                PrfDecision::Expanded {
                    confidence: assessment.confidence.score,
                    terms: cfg.terms,
                }
            }
        }
        Recommendation::IncreaseTopK => PrfDecision::WeakAgreement {
            agreement_at_k: assessment.confidence.overlap as usize,
        },
        Recommendation::ClarifyQuery => PrfDecision::LowConfidence {
            confidence: assessment.confidence.score,
        },
        Recommendation::RunReranker => PrfDecision::Expanded {
            confidence: assessment.confidence.score,
            terms: cfg.terms,
        },
    };
    if let Some(top) = pass1.first_mut() {
        top.provenance.prf_decision = Some(prf_decision);
    }

    let t_prf = Instant::now();
    let (mut candidates, final_strategy) = match assessment.recommendation {
        Recommendation::Return | Recommendation::RunReranker | Recommendation::IncreaseTopK => {
            let mut p1 = pass1;
            p1.truncate(window);
            (p1, RetrievalStrategy::Hybrid)
        }
        Recommendation::ClarifyQuery => {
            // v1.12.0 "Discern": complexity-gated graph activation. The
            // calibrated estimator said this query is too weak to answer —
            // but the graph leg (the associative-memory retriever) never ran
            // (caller did not set `graph`). Run one bounded graph pass and
            // fuse; strictly additive — this path would otherwise return
            // zero hits (v1.5.0 abstention), so any result is a rescue and
            // an empty result keeps the abstention contract intact.
            let mut p1 = pass1;
            p1.truncate(window);
            if should_attempt_graph_rescue(
                Some(assessment.recommendation),
                filters.graph,
                crate::config::brain_graph_rescue_enabled(),
            ) {
                let mut rescue_filters = filters.clone();
                rescue_filters.graph = true;
                let (pass2, _) = perform_search_traced(
                    pool,
                    model,
                    q.clone(),
                    prf_depth,
                    &rescue_filters,
                    &mut tel,
                )?;
                tel.graph_rescued = true;
                let fused = fuse_pass_lists(&p1, &pass2, RRF_K, window);
                (fused, RetrievalStrategy::HybridGraph)
            } else {
                (p1, RetrievalStrategy::Hybrid)
            }
        }
        Recommendation::RunPrf => {
            // v0.9.1 M4.1: prefer FTS5-vocabulary-weighted term extraction
            // (corpus-aware BM25-style signal). Falls back to the pure DF variant
            // when the vocab table is unavailable or the query errors.
            let expansion = match pool.get() {
                Ok(conn) => prf_extract_terms_fts(&conn, &pass1, &q, cfg.terms),
                Err(_) => prf_extract_terms(&pass1, &q, cfg.terms),
            };
            if expansion.is_empty() {
                let mut p1 = pass1;
                p1.truncate(window);
                (p1, RetrievalStrategy::Hybrid)
            } else {
                let expanded_query = format!("{} {}", q, expansion.join(" "));
                let (pass2, _) = perform_search_traced(
                    pool,
                    model,
                    expanded_query,
                    prf_depth,
                    filters,
                    &mut tel,
                )?;
                // Deterministic fusion protecting original-query matches.
                let fused = fuse_prf_passes(&pass1, &pass2, RRF_K, window);
                (fused, RetrievalStrategy::HybridPrf)
            }
        }
    };
    tel.prf_ms = t_prf.elapsed().as_secs_f32() * 1000.0;

    // Update retrieval_strategy on top result based on final pipeline.
    if let Some(top) = candidates.first_mut() {
        top.provenance.retrieval_strategy = Some(final_strategy);
    }

    let t_rr = Instant::now();
    candidates.truncate(k);
    tel.rerank_ms = t_rr.elapsed().as_secs_f32() * 1000.0;

    // Final strategy is already resolved; no reranker tier is wired in.
    if let Some(top) = candidates.first_mut() {
        top.provenance.retrieval_strategy = Some(final_strategy);
    }

    tracing::debug!(
        embed_ms = tel.embed_ms,
        vector_ms = tel.vector_ms,
        fts_ms = tel.fts_ms,
        fusion_ms = tel.fusion_ms,
        prf_ms = tel.prf_ms,
        rerank_ms = tel.rerank_ms,
        confidence = tel.confidence,
        recommendation = ?tel.recommendation,
        strategy = ?final_strategy,
        "search telemetry"
    );
    Ok((candidates, tel))
}

// ── v0.9.5 M1 structured query document ───────────────────────────────────
pub mod query;

// ── Retrieval quality estimation (QPP) ────────────────────────────────────
pub mod quality;

// ── v1.4.0 "Calibrate" M2: budgeted monotone submodular evidence packing ──
pub mod packing;

// ── v1.11.0 "Associate": deterministic PPR over the existing KG ──────────
pub mod graph_ppr;

#[cfg(test)]
mod tests;
