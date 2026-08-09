//! v0.9.5 M1 — versioned structured query document.
//!
//! One contract shared by `GET /search` and `POST /recall`. A plain-text
//! query remains backwards compatible: `QueryDoc::from_text` treats the bare
//! string as the embedding/lexical fallback so existing callers (the OpenClaw
//! plugin) are untouched.

use serde::Deserialize;

use super::{normalize_since, SearchFilters};

/// Schema version of the query document. Bump when the wire shape changes so
/// the server can reject unsupported clients instead of mis-parsing them.
const QUERY_DOC_VERSION: u8 = 1;

/// Top-level structured query. Every field defaults, so an empty `{}` or a
/// bare `q` string both parse.
#[derive(Debug, Clone, Deserialize)]
pub struct QueryDoc {
    /// Schema version. Unknown versions error out rather than silently
    /// mis-parse (the whole point of versioning the contract).
    #[serde(default)]
    pub v: u8,
    /// Plain-text shorthand (back-compat). Used as the embedding query and the
    /// lexical fallback when no structured `lex` is supplied.
    #[serde(default)]
    pub q: Option<String>,
    /// Structured lexical controls (phrases, exclusions, exact code).
    #[serde(default)]
    pub lex: LexSpec,
    /// Semantic embedding override (takes priority over `q`).
    #[serde(default)]
    pub vec: Option<String>,
    /// Caller-supplied hypothetical answer (priority over `vec`).
    #[serde(default)]
    pub hyde: Option<String>,
    /// Free-form intent label, recorded for provenance/explain only.
    #[serde(default)]
    pub intent: Option<String>,
    /// OR filter over ingest kind (`memory` | `markdown` | `structured` |
    /// `manual` | `vault`) — applied to the `source` column, NOT to source URIs.
    /// Empty = unrestricted. Document/source-URI scoping is a future param.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Single-source filter. Accepts an ingest kind (`memory` | `markdown` |
    /// `structured` | `manual` | `vault`) OR a retrieval leg (`vector` | `fts` |
    /// `graph`) OR `both`. Kinds filter in SQL; legs filter post-fusion; `both`
    /// is unrestricted. Unknown values are rejected with 422 at the handler.
    #[serde(default)]
    pub source: Option<String>,
    /// Validated ISO-8601 / `YYYY-MM-DD HH:MM:SS` lower bound on `created_at`.
    #[serde(default)]
    pub since: Option<String>,
    /// Domain isolation filter (single-DB tagged model today).
    #[serde(default)]
    pub domain: Option<String>,
    /// Result count. `None` → route default (`DEFAULT_K` / `DEFAULT_RECALL_LIMIT`).
    #[serde(default)]
    pub k: Option<u32>,
    /// Retrieval profile hint (passthrough in M1; no rerank plumbing yet).
    #[serde(default)]
    pub profile: Option<String>,
    /// When true, responses include the query plan / telemetry / provenance.
    #[serde(default)]
    pub explain: bool,
    /// v0.9.7 Guard: include quarantined (`flagged`) chunks in results. Operator
    /// review path only; the default agent path keeps them excluded.
    #[serde(default)]
    pub include_flagged: bool,
    /// v0.9.8 "Evidence": point-in-time recall. When set, recall returns the
    /// revision of each source that was current *at* this RFC3339 instant
    /// (historical mode); superseded chunks become visible. `None` (default) ⇒
    /// only current evidence is returned.
    #[serde(default)]
    pub as_of: Option<String>,
    /// v0.9.8 "Evidence": include structured `Evidence` (time + lifecycle +
    /// links) on every hit. Serialization switch.
    #[serde(default)]
    pub evidence: bool,
    /// v1.4.0 "Calibrate" M1: bi-temporal valid-time point-in-time filter.
    /// RFC3339 or `YYYY-MM-DD`; only chunks whose valid-interval contains this
    /// instant are returned. Distinct from `as_of` (transaction-time recall).
    #[serde(default)]
    pub at: Option<String>,
    /// v1.11.0 "Associate": enable the graph-PPR retriever as a third RRF leg
    /// (opt-in; default `false` keeps the two-retriever path unchanged).
    #[serde(default)]
    pub graph: bool,
}

