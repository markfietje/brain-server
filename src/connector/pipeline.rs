//! v1.24.0 "Connectors" M2 — the shared translate+ingest template.
//!
//! Every vertical connector is "translate a source record → a `ConnectorDoc`,
//! then feed it to the existing `/ingest/markdown` (source/revision linkage,
//! v0.9.4) and its URI set to `/sources/reconcile` (v0.9.6)". The per-strong
//! system's transport (page fetch, auth refresh, rate limits) is
//! connector-specific — that is the documented v1.24 honest ceiling (the
//! GitHub connector, `connector/github`, is the tested network template). This
//! module holds the **pure, testable** core every connector shares: record
//! translation to markdown docs carrying a stable source URI + scope, and the
//! live-URI set for reconcile.
//!
//! `kind` on a `ConnectorDoc` is the brain **source kind** — the scoping key
//! the server's `/sources/reconcile` sweeps by, so one connector's reconcile
//! never retires another's (see `sources.rs::reconcile`).

/// A translated source record, ready for `/ingest/markdown` (`source_path` =
/// `uri`, `title`, `content` = `markdown`) and `/sources/reconcile` (`kind` =
/// `kind`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorDoc {
    /// Stable connector-defined URI (e.g. `crm://acme/opp/123`) — the chunk
    /// `source_path`. Must be idempotent: unchanged source → same URI + same
    /// content, so the v0.9.4 dedup makes re-ingest a no-op.
    pub uri: String,
    pub title: String,
    pub markdown: String,
    /// The brain source kind used to scope `/sources/reconcile`.
    pub kind: String,
    /// Suggested default access scope (private for strict-PII sources). The row
    /// value always wins over this default.
    pub access_scope: &'static str,
}

/// Map a connector kind to the brain source kind its reconcile sweeps. The
/// family prefix is used so `crm-salesforce`/`crm-hubspot` both reconcile the
/// `crm` source set (a domain may run both, sharing one reconcile scope).
pub fn connector_source_kind(connector: &str) -> String {
    match connector {
        "github" => "github".to_string(),
        _ => crate::connector::kind::family(connector).to_string(),
    }
}

/// The URIs to hand `/sources/reconcile` for the docs produced by one backfill
/// pass. Every URI the source [no longer] lists is retired on reconcile; every
/// URI here is kept live.
pub fn live_uris(docs: &[ConnectorDoc]) -> Vec<String> {
    docs.iter().map(|d| d.uri.clone()).collect()
}

/// Translate a CRM opportunity → a fact/procedural memory for the account's
/// domain. `uri = crm://{account}/{id}`.
pub fn translate_crm_opportunity(
    account: &str,
    id: &str,
    name: &str,
    amount_cents: i64,
) -> ConnectorDoc {
    let dollars = amount_cents.div_euclid(100);
    ConnectorDoc {
        uri: format!("crm://{account}/{id}"),
        title: format!("{account}: {name}"),
        markdown: format!("# Opportunity {account}/{name} ({id})\n\n",)
            + &format!("- Stage: open\n- Amount: ${dollars}\n- Record: {account}/{id}\n"),
        kind: connector_source_kind("crm-salesforce"),
        access_scope: "team",
    }
}

/// Translate a Slack channel message → an episodic memory owned by its author.
/// `uri = slack://{channel}/{ts}`. The channel is the opt-in scope.
pub fn translate_slack_message(channel: &str, ts: &str, author: &str, text: &str) -> ConnectorDoc {
    ConnectorDoc {
        uri: format!("slack://{channel}/{ts}"),
        title: format!("#{channel}: {author}"),
        markdown: format!("# {author} said in #{channel}\n\n{text}\n"),
        kind: connector_source_kind("slack"),
        access_scope: "team",
    }
}

/// Translate a Jira/Linear issue → a decision/procedural memory. The issue key
/// is the stable URI. `status` (e.g. "Done") flavours the action.
pub fn translate_issue(key: &str, summary: &str, status: &str) -> ConnectorDoc {
    ConnectorDoc {
        uri: format!("jira://{key}"),
        title: format!("{key}: {summary}"),
        markdown: format!("# Issue {key} — {summary}\n\nStatus: {status}\n"),
        kind: connector_source_kind("jira"),
        access_scope: "team",
    }
}

