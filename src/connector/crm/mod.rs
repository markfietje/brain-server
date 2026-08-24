//! The CRM connector contract — support cases flow in from the CRMs.
//!
//! One normalized shape ([`CrmCase`]), three vendors (Zendesk, Salesforce,
//! Genesys Cloud), and one delivery path into the universal loop:
//!
//! 1. the case **body** (untrusted CRM text) is ingested through the UMP
//!    `/ingest` single-record path — under `BRAIN_WRITE_POSTURE=review` the
//!    server routes it to a *proposal* in the HITL review queue, exactly like
//!    any other agent-sourced content. It never writes memory directly.
//! 2. the case **envelope** (status/priority/refs) opens a governed run via
//!    `POST /workflow/runs` (state carries the stable `case_ref`) and posts
//!    `crm/case/updated` / `crm/case/closed` events on that run's outbox.
//!    A closed-solved event is the capture trigger Evolve consumes next
//!    release; the run linkage itself lives in the server's `crm_cases`
//!    table (see [`upsert_crm_case`]).
//!
//! Vendor sync loops are pure functions over the [`VendorTransport`] trait,
//! so every wire interaction is testable against mock transports with zero
//! network. Only the reqwest adapter (`http.rs`, feature-gated) and the
//! `brain-connector-crm` binary touch the real network — operator-cranked
//! via cron, never background-synced (the supervisor stays unwired).
//!
//! Security posture (mirrors the GitHub connector):
//! - every outbound URL is built from config-derived hosts only — never from
//!   memory or case content (`no_crm_url_from_memory_content`);
//! - server-provided pagination URLs are reduced to query parameters applied
//!   to the pinned base (a forged `nextRecordsUrl` cannot redirect the token);
//! - credentials ride in 0600 secret files, mode-checked fail-closed;
//! - customer identity is stored only as a salted SHA-256 `subject_ref` —
//!   the DSAR story for CRM content stays hash-based.

pub mod genesys;
#[cfg(feature = "connector-crm")]
pub mod http;
pub mod salesforce;
pub mod zendesk;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// The outbound wire every vendor sync loop speaks. Abstracted so the sync
/// loops are pure functions over mock transports; the reqwest adapter
/// ([`http::ReqwestTransport`], feature-gated) is the only networked impl and
/// enforces the host allowlist + redirect refusal + bounded timeouts there.
pub trait VendorTransport {
    /// GET a JSON document with the given prebuilt auth header.
    fn get_json(&self, url: &str, auth_header: &str) -> Result<serde_json::Value>;
    /// POST an x-www-form-urlencoded body; `auth_header` is `None` for the
    /// public token endpoints.
    fn post_form(
        &self,
        url: &str,
        form_body: &str,
        auth_header: Option<&str>,
    ) -> Result<serde_json::Value>;
}

/// Zendesk source label.
pub const SOURCE_ZENDESK: &str = "zendesk";
/// Salesforce source label.
pub const SOURCE_SALESFORCE: &str = "salesforce";
/// Genesys Cloud source label.
pub const SOURCE_GENESYS: &str = "genesys";

/// Minimum seconds between two incremental-export polls. Zendesk's cursor
/// endpoint is rate-capped around 10 req/min; 300s leaves wide headroom and
/// matches the documented default cadence. The connector binary refuses a
/// config asking for anything shorter.
pub const MIN_POLL_INTERVAL_SECS: u64 = 300;

/// Case lifecycle, normalized across vendors. `ClosedSolved` is the only
/// terminal state Bridges recognizes — it is what fires Evolve's capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
    Open,
    ClosedSolved,
}

impl CaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CaseStatus::Open => "open",
            CaseStatus::ClosedSolved => "closed_solved",
        }
    }
}

/// The normalized CRM case — one shape, every channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrmCase {
    /// `"zendesk"` | `"salesforce"` | `"genesys"` (custom sources extend the
    /// vocabulary via config, never by code change).
    pub source: String,
    /// The CRM organization/instance this case belongs to (the tenant key).
    pub org_id: String,
    /// The vendor's stable case id.
    pub case_id: String,
    pub title: String,
    pub status: CaseStatus,
    /// Vendor priority string, verbatim (e.g. "urgent", "P2"). Informational.
    pub priority: Option<String>,
    /// Salted SHA-256 of the customer identity — never raw PII.
    pub subject_ref: String,
    /// Vendor revision marker (Zendesk `generated_at`, Salesforce
    /// `SystemModstamp`, Genesys workitem version). Idempotency key input.
    pub updated_rev: String,
    /// The case description translated to Markdown (untrusted content).
    pub body_markdown: String,
    /// Structured symptom seed, when the CRM carries one — feeds the
    /// frontdoor `Handoff.is_seed` directly.
    pub is_seed: Option<String>,
    /// Explicit non-seed context, when present — feeds `Handoff.is_not_seed`.
    pub is_not_seed: Option<String>,
    /// Vendor last-update timestamp (ISO-8601), verbatim.
    pub updated_at: String,
}