impl QueryDoc {
    /// Back-compat constructor: a bare query string.
    pub fn from_text(q: String) -> Self {
        QueryDoc {
            v: QUERY_DOC_VERSION,
            q: Some(q),
            ..Default::default()
        }
    }

    /// Validate the document and lower it into the retrieval-engine
    /// [`SearchFilters`]. Returns the effective query text (the embedding/
    /// lexical fallback) alongside the filters so callers don't re-derive it.
    pub fn into_filters(self) -> Result<(String, SearchFilters), QueryDocError> {
        if self.v != 0 && self.v != QUERY_DOC_VERSION {
            return Err(QueryDocError::UnsupportedVersion(self.v));
        }
        let q = self.q.clone().unwrap_or_default();
        if q.trim().is_empty() && self.lex.is_empty() {
            return Err(QueryDocError::EmptyQuery);
        }
        let lex = compile_lex(&self.lex);
        let since = match &self.since {
            Some(s) => Some(normalize_since(s).map_err(QueryDocError::InvalidSince)?),
            None => None,
        };
        // v1.4.0 "Calibrate" M1: normalize the bi-temporal `at` filter.
        let at = match &self.at {
            Some(s) => Some(normalize_since(s).map_err(QueryDocError::InvalidAt)?),
            None => None,
        };
        let embedding_query = self
            .hyde
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.vec.filter(|s| !s.trim().is_empty()));
        Ok((
            q,
            SearchFilters {
                // `lex` is now the compiled FTS5 MATCH string; `None` when the
                // caller supplied nothing lexical (revert to the bare query).
                lex: if lex.is_empty() { None } else { Some(lex) },
                embedding_query,
                intent: self.intent.filter(|s| !s.trim().is_empty()),
                sources: self
                    .sources
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect(),
                source: self.source.filter(|s| !s.trim().is_empty()),
                source_leg: None,
                since,
                domain: self.domain.filter(|s| !s.trim().is_empty()),
                profile: self.profile.filter(|s| !s.trim().is_empty()),
                include_flagged: self.include_flagged,
                as_of: self.as_of.filter(|s| !s.trim().is_empty()),
                evidence: self.evidence,
                freshness_tiebreak: true,
                at,
                graph: self.graph,
                include_decayed: false,
                now_unix: 0,
                memory_kind: None,
                min_relevance: None,
                access_scopes: None,
                retention_days: Vec::new(),
            },
        ))
    }
}

impl Default for QueryDoc {
    fn default() -> Self {
        QueryDoc {
            v: QUERY_DOC_VERSION,
            q: None,
            lex: LexSpec::default(),
            vec: None,
            hyde: None,
            intent: None,
            sources: Vec::new(),
            source: None,
            since: None,
            domain: None,
            k: None,
            profile: None,
            explain: false,
            include_flagged: false,
            as_of: None,
            evidence: false,
            at: None,
            graph: false,
        }
    }
}

/// Structured lexical controls. Compiled into a single FTS5 `MATCH` string by
/// [`compile_lex`]; never sent to SQLite verbatim from a caller.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LexSpec {
    /// Implicit-AND terms. FTS5-quoted so embedded spaces/punctuation are
    /// treated as one token-run, not a phrase operator.
    #[serde(default)]
    pub terms: Vec<String>,
    /// Quoted phrases — matched as an ordered token sequence.
    #[serde(default)]
    pub phrases: Vec<String>,
    /// Exclusions — prepended with `-` (FTS5 NOT).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Exact identifier / code path — matched verbatim, FTS5-quoted.
    #[serde(default)]
    pub code: Vec<String>,
}

impl LexSpec {
    fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.phrases.is_empty()
            && self.exclude.is_empty()
            && self.code.is_empty()
    }
}

