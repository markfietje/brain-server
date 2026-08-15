//! v1.26.0 "Cross-Border" — the multi-jurisdiction evidence layer for a PH BPO
//! serving US/UK/EU/AU/SG/CA clients.
//!
//! Honest framing (mirrors the plan): a PH BPO is a sub-processor; it must
//! satisfy RA 10173 AND the client country's law (GDPR Art 46 SCCs + TIA, UK
//! IDTA, US DPF/HIPAA-BAA, AU APPs, SG PDPA, CA PIPEDA). This module ships the
//! **evidence + tagging** layer on existing primitives (DSAR, region-pin,
//! audit, provenance) — no new enforcement. It makes the operator *defensible*
//! to client-country regulators; it does not make them a controller (the
//! client stays controller; the BPO + brain-server are processors/
//! sub-processors).
//!
//! M1 — the Art 30 + Art 46 cross-border transfer register (`transfers`
//! table). M2 — per-jurisdiction DSAR deadlines + rights (`JurisdictionRule`).
//! M3 — `lawful_basis`/`purpose` tagging (the data-minimization +
//! purpose-limitation evidence; a strict-mode record without a basis is
//! flagged). M4 — the TIA (Schrems II) + DPA (Art 28) evidence templates,
//! pre-filled from the register. Jurisdiction rules + destination-surveillance
//! postures are a **curated, versioned table** — law evolves; a human (DPO/
//! legal) signs the artifacts this module pre-fills.

use rusqlite::{Connection, Transaction};

use crate::handlers::HandlerError;

/// Max free-text `purpose` label on a transfer / record.
pub(crate) const MAX_TRANSFER_PURPOSE: usize = 500;
/// Max counterparty/notes length on a transfer row.
pub(crate) const MAX_TRANSFER_CP: usize = 500;
/// Max rows `GET /transfers` returns (the `MAX_BREACH_LIMIT`-style clamp).
pub(crate) const MAX_TRANSFER_LIMIT: i64 = 300;

/// The legal transfer mechanisms (Art 46 safeguards) the register accepts.
/// Operator-set — the operator knows which SCC/DPF they signed; we don't
/// scrape commission decisions. Each is a fixed identifier the TIA/DPA
/// templates branch on.
pub const MECHANISMS: &[&str] = &[
    "scc-eu-2021",
    "uk-idta",
    "dpf-us",
    "cbpr",
    "bcr",
    "adequacy",
];

/// The GDPR Art 6 / RA 10173 lawful bases accepted for `lawful_basis` tagging.
pub const LAWFUL_BASISES: &[&str] = &[
    "consent",
    "contract",
    "legal-obligation",
    "vital-interest",
    "public-task",
    "legitimate-interest",
];

/// True for a code the register knows (lowercased). Used by the validation +
/// the DSAR jurisdiction gate. Falls back to accepting any short lowercase
/// code so a future law can be added without a release (the curated table is
/// for deadlines/rights, not for an allowlist of existence).
pub fn is_jurisdiction_code(code: &str) -> bool {
    let c = code.trim().to_ascii_lowercase();
    !c.is_empty()
        && c.len() <= 16
        && c.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
}

/// One jurisdiction's DSAR rule: the law name, the response deadline, and the
/// applicable data-subject rights. `deadline_days: None` = "reasonable
/// (commensurate)" per the law (PH RA 10173) — the operator window is used as
/// the practical countdown. Curated + versioned; re-checked on each release.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct JurisdictionRule {
    pub code: &'static str,
    pub law: &'static str,
    pub deadline_days: Option<i64>,
    pub rights: &'static [&'static str],
}

const JURISDICTIONS: &[JurisdictionRule] = &[
    JurisdictionRule {
        code: "eu",
        law: "GDPR (Art 12/15/17)",
        deadline_days: Some(30),
        rights: &[
            "access",
            "erasure",
            "portability",
            "rectification",
            "objection",
            "automated-decision",
        ],
    },
    JurisdictionRule {
        code: "uk",
        law: "UK GDPR (Art 12/15/17)",
        deadline_days: Some(30),
        rights: &[
            "access",
            "erasure",
            "portability",
            "rectification",
            "objection",
        ],
    },
    JurisdictionRule {
        code: "us",
        law: "CCPA/CPRA (Cal. Civ. Code 1798)",
        deadline_days: Some(45),
        rights: &["access", "erasure", "rectification", "opt-out-of-sale"],
    },
    JurisdictionRule {
        code: "au",
        law: "Privacy Act 1988 (APPs)",
        deadline_days: Some(30),
        rights: &["access", "erasure", "rectification"],
    },
    JurisdictionRule {
        code: "sg",
        law: "PDPA 2012",
        deadline_days: Some(30),
        rights: &["access", "erasure", "rectification"],
    },
    JurisdictionRule {
        code: "ca",
        law: "PIPEDA (SC 2000 c.5)",
        deadline_days: Some(30),
        rights: &["access", "erasure", "rectification"],
    },
    JurisdictionRule {
        code: "ph",
        law: "RA 10173 (DPA 2012)",
        deadline_days: None, // "reasonable (commensurate)" — operator window used
        rights: &["access", "erasure", "rectification"],
    },
];

