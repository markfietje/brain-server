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
/// warning / 72 h notification / final report via the single reporting
/// platform to the CSIRT designated as coordinator + ENISA simultaneously.
/// Source: Regulation (EU) 2024/2847, FINAL-OJ numbering — the two trigger
/// schedules sit at 14(1)-(2) (vulnerabilities: early warning 14(2)(a),
/// notification 14(2)(b), final report ≤ 14 days after a corrective or
/// mitigating measure is available 14(2)(c)) and 14(3)-(4) (severe
/// incidents: final report within one month of the incident notification
/// 14(4)(c)), with the severe-incident definition at 14(5). Reporting
/// obligations apply from 11 September 2026 (Art 71(2) — the pre-OJ
/// numbering this file previously cited named Art 69(2)).
/// Re-verified 2026-09-14 vs the EUR-Lex full text + the Commission
/// reporting page (digital-strategy.ec.europa.eu/en/policies/cra-reporting).
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
/// Source: Regulation (EU) 2024/1689 Art 50(2) as amended by Regulation
/// (EU) 2026/1744 (the "Digital Omnibus on AI" package; adopted 8 Jul 2026,
/// OJ L 24.7.2026, in force 27 Jul 2026): recital 38 grants a four-month
/// transitional period for systems placed on the market before 2 Aug 2026 —
/// i.e. through 2 Dec 2026. The pre-1.28.88 comment cited the C(2026) 4935
/// guidelines as the horizon's legal basis; the amending regulation is the
/// instrument (L7-04; OJ number confirmed 2026-09-14).
/// DEPLOYER horizons from the same regulation (recital 40; no component duty
/// moves — tracked in docs, not in code): Annex III high-risk obligations
/// apply from 2 Dec 2027, Annex I (embedded in regulated products) from
/// 2 Aug 2028. https://eur-lex.europa.eu/eli/reg/2024/1689/oj
const AI_ACT_ART50_MARKING: (i64, u32, u32) = deadline(2026, 12, 2);
/// AI Act GENERAL APPLICATION (Art 113): Art 50 transparency duties apply
/// from 2 Aug 2026 for systems placed on the market from that date — this
/// clock has already started; the 2026-12-02 row above is the separate
/// LEGACY-system grace horizon, NOT the start. Fourth-pass L4-01: an operator
/// reading only the December row could conclude the duties begin then.
/// Source: Regulation (EU) 2024/1689 Art 50 + Art 113 — verified live
/// 2026-09-12 (artificialintelligenceact.eu/article/50: "Comes into force
/// 2 August 2026, according to Article 113").
const AI_ACT_APPLICATION: (i64, u32, u32) = deadline(2026, 8, 2);
/// NIST IR 8547 / OMB M-26-15 / CNSA 2.0: PQC key-establishment across
/// national security systems by 31 Dec 2030 (signatures 2031) — our seam is
/// the crypto inventory + algorithm-agility doc (the Enterprise Line's
/// E-milestone).
/// Source: https://csrc.nist.gov/pubs/ir/8547/final · https://www.whitehouse.gov/wp-content/uploads/2026/01/M-26-15.pdf
const PQC_INVENTORY_SEAM: (i64, u32, u32) = deadline(2030, 12, 31);

/// Singapore Model AI Governance Framework for Agentic AI: published
/// 2026-01-22 by IMDA with the AI Verify Foundation; the pinned date is the
/// 2026-05-20 update of the primary document. VOLUNTARY — this stamp is
/// buyer evidence (the law_version stamp + the DPO quarterly diff are the
/// traceability posture), never a duty claim. The primary document maps
/// governance onto FOUR dimensions (assess and bound the risks upfront;
/// make humans meaningfully accountable; implement technical controls;
/// end-user enablement); the "five dimensions" grouping in secondary
/// sources is a grouping variance, not a different framework.
/// Source: https://www.imda.gov.sg (the primary PDF's own text names
/// "Four dimensions of the MGF for Agentic AI").
const MGF_AGENTIC_AI_PUBLISHED: (i64, u32, u32) = deadline(2026, 1, 22);
/// The MGF for Agentic AI's update stamp — the load-bearing half of the
/// pair above (a framework date re-mapped in code without the compliance
/// doc, or vice versa, fails the derived pin).
const MGF_AGENTIC_AI_UPDATED: (i64, u32, u32) = deadline(2026, 5, 20);
/// CoE Framework Convention on Artificial Intelligence (CETS 225): IN FORCE
/// 2025-11-01. Party-facing duties only — this repo is not a party, so no
/// component duty moves; the stamp records the calendar a party-deployment
/// plans against.
/// Source: https://www.coe.int/en/web/conventions/full-list?module=treaty-detail&treatynum=225
const CETS_225_IN_FORCE: (i64, u32, u32) = deadline(2025, 11, 1);

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

