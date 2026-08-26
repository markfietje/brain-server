//! The WFM interop seam — first-party and versioned.
//!
//! No interchange standard exists for workforce-management data in this
//! space, so the seam IS the standard: a documented, additive-only JSON
//! contract over the shift ring ([`crate::workflow::shifts`]) and the
//! HITL-maintained skills registry ([`crate::workflow::crew`]), stamped with
//! [`WFM_SCHEMA_VERSION`] on every read. Vendor-specific Verint/NICE
//! connectors are explicitly later work; the generic CSV/JSON adapters here
//! ([`parse_shifts_csv`], [`parse_shifts_json`], [`parse_skills_csv`],
//! [`parse_skills_json`]) are what any WFM maps through today.
//!
//! The contract lives twice on purpose: emitted by these payload builders
//! (the handlers call them, so drift is impossible) and declared in
//! `docs/wfm-seam.md`. The `wfm_schema_is_versioned_and_additive_only` test
//! pins the two together — a field removed or renamed without a doc change
//! fails the gate.

use crate::workflow::shifts::{RingView, Shift};

#[path = "../bin_common/wfm_import.rs"]
mod bin_common_wfm_import;

pub(crate) use bin_common_wfm_import::WFM_SCHEMA_VERSION;
#[cfg(test)]
pub(crate) use bin_common_wfm_import::{
    parse_shifts_csv, parse_shifts_json, parse_skills_csv, parse_skills_json,
};

/// The `GET /ops/shifts` response body — THE wire shape of the seam's shift
/// feed. Handlers render through this builder so the pinned schema cannot
/// drift from what ships.
pub fn shifts_response(view: &RingView, all: &[Shift]) -> serde_json::Value {
    let payload: Vec<serde_json::Value> = all
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "domain": s.domain,
                "site": s.site,
                "tz": s.tz,
                "start_epoch": s.start_epoch,
                "end_epoch": s.end_epoch,
                "overlap_minutes": s.overlap_minutes,
                "roster": s.roster,
            })
        })
        .collect();
    let mut out = serde_json::json!({
        "schema_version": WFM_SCHEMA_VERSION,
        "now": view.now,
        "domain": view.domain,
        "queue_scope_site": view.queue_scope_site,
        "incoming_site": view.incoming_site,
        "in_overlap": view.in_overlap,
        "next_boundary_epoch": view.next_boundary_epoch,
    });
    out["shifts"] = serde_json::Value::Array(payload);
    out
}

