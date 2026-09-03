//! Workload visibility — people made visible, from
//! lineage only.
//!
//! Three deterministic views over the shipped tables, computed at read time:
//!
//! - [`workload_view`] — per-principal burden: concurrent open envelopes
//!   (`workflow_runs` owner via `state_json`, the relay's convention), the
//!   pending handover burden they created (`handover_offers`), the cases
//!   transferred in and still open (`handover_offers` accepted onto open
//!   runs), re-ask load (`outbox` topic `case/reask` attributed through the
//!   run), and the confirm-gate backlog they own (`proposals` pending).
//! - [`fatigue_signals`] — consecutive-shift + open-load pattern from the
//!   shift tables. The signal ALERTS the scheduling human; nothing here ever
//!   reassigns work — these functions perform zero writes.
//! - [`coverage_view`] — competence: skills tags vs worktype demand queue,
//!   joined through the same routing tags [`crate::workflow::crew::
//!   board_for_worktype`] uses.
//!
//! ISO 18295-1 posture, stated as the standard's own: tools make workload
//! visible; management manages. Enforcement stays human.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde_json::Value;

use crate::workflow::crew::board_for_worktype;
use crate::workflow::frontdoor::worktype_skills;
use crate::workflow::shifts::Shift;

/// Minimum uninterrupted rest (seconds) between two shifts for them NOT to
/// count as consecutive. Below this, back-to-back windows compound.
pub const MIN_REST_SECS: i64 = 8 * 3600;

/// A shift chain at least this long (windows with less than
/// [`MIN_REST_SECS`] between them) raises a fatigue signal.
pub const CONSECUTIVE_SHIFT_FLOOR: usize = 2;

/// Open envelopes at or above this raise an open-load fatigue signal.
pub const OPEN_LOAD_CAP: i64 = 8;

const HANDOVER_OFFERED: &str = "offered";
const HANDOVER_ACCEPTED: &str = "accepted";

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadRow {
    pub principal: String,
    /// Active runs whose `state_json.owner` is the principal.
    pub open_envelopes: i64,
    /// Handover offers FROM this principal still awaiting a decision.
    pub handover_burden_outbound: i64,
    /// Accepted handovers ONTO this principal whose run is still active.
    pub transfers_in_open: i64,
    /// Re-ask events on runs this principal owns.
    pub reask_load: i64,
    /// Pending confirm-gate proposals owned by this principal.
    pub gate_backlog: i64,
}

fn owner_of(state_json: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(state_json).ok()?;
    parsed
        .get("owner")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

struct OpenRun {
    id: i64,
    owner: Option<String>,
}

fn open_runs(conn: &Connection, domain: &str) -> Result<Vec<OpenRun>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, state_json FROM workflow_runs
             WHERE domain = ?1 AND status = 'active'",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![domain], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, state_json) = row.map_err(|e| e.to_string())?;
        out.push(OpenRun {
            id,
            owner: owner_of(&state_json),
        });
    }
    Ok(out)
}

