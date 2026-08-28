//! The scoreboard core: the outcome/efficiency derivation behind
//! `GET /workflow/scoreboard`, the aftersales KPI cohort, and the monthly
//! calibration's signed score.
//!
//! OWNS the scoreboard aggregate's storage story: the bounded runs page
//! (last-1000 `workflow_runs` rows), the audit-linkage reconstruction
//! (`audit_events` stores only SHA-256 target hashes — a run is audit-linked
//! iff `hash("run:{id}")` appears among workflow-kind rows), and the
//! aftersales cohort read. The pure derivations live beside their only SQL
//! so the signed gate and the board measure the same numbers.
//!
//! Fail-closed postures are pinned: `audited_run_ids` links nothing on a
//! storage error (absence is never green); `score_units_now` scores 0 when
//! the page is unreadable. Wire shaping stays handler-side.

use rusqlite::Connection;

/// The scoreboard runs page: (id, status, state_json) for the last 1000
/// runs, newest first. Row errors propagate (the handler maps to a 500);
/// see [`score_units_now`] for the fail-closed sibling.
pub(crate) fn runs_page(conn: &Connection) -> Result<Vec<(i64, String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, status, state_json FROM workflow_runs ORDER BY id DESC LIMIT 1000")
        .map_err(|e| format!("{e}"))?;
    let mut rows = Vec::new();
    for row in stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("{e}"))?
    {
        rows.push(row.map_err(|e| format!("{e}"))?);
    }
    Ok(rows)
}

/// The aftersales KPI cohort: return / warranty_claim / repair_field runs of
/// the last 1000, derived from the same columns the scoreboard reads.
/// `repeat_within_window` uses the FCR window expression verbatim so FTFR
/// and FCR measure repeats identically; resolution rides terminal status.
pub(crate) fn aftersales_cohort(
    conn: &Connection,
) -> Result<Vec<brain_engine_sdk::aftersales::AftersalesRun>, String> {
    use brain_engine_sdk::aftersales::{AftersalesKind, AftersalesRun};
    let mut stmt = conn
        .prepare(
            "SELECT kind, status, state_json, created_at, updated_at FROM workflow_runs
             ORDER BY id DESC LIMIT 1000",
        )
        .map_err(|e| format!("{e}"))?;
    let mut rows = Vec::new();
    let result = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    });
    for row in result.map_err(|e| format!("{e}"))?.filter_map(Result::ok) {
        let Some(kind) = AftersalesKind::from_kind(&row.0) else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(&row.2).unwrap_or(serde_json::Value::Null);
        let repeat_within_window = v
            .get("repeat_contact")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
            || v.get("prev_contact_age_secs")
                .and_then(|a| a.as_i64())
                .map(|age| age >= 0 && age <= crate::config::fcr_window_days() * 86400)
                .unwrap_or(false);
        rows.push(AftersalesRun {
            kind,
            created_at: row.3,
            resolved_at: matches!(row.1.as_str(), "completed" | "closed").then_some(row.4),
            repeat_within_window,
            returnless: v
                .get("returnless")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
            fraud_flagged: v
                .get("fraud_flagged")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
        });
    }
    Ok(rows)
}

/// Reconstruct the set of run ids that a workflow audit row references.
///
/// `audit_events` stores only SHA-256 target hashes — there is no plain-text
/// `target` column to cast. Every run-bound substrate write (open, CAS
/// transition, answer, state_read) targets the canonical `run:{id}` string,
/// so a run is audit-linked iff `hash("run:{id}")` appears among the
/// workflow-kind rows. Anything else (outbox/calibration rows) targets other
/// strings and must never light up a run.
pub(crate) fn audited_run_ids(
    conn: &Connection,
    run_ids: impl Iterator<Item = i64>,
) -> std::collections::HashSet<i64> {
    let Ok(mut stmt) =
        conn.prepare("SELECT DISTINCT target_hash FROM audit_events WHERE kind = 'workflow'")
    else {
        return Default::default();
    };
    let targets: std::collections::HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    run_ids
        .filter(|id| targets.contains(&crate::audit::hash(&format!("run:{id}"))))
        .collect()
}