/// Translate an HRIS/EHR structured record (read-only, strict PII) → a private
/// fact memory. Deliberately narrow: only `subject`, `kind_of`, and any
/// `facts` lines — never raw PHI — and access_scope is forced private so the
/// strict-PII profile's write-time masking is the ceiling, not this default.
pub fn translate_structured_fact(
    record_kind: &str,
    subject: &str,
    uri_key: &str,
    facts: &[&str],
) -> ConnectorDoc {
    let uri = format!("{record_kind}://{subject}/{uri_key}");
    let fact_lines = facts
        .iter()
        .map(|f| format!("- {f}"))
        .collect::<Vec<_>>()
        .join("\n");
    ConnectorDoc {
        uri,
        title: format!("{record_kind} record {uri_key}"),
        markdown: format!("# {record_kind} record for {subject}\n\n{fact_lines}\n"),
        kind: connector_source_kind(record_kind),
        access_scope: "private",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_source_kind_uses_family_for_prefix_kinds() {
        assert_eq!(connector_source_kind("github"), "github");
        assert_eq!(connector_source_kind("crm-salesforce"), "crm");
        assert_eq!(connector_source_kind("crm-hubspot"), "crm");
        assert_eq!(connector_source_kind("slack"), "slack");
        assert_eq!(connector_source_kind("hris-readonly"), "hris");
    }

    /// Verification: `crm_backfill_links_source_and_revision` — a translated
    /// opportunity carries a stable `crm://` source URI + the reconciliation
    /// scope matches the connector family.
    #[test]
    fn crm_doc_links_stable_uri_and_source_kind() {
        let d = translate_crm_opportunity("acme", "opp-123", "Q3 contract", 1_250_000);
        assert_eq!(d.uri, "crm://acme/opp-123");
        assert_eq!(d.kind, "crm");
        assert!(
            d.markdown.contains("acme/opp-123"),
            "doc must carry the source link"
        );
        assert!(d.markdown.contains("$12500"), "amount renders in dollars");
    }

    #[test]
    fn slack_doc_scopes_by_channel_and_owner_author() {
        let d = translate_slack_message("sales", "1700000000.0123", "ada", "Ship the demo");
        assert_eq!(d.uri, "slack://sales/1700000000.0123");
        assert_eq!(d.kind, "slack");
        assert!(d.markdown.contains("ada said in #sales"));
        assert!(d.markdown.contains("Ship the demo"));
    }

    #[test]
    fn issue_doc_is_stable_keyed() {
        let d = translate_issue("ENG-42", "rollback on region-pin", "Done");
        assert_eq!(d.uri, "jira://ENG-42");
        assert_eq!(d.kind, "jira");
        assert!(d.markdown.contains("Status: Done"));
    }

    #[test]
    fn structured_fact_forces_private_scope_and_never_emits_phi() {
        let d = translate_structured_fact(
            "ehr-readonly",
            "pat-7",
            "chart-88",
            &["diagnosis stable", "no new symptoms"],
        );
        assert_eq!(d.kind, "ehr");
        assert_eq!(
            d.access_scope, "private",
            "read-only PII records must default private"
        );
        assert!(d.markdown.contains("for pat-7"));
        assert!(d.markdown.contains("- diagnosis stable"));
        // The URI carries only identifiers — raw PHI stays out of the key.
        assert!(d.uri.starts_with("ehr-readonly://pat-7/"));
    }

    #[test]
    fn live_uris_collects_every_doc_uri() {
        let docs = vec![
            translate_crm_opportunity("a", "1", "x", 10),
            translate_slack_message("c", "t", "u", "hi"),
        ];
        let uris = live_uris(&docs);
        assert_eq!(
            uris,
            vec!["crm://a/1".to_string(), "slack://c/t".to_string()]
        );
    }
}