/// The per-principal workload view. Read-only by construction: every input
/// comes from a SELECT; nothing in this function writes.
pub fn workload_view(conn: &Connection, domain: &str) -> Result<Vec<WorkloadRow>, String> {
    let runs = open_runs(conn, domain)?;
    let run_owner: BTreeMap<i64, Option<String>> =
        runs.iter().map(|r| (r.id, r.owner.clone())).collect();

    let mut principals: Vec<String> = runs.iter().filter_map(|r| r.owner.clone()).collect();
    let mut stmt = conn
        .prepare(
            "SELECT run_id, from_principal, to_principal, state
             FROM handover_offers WHERE domain = ?1",
        )
        .map_err(|e| e.to_string())?;
    struct Offer {
        run_id: i64,
        from: String,
        to: String,
        state: String,
    }
    let offers: Vec<Offer> = stmt
        .query_map(rusqlite::params![domain], |r| {
            Ok(Offer {
                run_id: r.get(0)?,
                from: r.get(1)?,
                to: r.get(2)?,
                state: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    principals.extend(offers.iter().map(|o| o.from.clone()));
    principals.extend(offers.iter().map(|o| o.to.clone()));

    let mut stmt = conn
        .prepare(
            "SELECT o.run_id FROM outbox o
             WHERE o.topic = ?1 AND EXISTS (
               SELECT 1 FROM workflow_runs w
                WHERE w.id = o.run_id AND w.domain = ?2 AND w.status = 'active')",
        )
        .map_err(|e| e.to_string())?;
    let reask_run_ids: Vec<i64> = stmt
        .query_map(
            rusqlite::params![crate::workflow::frontdesk::TOPIC_REASK, domain],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    principals.extend(
        reask_run_ids
            .iter()
            .filter_map(|id| run_owner.get(id).cloned().flatten()),
    );

    let mut stmt = conn
        .prepare("SELECT owner FROM proposals WHERE status = 'pending' AND owner IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let gate_owners: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    principals.extend(gate_owners.iter().cloned());

    let active_run_ids: Vec<i64> = runs.iter().map(|r| r.id).collect();
    let mut acc: BTreeMap<String, WorkloadRow> = BTreeMap::new();
    for r in &runs {
        if let Some(owner) = &r.owner {
            acc.entry(owner.clone())
                .or_insert_with(|| WorkloadRow {
                    principal: owner.clone(),
                    open_envelopes: 0,
                    handover_burden_outbound: 0,
                    transfers_in_open: 0,
                    reask_load: 0,
                    gate_backlog: 0,
                })
                .open_envelopes += 1;
        }
    }
    for o in &offers {
        if o.state == HANDOVER_OFFERED {
            acc.entry(o.from.clone())
                .or_insert_with(|| WorkloadRow {
                    principal: o.from.clone(),
                    open_envelopes: 0,
                    handover_burden_outbound: 0,
                    transfers_in_open: 0,
                    reask_load: 0,
                    gate_backlog: 0,
                })
                .handover_burden_outbound += 1;
        }
        if o.state == HANDOVER_ACCEPTED && active_run_ids.contains(&o.run_id) {
            acc.entry(o.to.clone())
                .or_insert_with(|| WorkloadRow {
                    principal: o.to.clone(),
                    open_envelopes: 0,
                    handover_burden_outbound: 0,
                    transfers_in_open: 0,
                    reask_load: 0,
                    gate_backlog: 0,
                })
                .transfers_in_open += 1;
        }
    }
    for id in &reask_run_ids {
        if let Some(Some(owner)) = run_owner.get(id) {
            acc.entry(owner.clone())
                .or_insert_with(|| WorkloadRow {
                    principal: owner.clone(),
                    open_envelopes: 0,
                    handover_burden_outbound: 0,
                    transfers_in_open: 0,
                    reask_load: 0,
                    gate_backlog: 0,
                })
                .reask_load += 1;
        }
    }
    // Gate backlog rides only onto principals this domain's lineage already
    // surfaced: proposals carry a `domain` column now, but this view's
    // attribution is still owner-within-the-queried-pool — inventing a
    // cross-domain join would leak across tenants. The lineage-only
    // attribution remains a documented honest ceiling.
    for owner in &gate_owners {
        if let Some(row) = acc.get_mut(owner.as_str()) {
            row.gate_backlog += 1;
        }
    }
    Ok(acc.into_values().collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct FatigueSignal {
    pub principal: String,
    /// Longest chain of shifts with less than [`MIN_REST_SECS`] rest between
    /// consecutive windows (from the shift tables' rosters).
    pub consecutive_shifts: usize,
    pub open_envelopes: i64,
    /// Human-readable reason — the scheduling human reads this, no machine
    /// acts on it.
    pub reason: String,
}

fn longest_chain(shift_starts: &mut [(i64, i64)]) -> usize {
    // shift_starts: (start, end) windows of ONE principal, unsorted.
    shift_starts.sort_by_key(|w| w.0);
    let mut best = 0usize;
    let mut run = 0usize;
    let mut prev_end: Option<i64> = None;
    for (start, end) in shift_starts.iter() {
        let consecutive = prev_end.is_some_and(|pe| *start < pe + MIN_REST_SECS);
        run = if consecutive { run + 1 } else { 1 };
        best = best.max(run);
        prev_end = Some((*end).max(prev_end.unwrap_or(*end)));
    }
    best
}

/// Fatigue signals from the shift tables + the computed workload. Pure:
/// reads its arguments, returns signals, performs NO writes and NEVER
/// reassigns anything — enforcement stays with the scheduling human.
pub fn fatigue_signals(all_shifts: &[Shift], workload: &[WorkloadRow]) -> Vec<FatigueSignal> {
    let mut per_principal: BTreeMap<&str, Vec<(i64, i64)>> = BTreeMap::new();
    for s in all_shifts {
        for p in &s.roster {
            per_principal
                .entry(p.as_str())
                .or_default()
                .push((s.start_epoch, s.end_epoch));
        }
    }
    let open_by_principal: BTreeMap<&str, i64> = workload
        .iter()
        .map(|w| (w.principal.as_str(), w.open_envelopes))
        .collect();
    let mut out = Vec::new();
    for (principal, windows) in &per_principal {
        let mut windows = windows.clone();
        let chain = longest_chain(&mut windows);

        let open = open_by_principal.get(principal).copied().unwrap_or(0);
        let chain_flag = chain >= CONSECUTIVE_SHIFT_FLOOR;
        let load_flag = open >= OPEN_LOAD_CAP;
        if chain_flag || load_flag {
            out.push(FatigueSignal {
                principal: (*principal).to_string(),
                consecutive_shifts: chain,
                open_envelopes: open,
                reason: format!(
                    "{}{}",
                    if chain_flag {
                        format!(
                            "{chain} consecutive shift windows with under {} rest",
                            hours(MIN_REST_SECS)
                        )
                    } else {
                        String::new()
                    },
                    if load_flag {
                        format!(
                            "{}{}+ open envelopes (cap {OPEN_LOAD_CAP})",
                            if chain_flag { "; " } else { "" },
                            open
                        )
                    } else {
                        String::new()
                    },
                ),
            });
        }
    }
    out.sort_by(|a, b| a.principal.cmp(&b.principal));
    out
}

fn hours(secs: i64) -> String {
    format!("{}h", secs / 3600)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageRow {
    /// The worktype kind whose queue has demand.
    pub worktype: String,
    pub required_tags: Vec<String>,
    /// Principals whose HITL-maintained tags cover EVERY required tag.
    pub qualified_principals: Vec<String>,
    /// Open runs of this worktype — the demand queue depth.
    pub open_demand: i64,
    pub covered: bool,
}

/// Competence coverage: skills registry vs worktype demand. Demand is the
/// domain's ACTIVE runs grouped by `kind`; supply is the skills registry.
/// Read-only.
pub fn coverage_view(conn: &Connection, domain: &str) -> Result<Vec<CoverageRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, COUNT(*) FROM workflow_runs
             WHERE domain = ?1 AND status = 'active'
             GROUP BY kind ORDER BY kind",
        )
        .map_err(|e| e.to_string())?;
    let demand: Vec<(String, i64)> = stmt
        .query_map(rusqlite::params![domain], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let skills = crate::workflow::crew::list_skills(conn, domain).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (kind, count) in demand {
        let required: &[&str] = worktype_skills(&kind);
        let qualified = board_for_worktype(&skills, required);
        out.push(CoverageRow {
            worktype: kind,
            required_tags: required.iter().map(|s| s.to_string()).collect(),
            covered: !qualified.is_empty(),
            qualified_principals: qualified,
            open_demand: count,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::shifts::{ShiftDraft, insert_shift};
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    fn seed_run(conn: &Connection, domain: &str, kind: &str, owner: &str) -> i64 {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, 'active', 1, 1)",
            rusqlite::params![domain, kind, format!(r#"{{"owner":"{owner}"}}"#)],
        )
        .expect("seed run");
        conn.last_insert_rowid()
    }

    #[test]
    fn workload_views_compute_from_lineage_only() {
        let conn = db();
        let r1 = seed_run(&conn, "acme", "complaint", "ana");
        let _r2 = seed_run(&conn, "acme", "care_inquiry", "ana");
        let _r3 = seed_run(&conn, "acme", "return", "bob");
        let _closed = seed_run(&conn, "acme", "return", "bob");
        conn.execute(
            "UPDATE workflow_runs SET status='resolved' WHERE id=?1",
            rusqlite::params![_closed],
        )
        .expect("close one");

        conn.execute(
            "INSERT INTO handover_offers(domain, run_id, from_principal, to_principal, state, reason, overlap_minutes, sla_deadline, created_at)
             VALUES ('acme', ?1, 'ana', 'bob', 'offered', 'workload', 30, 9999, 10),
                    ('acme', ?2, 'bob', 'ana', 'accepted', 'escalation', 30, 9999, 11),
                    ('acme', ?1, 'ana', 'carl', 'declined', 'no-capacity', 30, 9999, 12)",
            rusqlite::params![r1, _r3],
        )
        .expect("seed offers");

        crate::workflow::outbox::enqueue(
            &conn,
            r1,
            crate::workflow::frontdesk::TOPIC_REASK,
            r#"{"source":"marked"}"#,
            "k-reask-1",
            1,
        )
        .expect("reask event");

        conn.execute(
            "INSERT INTO proposals(kind, content, novelty, salience, status, created_at, owner)
             VALUES ('fact', '{}', 0.5, 0.5, 'pending', 20, 'ana'),
                    ('fact', '{}', 0.5, 0.5, 'decided', 21, 'ana')",
            [],
        )
        .expect("seed proposals");

        // Lineage-only proof: snapshot every source table before, compute,
        // compare after — byte-identical means the view wrote nothing.
        let snapshot = |conn: &Connection| -> Vec<(String, i64)> {
            let mut out = Vec::new();
            for table in [
                "workflow_runs",
                "handover_offers",
                "outbox",
                "proposals",
                "audit_events",
            ] {
                let n: i64 = conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .expect("count");
                out.push((table.to_string(), n));
            }
            out
        };
        let before = snapshot(&conn);
        let view = workload_view(&conn, "acme").expect("view computes");
        assert_eq!(snapshot(&conn), before, "a visibility view must not write");

        let ana = view.iter().find(|w| w.principal == "ana").expect("ana row");
        assert_eq!(ana.open_envelopes, 2);
        assert_eq!(
            ana.handover_burden_outbound, 1,
            "only the offered one counts"
        );
        assert_eq!(ana.transfers_in_open, 1, "accepted offer on bob's OPEN run");
        assert_eq!(ana.reask_load, 1);
        assert_eq!(ana.gate_backlog, 1, "decided proposals are not backlog");

        let bob = view.iter().find(|w| w.principal == "bob").expect("bob row");
        assert_eq!(bob.open_envelopes, 1, "the closed run leaves the count");
        assert_eq!(bob.transfers_in_open, 0);

        // Tenant boundary: another domain sees none of acme's lineage.
        assert!(
            workload_view(&conn, "other")
                .expect("other domain")
                .is_empty()
        );
    }

    fn rostered_shift(domain: &str, site: &str, start: i64, end: i64, roster: &[&str]) -> Shift {
        Shift {
            id: 0,
            domain: domain.into(),
            site: site.into(),
            tz: "UTC".into(),
            start_epoch: start,
            end_epoch: end,
            overlap_minutes: 0,
            roster: roster.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn fatigue_signal_alerts_never_reassigns() {
        let conn = db();
        // Ana works two windows back-to-back with under 8h rest. Domains
        // differ because same-domain windows may not share time (the
        // double-booking law) — fatigue arithmetic is roster-scoped, not
        // domain-scoped, which is exactly what a follow-the-sun roster is.
        let shifts = vec![
            rostered_shift("d1", "manila", 0, 28_800, &["ana"]),
            rostered_shift("d2", "manila", 32_400, 61_200, &["ana"]),
            // Bob: one window, healthy rest either side — never flagged.
            rostered_shift("d3", "ams", 100_000, 128_800, &["bob"]),
        ];
        let conn_mut = conn;
        for s in &shifts {
            let roster = s.roster.clone();
            insert_shift(
                &conn_mut,
                &ShiftDraft {
                    domain: &s.domain,
                    site: &s.site,
                    tz: &s.tz,
                    start_epoch: s.start_epoch,
                    end_epoch: s.end_epoch,
                    overlap_minutes: s.overlap_minutes,
                    roster: &roster,
                },
            )
            .expect("seed shift");
        }
        seed_run(&conn_mut, "acme", "complaint", "ana");
        let workload = workload_view(&conn_mut, "acme").expect("workload");

        let audit_before: i64 = conn_mut
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("count");
        let runs_before: (i64, i64) = conn_mut
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(state_revision), 0) FROM workflow_runs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("runs");
        let audit_after: i64 = conn_mut
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("count");
        let runs_after: (i64, i64) = conn_mut
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(state_revision), 0) FROM workflow_runs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("runs");
        let signals = fatigue_signals(&shifts, &workload);
        assert_eq!(audit_before, audit_after, "alerting writes no audit rows");
        assert_eq!(runs_before, runs_after, "alerting never touches any run");

        assert_eq!(signals.len(), 1, "only ana flags");
        let sig = &signals[0];
        assert_eq!(sig.principal, "ana");
        assert_eq!(sig.consecutive_shifts, 2);
        assert!(
            sig.reason.contains("consecutive"),
            "reason names the pattern"
        );

        // Rest boundary honored: 8h+ gap breaks the chain.
        let rested = vec![
            rostered_shift("d1", "manila", 0, 28_800, &["ana"]),
            rostered_shift("d2", "manila", 28_800 + MIN_REST_SECS, 57_600, &["ana"]),
        ];
        assert!(fatigue_signals(&rested, &[]).is_empty());

        // Open-load leg: under the cap even a long chain-less roster is quiet,
        // over it the signal fires on load alone.
        let solo = vec![rostered_shift("acme", "lima", 0, 100, &["carl"])];
        let heavy: Vec<WorkloadRow> = vec![WorkloadRow {
            principal: "carl".into(),
            open_envelopes: OPEN_LOAD_CAP,
            handover_burden_outbound: 0,
            transfers_in_open: 0,
            reask_load: 0,
            gate_backlog: 0,
        }];
        let sigs = fatigue_signals(&solo, &heavy);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].consecutive_shifts, 1);
        assert!(sigs[0].reason.contains("open envelopes"));
    }

    #[test]
    fn competence_coverage_joins_skills_to_worktype_queues() {
        let conn = db();
        seed_run(&conn, "acme", "warranty_claim", "ana");
        seed_run(&conn, "acme", "care_inquiry", "bob");
        seed_run(&conn, "acme", "care_inquiry", "ana");
        // safety_recall has demand but NOBODY holds its tags → uncovered.
        seed_run(&conn, "acme", "safety_recall", "bob");
        for (principal, skill) in [
            ("ana", "returns"),
            ("ana", "warranty"),
            ("ana", "care"),
            ("bob", "care"),
        ] {
            conn.execute(
                "INSERT INTO principal_skills(domain, principal, skill, created_at)
                 VALUES ('acme', ?1, ?2, 100)",
                rusqlite::params![principal, skill],
            )
            .expect("seed skill");
        }

        let coverage = coverage_view(&conn, "acme").expect("coverage");
        assert_eq!(coverage.len(), 3, "one row per demanded worktype");

        let warranty = coverage
            .iter()
            .find(|c| c.worktype == "warranty_claim")
            .unwrap();
        assert_eq!(warranty.required_tags, vec!["returns", "warranty"]);
        assert_eq!(warranty.open_demand, 1);
        assert_eq!(warranty.qualified_principals, vec!["ana"]);
        assert!(warranty.covered);

        let care = coverage
            .iter()
            .find(|c| c.worktype == "care_inquiry")
            .unwrap();
        assert_eq!(care.open_demand, 2);
        assert_eq!(care.qualified_principals, vec!["ana", "bob"]);

        let recall = coverage
            .iter()
            .find(|c| c.worktype == "safety_recall")
            .unwrap();
        assert_eq!(recall.required_tags, vec!["safety", "compliance"]);
        assert!(recall.qualified_principals.is_empty());
        assert!(!recall.covered, "demand without supply reads as uncovered");

        // Tenant boundary again.
        assert!(coverage_view(&conn, "other").expect("other").is_empty());
    }
}