/// Pure derivation of scorer artifacts from run rows + the audited-id set.
/// Fail-closed: `audit_ok` only when a state flag says so OR an audit row
/// references the run.
pub(crate) fn derive_artifacts(
    rows: &[(i64, String, String)],
    audited: &std::collections::HashSet<i64>,
) -> Vec<brain_engine_sdk::scoreboard::RunArtifacts> {
    rows.iter()
        .map(|(id, status, state_json)| {
            let v: serde_json::Value =
                serde_json::from_str(state_json).unwrap_or(serde_json::Value::Null);
            let flag = v.get("audit_ok").and_then(|b| b.as_bool()).unwrap_or(false);
            brain_engine_sdk::scoreboard::RunArtifacts {
                audit_ok: flag || audited.contains(id),
                ..artifacts_from_row(status, &v)
            }
        })
        .collect()
}

/// The current mean per-run score, derived exactly as the scoreboard derives
/// it (same queries, same fail-closed audit linkage) — the signed gate and
/// the weekly report must measure the same number.
pub(crate) fn score_units_now(conn: &Connection) -> i32 {
    let mut stmt = match conn
        .prepare("SELECT id, status, state_json FROM workflow_runs ORDER BY id DESC LIMIT 1000")
    {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let audited = audited_run_ids(conn, rows.iter().map(|(id, _, _)| *id));
    let runs = derive_artifacts(&rows, &audited);
    if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|r| brain_engine_sdk::pure::qa_score::score_run(r).total_units)
            .sum::<i32>()
            / runs.len() as i32
    }
}