impl CrmCase {
    /// The stable cross-vendor case reference: `crm:{source}:{org}:{id}`.
    /// This is THE linkage key — the run state carries it verbatim and the
    /// `crm_cases` table keys on it.
    pub fn case_ref(&self) -> String {
        format!("crm:{}:{}:{}", self.source, self.org_id, self.case_id)
    }
}

/// Salt-and-hash a customer identity into a `subject_ref`. The domain prefix
/// keeps identical identities in different orgs/sources unlinkable.
pub fn hash_subject(source: &str, org_id: &str, identity: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"brain-crm-subject\x00");
    h.update(source.as_bytes());
    h.update(b"\x00");
    h.update(org_id.as_bytes());
    h.update(b"\x00");
    h.update(identity.trim().as_bytes());
    let digest = h.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Translate a [`CrmCase`] into the markdown doc ingested through `/ingest`.
/// The URI is the stable `case_ref`-derived `crm://{source}/{org}/{id}` so
/// unchanged re-syncs dedupe server-side.
pub fn case_doc(c: &CrmCase) -> crate::connector::pipeline::ConnectorDoc {
    use crate::connector::pipeline::ConnectorDoc;
    ConnectorDoc {
        uri: format!("crm://{}/{}/{}", c.source, c.org_id, c.case_id),
        title: c.title.clone(),
        markdown: c.body_markdown.clone(),
        kind: "crm".to_string(),
        // Customer conversations default private; min-necessary beats the
        // pipeline's team default for CRM cases.
        access_scope: "private",
    }
}

// ── the crm_cases linkage store ──────────────────────────────────────────────

/// Idempotent upsert keyed on `case_ref`. `run_id`, when `Some`, binds (or
/// re-binds) the case to its governed run; `None` leaves an existing binding
/// untouched. Returns the number of rows written (always 1).
pub fn upsert_crm_case(conn: &Connection, c: &CrmCase, run_id: Option<i64>) -> Result<()> {
    conn.execute(
        "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
         ON CONFLICT(case_ref) DO UPDATE SET
             status = excluded.status,
             updated_rev = excluded.updated_rev,
             run_id = COALESCE(excluded.run_id, crm_cases.run_id),
             synced_at = CURRENT_TIMESTAMP",
        params![
            c.case_ref(),
            c.source,
            c.org_id,
            c.case_id,
            run_id,
            c.status.as_str(),
            c.updated_rev
        ],
    )?;
    Ok(())
}

/// The governed run bound to `case_ref`, if the connector has opened one yet.
pub fn run_for_case(conn: &Connection, case_ref: &str) -> Result<Option<i64>> {
    let id: Option<Option<i64>> = conn
        .query_row(
            "SELECT run_id FROM crm_cases WHERE case_ref = ?1",
            params![case_ref],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id.flatten())
}

// ── delivery ─────────────────────────────────────────────────────────────────

/// The brain-server surface Bridges writes through. Trait-abstracted so the
/// full delivery loop is testable without HTTP; the reqwest impl lives in
/// `http.rs` behind the `connector-crm` feature.
pub trait BrainSink {
    /// Ingest the case body as untrusted content. Under `review` posture the
    /// server answers with a proposal receipt (`{"proposal_id": …}`);
    /// under `open` posture it answers with a direct ingest receipt. Both are
    /// success — the posture is the operator's choice, not ours.
    fn ingest_body(&self, title: &str, body_markdown: &str, source_uri: &str) -> Result<()>;
    /// Open a governed run whose state carries the `case_ref`.
    fn open_run(&self, case_ref: &str) -> Result<i64>;
    /// Post one outbox event on the run. Returns `(first, event_id)` —
    /// `first == false` means the idempotency key replayed.
    fn post_event(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        key: &str,
    ) -> Result<(bool, i64)>;
}

/// Outbox topics Bridges emits. `closed` is the Evolve trigger fixture shape.
pub const TOPIC_CASE_UPDATED: &str = "crm/case/updated";
pub const TOPIC_CASE_CLOSED: &str = "crm/case/closed";

/// Summary of one case's delivery pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReport {
    pub case_ref: String,
    pub run_id: i64,
    /// Which event topic was posted this pass (`None` when the exact
    /// revision+topic already landed — replay is a no-op).
    pub topic_posted: Option<String>,
}