/// Resolve a jurisdiction code to its curated rule. Unknown → `None` (the DPO
/// confirms; fail-open on the producer, fail-closed only on what we know).
pub fn jurisdiction_rule(code: &str) -> Option<&'static JurisdictionRule> {
    let c = code.trim().to_ascii_lowercase();
    JURISDICTIONS.iter().find(|j| j.code == c)
}

/// The DSAR erasure deadline for a subject of `code`: the curated days when the
/// law fixes a number, else the operator's `BRAIN_DSAR_WINDOW_DAYS` window
/// (the PH "reasonable" fallback). Unknown code → operator window. Pure, so
/// the `dsar_deadline_matches_jurisdiction` test pins the law directly.
pub fn dsar_deadline_for(created_at: i64, code: &str) -> i64 {
    match jurisdiction_rule(code) {
        Some(r) => match r.deadline_days {
            Some(days) => created_at + days * 86400,
            // PH RA 10173 "reasonable (commensurate)": the operator window is
            // the practical countdown (BRAIN_DSAR_WINDOW_DAYS).
            None => created_at + crate::config::dsar_window_secs(),
        },
        // Unknown law → the operator window (the DPO confirms the law).
        None => created_at + crate::config::dsar_window_secs(),
    }
}

/// The destination country's surveillance posture for the Schrems II TIA.
/// A curated snapshot (not a legal opinion) — the operator's counsel confirms
/// the assessment. `note` names the specific authority to weigh.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct SurveillancePosture {
    pub country: &'static str,
    pub note: &'static str,
}

const SURVEILLANCE: &[SurveillancePosture] = &[
    SurveillancePosture { country: "us", note: "FISA 702 bulk authorities + CLOUD Act extraterritorial reach — s.46(2) (equivalent protection) must be assessed." },
    SurveillancePosture { country: "eu", note: "GDPR-compliant member state; law-enforcement access regulated under Art 48." },
    SurveillancePosture { country: "uk", note: "Investigatory Powers Act 2016 — bulk powers weigh on the equivalent-protection assessment." },
    SurveillancePosture { country: "au", note: "Telecommunications interception + law-enforcement access regimes assessed." },
    SurveillancePosture { country: "sg", note: "PDPA + regulated law-enforcement access; assessed." },
    SurveillancePosture { country: "ca", note: "PIPEDA + regulated law-enforcement access; assessed." },
];

/// The destination-surveillance posture for a TIA. Unknown → `None`.
pub fn destination_posture(code: &str) -> Option<&'static SurveillancePosture> {
    let c = code.trim().to_ascii_lowercase();
    SURVEILLANCE.iter().find(|p| p.country == c)
}

/// M3: the purpose-limitation + data-minimization flag. A record ingested into
/// a **strict-mode domain** without a documented `lawful_basis` is flagged
/// (NPC 2024-04 + GDPR Art 5/6). `None`/blank basis → flagged; a non-strict
/// domain never flags (the operator chose a lighter posture).
pub fn lawful_basis_flag(strict_domain: bool, basis: Option<&str>) -> bool {
    strict_domain && basis.map(str::trim).filter(|b| !b.is_empty()).is_none()
}

/// One cross-border transfer register row (Art 30 processing-activities +
/// Art 46 transfer-safeguard evidence).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Transfer {
    pub id: i64,
    pub dataset: String,
    pub origin_jurisdiction: String,
    pub destination_jurisdiction: String,
    pub mechanism: String,
    pub counterparty: String,
    pub lawful_basis: String,
    pub purpose: String,
    pub signed_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// Validate the register payload before any write. Shared by the handler.