fn artifacts_from_row(
    status: &str,
    v: &serde_json::Value,
) -> brain_engine_sdk::scoreboard::RunArtifacts {
    use brain_engine_sdk::pure::qa_score::StepRow;
    brain_engine_sdk::scoreboard::RunArtifacts {
        steps: v
            .get("steps")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|s| StepRow {
                        expected: s
                            .get("expected")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .into(),
                        actual: s
                            .get("actual")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .into(),
                        skipped_verify: s
                            .get("skipped_verify")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        abstained: s
                            .get("abstained")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        guidance_accepted: s.get("guidance_accepted").and_then(|x| x.as_bool()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        findings: v
            .get("findings")
            .and_then(|f| f.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        contradictions: v
            .get("contradictions")
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as usize,
        audit_ok: false, // overridden by the caller's fail-closed check
        repeat_contact: v
            .get("repeat_contact")
            .and_then(|r| r.as_bool())
            .unwrap_or(false)
            // FCR window (SQM-style, docs/metrics.md): a recurrence whose
            // recorded age falls inside the window marks this run's
            // predecessor as NOT first-contact-resolved.
            || v.get("prev_contact_age_secs")
                .and_then(|a| a.as_i64())
                .map(|age| age >= 0 && age <= crate::config::fcr_window_days() * 86400)
                .unwrap_or(false),
        handoff_complete: status == "completed",
        verified: v.get("verified").and_then(|b| b.as_bool()).unwrap_or(false),
        escalation_honored: v
            .get("escalation_honored")
            .and_then(|b| b.as_bool())
            .unwrap_or(true),
    }
}

#[cfg(test)]
mod scoreboard_tests {
    use super::*;
    use brain_engine_sdk::scoreboard::StepRow;

    #[test]
    fn derivation_defaults_and_fail_closed_audit_ok() {
        let audited = std::collections::HashSet::from([7]);
        let rows = vec![
            // completed run WITH an audit row: audit_ok true via linkage.
            (
                7i64,
                "completed".to_string(),
                r#"{"steps":[{"expected":"a","actual":"a"}]}"#.to_string(),
            ),
            // completed run WITHOUT any audit linkage and no flag: audit_ok
            // stays FALSE — absence never counts green.
            (8, "completed".to_string(), "{}".to_string()),
            // recorded flag wins over missing linkage.
            (9, "failed".to_string(), r#"{"audit_ok":true}"#.to_string()),
        ];
        let runs = derive_artifacts(&rows, &audited);
        assert_eq!(runs.len(), 3);
        assert!(runs[0].audit_ok && runs[2].audit_ok);
        assert!(!runs[1].audit_ok, "no audit row + no flag => not green");
        assert!(!runs[0].handoff_complete.eq(&false));
        assert_eq!(
            runs[0].steps,
            vec![StepRow {
                expected: "a".into(),
                actual: "a".into(),
                skipped_verify: false,
                abstained: false,
                guidance_accepted: None,
            }]
        );
    }

    #[test]
    fn empty_input_scores_zero_not_panic() {
        let sb = brain_engine_sdk::scoreboard::build(&[]);
        assert_eq!(sb.fcr_units, 0);
        assert!(sb.audit_green, "vacuous conjunction is true by definition");
    }

    /// Regression pin: `audit_events` has no plain-text `target` column (the
    /// old `CAST(target AS INTEGER)` query 500s). The audited set must
    /// reconstruct via `hash("run:{id}")` membership and stay fail-closed.
    #[test]
    fn audited_run_ids_reconstructs_hashed_targets() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_events(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT DEFAULT CURRENT_TIMESTAMP,
                kind TEXT NOT NULL,
                actor TEXT,
                target_hash TEXT,
                status TEXT,
                detail_hash TEXT,
                tenant_id TEXT NOT NULL DEFAULT 'global',
                prev_hash TEXT);",
        )
        .unwrap();
        for (target, kind) in [
            ("run:7", "workflow"),     // canonical run-bound row → links run 7
            ("outbox:k1", "workflow"), // substrate row bound to no run id
            ("run:9", "client"),       // wrong kind must never link
        ] {
            conn.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status)
                 VALUES (?1, 'workflow', ?2, 'ok')",
                rusqlite::params![kind, crate::audit::hash(target)],
            )
            .unwrap();
        }
        let audited = audited_run_ids(&conn, [7i64, 8, 9].into_iter());
        assert!(audited.contains(&7), "hash(run:7) linkage must reconstruct");
        assert!(!audited.contains(&8), "absence never counts green");
        assert!(
            !audited.contains(&9),
            "non-workflow kinds must not satisfy workflow audit linkage"
        );
    }

    /// Env-var config is process-global: env-mutating tests take the shared
    /// lock, tolerantly (poison never cascades).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// fcr_window_is_configurable_and_deterministic
    #[test]
    fn fcr_window_is_configurable_and_deterministic() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Default: 7 days, deterministic across reads.
        // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
        unsafe { std::env::remove_var("BRAIN_FCR_WINDOW_DAYS") };
        assert_eq!(crate::config::fcr_window_days(), 7);
        assert_eq!(crate::config::fcr_window_days(), 7);
        // Override applies; invalid values fall back to the default.
        // SAFETY: single-threaded under ENV_LOCK.
        unsafe { std::env::set_var("BRAIN_FCR_WINDOW_DAYS", "3") };
        assert_eq!(crate::config::fcr_window_days(), 3);
        // SAFETY: single-threaded under ENV_LOCK.
        unsafe { std::env::set_var("BRAIN_FCR_WINDOW_DAYS", "-2") };
        assert_eq!(crate::config::fcr_window_days(), 7);
        unsafe { std::env::set_var("BRAIN_FCR_WINDOW_DAYS", "banana") };
        assert_eq!(crate::config::fcr_window_days(), 7);

        // The derivation honors the window: a recurrence inside the window
        // counts the run as a repeat contact; outside it does not.
        let row_within = (
            11i64,
            "completed".to_string(),
            r#"{"prev_contact_age_secs":172800}"#.to_string(), // 2 days < 3
        );
        unsafe { std::env::set_var("BRAIN_FCR_WINDOW_DAYS", "3") };
        assert!(
            derive_artifacts(
                std::slice::from_ref(&row_within),
                &std::collections::HashSet::new()
            )[0]
            .repeat_contact
        );
        let row_outside = (
            12i64,
            "completed".to_string(),
            r#"{"prev_contact_age_secs":259200}"#.to_string(), // exactly 3 days
        );
        // Boundary IS inside (age <= window): deterministic closed window.
        assert!(
            derive_artifacts(&[row_outside], &std::collections::HashSet::new())[0].repeat_contact
        );
        let row_far = (
            13i64,
            "completed".to_string(),
            r#"{"prev_contact_age_secs":259201}"#.to_string(),
        );
        assert!(!derive_artifacts(&[row_far], &std::collections::HashSet::new())[0].repeat_contact);
        unsafe { std::env::remove_var("BRAIN_FCR_WINDOW_DAYS") };
        // With the 7-day default restored, the same 3-day-old recurrence
        // still reads as a repeat — the window moves the boundary, never the
        // arithmetic. (row_within reused deliberately.)
        assert!(
            derive_artifacts(&[row_within], &std::collections::HashSet::new())[0].repeat_contact
        );
    }

    /// Lexicon: the canonical emitted-field list — shared by the docs↔code↔JSON
    /// parity meta-tests below.
    const SCOREBOARD_FIELDS: &[&str] = &[
        "fcr_units",
        "repeat_contact_rate_units",
        "correctness_units",
        "override_rate_units",
        "gap_rate_units",
        "abstention_rate_units",
        "guidance_acceptance_units",
        "handoff_completeness_units",
        "audit_green",
        "escalation_honored_units",
        "runs_scored",
        "calibration_report_emitted",
        "kcs_linkage_rate_units",
        "searched_found_rate_units",
        "article_freshness_median_age_secs",
        "self_service_deflection_units",
        "kb_feedback_total",
        "kb_hot_topics",
        "return_rate_units",
        "warranty_claim_rate_units",
        "ftfr_units",
        "refund_cycle_time_median_secs",
        "returnless_share_units",
        "aftersales_fraud_flag_rate_units",
        "goodwill_total_cents_30d",
        "goodwill_entries_30d",
        "goodwill_unaudited_excluded_30d",
        // v1.28.35 Outreach: ISO 10004 VoC as data.
        "voc_contacts_total",
        "voc_complaints_total",
        "voc_complaints_per_thousand_contacts_units",
        // v1.28.36 Keystone: the re-ask is now counted.
        "reask_rate",
    ];

    /// Dictionary fields defined but deliberately not yet emitted by code
    /// (formula fixed before any emitter ships — the Lexicon posture).
    const PLANNED_DICTIONARY_FIELDS: &[&str] = &["customer_effort_events"];

    fn metrics_doc() -> String {
        let doc_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/metrics.md");
        std::fs::read_to_string(&doc_path)
            .unwrap_or_else(|e| panic!("docs/metrics.md must exist and be readable: {e}"))
    }

    fn metrics_json() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("metrics/metrics.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("metrics/metrics.json must exist and be readable: {e}"));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("metrics/metrics.json must parse: {e}"))
    }

    /// every_scoreboard_field_has_a_dictionary_entry — three-way docs↔code↔JSON
    /// parity: every field the scoreboard emits carries a dictionary entry in
    /// BOTH twins, and neither twin lists a field the scoreboard does not
    /// emit (beyond the explicitly planned set).
    #[test]
    fn every_scoreboard_field_has_a_dictionary_entry() {
        let doc = metrics_doc();
        let json = metrics_json();
        let entries = json["metrics"]
            .as_array()
            .unwrap_or_else(|| panic!("metrics.json must carry a metrics array"));
        for field in SCOREBOARD_FIELDS {
            assert!(
                doc.contains(&format!("`{field}`")),
                "metrics dictionary is missing an entry for `{field}`"
            );
            assert!(
                entries.iter().any(|m| m["name"] == *field),
                "metrics.json twin is missing an entry for `{field}`"
            );
        }
        // Reverse parity over both twins: nothing invented beyond the
        // planned allowlist.
        for entry in entries {
            let name = entry["name"].as_str().unwrap_or_default();
            if !SCOREBOARD_FIELDS.contains(&name) {
                assert!(
                    PLANNED_DICTIONARY_FIELDS.contains(&name),
                    "metrics.json lists `{name}` which the scoreboard does not emit \
                     and the plan does not reserve"
                );
            }
        }
        for line in doc.lines().filter(|l| l.starts_with("| `")) {
            let name = line
                .trim_start_matches("| `")
                .split('`')
                .next()
                .unwrap_or_default();
            if name.contains("_units")
                || [
                    "audit_green",
                    "runs_scored",
                    "calibration_report_emitted",
                    "kb_feedback_total",
                    "kb_hot_topics",
                ]
                .contains(&name)
            {
                assert!(
                    SCOREBOARD_FIELDS.contains(&name),
                    "metrics dictionary lists `{name}` which the scoreboard does not emit"
                );
            }
        }
        // The twin is schema-versioned and every entry is fully attributed.
        assert!(
            json["schema_version"].is_u64(),
            "metrics.json must pin a schema_version"
        );
        for entry in entries {
            let name = entry["name"].as_str().unwrap_or_default();
            for attr in [
                "unit",
                "formula",
                "sources",
                "window",
                "inclusion",
                "exclusion",
                "citation",
                "tier_availability",
                "status",
            ] {
                assert!(
                    entry[attr].is_string() || entry[attr].is_array(),
                    "metrics.json entry `{name}` is missing attribute `{attr}`"
                );
            }
        }
    }

    /// every_entry_source_table_exists_in_schema — every lineage table AND
    /// column named in the machine twin must exist in the real migrated
    /// schema (in-memory run of src/migration.rs). A renamed/dropped table
    /// or column that leaves a stale source reference behind fails here,
    /// not in production reads. Reserved (planned) entries pin tables only.
    #[test]
    fn every_entry_source_table_exists_in_schema() {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 0)
            .unwrap_or_else(|e| panic!("in-memory migration must succeed: {e}"));
        let json = metrics_json();
        for entry in json["metrics"]
            .as_array()
            .unwrap_or_else(|| panic!("metrics.json must carry a metrics array"))
        {
            let name = entry["name"].as_str().unwrap_or_default();
            let planned = entry["status"] == "planned";
            for source in entry["sources"].as_array().unwrap_or(&Vec::new()) {
                let table = source["table"].as_str().unwrap_or_default();
                let n: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        rusqlite::params![table],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(
                    n == 1,
                    "metrics.json entry `{name}` cites table `{table}` which the schema does not define"
                );
                if planned {
                    continue;
                }
                let column = source["column"].as_str().unwrap_or_default();
                if column.is_empty() || column == "-" {
                    continue;
                }
                let cols: Vec<String> = {
                    let mut stmt = conn
                        .prepare(&format!("PRAGMA table_info({table})"))
                        .unwrap();
                    let rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
                    rows.filter_map(|r| r.ok()).collect()
                };
                assert!(
                    cols.iter().any(|c| c == column),
                    "metrics.json entry `{name}` cites `{table}.{column}` which the schema does not define"
                );
            }
        }
    }

    /// formula_change_bumps_scorer_version — the versioning discipline: the
    /// SCORER_VERSION constant stamps the gold packs fail-closed, the JSON
    /// twin pins the same value, and the documented law says a formula change
    /// moves all of them in one PR. Drift between any two anchors fails here.
    #[test]
    fn formula_change_bumps_scorer_version() {
        let json = metrics_json();
        assert_eq!(
            json["scorer_version"].as_str().unwrap_or_default(),
            brain_engine_sdk::calibration::CALIBRATION_SCORER_VERSION,
            "metrics.json scorer_version drifted from SCORER_VERSION"
        );
        let gold_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/gold-sets/gold");
        let mut checked = 0usize;
        let pack = std::fs::read_to_string(gold_dir.join("qc_report.json"))
            .unwrap_or_else(|e| panic!("gold pack qc_report.json must be readable: {e}"));
        for line in pack.lines() {
            if let Some(rest) = line.trim().strip_prefix("\"scorer_version\"") {
                let stated = rest
                    .trim_start_matches([':', ' ', '"'])
                    .trim_end_matches([',', '"']);
                assert_eq!(
                    stated,
                    brain_engine_sdk::calibration::CALIBRATION_SCORER_VERSION,
                    "gold pack qc_report.json pins a stale scorer_version"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "gold pack must pin a scorer_version");
        for entry in std::fs::read_dir(gold_dir.join("gdl_cases"))
            .unwrap_or_else(|e| panic!("gdl_cases dir must be readable: {e}"))
        {
            let path = entry
                .unwrap_or_else(|e| panic!("gdl_cases entry readable: {e}"))
                .path();
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} readable: {e}", path.display()));
            assert!(
                raw.contains(&format!(
                    "\"scorer_version\": \"{}\"",
                    brain_engine_sdk::calibration::CALIBRATION_SCORER_VERSION
                )),
                "{} pins a stale scorer_version",
                path.display()
            );
        }
        let doc = metrics_doc();
        assert!(
            doc.contains("formula change bumps the version"),
            "docs/metrics.md must state the one-PR version-bump law"
        );
    }

    /// Keystone M3: metrics_dictionary_has_reask_rate_entry
    #[test]
    fn metrics_dictionary_has_reask_rate_entry() {
        let doc_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/metrics.md");
        let doc = std::fs::read_to_string(&doc_path)
            .unwrap_or_else(|e| panic!("docs/metrics.md must exist and be readable: {e}"));
        assert!(
            doc.contains("`reask_rate`"),
            "the dictionary carries a reask_rate entry"
        );
        // The entry pins all three deterministic sources and the window var.
        let row = doc
            .lines()
            .find(|l| l.contains("`reask_rate`"))
            .expect("reask_rate table row");
        for term in ["crm_merge", "marked", "derived", "BRAIN_REASK_WINDOW_DAYS"] {
            assert!(
                row.contains(term),
                "reask_rate entry missing '{term}': {row}"
            );
        }
    }

    /// Keystone M3: reask_window_is_env_tunable (mirrors the FCR pattern)
    #[test]
    fn reask_window_is_env_tunable() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
        unsafe { std::env::remove_var("BRAIN_REASK_WINDOW_DAYS") };
        assert_eq!(
            brain_server::connector::crm::reask_window_days(),
            brain_server::connector::crm::DEFAULT_REASK_WINDOW_DAYS
        );
        assert_eq!(brain_server::connector::crm::DEFAULT_REASK_WINDOW_DAYS, 3);
        unsafe { std::env::set_var("BRAIN_REASK_WINDOW_DAYS", "5") };
        assert_eq!(brain_server::connector::crm::reask_window_days(), 5);
        // Garbage and non-positive values fall back, deterministically.
        unsafe { std::env::set_var("BRAIN_REASK_WINDOW_DAYS", "-1") };
        assert_eq!(brain_server::connector::crm::reask_window_days(), 3);
        unsafe { std::env::set_var("BRAIN_REASK_WINDOW_DAYS", "zero") };
        assert_eq!(brain_server::connector::crm::reask_window_days(), 3);
        unsafe { std::env::remove_var("BRAIN_REASK_WINDOW_DAYS") };
        assert_eq!(brain_server::connector::crm::reask_window_days(), 3);
    }
}