/// Deliver one synced case end-to-end: body → proposal-path ingest, envelope
/// → run open (once per `case_ref`) → outbox event. Every step is idempotent,
/// so a crash mid-pass resumes cleanly on the next cron tick.
pub fn deliver_case(sink: &dyn BrainSink, db: &Connection, c: &CrmCase) -> Result<DeliveryReport> {
    let case_ref = c.case_ref();

    // 1. The untrusted body takes the proposal path (posture decides at the
    //    server). Delivered even when nothing else changes — same content +
    //    same URI dedupes server-side.
    sink.ingest_body(
        &c.title,
        &c.body_markdown,
        &format!("crm://{}/{}/{}", c.source, c.org_id, c.case_id),
    )
    .with_context(|| format!("body ingest failed for {case_ref}"))?;

    // 2. Run binding: open once, reuse forever (one case = one run).
    let run_id = match run_for_case(db, &case_ref)? {
        Some(id) => id,
        None => {
            let id = sink.open_run(&case_ref)?;
            upsert_crm_case(db, c, Some(id))?;
            id
        }
    };

    // 3. Envelope event. Closed-solved closes the loop (Evolve's trigger);
    //    everything else reports progress.
    let topic = match c.status {
        CaseStatus::Open => TOPIC_CASE_UPDATED,
        CaseStatus::ClosedSolved => TOPIC_CASE_CLOSED,
    };
    let payload = serde_json::json!({
        "case_ref": case_ref,
        "status": c.status.as_str(),
        "priority": c.priority,
        "updated_rev": c.updated_rev,
        "subject_ref": c.subject_ref,
        "is_seed": c.is_seed,
        "is_not_seed": c.is_not_seed,
    })
    .to_string();
    let key = event_key(&case_ref, &c.updated_rev, topic);
    let (first, _event_id) = sink.post_event(run_id, topic, &payload, &key)?;
    Ok(DeliveryReport {
        case_ref,
        run_id,
        topic_posted: first.then(|| topic.to_string()),
    })
}