/// L7-01 (seventh pass): the runbook's final-report section carried the
/// INCIDENT trigger's one-month clock for BOTH triggers — legally wrong for
/// actively exploited VULNERABILITIES, whose final report is due no later
/// than 14 days after a corrective or mitigating measure is available
/// (Art 14(2)(c), final-OJ numbering; re-verified 2026-09-14 vs the EUR-Lex
/// full text + the Commission reporting page). The lesson of L7-01 is that
/// a pin checked anchors, not clocks — this pin anchors the clock words
/// themselves, so a revert of the runbook to the one-month-for-both form
/// fails here.
#[test]
fn reg_watch_runbook_clock_anchor() {
    let runbook = doc("docs/cra-reporting-runbook.md");
    assert!(
        runbook.contains("14 days") && runbook.contains("corrective"),
        "CRA runbook lost the VULNERABILITY trigger's final-report clock \
         (Art 14(2)(c): no later than 14 days after a corrective or \
         mitigating measure is available) — the one-month clock binds the \
         severe-INCIDENT trigger only (Art 14(4)(c))"
    );
    // The split must be visible as a split: both trigger labels ride the
    // section, so a reader cannot mistake one clock for the whole rule.
    assert!(
        runbook.contains("corrective or mitigating measure is available")
            && runbook.contains("one month"),
        "CRA runbook must carry BOTH final-report clocks with their trigger \
         labels — vulnerability (14 days after the fix/mitigation is \
         available) and severe incident (one month after the notification)"
    );
}

/// AI Act Art 50(2) machine-readable marking — DELIVERABLE form (flipped
/// from WATCH at the Attestation milestone). The deadline
/// (2026-12-02) stays pinned as the compliance horizon, but the pin now
/// asserts the deliverable EXISTS: the provenance module is wired on all
/// four engine-generated artifact classes (remedy drafts, ADR packets,
/// outreach export packets, KB build manifests), the meta-test
/// `provenance_marks_present_on_all_four_classes` proves valid marks through
/// the real emission shapes, and the honest-scope boundaries are stated.
/// Removing any wiring fails HERE, not at the next audit.
#[test]
fn ai_act_art50_marking_deliverable() {
    // The horizon stays stamped (the calendar keeps its source URL).
    assert_eq!(
        AI_ACT_ART50_MARKING,
        deadline(2026, 12, 2),
        "the Art 50 marking horizon is 2026-12-02 — re-mapping it requires a \
         source URL in the same change"
    );
    // The deliverable: the provenance module exists and names its law.
    let provenance = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provenance.rs"),
    )
    .expect("src/provenance.rs must exist — the Art 50(2) marking module");
    for anchor in [
        "pub const MARK_AIGEN",
        "pub const MARK_HUMAN",
        "pub fn attach_aigen",
        "pub fn verify",
        "NOT C2PA",
    ] {
        assert!(
            provenance.contains(anchor),
            "the provenance module is missing `{anchor}` — the marking deliverable \
             is the signed AIGEN|HUMAN object with its honest scope"
        );
    }
    // The four classes wire the seal (the emission points, by file). A class
    // that stops carrying its mark fails this scan before any user sees it.
    let wiring: &[(&str, &str)] = &[
        ("src/handlers/workflow.rs", "fn remedy_response"),
        ("src/handlers/workflow.rs", "fn seal_adr_packet"),
        ("src/handlers/workflow.rs", "fn seal_campaign_packet"),
        ("src/kb.rs", "pub fn sealed_manifest_json"),
    ];
    for (file, anchor) in wiring {
        let src =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
                .unwrap_or_else(|e| panic!("{file} must exist: {e}"));
        assert!(
            src.contains(anchor),
            "{file} is missing `{anchor}` — all four artifact classes carry the \
             Art 50(2) mark"
        );
        assert!(
            src.contains("attach_aigen"),
            "{file} lost its `attach_aigen` seal — the Art 50(2) mark rides \
             EVERY boundary artifact"
        );
    }
    // The meta-test exists by name (the deliverable's proof, discoverable).
    assert!(
        provenance.contains("fn provenance_marks_present_on_all_four_classes")
            && provenance.contains("fn tampered_provenance_fails_verify"),
        "the four-class meta-test + tamper pin are the Art 50 deliverable's \
         proof — they cannot be dropped silently"
    );
}