pub(crate) fn validate_register(
    dataset: &str,
    origin: &str,
    destination: &str,
    mechanism: &str,
    counterparty: &str,
    purpose: &str,
    lawful_basis: Option<&str>,
) -> Result<(), HandlerError> {
    if dataset.trim().is_empty() || dataset.len() > MAX_TRANSFER_CP {
        return Err(HandlerError::bad_request(
            "transfer_dataset_invalid",
            format!("dataset is required and ≤ {MAX_TRANSFER_CP} characters"),
        ));
    }
    if !is_jurisdiction_code(origin) || !is_jurisdiction_code(destination) {
        return Err(HandlerError::bad_request(
            "transfer_jurisdiction_invalid",
            "origin/destination must be a short lowercase jurisdiction code",
        ));
    }
    if !MECHANISMS.contains(&mechanism.trim().to_ascii_lowercase().as_str()) {
        return Err(HandlerError::bad_request(
            "transfer_mechanism_invalid",
            format!("mechanism must be one of {MECHANISMS:?}"),
        ));
    }
    if counterparty.trim().is_empty() || counterparty.len() > MAX_TRANSFER_CP {
        return Err(HandlerError::bad_request(
            "transfer_counterparty_invalid",
            format!("counterparty is required and ≤ {MAX_TRANSFER_CP} characters"),
        ));
    }
    if purpose.trim().is_empty() || purpose.len() > MAX_TRANSFER_PURPOSE {
        return Err(HandlerError::bad_request(
            "transfer_purpose_invalid",
            format!("purpose is required and ≤ {MAX_TRANSFER_PURPOSE} characters"),
        ));
    }
    if let Some(b) = lawful_basis {
        let b = b.trim();
        if !LAWFUL_BASISES.contains(&b.to_ascii_lowercase().as_str()) {
            return Err(HandlerError::bad_request(
                "transfer_basis_invalid",
                format!("lawful_basis must be one of {LAWFUL_BASISES:?}"),
            ));
        }
    }
    Ok(())
}

/// Record a cross-border transfer. Runs inside the caller's transaction.
#[allow(clippy::too_many_arguments)] // 9 register fields; a struct would add ceremony to the single-write path
pub(crate) fn register(
    tx: &Transaction,
    dataset: &str,
    origin: &str,
    destination: &str,
    mechanism: &str,
    counterparty: &str,
    lawful_basis: Option<&str>,
    purpose: &str,
    signed_at: Option<i64>,
    expires_at: Option<i64>,
) -> Result<i64, HandlerError> {
    tx.execute(
        "INSERT INTO transfers(dataset, origin_jurisdiction, destination_jurisdiction,
            mechanism, counterparty, lawful_basis, purpose, signed_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            dataset.trim(),
            origin.trim().to_ascii_lowercase(),
            destination.trim().to_ascii_lowercase(),
            mechanism.trim().to_ascii_lowercase(),
            counterparty.trim(),
            lawful_basis.map(str::trim),
            purpose.trim(),
            signed_at,
            expires_at
        ],
    )
    .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(tx.last_insert_rowid())
}

fn transfer_row(r: &rusqlite::Row) -> rusqlite::Result<Transfer> {
    Ok(Transfer {
        id: r.get(0)?,
        dataset: r.get(1)?,
        origin_jurisdiction: r.get(2)?,
        destination_jurisdiction: r.get(3)?,
        mechanism: r.get(4)?,
        counterparty: r.get(5)?,
        lawful_basis: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        purpose: r.get(7)?,
        signed_at: r.get(8)?,
        expires_at: r.get(9)?,
    })
}

const TRANSFER_SELECT: &str = "SELECT id, dataset, origin_jurisdiction, destination_jurisdiction,
            mechanism, counterparty, lawful_basis, purpose, signed_at, expires_at
         FROM transfers";

/// `GET /transfers` — the register, newest-first, bounded, with optional
/// filters (mechanism / jurisdiction / dataset). Filtering is an exact match on
/// the curated codes; absent filter = all rows (legacy behavior).
pub(crate) fn list(
    conn: &Connection,
    limit: i64,
    mechanism: Option<&str>,
    jurisdiction: Option<&str>,
    dataset: Option<&str>,
) -> Result<Vec<Transfer>, HandlerError> {
    let limit = limit.clamp(1, MAX_TRANSFER_LIMIT);
    let mut sql = String::from(TRANSFER_SELECT);
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(m) = mechanism.map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push("mechanism = ?".to_string());
        params.push(Box::new(m.to_ascii_lowercase()));
    }
    if let Some(j) = jurisdiction.map(str::trim).filter(|s| !s.is_empty()) {
        let j = j.to_ascii_lowercase();
        clauses.push("(origin_jurisdiction = ? OR destination_jurisdiction = ?)".to_string());
        params.push(Box::new(j.clone()));
        params.push(Box::new(j));
    }
    if let Some(d) = dataset.map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push("dataset = ?".to_string());
        params.push(Box::new(d.to_string()));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    params.push(Box::new(limit));
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), transfer_row)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