/// Compile a [`LexSpec`] into an FTS5 `MATCH` expression. Each entry is
/// individually quoted so caller input can never inject FTS5 operators or
/// break the query; the join is implicit AND (space-separated), which is the
/// FTS5 default.
///
/// ponytail: we strip FTS5's own double-quote delimiter from inputs and wrap
/// each in double quotes; we do NOT implement NEAR/prefix/`*` wildcards — the
/// plan asks for phrases/exclusions/exact-code, nothing more. Upgrade path:
/// expose a `near` field if ranking-by-proximity is ever needed.
pub fn compile_lex(spec: &LexSpec) -> String {
    let mut parts: Vec<String> = Vec::new();
    let quote = |s: &str| format!("\"{}\"", s.replace('"', ""));
    for t in &spec.terms {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(quote(t));
        }
    }
    for p in &spec.phrases {
        let p = p.trim();
        if !p.is_empty() {
            parts.push(quote(p));
        }
    }
    for c in &spec.code {
        let c = c.trim();
        if !c.is_empty() {
            parts.push(quote(c));
        }
    }
    for e in &spec.exclude {
        let e = e.trim();
        if !e.is_empty() {
            parts.push(format!("-{}", quote(e)));
        }
    }
    parts.join(" ")
}

// ── v1.13.3 "SourceFix": the `source` retrieval-filter contract ───────────

/// The parsed `source` retrieval filter. One parser ([`parse_source_filter`])
/// is used at both handler boundaries (`POST /recall`, `GET /search`) so the
/// wire contract and the retrieval engine can never drift.
///
/// Before this, every documented value (`vector` | `fts` | `both` | `graph`)
/// returned 0 hits: the legacy single-source filter was SQL equality against the
/// ingest-kind column, where retrieval-leg names exist nowhere, and `"both"` is
/// a fusion concept equality can never match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFilter {
    /// Ingest-kind equality (`memory` | `markdown` | `structured` | `manual` |
    /// `vault`). Applied in SQL *before* ranking on both retriever legs, so the
    /// kind-restricted top-k is returned (not the top-of-mixed with other kinds
    /// filtered out post-hoc — that would silently starve the result).
    Kind(String),
    /// Retrieval-leg restriction (`vector` | `fts` | `graph`). Applied
    /// post-fusion on the already-computed `SearchSource` tag — a fusion concept
    /// SQL equality cannot express.
    Leg(LegFilter),
    /// `"both"` — no restriction (the union of all legs). Same shape as omitting
    /// the param, but explicit.
    Any,
}

/// Which retrieval leg to keep after fusion. `Both`-tagged hits survive every
/// variant (they appeared in ≥2 legs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegFilter {
    Vector,
    Fts,
    Graph,
}

/// ponytail: this mirrors the live `k.source` / `v.source` column values. The
/// columns are the source of truth; this is the parser's allow-list. If a new
/// ingest kind is added, extend it here.
pub const INGEST_KINDS: &[&str] = &["memory", "markdown", "structured", "manual", "vault"];

/// Parse a raw `source` string into a [`SourceFilter`]. Lowercases nothing —
/// values are case-sensitive (the stored column values are lowercase). Empty
/// input is the caller's "omitted" signal and must NOT reach here.
pub fn parse_source_filter(raw: &str) -> Result<SourceFilter, SourceFilterError> {
    match raw.trim() {
        "vector" => Ok(SourceFilter::Leg(LegFilter::Vector)),
        "fts" => Ok(SourceFilter::Leg(LegFilter::Fts)),
        "graph" => Ok(SourceFilter::Leg(LegFilter::Graph)),
        "both" => Ok(SourceFilter::Any),
        kind if INGEST_KINDS.contains(&kind) => Ok(SourceFilter::Kind(kind.to_string())),
        other => Err(SourceFilterError {
            raw: other.to_string(),
        }),
    }
}

/// Lower a parsed [`SourceFilter`] into the two `SearchFilters` slots: the SQL
/// ingest-kind string (present only for [`SourceFilter::Kind`]) and the
/// post-fusion leg (present only for [`SourceFilter::Leg`]). Called once per
/// handler so the engine never re-parses.
pub fn split_source_filter(filter: Option<&SourceFilter>) -> (Option<String>, Option<LegFilter>) {
    match filter {
        None | Some(SourceFilter::Any) => (None, None),
        Some(SourceFilter::Kind(k)) => (Some(k.clone()), None),
        Some(SourceFilter::Leg(l)) => (None, Some(*l)),
    }
}

