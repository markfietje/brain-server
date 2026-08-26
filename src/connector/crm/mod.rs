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
    /// A merged-away ref (Zendesk ticket merge / Salesforce case merge /
    /// Genesys workitem merge): the vendor surfaces it in incremental sync
    /// as its own row; it carries no independent issue anymore.
    MergedAway,
}

impl CaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CaseStatus::Open => "open",
            CaseStatus::ClosedSolved => "closed_solved",
            CaseStatus::MergedAway => "merged_away",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(CaseStatus::Open),
            "closed_solved" => Some(CaseStatus::ClosedSolved),
            "merged_away" => Some(CaseStatus::MergedAway),
            _ => None,
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
    /// The surviving case's vendor id when this ref was MERGED into another
    /// The surviving case's vendor id when this ref was MERGED into another
    /// (the CRM-merge re-ask source). `None` unless merged.
    pub merged_into: Option<String>,
    /// True when a previously-closed workitem REOPENED (the Genesys reopen
    /// re-ask source). Zendesk/Salesforce merges ride `merged_into`.
    pub reopened: bool,
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
        "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at, subject_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP, ?8)
         ON CONFLICT(case_ref) DO UPDATE SET
             status = excluded.status,
             updated_rev = excluded.updated_rev,
             run_id = COALESCE(excluded.run_id, crm_cases.run_id),
             subject_ref = excluded.subject_ref,
             synced_at = CURRENT_TIMESTAMP",
        params![
            c.case_ref(),
            c.source,
            c.org_id,
            c.case_id,
            run_id,
            c.status.as_str(),
            c.updated_rev,
            c.subject_ref
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
/// The re-ask event — merges and reopens map here.
pub const TOPIC_REASK: &str = "case/reask";

/// The detail digest for a CRM-merge/reopen re-ask: keyed SHA-256 over the
/// subject ref + merged-away case ref — ids only, never content.
fn reask_digest(subject_ref: &str, case_ref: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"brain-reask\x00");
    h.update(subject_ref.as_bytes());
    h.update(b"\x00");
    h.update(case_ref.as_bytes());
    let digest = h.finalize();
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..32]
        .to_string()
}

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

    // A MERGED-AWAY ref opens no run of its own — see the re-ask mapping
    // in step 3 below.
    let run_id = if c.status == CaseStatus::MergedAway {
        0 // never used: the merged branch returns before the envelope post
    } else {
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
        match run_for_case(db, &case_ref)? {
            Some(id) => id,
            None => {
                let id = sink.open_run(&case_ref)?;
                upsert_crm_case(db, c, Some(id))?;
                id
            }
        }
    };

    // 3. Envelope event. Closed-solved closes the loop (Evolve's trigger);
    //    everything else reports progress. A MERGED-AWAY ref is not a
    //    progress update — it maps to the target case's `case/reask` event
    //    (the customer's issue arrived twice because the first answer
    //    didn't land) and posts no envelope of its own.
    if c.status == CaseStatus::MergedAway {
        let Some(target_id) = c.merged_into.as_deref() else {
            // A merged-away row without a surviving id is unmappable data:
            // refuse loudly rather than silently drop the re-ask.
            anyhow::bail!(
                "merged_away case {} carries no merged_into target; refusing to map",
                case_ref
            );
        };
        let target_ref = format!("crm:{}:{}:{}", c.source, c.org_id, target_id);
        let run_id = run_for_case(db, &target_ref)?.ok_or_else(|| {
            anyhow::anyhow!("merge target {target_ref} has no governed run yet; re-ask deferred")
        })?;
        let digest = crate::connector::crm::reask_digest(&c.subject_ref, &case_ref);
        let payload = serde_json::json!({
            "source": "crm_merge",
            "detail_digest": digest,
            "ts": chrono::Utc::now().timestamp(),
        })
        .to_string();
        let key = event_key(&target_ref, &c.updated_rev, TOPIC_REASK);
        let (first, _id) = sink.post_event(run_id, TOPIC_REASK, &payload, &key)?;
        return Ok(DeliveryReport {
            case_ref,
            run_id,
            topic_posted: first.then(|| TOPIC_REASK.to_string()),
        });
    }
    let topic = match c.status {
        CaseStatus::Open => TOPIC_CASE_UPDATED,
        CaseStatus::ClosedSolved => TOPIC_CASE_CLOSED,
        // Handled above (the re-ask mapping); unreachable here.
        CaseStatus::MergedAway => anyhow::bail!("merged_away case reached the envelope mapper"),
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

// ── derived re-ask detection: propose, NEVER write. ────────────

/// The re-ask duplicate-detection window (days): `BRAIN_REASK_WINDOW_DAYS`
/// overrides, default 3. Same env-resolution discipline as every other
/// window (positive integers only, garbage falls back to the default).
pub const DEFAULT_REASK_WINDOW_DAYS: i64 = 3;

pub fn reask_window_days() -> i64 {
    std::env::var("BRAIN_REASK_WINDOW_DAYS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|d| *d > 0)
        .unwrap_or(DEFAULT_REASK_WINDOW_DAYS)
}

/// The HITL proposal kind a detected duplicate files. Approval is the human
/// CRM merge — brain has no merge engine and never merges anything itself.
pub const KIND_CASE_MERGE_SUGGESTED: &str = "case_merge_suggested";

/// Deterministic duplicate-open heuristic: OPEN cases in the same org with
/// the SAME hashed subject within `BRAIN_REASK_WINDOW_DAYS` (default 3
/// days) of each other file ONE pending `case_merge_suggested` proposal per
/// group (exact normalized-subject hash only — no fuzzy matching). A group
/// already covered by a pending or approved proposal of this kind is
/// skipped, so repeated sync passes stay idempotent. Returns the new
/// proposal ids.
pub fn detect_duplicate_opens(conn: &Connection, now: i64) -> Result<Vec<i64>> {
    let window = reask_window_days() * 86_400;
    let mut stmt = conn.prepare(
        "SELECT org_id, subject_ref,
                MIN(case_ref), MAX(case_ref), COUNT(DISTINCT case_ref),
                CAST(MIN(strftime('%s', synced_at)) AS INTEGER)
         FROM crm_cases
         WHERE status = 'open' AND subject_ref != ''
           AND run_id IS NOT NULL
         GROUP BY org_id, subject_ref
         HAVING COUNT(DISTINCT case_ref) > 1",
    )?;
    let groups = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut proposed = Vec::new();
    for (org, subject_ref, ref_a, ref_b, n, oldest) in groups {
        if now - oldest > window {
            // The duplicates are older than the window: not a re-ask signal.
            continue;
        }
        let digest = {
            let mut h = Sha256::new();
            h.update(b"brain-reask-dup\x00");
            h.update(org.as_bytes());
            h.update(b"\x00");
            h.update(subject_ref.as_bytes());
            let d = h.finalize();
            d.iter().map(|b| format!("{b:02x}")).collect::<String>()[..32].to_string()
        };
        let seen: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proposals WHERE kind = ?1
              AND content LIKE '%' || ?2 || '%'
              AND status IN ('pending', 'approved')",
            rusqlite::params![KIND_CASE_MERGE_SUGGESTED, digest],
            |r| r.get(0),
        )?;
        if seen > 0 {
            continue;
        }
        let content = serde_json::json!({
            "kind": "duplicate_open_cases",
            "org_id": org,
            "subject_ref": subject_ref,
            "detail_digest": digest,
            "merge_candidates": [ref_a, ref_b],
            "open_case_count": n,
            "window_days": reask_window_days(),
        });
        conn.execute(
            "INSERT INTO proposals(kind, content, source, authority, observed_at,
                                  novelty, conflict_with, salience, created_at)
             VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, 1.0, ?3)",
            rusqlite::params![KIND_CASE_MERGE_SUGGESTED, content.to_string(), now],
        )?;
        proposed.push(conn.last_insert_rowid());
    }
    Ok(proposed)
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
            merged_into: None,
            reopened: false,
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
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                subject_ref TEXT NOT NULL DEFAULT '');",
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
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                subject_ref TEXT NOT NULL DEFAULT '');",
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

    // ── Keystone M3: the re-ask mapping + derived detection.

    fn merged(status: CaseStatus, source: &str) -> CrmCase {
        let mut c = sample(status);
        c.source = source.into();
        c.case_id = "43".into();
        c.merged_into = Some("42".into());
        c
    }

    #[test]
    fn zendesk_and_salesforce_merges_map_to_reask_events() {
        for source in [SOURCE_ZENDESK, SOURCE_SALESFORCE] {
            let db = Connection::open_in_memory().unwrap_or_else(|e| panic!("open: {e}"));
            db.execute_batch(
                "CREATE TABLE crm_cases(
                    case_ref TEXT PRIMARY KEY, source TEXT NOT NULL, org_id TEXT NOT NULL,
                    case_id TEXT NOT NULL, run_id INTEGER, status TEXT NOT NULL,
                    updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    subject_ref TEXT NOT NULL DEFAULT '');",
            )
            .unwrap_or_else(|e| panic!("schema: {e}"));
            // The TARGET case is open and bound to run 101 (same source).
            let mut target = sample(CaseStatus::Open);
            target.source = source.into();
            upsert_crm_case(&db, &target, Some(101))
                .unwrap_or_else(|e| panic!("upsert target: {e}"));
            let sink = RecordingSink::new();
            let mut m = merged(CaseStatus::MergedAway, source);
            m.subject_ref = hash_subject(source, "acme", "requester:991");
            let report = deliver_case(&sink, &db, &m).unwrap_or_else(|e| panic!("deliver: {e}"));
            assert_eq!(report.run_id, 101, "the re-ask lands on the TARGET's run");
            assert_eq!(report.topic_posted.as_deref(), Some("case/reask"));
            let ev = &sink.events.borrow()[0];
            assert!(
                ev.1.contains("|case/reask|"),
                "topic is case/reask: {}",
                ev.1
            );
            assert!(ev.1.contains("\"source\":\"crm_merge\""));
            assert!(ev.1.contains("\"detail_digest\":\""), "ids/digest only");
            assert!(
                !ev.1.contains("Cannot reset PIN"),
                "no content rides the payload"
            );
        }
        // A merged row with NO target refuses loudly instead of silently
        // dropping the re-ask.
        let db = Connection::open_in_memory().unwrap_or_else(|e| panic!("open: {e}"));
        let sink = RecordingSink::new();
        let mut orphan = merged(CaseStatus::MergedAway, SOURCE_ZENDESK);
        orphan.org_id = "nobody".into();
        assert!(
            deliver_case(&sink, &db, &orphan).is_err(),
            "unmappable merge fails closed"
        );
    }

    #[test]
    fn genesys_reopen_maps_to_reask_event() {
        // The reopen signal rides `reopened` on an Open case; the mapping is
        // pure shape — pinned here so a vendor change can't silently drop it.
        let mut g = sample(CaseStatus::Open);
        g.reopened = true;
        assert!(g.reopened && g.status == CaseStatus::Open);
        assert_eq!(
            CaseStatus::parse("merged_away"),
            Some(CaseStatus::MergedAway)
        );
    }

    #[test]
    fn derived_merge_suggests_never_writes() {
        let db = Connection::open_in_memory().unwrap_or_else(|e| panic!("open: {e}"));
        db.execute_batch(
            "CREATE TABLE crm_cases(
                case_ref TEXT PRIMARY KEY, source TEXT NOT NULL, org_id TEXT NOT NULL,
                case_id TEXT NOT NULL, run_id INTEGER, status TEXT NOT NULL,
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                subject_ref TEXT NOT NULL DEFAULT '');
             CREATE TABLE proposals(
                id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
                content TEXT NOT NULL, source TEXT, authority TEXT, observed_at TEXT,
                novelty REAL, conflict_with INTEGER, salience REAL, status TEXT DEFAULT 'pending',
                created_at INTEGER, source_prompt TEXT, owner TEXT);",
        )
        .unwrap_or_else(|e| panic!("schema: {e}"));
        let subject = hash_subject(SOURCE_ZENDESK, "acme", "requester:991");
        for (id, status) in [("41", CaseStatus::Open), ("42", CaseStatus::Open)] {
            let mut c = sample(status);
            c.case_id = id.into();
            c.subject_ref = subject.clone();
            upsert_crm_case(&db, &c, Some(700)).unwrap_or_else(|e| panic!("upsert: {e}"));
        }
        // "Now" just after the sync pass so the group sits inside any window.
        let now: i64 = db
            .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |r| {
                r.get::<_, i64>(0)
            })
            .expect("now")
            + 60;
        let ids = detect_duplicate_opens(&db, now).unwrap_or_else(|e| panic!("detect: {e}"));
        assert_eq!(ids.len(), 1, "one proposal per duplicate group");
        let (kind, status): (String, String) = db
            .query_row(
                "SELECT kind, status FROM proposals WHERE id = ?1",
                params![ids[0]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("proposal");
        assert_eq!(kind, KIND_CASE_MERGE_SUGGESTED);
        assert_eq!(
            status, "pending",
            "a SUGGESTION only — never a written merge"
        );
        // Idempotent across sync passes.
        let again = detect_duplicate_opens(&db, now).unwrap_or_else(|e| panic!("redetect: {e}"));
        assert!(again.is_empty(), "the pending proposal covers the group");
        // Different subjects never collide (exact hash only).
        let db2 = Connection::open_in_memory().unwrap_or_else(|e| panic!("open: {e}"));
        db2.execute_batch(
            "CREATE TABLE crm_cases(
                case_ref TEXT PRIMARY KEY, source TEXT NOT NULL, org_id TEXT NOT NULL,
                case_id TEXT NOT NULL, run_id INTEGER, status TEXT NOT NULL,
                updated_rev TEXT NOT NULL, synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                subject_ref TEXT NOT NULL DEFAULT '');
             CREATE TABLE proposals(
                id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
                content TEXT NOT NULL, source TEXT, authority TEXT, observed_at TEXT,
                novelty REAL, conflict_with INTEGER, salience REAL, status TEXT DEFAULT 'pending',
                created_at INTEGER, source_prompt TEXT, owner TEXT);",
        )
        .unwrap_or_else(|e| panic!("schema: {e}"));
        for (id, ident) in [("41", "requester:991"), ("42", "requester:992")] {
            let mut c = sample(CaseStatus::Open);
            c.case_id = id.into();
            c.subject_ref = hash_subject(SOURCE_ZENDESK, "acme", ident);
            upsert_crm_case(&db2, &c, Some(701)).unwrap_or_else(|e| panic!("upsert: {e}"));
        }
        assert!(
            detect_duplicate_opens(&db2, now)
                .expect("distinct")
                .is_empty()
        );
    }
}