/// AI Act GENERAL APPLICATION clock: the December
/// marking row is the LEGACY grace horizon — the duties themselves started
/// 2026-08-02 for systems placed on the market from that date, and this pin
/// keeps BOTH truths visible: the date stamp AND the compliance doc's
/// dual-date statement, so no operator reads December as the start.
#[test]
fn ai_act_application_clock_recorded() {
    assert_eq!(
        AI_ACT_APPLICATION,
        deadline(2026, 8, 2),
        "Art 50 duties apply from general application (Art 113) on 2026-08-02 — \
         re-mapping it requires a source URL in the same change"
    );
    assert!(
        AI_ACT_APPLICATION < AI_ACT_ART50_MARKING,
        "the application date strictly precedes the legacy-grace end — inverting \
         them is a transcription error"
    );
    let compliance = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/compliance.md"),
    )
    .expect("docs/compliance.md must exist — the compliance map carries both dates");
    assert!(
        compliance.contains("2026-08-02") && compliance.contains("2026-12-02"),
        "docs/compliance.md must state BOTH AI Act dates: general application \
         (2026-08-02) and the legacy-grace horizon (2026-12-02) — December alone \
         misreads as the start"
    );
}

/// PQC inventory + algorithm-agility seam — DELIVERABLE form (flipped from
/// WATCH at the Attestation milestone). The 2030-12-31 horizon
/// stays pinned as the planning input, but the pin now asserts the
/// deliverable EXISTS: docs/crypto-inventory.md in SP 1800-38B shape (every
/// algorithm + HNDL verdict + swap path) and BOTH agility seams it names —
/// the JWT ML-DSA landing procedure against the auth/jwt.rs enum whitelist,
/// and the UMP did:key multicodec version-prefix rule. The inventory
/// rotting (a shipped algorithm vanishing from the doc, or a seam losing
/// its named file) fails HERE.
#[test]
fn pqc_inventory_seam_deliverable() {
    // The horizon stays stamped (the calendar keeps its source URL).
    assert_eq!(
        PQC_INVENTORY_SEAM,
        deadline(2030, 12, 31),
        "the PQC key-establishment horizon is 2030-12-31 — re-mapping it \
         requires a source URL in the same change"
    );
    let inventory = doc("docs/crypto-inventory.md");
    for anchor in [
        "## Algorithm inventory",
        "## HNDL exposure verdicts",
        "### JWT: the ML-DSA landing procedure",
        "### UMP signatures: the algorithm version-prefix rule",
        "What this file does NOT claim",
    ] {
        assert!(
            inventory.contains(anchor),
            "crypto-inventory.md is missing `{anchor}` — the SP 1800-38B shape \
             is the deliverable"
        );
    }
    // Every shipped algorithm family is still inventoried (the doc cannot
    // silently drop a primitive the code ships).
    for alg in [
        "Ed25519",
        "HMAC-SHA256",
        "SHA-256",
        "BLAKE3",
        "AES-256-GCM",
        "Argon2id",
        "RS256",
    ] {
        assert!(
            inventory.contains(alg),
            "crypto-inventory.md no longer inventories {alg} — the inventory \
             must cover every shipped algorithm"
        );
    }
    // The two agility seams name their REAL files (a seam that stops
    // pointing at code is prose, not a seam).
    assert!(
        inventory.contains("auth/jwt.rs") && inventory.contains("ALLOWED_ALGS"),
        "the JWT seam must name the actual whitelist (auth/jwt.rs ALLOWED_ALGS)"
    );
    assert!(
        inventory.contains("did_key_from_ed25519") && inventory.contains("verifying_key_from_did"),
        "the UMP seam must name the actual did:key machinery in ump_integrity"
    );
    // The JWT whitelist still exists at the named seam (the enum isolation
    // the ML-DSA procedure leans on).
    let jwt = doc("src/auth/jwt.rs");
    assert!(
        jwt.contains("pub const ALLOWED_ALGS"),
        "auth/jwt.rs lost ALLOWED_ALGS — the ML-DSA landing procedure's seam \
         moved; update the inventory in the same change"
    );
}