/// Grouped `(principal, [skill])` rows as the seam's skills feed renders
/// them (`GET /ops/skills`).
pub fn skills_response(domain: &str, grouped: &[(String, Vec<String>)]) -> serde_json::Value {
    serde_json::json!({
        "schema_version": WFM_SCHEMA_VERSION,
        "domain": domain,
        "skills": grouped
            .iter()
            .map(|(p, skills)| serde_json::json!({
                "principal": p,
                "skills": skills,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::crew;
    use crate::workflow::shifts::{ShiftDraft, insert_shift, list_shifts, ring_view};
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn seed_file_db() -> Connection {
        register_sqlite_vec();
        let dir = std::env::temp_dir().join(format!(
            "brain-wfm-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let mut conn = Connection::open(dir.join("wfm.db")).expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    #[test]
    fn wfm_schema_is_versioned_and_additive_only() {
        // Every emitted top-level key of both feeds must be DECLARED in
        // docs/wfm-seam.md, and the declared version must equal the shipped
        // constant — a field renamed or dropped server-side without its doc
        // half fails here (additive-only is enforced, not aspirational).
        let doc_path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/wfm-seam.md");
        let doc = std::fs::read_to_string(doc_path).expect("wfm-seam.md must exist");
        let block = doc
            .split("<!-- wfm-schema")
            .nth(1)
            .and_then(|rest| rest.split("-->").next())
            .expect("wfm-schema declaration block present");
        let declared = |key: &str| -> Option<Vec<String>> {
            block.lines().find_map(|l| {
                let l = l.trim();
                l.strip_prefix(&format!("{key}:")).map(|rest| {
                    rest.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
            })
        };
        let shifts_top = declared("GET /ops/shifts").expect("declared GET /ops/shifts fields");
        let shift_obj = declared("shift object").expect("declared shift object fields");
        let skills_top = declared("GET /ops/skills").expect("declared GET /ops/skills fields");
        let skills_obj = declared("skills object").expect("declared skills object fields");

        let view = ring_view(&[], "global", 100);
        // A sample row guarantees the per-object key check runs even with an
        // empty registry — the shape, not the data, is what's pinned.
        let sample = Shift {
            id: 1,
            domain: "global".into(),
            site: "sample".into(),
            tz: "UTC".into(),
            start_epoch: 0,
            end_epoch: 1,
            overlap_minutes: 0,
            roster: vec![],
        };
        let shifts_body = shifts_response(&view, &[sample]);
        let top_keys = |v: &serde_json::Value| -> Vec<String> {
            v.as_object()
                .expect("object body")
                .keys()
                .cloned()
                .collect()
        };
        let mut got_top = top_keys(&shifts_body);
        got_top.sort();
        let mut want_top = shifts_top.clone();
        want_top.sort();
        assert_eq!(got_top, want_top, "shift feed keys must match the doc");
        let mut got_obj: Vec<String> = shifts_body["shifts"]
            .as_array()
            .expect("array")
            .iter()
            .flat_map(&top_keys)
            .collect();
        got_obj.sort();
        got_obj.dedup();
        let mut want_obj = shift_obj.clone();
        want_obj.sort();
        assert_eq!(got_obj, want_obj, "shift object keys must match the doc");

        let grouped = vec![("op-a".to_string(), vec!["billing".to_string()])];
        let skills_body = skills_response("acme", &grouped);
        let mut got_skills_top = top_keys(&skills_body);
        got_skills_top.sort();
        let mut want_skills_top = skills_top.clone();
        want_skills_top.sort();
        assert_eq!(got_skills_top, want_skills_top);
        let mut got_skills_obj: Vec<String> = skills_body["skills"]
            .as_array()
            .expect("array")
            .iter()
            .flat_map(&top_keys)
            .collect();
        got_skills_obj.sort();
        got_skills_obj.dedup();
        let mut want_skills_obj = skills_obj.clone();
        want_skills_obj.sort();
        assert_eq!(got_skills_obj, want_skills_obj);

        assert!(
            doc.contains(WFM_SCHEMA_VERSION),
            "doc must carry the shipped schema version"
        );
        assert!(
            doc.contains("Additive-only"),
            "doc must state the additive-only change policy"
        );
        assert!(
            doc.contains("## Change log"),
            "doc must carry the change log"
        );
    }

    #[test]
    fn wfm_import_round_trips_shifts_and_skills() {
        let conn = seed_file_db();
        let shifts_csv = "domain,site,tz,start_epoch,end_epoch,overlap_minutes,roster\n\
                          acme,manila,+08:00,800,1600,60,op-a;op-b\n\
                          acme,ams,UTC,1600,2400,,\n";
        let imported = parse_shifts_csv(shifts_csv).expect("csv parses");
        assert_eq!(imported.len(), 2);
        assert_eq!(
            imported[0].roster,
            vec!["op-a".to_string(), "op-b".to_string()]
        );
        assert_eq!(imported[1].overlap_minutes, 0);
        assert_eq!(imported[1].tz, "UTC");

        // Round-trip THROUGH STORAGE: every imported shift lands as a real
        // row the feed reads back field-complete.
        for s in &imported {
            let roster = s.roster.clone();
            insert_shift(
                &conn,
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
            .expect("imported shift stores");
        }
        let listed = list_shifts(&conn, "acme").expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].site, "manila");
        assert_eq!(listed[0].roster.len(), 2);

        // Skills import lands as HITL PROPOSALS, never direct registry
        // writes; approval applies them into the registry the feed reads.
        let skills_csv = "principal,skill\nop-a,billing\nop-a,retention\nop-b,billing\n";
        let imported_skills = parse_skills_csv(skills_csv).expect("csv parses");
        assert_eq!(imported_skills.len(), 3);
        for row in &imported_skills {
            crew::apply_skills_change_probe(&crew::SkillsChange {
                domain: "acme".into(),
                principal: row.principal.clone(),
                add: vec![row.skill.clone()],
                remove: Vec::new(),
            })
            .expect("valid skill tag");
        }
        for row in &imported_skills {
            crew::apply_skills_change(
                &conn,
                "acme",
                &crew::SkillsChange {
                    domain: "acme".into(),
                    principal: row.principal.clone(),
                    add: vec![row.skill.clone()],
                    remove: Vec::new(),
                },
                100,
            )
            .expect("apply approved skill");
        }
        let registry = crew::list_skills(&conn, "acme").expect("registry read");
        assert_eq!(registry.len(), 3);
        assert!(registry.contains(&("op-a".to_string(), "billing".to_string())));

        // The JSON adapters accept the same shapes.
        let shifts_json = r#"[{"domain":"acme","site":"tokyo","tz":"+09:00",
                               "start_epoch":2400,"end_epoch":3200,"roster":["op-c"]},
                              {"domain":"acme","site":"lima","start_epoch":3200,"end_epoch":4000}]"#;
        let imported_json = parse_shifts_json(shifts_json).expect("json parses");
        assert_eq!(imported_json[0].tz, "+09:00");
        assert_eq!(imported_json[0].overlap_minutes, 0);
        assert_eq!(imported_json[1].roster.len(), 0);
        let skills_json = r#"[{"principal":"op-c","skill":"troubleshooting"}]"#;
        assert_eq!(
            parse_skills_json(skills_json).expect("json parses")[0].skill,
            "troubleshooting"
        );

        // Malformed input refuses loudly with context, never silently drops.
        assert!(parse_shifts_csv("site,tz\nx,y").is_err());
        assert!(parse_shifts_csv("domain,site,tz,start_epoch,end_epoch,overlap_minutes,roster\nacme,site,UTC,notanumber,10,0,").is_err());
        assert!(parse_shifts_json("[{\"domain\":\"acme\"}]").is_err());
        assert!(parse_skills_csv("principal,skill\nonly-one-cell").is_err());
        assert!(parse_skills_csv("principal,skill\n,billing").is_err());
    }
}
