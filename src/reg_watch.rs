//! Regulation-date watch — the calendar as code (Enterprise Line law 13).
//!
//! Each pinned deadline carries its SOURCE URL (so a re-verification is one
//! click) and a deliverable assertion shaped by the date: until the deadline
//! the pin is a WATCH (`today < DATE` — green, self-documenting); once the
//! date passes, the pin ASSERTS THE DELIVERABLE EXISTS. A passing date
//! without the artifact fails CI — the calendar itself becomes a test.
//!
//! Red-then-green provenance: `reg_watch_cra_pin_is_green` was landed RED
//! (no runbook) and flipped GREEN in the same release the runbook shipped,
//! proving the mechanism catches lateness. Test-only by construction — the
//! whole module is `#[cfg(test)]`.

fn doc(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{rel} must exist and be readable: {e}"))
}

/// Days since the Unix epoch → civil (year, month, day).
/// Howard Hinnant's `civil_from_days` algorithm — std-only so this module
/// (and the whole watch) stays dependency-free.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Today's civil date (UTC) from the system clock.
fn today() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs() as i64;
    civil_from_days(secs.div_euclid(86_400))
}

/// (y, m, d) → comparable ordinal (lexicographic tuple compare does the rest).
const fn deadline(y: i64, m: u32, d: u32) -> (i64, u32, u32) {
    (y, m, d)
}

// ── the deadlines (source URLs + verified dates live in each doc comment) ────

/// CRA Art 14 vulnerability & incident reporting goes live: 24 h early
/// warning / 72 h notification / final report to ENISA + the national CSIRT.
/// Source: Regulation (EU) 2024/2847, Art 14(1)/(4)/(6) — reporting
/// obligations apply from 11 September 2026 (Art 69(2) application dates).
/// Re-verify at: https://eur-lex.europa.eu/eli/reg/2024/2847/oj
const CRA_ART14_APPLIES: (i64, u32, u32) = deadline(2026, 9, 11);

/// The pinned deadline as the `YYYY-MM-DD` stamp the runbook must carry —
/// derived from [`CRA_ART14_APPLIES`] so the constant is load-bearing: a
/// deadline re-mapped in code without the doc (or vice versa) fails the pin.
fn stamped_application_date() -> String {
    let (y, m, d) = CRA_ART14_APPLIES;
    format!("{y:04}-{m:02}-{d:02}")
}
/// AI Act Art 50 transparency: provider machine-readable marking of synthetic
/// content — legacy-system grace ends 2 Dec 2026.
/// Source: Regulation (EU) 2024/1689 Art 50(2); C(2026) 4935 guidelines
/// (20 Jul 2026). https://eur-lex.europa.eu/eli/reg/2024/1689/oj
const AI_ACT_ART50_MARKING: (i64, u32, u32) = deadline(2026, 12, 2);
/// NIST IR 8547 / OMB M-26-15 / CNSA 2.0: PQC key-establishment across
/// national security systems by 31 Dec 2030 (signatures 2031) — our seam is
/// the crypto inventory + algorithm-agility doc (the Enterprise Line's
/// E-milestone).
/// Source: https://csrc.nist.gov/pubs/ir/8547/final · https://www.whitehouse.gov/wp-content/uploads/2026/01/M-26-15.pdf
const PQC_INVENTORY_SEAM: (i64, u32, u32) = deadline(2030, 12, 31);

/// CRA Art 14 reporting is LIVE from 2026-09-11 — the runbook must exist and
/// carry the three reporting clocks + the channel names BEFORE the date, so
/// this pin is green on arrival of the artifact and cannot silently rot.
/// (Red-then-green: this test landed while the runbook was still missing and
/// went green in the same release — the mechanism demonstrably fires.)
#[test]
fn reg_watch_cra_pin_is_green() {
    let runbook = doc("docs/cra-reporting-runbook.md");
    for anchor in [
        "## 24-hour early warning",
        "## 72-hour notification",
        "## Final report",
    ] {
        assert!(
            runbook.contains(anchor),
            "CRA runbook is missing its `{anchor}` section — the three Art 14 clocks are the deliverable"
        );
    }
    assert!(
        runbook.contains("ENISA"),
        "CRA runbook must name ENISA as a reporting channel"
    );
    assert!(
        runbook.contains("CSIRT"),
        "CRA runbook must name the national CSIRT as a reporting channel"
    );
    assert!(
        runbook.contains(&stamped_application_date()),
        "CRA runbook must stamp the pinned application date ({}) so the operator \
         sees the clock — update the constant and the doc together",
        stamped_application_date()
    );
    assert!(
        runbook.contains("scripts/cra-report-drill.sh"),
        "CRA runbook must reference the timed drill script (the rehearsal is part of readiness)"
    );
}

/// AI Act Art 50(2) machine-readable marking — WATCH form until 2026-12-02.
/// The deliverable (signed provenance fields riding engine-generated exports,
/// the parcels-signing pattern) lands with the Enterprise Line's attestation
/// milestone; until then this pin only asserts the clock hasn't run out. The
/// day the date passes without the deliverable, CI goes red HERE.
#[test]
fn ai_act_art50_marking_watch() {
    assert!(
        today() < AI_ACT_ART50_MARKING,
        "AI Act Art 50 marking deadline (2026-12-02) has passed and the \
         provenance-mark deliverable is not yet pinned — ship it (the \
         Enterprise Line's attestation milestone) \
         or re-map the deadline with a source URL in the same change"
    );
}

/// PQC inventory + algorithm-agility seam — WATCH form until 2030-12-31
/// (NIST IR 8547 key-establishment deadline). The deliverable lands with
/// Enterprise Line's attestation milestone; the watch only asserts the horizon.
#[test]
fn pqc_inventory_seam_watch() {
    assert!(
        today() < PQC_INVENTORY_SEAM,
        "PQC key-establishment deadline (2030-12-31) has passed and the \
         crypto inventory + alg-agility seam is not pinned — deliver it \
         (the Enterprise Line's attestation milestone) \
         or re-map with a source URL in the same change"
    );
}