pub(crate) fn transfer_by_id(conn: &Connection, id: i64) -> Result<Option<Transfer>, HandlerError> {
    conn.query_row(
        &format!("{TRANSFER_SELECT} WHERE id = ?1"),
        rusqlite::params![id],
        transfer_row,
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
    .map_err(|e| HandlerError::internal(e.to_string()))
}

/// M4 — the Transfer Impact Assessment (Schrems II) template, pre-filled from
/// the register row + the destination jurisdiction's law + surveillance
/// posture. An **evidence artifact**: a human (DPO/legal) reviews + signs it.
#[derive(Debug, serde::Serialize)]
pub struct TiaTemplate {
    pub transfer: Transfer,
    pub destination_law: String,
    pub destination_posture: Option<SurveillancePosture>,
    pub sections: Vec<TiaSection>,
}

#[derive(Debug, serde::Serialize)]
pub struct TiaSection {
    pub key: &'static str,
    pub title: &'static str,
    pub prompt: String,
}

/// Build the TIA template for a transfer. Returns `None` when the transfer is
/// unknown (→ 404). The sections are pre-filled prompts — the operator answers
/// the "does the SCC provide sufficient protection given the destination's
/// surveillance?" question the regulator asks.
pub(crate) fn tia_from(conn: &Connection, id: i64) -> Result<Option<TiaTemplate>, HandlerError> {
    let Some(transfer) = transfer_by_id(conn, id)? else {
        return Ok(None);
    };
    let law = jurisdiction_rule(&transfer.destination_jurisdiction)
        .map(|r| r.law)
        .unwrap_or("no curated law — confirm with counsel")
        .to_string();
    let posture = destination_posture(&transfer.destination_jurisdiction);
    let assess_surveillance = match posture {
        Some(p) => p.note.to_string(),
        None => "no curated surveillance posture — confirm the destination's surveillance regime with counsel."
            .to_string(),
    };
    let sections = vec![
        TiaSection {
            key: "transfer",
            title: "Transfer to be assessed",
            prompt: format!(
                "Dataset '{}' from {} to {} under mechanism '{}', counterparty '{}'.",
                transfer.dataset,
                transfer.origin_jurisdiction,
                transfer.destination_jurisdiction,
                transfer.mechanism,
                transfer.counterparty
            ),
        },
        TiaSection {
            key: "equivalent_protection",
            title: "Equivalent protection (s.46(2) / Schrems II)",
            prompt: format!("Applicable law: {law}. {assess_surveillance}"),
        },
        TiaSection {
            key: "supplementary",
            title: "Supplementary measures",
            prompt: "List any additional safeguards (encryption, pseudonymisation, access controls) that mitigate the risk identified above."
                .to_string(),
        },
        TiaSection {
            key: "signoff",
            title: "Sign-off (DPO / legal)",
            prompt: "Signed by (name, role, date) after review. This artifact does not replace legal advice."
                .to_string(),
        },
    ];
    Ok(Some(TiaTemplate {
        transfer,
        destination_law: law,
        destination_posture: posture.copied(),
        sections,
    }))
}

/// M4 — the DPA (Art 28 sub-processor terms) fields, pre-filled from the
/// register row + mechanism defaults. Exported as the evidence a client's
/// controller demands before authorizing the sub-processor.
pub(crate) fn dpa_fields(t: &Transfer) -> serde_json::Value {
    serde_json::json!({
        "counterparty": t.counterparty,
        "dataset": t.dataset,
        "mechanism": t.mechanism,
        "role": "processor/sub-processor",
        "retention": "per the domain retention policy; erased on termination",
        "deletion_on_termination": true,
        "audit_rights": "documented + exercised on request",
        "breach_notification": "within the affected jurisdictions' statutory windows (see /breaches)",
        "onward_transfers": format!("per the registered mechanism ({}) — no onward transfer without the controller's authorization", t.mechanism),
        "lawful_basis": t.lawful_basis,
        "purpose": t.purpose,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transfers(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dataset TEXT NOT NULL,
                origin_jurisdiction TEXT NOT NULL,
                destination_jurisdiction TEXT NOT NULL,
                mechanism TEXT NOT NULL,
                counterparty TEXT NOT NULL,
                lawful_basis TEXT,
                purpose TEXT NOT NULL,
                signed_at INTEGER,
                expires_at INTEGER);",
        )
        .unwrap();
        conn
    }

    fn add_one(conn: &mut Connection) -> i64 {
        let tx = conn.transaction().unwrap();
        let id = register(
            &tx,
            "esb-support-records",
            "ph",
            "us",
            "scc-eu-2021",
            "Acme Corp",
            Some("contract"),
            "support ticket handling",
            Some(1000),
            Some(2000),
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    #[test]
    fn transfer_register_records_every_cross_border_flow() {
        // Verification 1: a transfer added with mechanism=scc-eu-2021 is
        // queryable (list) + exportable (TIA/DPA templates render it).
        let mut conn = db();
        let id = add_one(&mut conn);
        let rows = list(&conn, 10, None, None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mechanism, "scc-eu-2021");
        assert_eq!(rows[0].destination_jurisdiction, "us");
        // Filtering by mechanism + jurisdiction + dataset all resolve it.
        assert_eq!(
            list(&conn, 10, Some("scc-eu-2021"), None, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(list(&conn, 10, None, Some("us"), None).unwrap().len(), 1);
        assert_eq!(
            list(&conn, 10, None, None, Some("esb-support-records"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list(&conn, 10, Some("dpf-us"), None, None).unwrap().len(),
            0
        );
        // The TIA + DPA templates render the registered transfer.
        let tia = tia_from(&conn, id).unwrap().unwrap();
        assert_eq!(tia.transfer.id, id);
        assert_eq!(tia.destination_law, "CCPA/CPRA (Cal. Civ. Code 1798)");
        assert!(tia.destination_posture.is_some());
        assert_eq!(tia.sections.len(), 4);
        let dpa = dpa_fields(&rows[0]);
        assert_eq!(dpa["counterparty"], "Acme Corp");
        // Unknown id → None (404).
        assert!(tia_from(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn dsar_deadline_matches_jurisdiction() {
        // Verification 2: an EU subject's DSAR deadline is 30 days out; a CCPA
        // subject is 45 days out; PH (reasonable) falls back to the operator
        // window (30 days default).
        let created = 1_000_000;
        assert_eq!(dsar_deadline_for(created, "eu"), created + 30 * 86400);
        assert_eq!(dsar_deadline_for(created, "us"), created + 45 * 86400);
        assert_eq!(dsar_deadline_for(created, "sg"), created + 30 * 86400);
        // PH: reasonable — the operator window (BRAIN_DSAR_WINDOW_DAYS default 30).
        assert_eq!(
            dsar_deadline_for(created, "ph"),
            created + crate::config::dsar_window_secs()
        );
        // Unknown code → operator window.
        assert_eq!(
            dsar_deadline_for(created, "xx"),
            created + crate::config::dsar_window_secs()
        );
    }

    #[test]
    fn jurisdiction_rights_surface_are_curated() {
        // Verification 2b: the EU rule lists the full GDPR rights surface;
        // CCPA lists its own (no portability/objection).
        let eu = jurisdiction_rule("eu").unwrap();
        assert!(eu.rights.contains(&"portability"));
        assert!(eu.rights.contains(&"automated-decision"));
        let us = jurisdiction_rule("us").unwrap();
        assert!(us.rights.contains(&"opt-out-of-sale"));
        assert!(!us.rights.contains(&"portability"));
        // RA 10173 rule carries no fixed deadline (reasonable).
        assert_eq!(jurisdiction_rule("ph").unwrap().deadline_days, None);
        assert!(jurisdiction_rule("xx").is_none());
    }

    #[test]
    fn lawful_basis_strict_flagged_only_when_missing_in_strict_domain() {
        // Verification 4: a record without a basis in a strict domain is
        // flagged; a documented basis (or a non-strict domain) is not.
        assert!(lawful_basis_flag(true, None));
        assert!(lawful_basis_flag(true, Some(" ")));
        assert!(!lawful_basis_flag(true, Some("contract")));
        assert!(!lawful_basis_flag(false, None));
    }

    #[test]
    fn tia_prefilled_from_register_and_posture() {
        // Verification 5: the TIA template pulls the transfer + the
        // destination-surveillance table (Schrems II).
        let mut conn = db();
        let id = add_one(&mut conn);
        let tia = tia_from(&conn, id).unwrap().unwrap();
        let eq = tia
            .sections
            .iter()
            .find(|s| s.key == "equivalent_protection")
            .unwrap();
        assert!(eq.prompt.contains("CCPA/CPRA"));
        assert!(eq.prompt.contains("FISA 702"), "posture named");
        // A destination without a curated posture is handled gracefully.
        let tx = conn.transaction().unwrap();
        let xx = register(&tx, "d", "ph", "zz", "bcr", "C", None, "p", None, None).unwrap();
        tx.commit().unwrap();
        let t = tia_from(&conn, xx).unwrap().unwrap();
        assert!(t.destination_posture.is_none());
        assert!(t.destination_law.contains("confirm with counsel"));
    }

    #[test]
    fn breach_scope_covers_register_jurisdictions() {
        // Verification 6 (v1.25.0 integration): the register's jurisdiction
        // vocabulary feeds the breach-workflow scoping — every affected
        // jurisdiction yields the ph::notification_deadlines rows it has a law
        // for, and a register-known code without a 72h rule (e.g. `us`) yields
        // no deadline (the DPO confirms).
        let eu = crate::ph::notification_deadlines(&["eu".to_string()], 1000);
        assert!(eu
            .iter()
            .any(|d| d.audience == "authority" && d.hours == 72));
        let us = crate::ph::notification_deadlines(&["us".to_string()], 1000);
        assert!(us.is_empty(), "no curated US breach window → DPO confirms");
        // Every curated jurisdiction is a valid register code.
        for j in JURISDICTIONS {
            assert!(is_jurisdiction_code(j.code));
        }
    }

    #[test]
    fn dpa_fields_resolve_any_row_by_id() {
        // Regression: GET /transfers/{id}/dpa must resolve a non-newest row —
        // a by-id lookup, never "the newest N rows then filter".
        let mut conn = db();
        let a = add_one(&mut conn); // newest
        let tx = conn.transaction().unwrap();
        let b = register(
            &tx,
            "older-dataset",
            "eu",
            "uk",
            "uk-idta",
            "C2",
            None,
            "p2",
            None,
            None,
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(a < b, "b is the newest row");
        // Resolve the OLDER (non-newest) row `a` by id — the DPA + TIA must
        // not depend on "list then filter" (which would only ever see `b`).
        let older = transfer_by_id(&conn, a).unwrap().unwrap();
        assert_eq!(older.dataset, "esb-support-records");
        assert_eq!(dpa_fields(&older)["counterparty"], "Acme Corp");
        let tia = tia_from(&conn, a).unwrap().unwrap();
        assert_eq!(tia.transfer.id, a);
        assert_eq!(tia.destination_law, "CCPA/CPRA (Cal. Civ. Code 1798)");
    }

    #[test]
    fn validate_register_bounds_fields() {
        assert!(
            validate_register("d", "ph", "us", "scc-eu-2021", "C", "p", Some("contract")).is_ok()
        );
        assert!(validate_register(" ", "ph", "us", "scc-eu-2021", "C", "p", None).is_err());
        assert!(
            validate_register("d", "PH", "us", "scc-eu-2021", "C", "p", None).is_ok(),
            "case-insensitive codes"
        );
        assert!(validate_register("d", "ph", "us", "not-a-mechanism", "C", "p", None).is_err());
        assert!(validate_register("d", "ph", "us", "scc-eu-2021", " ", "p", None).is_err());
        assert!(validate_register("d", "ph", "us", "scc-eu-2021", "C", "", None).is_err());
        assert!(validate_register(
            "d",
            "ph",
            "us",
            "scc-eu-2021",
            "C",
            "p",
            Some("not-a-basis")
        )
        .is_err());
    }

    #[test]
    fn transfer_list_is_newest_first_and_bounded() {
        let mut conn = db();
        let a = add_one(&mut conn);
        let tx = conn.transaction().unwrap();
        let b = register(
            &tx, "d2", "eu", "uk", "uk-idta", "C2", None, "p2", None, None,
        )
        .unwrap();
        tx.commit().unwrap();
        let rows = list(&conn, 10, None, None, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, b, "newest-first");
        assert_eq!(rows[1].id, a);
        let one = list(&conn, 1, None, None, None).unwrap();
        assert_eq!(one.len(), 1);
    }
}