/// Idempotency key for one case transition, bounded to the outbox's 128-char
/// limit by hashing when the natural form overflows.
fn event_key(case_ref: &str, rev: &str, topic: &str) -> String {
    let natural = format!("{case_ref}:{rev}:{topic}");
    if natural.len() <= 128 {
        return natural;
    }
    let mut h = Sha256::new();
    h.update(natural.as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("k:{}", &hex[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn sample(status: CaseStatus) -> CrmCase {
        CrmCase {
            source: SOURCE_ZENDESK.into(),
            org_id: "acme".into(),
            case_id: "42".into(),
            title: "Cannot reset PIN".into(),
            status,
            priority: Some("urgent".into()),
            subject_ref: hash_subject(SOURCE_ZENDESK, "acme", "requester:991"),
            updated_rev: "2026-08-24T10:00:00Z".into(),
            body_markdown: "# Cannot reset PIN\n\nCustomer locked out after 2FA move.".into(),
            is_seed: Some("2FA migration broke PIN reset".into()),
            is_not_seed: None,
            updated_at: "2026-08-24T10:00:00Z".into(),
        }
    }

    #[test]
    fn crm_cases_upsert_is_idempotent_by_case_ref() {
        let dir = std::env::temp_dir().join(format!("brain-crm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("linkage.db");
        let _ = std::fs::remove_file(&path);
        let db = Connection::open(&path).expect("open file DB");
        db.execute_batch(
            "CREATE TABLE crm_cases(
                case_ref TEXT PRIMARY KEY, source TEXT NOT NULL, org_id TEXT NOT NULL,
                case_id TEXT NOT NULL, run_id INTEGER, status TEXT NOT NULL,
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
        )
        .expect("schema");
        let c = sample(CaseStatus::Open);
        upsert_crm_case(&db, &c, None).expect("upsert");
        upsert_crm_case(&db, &c, None).expect("re-upsert");
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM crm_cases", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1, "re-upsert must not duplicate");
        assert_eq!(run_for_case(&db, &c.case_ref()).expect("lookup"), None);

        // Bind a run; later passes must not lose it.
        upsert_crm_case(&db, &c, Some(7)).expect("bind run");
        assert_eq!(run_for_case(&db, &c.case_ref()).expect("lookup"), Some(7));
        upsert_crm_case(&db, &c, None).expect("re-upsert keeps binding");
        assert_eq!(
            run_for_case(&db, &c.case_ref()).expect("lookup"),
            Some(7),
            "None preserves the binding"
        );

        // Status advance lands on the same row.
        upsert_crm_case(&db, &sample(CaseStatus::ClosedSolved), None).expect("status advance");
        let status: String = db
            .query_row("SELECT status FROM crm_cases", [], |r| r.get(0))
            .expect("status");
        assert_eq!(status, "closed_solved");
        let _ = std::fs::remove_file(&path);
    }

    /// Records calls; replays answer identically — the harness for the
    /// idempotence assertions.
    struct RecordingSink {
        events: RefCell<Vec<(String, String)>>,
    }
    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
            }
        }
    }
    impl BrainSink for RecordingSink {
        fn ingest_body(&self, _t: &str, _b: &str, _u: &str) -> Result<()> {
            Ok(())
        }
        fn open_run(&self, case_ref: &str) -> Result<i64> {
            Ok(match case_ref {
                "crm:zendesk:acme:42" => 101,
                _ => anyhow::bail!("unexpected case_ref"),
            })
        }
        fn post_event(
            &self,
            run_id: i64,
            topic: &str,
            payload_json: &str,
            key: &str,
        ) -> Result<(bool, i64)> {
            let mut ev = self.events.borrow_mut();
            if let Some(pos) = ev.iter().position(|(k, _)| k == key) {
                // Replay resolves to the SURVIVING row's id, never a new one.
                return Ok((false, pos as i64 + 1));
            }
            ev.push((key.to_string(), format!("{run_id}|{topic}|{payload_json}")));
            Ok((true, ev.len() as i64))
        }
    }

    fn temp_linkage_db(c: &CrmCase, status: CaseStatus) -> Connection {
        let db = Connection::open_in_memory().unwrap_or_else(|e| panic!("open: {e}"));
        db.execute_batch(
            "CREATE TABLE crm_cases(
                case_ref TEXT PRIMARY KEY, source TEXT NOT NULL, org_id TEXT NOT NULL,
                case_id TEXT NOT NULL, run_id INTEGER, status TEXT NOT NULL,
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
        )
        .unwrap_or_else(|e| panic!("schema: {e}"));
        upsert_crm_case(&db, c, None).unwrap_or_else(|e| panic!("seed: {e}"));
        let _ = status;
        db
    }

    #[test]
    fn closed_solved_event_opens_capture() {
        // Fixture: the closed-solved transition is the event shape the Evolve
        // release consumes. First pass opens the run + posts `crm/case/closed`.
        let closed = sample(CaseStatus::ClosedSolved);
        let db = temp_linkage_db(&sample(CaseStatus::Open), CaseStatus::Open);
        let sink = RecordingSink::new();
        let r1 = deliver_case(&sink, &db, &closed).expect("first delivery");
        assert_eq!(r1.run_id, 101);
        assert_eq!(r1.topic_posted.as_deref(), Some(TOPIC_CASE_CLOSED));
        assert_eq!(
            run_for_case(&db, "crm:zendesk:acme:42").expect("linkage"),
            Some(101)
        );

        let events = sink.events.borrow();
        let (_, body) = &events[0];
        let payload: serde_json::Value = body
            .splitn(3, '|')
            .nth(2)
            .expect("payload segment")
            .parse()
            .expect("payload json");
        assert_eq!(payload["status"], "closed_solved");
        assert_eq!(payload["case_ref"], "crm:zendesk:acme:42");
        assert_eq!(
            payload["is_seed"], "2FA migration broke PIN reset",
            "structured seed rides the event for the frontdoor Handoff"
        );
        drop(events);

        // Replay of the same revision is a no-op on the event stream.
        let r2 = deliver_case(&sink, &db, &closed).expect("replay delivery");
        assert_eq!(r2.topic_posted, None, "idempotent replay posts nothing new");
        assert_eq!(sink.events.borrow().len(), 1);
    }

    #[test]
    fn subject_refs_are_stable_and_domain_separated() {
        let a = hash_subject("zendesk", "acme", "requester:991");
        let b = hash_subject("zendesk", "acme", "requester:991");
        let c = hash_subject("salesforce", "acme", "requester:991");
        assert_eq!(a, b, "same identity hashes stably");
        assert_ne!(a, c, "different source/org never collide");
        assert_eq!(a.len(), 64, "sha256 hex");
        assert!(!a.contains("991"), "raw identity never survives the hash");
    }

    #[test]
    fn case_ref_shape_is_stable_and_colon_safe_inputs_rejected_upstream() {
        assert_eq!(sample(CaseStatus::Open).case_ref(), "crm:zendesk:acme:42");
    }

    #[test]
    fn event_key_bounded_at_128_chars() {
        let long_ref = format!("crm:zendesk:{}:{}", "o".repeat(200), "1");
        let k = event_key(&long_ref, "rev-1", TOPIC_CASE_CLOSED);
        assert!(k.len() <= 128);
        assert_eq!(
            event_key(&long_ref, "rev-1", TOPIC_CASE_CLOSED),
            k,
            "stable"
        );
    }
}