/// v1.13.4: resolve the `source` filter for `POST /recall` from BOTH the JSON
/// body and the query string (`?source=`), so a query-string value is honored
/// instead of silently ignored (parity with `GET /search`). Body `source` wins
/// when both are present; the query string fills in when the body omits it. An
/// unknown value in *either* is rejected. Pure + unit-testable; the handler
/// maps the error to HTTP 422.
pub fn resolve_source_filter(
    body: Option<&str>,
    query: Option<&str>,
) -> Result<(Option<String>, Option<LegFilter>), SourceFilterError> {
    let body_f = body
        .filter(|s| !s.trim().is_empty())
        .map(parse_source_filter)
        .transpose()?;
    let query_f = query
        .filter(|s| !s.trim().is_empty())
        .map(parse_source_filter)
        .transpose()?;
    let (body_kind, body_leg) = split_source_filter(body_f.as_ref());
    let (query_kind, query_leg) = split_source_filter(query_f.as_ref());
    Ok((body_kind.or(query_kind), body_leg.or(query_leg)))
}

/// Rejection of an unknown `source` value. Renders with the full valid-value
/// list so the 422 body tells the caller exactly what to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFilterError {
    pub raw: String,
}

impl std::fmt::Display for SourceFilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid source '{}': valid values are {}, vector, fts, graph, both",
            self.raw,
            INGEST_KINDS.join(", ")
        )
    }
}

impl std::error::Error for SourceFilterError {}

/// Query-document validation failure, rendered to the uniform error envelope by
/// the handlers.
#[derive(Debug)]
pub enum QueryDocError {
    UnsupportedVersion(u8),
    EmptyQuery,
    InvalidSince(anyhow::Error),
    /// v1.4.0 "Calibrate" M1: malformed `at` bi-temporal filter.
    InvalidAt(anyhow::Error),
}

impl std::fmt::Display for QueryDocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryDocError::UnsupportedVersion(v) => {
                write!(f, "unsupported query schema version {v}")
            }
            QueryDocError::EmptyQuery => write!(f, "query must not be empty"),
            QueryDocError::InvalidSince(e) => write!(f, "invalid 'since': {e}"),
            QueryDocError::InvalidAt(e) => write!(f, "invalid 'at': {e}"),
        }
    }
}