/// T7-04 (seventh pass): the census direction the hardcoded inventory
/// name-list cannot cover — a NEWLY shipped primitive crate never failed
/// anything before, because the list only checked the names it already
/// knew. The census is CLOSED both ways: every dependency in
/// `[dependencies]` whose name matches a crypto-family heuristic must
/// appear in CENSUS (ship the mapping row + the crypto-inventory.md entry
/// in the same change), and every CENSUS row must still be a real
/// dependency and still inventoried (prune both when the crate goes).
#[test]
fn crypto_inventory_census_maps_every_crypto_crate() {
    let toml = doc("Cargo.toml");
    let inventory = doc("docs/crypto-inventory.md");
    const CENSUS: &[(&str, &str)] = &[
        ("ed25519-dalek", "Ed25519"),
        ("hmac", "HMAC-SHA256"),
        ("sha2", "SHA-256"),
        ("blake3", "BLAKE3"),
        ("aes-gcm", "AES-256-GCM"),
        ("argon2", "Argon2id"),
        ("rsa", "RS256/RS384/RS512"),
        ("jsonwebtoken", "RS256/RS384/RS512"),
    ];
    for (crate_name, inv_name) in CENSUS {
        assert!(
            inventory.contains(inv_name),
            "crypto-inventory.md lost `{inv_name}` — the census row for \
             {crate_name} dangles; re-map or remove it in the same change"
        );
        assert!(
            toml.contains(crate_name),
            "{crate_name} left the dependency tree — prune its census row \
             and its inventory entry in the same change"
        );
    }
    const FAMILIES: &[&str] = &[
        "sha",
        "hmac",
        "aes",
        "rsa",
        "dsa",
        "ed25519",
        "ecdsa",
        "p256",
        "p384",
        "argon",
        "blake",
        "chacha",
        "kyber",
        "ml-dsa",
        "sphincs",
        "jsonwebtoken",
    ];
    let mut in_deps = false;
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_deps = l == "[dependencies]";
            continue;
        }
        if !in_deps || l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((name, _)) = l.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if FAMILIES.iter().any(|f| name.contains(f)) && !CENSUS.iter().any(|(c, _)| *c == name) {
            panic!(
                "new crypto-family dependency `{name}` is unmapped — add a \
                 CENSUS row and the crypto-inventory.md entry in the same \
                 change (the inventory must never trail the shipped \
                 primitives)"
            );
        }
    }
}

/// The watch's clock machinery, kept alive by its own pin: both 2026 watch
/// pins flipped to DELIVERABLE form, but every future deadline re-uses this
/// clock — so it stays TESTED (Hinnant's known vectors + a sane `today`),
/// not merely compiled.
#[test]
fn watch_clock_still_tells_time() {
    // Howard Hinnant's published civil_from_days vectors.
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    // today() is the real clock: the epoch math must land in the decade the
    // pinned deadlines live in (a broken civil conversion would desync every
    // future WATCH pin from the calendar it guards).
    let (y, m, d) = today();
    assert!(
        (2024..=2040).contains(&y),
        "implausible year {y} from today()"
    );
    assert!((1..=12).contains(&m) && (1..=31).contains(&d));
}

/// standby_drill_recorded — a warm standby that has never rehearsed its
/// promote is a rumor, not a capability (the CRA-drill precedent). The pin
/// goes GREEN only when the runbook carries the DATED drill record with its
/// measured timings: the baseline record from 2026-09-06 (a copy of the
/// live DB — RTO 0.55s, RPO 10.4s, tamper probe fail-closed). A future
/// re-drill appends a new dated subsection; the baseline anchors stay.
#[test]
fn standby_drill_recorded() {
    let runbook = doc("docs/runbooks.md");
    for anchor in [
        "## Warm standby (v1.28.61)",
        "### Promote procedure (warm — manual, rehearsed)",
        "### Ceilings (honest)",
        "### Drill record — 2026-09-06",
        "### Incident note — 2026-09-06",
    ] {
        assert!(
            runbook.contains(anchor),
            "warm-standby runbook is missing `{anchor}` — the rehearsed promote \
             and its dated record are the deliverable"
        );
    }
    for measured in ["RTO 0.55s", "RPO 10.4s", "9,091 rows", "fails closed"] {
        assert!(
            runbook.contains(measured),
            "drill record must carry the measured number/verdict `{measured}` — \
             a record without timings is not a rehearsal"
        );
    }
}

