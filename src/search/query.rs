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
    /// Multi-source OR scope. Empty = no source restriction.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Legacy single-source equality (back-compat with `GET /search?source=`).
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
                since,
                domain: self.domain.filter(|s| !s.trim().is_empty()),
                profile: self.profile.filter(|s| !s.trim().is_empty()),
                include_flagged: self.include_flagged,
                as_of: self.as_of.filter(|s| !s.trim().is_empty()),
                evidence: self.evidence,
                freshness_tiebreak: true,
                at,
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
}