impl std::error::Error for QueryDocError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(q: &str) -> String {
        compile_lex(&serde_json::from_str(q).unwrap())
    }

    #[test]
    fn lex_quotes_terms_as_token_runs() {
        // No phrase operator injected between the two words.
        assert_eq!(
            compile_lex(&LexSpec {
                terms: vec!["foo bar".into()],
                ..Default::default()
            }),
            "\"foo bar\""
        );
    }

    #[test]
    fn lex_phrases_are_quoted() {
        assert_eq!(lex(r#"{"phrases":["lazy dog"]}"#), "\"lazy dog\"");
    }

    #[test]
    fn lex_exclusions_get_minus_prefix() {
        assert_eq!(lex(r#"{"exclude":["spam"]}"#), "-\"spam\"");
    }

    #[test]
    fn lex_code_is_verbatim_quoted() {
        assert_eq!(lex(r#"{"code":["src/main.rs"]}"#), "\"src/main.rs\"");
    }

    #[test]
    fn lex_strips_embedded_quotes() {
        assert_eq!(lex(r#"{"terms":["he\"llo"]}"#), "\"hello\"");
    }

    #[test]
    fn lex_combines_all_kinds_with_implicit_and() {
        let s = lex(r#"{"terms":["a"],"phrases":["b c"],"code":["d/e"],"exclude":["f"]}"#);
        assert_eq!(s, "\"a\" \"b c\" \"d/e\" -\"f\"");
    }

    #[test]
    fn doc_from_text_is_backwards_compatible() {
        let (q, f) = QueryDoc::from_text("hello world".into())
            .into_filters()
            .unwrap();
        assert_eq!(q, "hello world");
        assert!(f.lex.is_none());
        assert!(f.sources.is_empty());
    }

    #[test]
    fn doc_rejects_empty_query() {
        assert!(matches!(
            QueryDoc::default().into_filters(),
            Err(QueryDocError::EmptyQuery)
        ));
    }

    #[test]
    fn doc_rejects_unsupported_version() {
        // q still empty so we also prove version is checked first only when non-empty;
        // supply a q to exercise the version gate.
        let d = QueryDoc {
            v: 99,
            q: Some("x".into()),
            ..Default::default()
        };
        assert!(matches!(
            d.into_filters(),
            Err(QueryDocError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn doc_lex_supplies_compiled_lex_and_skips_bare_query() {
        let d = QueryDoc {
            q: Some("fallback".into()),
            lex: LexSpec {
                phrases: vec!["needle".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let (q, f) = d.into_filters().unwrap();
        assert_eq!(q, "fallback");
        assert_eq!(f.lex.as_deref(), Some("\"needle\""));
    }

    #[test]
    fn doc_multi_source_scope_is_preserved() {
        let d = QueryDoc {
            q: Some("x".into()),
            sources: vec!["a".into(), "  ".into(), "b".into()],
            ..Default::default()
        };
        let (_, f) = d.into_filters().unwrap();
        assert_eq!(f.sources, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn doc_invalid_since_is_rejected() {
        let d = QueryDoc {
            q: Some("x".into()),
            since: Some("not-a-time".into()),
            ..Default::default()
        };
        assert!(matches!(
            d.into_filters(),
            Err(QueryDocError::InvalidSince(_))
        ));
    }

    // ── v1.13.3 "SourceFix": parse_source_filter contract ──────────────────

    #[test]
    fn parse_source_filter_accepts_all_ingest_kinds() {
        for k in INGEST_KINDS {
            assert_eq!(
                parse_source_filter(k),
                Ok(SourceFilter::Kind((*k).to_string())),
                "kind {k}"
            );
        }
    }

    #[test]
    fn parse_source_filter_accepts_retrieval_legs_and_both() {
        assert_eq!(
            parse_source_filter("vector"),
            Ok(SourceFilter::Leg(LegFilter::Vector))
        );
        assert_eq!(
            parse_source_filter("fts"),
            Ok(SourceFilter::Leg(LegFilter::Fts))
        );
        assert_eq!(
            parse_source_filter("graph"),
            Ok(SourceFilter::Leg(LegFilter::Graph))
        );
        assert_eq!(parse_source_filter("both"), Ok(SourceFilter::Any));
    }

    #[test]
    fn parse_source_filter_rejects_unknown_values_with_the_valid_list() {
        // `web` is the documented broken value; garbage and case-variants too
        // (column values are lowercase, the parser is case-sensitive).
        for bad in ["web", "VECTOR", "document.md", "src", "uri://x"] {
            let err = parse_source_filter(bad).expect_err(bad);
            let msg = err.to_string();
            assert!(msg.contains("memory"), "msg: {msg}");
            assert!(msg.contains("vector"), "msg: {msg}");
            assert!(msg.contains("both"), "msg: {msg}");
            assert!(msg.contains(bad), "should echo the bad value: {msg}");
        }
    }

    #[test]
    fn split_source_filter_routes_kind_to_sql_and_leg_to_post_fusion() {
        assert_eq!(split_source_filter(None), (None, None));
        assert_eq!(split_source_filter(Some(&SourceFilter::Any)), (None, None));
        assert_eq!(
            split_source_filter(Some(&SourceFilter::Kind("memory".into()))),
            (Some("memory".into()), None)
        );
        assert_eq!(
            split_source_filter(Some(&SourceFilter::Leg(LegFilter::Vector))),
            (None, Some(LegFilter::Vector))
        );
    }

    #[test]
    fn resolve_source_filter_body_wins_query_fills_unknown_rejected() {
        // Body wins when both present.
        assert_eq!(
            resolve_source_filter(Some("memory"), Some("markdown")).unwrap(),
            (Some("memory".into()), None)
        );
        // Query string fills in when the body omits `source`.
        assert_eq!(
            resolve_source_filter(None, Some("vector")).unwrap(),
            (None, Some(LegFilter::Vector))
        );
        // "both" in either slot is unrestricted.
        assert_eq!(
            resolve_source_filter(Some("both"), None).unwrap(),
            (None, None)
        );
        // Unknown value in EITHER slot -> Err (no silent ignore — the fix).
        assert!(resolve_source_filter(Some("web"), None).is_err());
        assert!(resolve_source_filter(None, Some("web")).is_err());
        assert!(resolve_source_filter(Some("memory"), Some("web")).is_err());
        // Both absent -> unrestricted.
        assert_eq!(resolve_source_filter(None, None).unwrap(), (None, None));
    }
}