/// revocation_drill_recorded — a kill-switch that has never been pulled is a
/// rumor, not a capability (the standby-drill precedent). The pin goes GREEN
/// only when the runbook carries the kill-switch section AND its dated drill
/// record with the measured verdicts: the 2026-09-06 baseline (a copy of the
/// live DB — cards fail closed, the owner's run drained through the existing
/// cancel path, the drain observed in events, the audit chain verified). A
/// future re-drill appends a new dated subsection; the baseline anchors stay.
#[test]
fn revocation_drill_recorded() {
    let runbook = doc("docs/runbooks.md");
    for anchor in [
        "## Principal kill-switch (v1.28.62)",
        "### What revocation does, in one transaction",
        "### Procedure",
        "### Ceilings (honest)",
        "### Drill record — 2026-09-06",
    ] {
        assert!(
            runbook.contains(anchor),
            "kill-switch runbook is missing `{anchor}` — the rehearsed revocation \
             and its dated record are the deliverable"
        );
    }
    for measured in [
        "403 principal_revoked",
        "\"runs_drained\":1",
        "status = cancelled",
        "delegation/revoked",
        "{\"ok\":true}",
    ] {
        assert!(
            runbook.contains(measured),
            "drill record must carry the measured verdict `{measured}` — a record \
             without observed behavior is not a rehearsal"
        );
    }
}

fn stamped_mgf_dates() -> (String, String) {
    let (y, m, d) = MGF_AGENTIC_AI_PUBLISHED;
    let published = format!("{y:04}-{m:02}-{d:02}");
    let (y, m, d) = MGF_AGENTIC_AI_UPDATED;
    (published, format!("{y:04}-{m:02}-{d:02}"))
}

fn stamped_cets_in_force_date() -> String {
    let (y, m, d) = CETS_225_IN_FORCE;
    format!("{y:04}-{m:02}-{d:02}")
}

/// The MGF-for-Agentic-AI stamp is load-bearing: the dates live in the
/// constants AND in docs/compliance.md — either side moving alone fails
/// here. Voluntary framework: the pin records the calendar, it never claims
/// a duty.
#[test]
fn mgf_agentic_ai_stamp_is_load_bearing() {
    assert_eq!(
        MGF_AGENTIC_AI_PUBLISHED,
        deadline(2026, 1, 22),
        "the MGF for Agentic AI was published 2026-01-22 — re-mapping it \
         requires a source URL in the same change"
    );
    assert_eq!(
        MGF_AGENTIC_AI_UPDATED,
        deadline(2026, 5, 20),
        "the MGF for Agentic AI's primary document was updated 2026-05-20 — \
         re-mapping it requires a source URL in the same change"
    );
    let (published, updated) = stamped_mgf_dates();
    assert_eq!(published, "2026-01-22");
    assert_eq!(updated, "2026-05-20");
    assert!(
        MGF_AGENTIC_AI_PUBLISHED < MGF_AGENTIC_AI_UPDATED,
        "publication precedes the update — inverting them is a transcription \
         error"
    );
    let compliance = doc("docs/compliance.md");
    for anchor in ["2026-01-22", "2026-05-20", "VOLUNTARY", "Four dimensions"] {
        assert!(
            compliance.contains(anchor),
            "docs/compliance.md lost the MGF anchor `{anchor}` — the voluntary \
             framework's stamp is the dates + the honest dimension mapping"
        );
    }
}

/// The CETS 225 stamp is load-bearing: in force 2025-11-01, party-facing
/// duties only — the pin records the calendar, it never claims the repo is a
/// party or that any duty moved.
#[test]
fn cets_225_stamp_is_load_bearing() {
    assert_eq!(
        CETS_225_IN_FORCE,
        deadline(2025, 11, 1),
        "CETS 225 entered into force 2025-11-01 — re-mapping it requires a \
         source URL in the same change"
    );
    assert_eq!(stamped_cets_in_force_date(), "2025-11-01");
    let compliance = doc("docs/compliance.md");
    for anchor in ["CETS 225", "2025-11-01", "Party-facing duties only"] {
        assert!(
            compliance.contains(anchor),
            "docs/compliance.md lost the CETS 225 anchor `{anchor}` — the \
             in-force stamp rides with its party-facing-only posture"
        );
    }
}
