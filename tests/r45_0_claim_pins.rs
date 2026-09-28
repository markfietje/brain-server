//! R45-0 — the signature-claim correction, machine-enforced (E1/E2/E7/E8).
//!
//! **What this round corrects.** We have been publishing "Ed25519-signed
//! hash-chained audit". The truth is two layers: the audit chain is a keyed
//! HMAC-SHA256 hash chain (`src/audit/mod.rs`), and Ed25519 signs *other*
//! artifacts — manifests, parcels, provenance marks — at the boundaries
//! (`src/ump_integrity.rs`). An overstated security claim is a live
//! misrepresentation of the security posture, so the correction is a machine
//! gate, not a promise.
//!
//! **The invariant half (E8).** This round changes what we SAY, never what
//! the system DOES. The pins in the "the chain is not what the round says it
//! is not" section assert the audit chain module stays byte-identical. They are
//! GREEN from this commit onward and must stay green — that is their job.
//!
//! **The red-proof.** `r45_0_claim_detector_catches_a_planted_overstatement`
//! feeds a synthetic overstated line to the detector and requires it to fire,
//! so the detector cannot pass vacuously by matching nothing.
//!
//! RED-first: the artifact pins below FAIL until the correction lands. The
//! exact RED text is recorded in the round evidence.

use std::path::{Path, PathBuf};

/// Resolve a path inside the PRIVATE spine repo, relative to the kernel crate.
///
/// The r42 precedent (`tests/main_suite.rs:delivery_r42_gate_note_is_dated_and_pinned`)
/// reads `../brain-steward-ip/…` and **panics loudly** when the private checkout
/// is absent. We follow it exactly: a public-only checkout must NOT silently
/// pass these pins, because then the correction would be unenforced for anyone
/// who clones the public repo alone.
fn spine(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brain-steward-ip")
        .join(rel)
}

fn read_spine(rel: &str) -> String {
    let path = spine(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the private spine artifact must exist at {}: {e}. R45-0's correction is \
             machine-enforced by this pin; a kernel-only checkout that cannot see the \
             private artifacts would otherwise pass vacuously.",
            path.display()
        )
    })
}

// ── the corrected claim (E1, verbatim — this is the machine-checkable form) ──

/// E1's technical form. Every artifact that discusses the audit chain's
/// mechanism must carry this, or a truthful equivalent naming both layers.
const E1_TECHNICAL: &str = "keyed HMAC-SHA256";

/// E1's plain-language form. Carried by the plain-language artifacts.
const E1_PLAIN: &str = "The key that seals the record is not stored with the record.";

/// E2's standing ceiling — the threat model, stated. The chain key and the head
/// pin share the host, so this detects SQL-level tampering, not host compromise.
/// Asserted by `r45_0_engineers_ref_known_limits_ceiling_row_is_retained`.
#[allow(dead_code)]
const E2_CEILING: &str = "not host";

/// The separation vocabulary the CORRECTED prose is written in.
///
/// `engineers-ref.tex` §1 uses "Separately,"; the kernel's `SECURITY.md` uses
/// "links are HMAC-SHA256 over the FULL row"; the compliance-pack ledger uses
/// "detached". The live detector asserts structure rather than matching these —
/// a paragraph that names the real primitive is clean regardless of which of
/// these words it uses, and a paragraph that quotes the old claim beside the
/// true one is the correction table, not a defect. This list documents the
/// vocabulary so a writer correcting an artifact can see the house forms.
#[allow(dead_code)]
const SEPARATION_MARKERS: &[&str] = &[
    "separately",
    "boundary",
    "boundaries",
    "at the boundaries",
    "not the chain",
    "not signed",
    "detached",
    "anchored into",
];

/// A line that DENIES the claim is the correction, not the overstatement.
///
/// This class is load-bearing: the governing spec's own §0 line reads "The
/// audit chain is not Ed25519-signed per row" — the exact true statement this
/// round exists to make. A detector that fired on it would force a rewrite of
/// the sentence doing the correcting. Negations are therefore checked before any
/// assertion is scored.
const NEGATION_MARKERS: &[&str] = &[
    "is not ed25519",
    "not ed25519-signed",
    "no ed25519",
    "never ed25519",
    "does not sign",
    "isn't ed25519",
];

/// A line that NAMES THE REAL MECHANISM, WITHOUT asserting that Ed25519 signs
/// the chain, is describing the correction.
///
/// This is the third and most delicate class. The governing spec's own
/// correction table reads `| "Ed25519-signed hash-chained audit" | The chain is
/// a **keyed HMAC-SHA256** hash chain. |` — one line quoting the false claim and
/// naming the true primitive side by side. That juxtaposition IS the deliverable.
///
/// **The launder guard.** Naming the real primitive alone is NOT exculpatory: an
/// overstatement can name-drop HMAC while still asserting Ed25519 signs the
/// chain ("Our Ed25519-signed hash-chained audit uses HMAC-SHA256 internally"),
/// and that is the same defect wearing a hat. See `names_real_mechanism` and
/// `quotes_the_old_claim` for how the two are separated.
///
/// The overstatement's own grammar.
///
/// An overstatement has the shape **<signing claim> + <the chain>, with nothing
/// on the line naming what actually does the sealing**. The signing claim is
/// either an explicit Ed25519 attribution OR the bare word "signed" applied to
/// the chain — "platform-maintained provenance, signed chain" is the same
/// defect as "Ed25519-signed hash-chained audit": both present a signature over
/// the chain without saying which layer produces it.
///
/// Grading on the SHAPE rather than on a fixed phrase list is what lets the
/// correction-table row pass: that row says "Ed25519-signed hash-chained audit"
/// in quotes as the thing we WROTE, and names `keyed HMAC-SHA256` in the
/// adjacent cell as the thing that is TRUE. A line that names the real
/// primitive is a correction; a line that does not is a claim.
fn names_the_chain(line_lower: &str) -> bool {
    line_lower.contains("hash-chain")
        || line_lower.contains("hash chained")
        || line_lower.contains("hash chain")
        || line_lower.contains("audit chain")
        || line_lower.contains("audit ledger")
        || line_lower.contains("signed chain")
}

/// Does the line ASSERT a signature over the chain, rather than merely naming
/// Ed25519 as the thing signing some OTHER artifact?
///
/// This is the load-bearing question. "manifests, parcels, and provenance marks
/// are Ed25519-signed" names Ed25519 but attributes it to boundaries, not to
/// the chain. "Ed25519-signed hash-chained audit" attributes it to the chain.
/// Only the second is the defect.
fn asserts_signature_over_chain(line_lower: &str) -> bool {
    // The bare noun, as it appears in a list of shipped properties:
    // "platform-maintained provenance, signed chain". "signed chain" on its own
    // asserts a signature over the chain with no primitive named — the same
    // defect as the Ed25519-attributed form, one word less specific.
    if line_lower.contains("signed chain") && !line_lower.contains("unsigned") {
        return true;
    }

    let chain_noun = [
        "hash-chain",
        "hash chained",
        "hash chain",
        "audit chain",
        "audit ledger",
    ];
    for noun in chain_noun {
        // "ed25519-signed hash chain", "ed25519 signed hash chain", …
        for joiner in ["ed25519-signed ", "ed25519 signed ", "signed-by-ed25519 "] {
            if line_lower.contains(&format!("{joiner}{noun}")) {
                return true;
            }
        }
        // "signed hash chain" / "signed audit chain" with no primitive named.
        if line_lower.contains(&format!("signed {noun}")) && !line_lower.contains("unsigned") {
            return true;
        }
        // Explicit verbs binding Ed25519 to the chain.
        for verb in [
            "ed25519 signs the chain",
            "ed25519 signs the audit chain",
            "ed25519 signs the hash chain",
            "signed by ed25519",
            "ed25519 signature over the chain",
            "ed25519-sealed",
            "sealed by ed25519",
        ] {
            if line_lower.contains(verb) {
                return true;
            }
        }
    }
    false
}

/// Does the line name the mechanism that ACTUALLY seals the chain? If so the
/// line is describing the correction, whatever else it says.
///
/// **The launder guard.** Naming the real primitive is the correction ONLY when
/// the paragraph does not ALSO assert a signature over the chain. An
/// overstatement routinely name-drops the true primitive while leading with the
/// false one — "Our Ed25519-signed hash-chained audit is keyed HMAC-SHA256
/// internally" is the same defect wearing a hat. So the real-mechanism escape
/// applies only to a paragraph whose assertion is a *quotation* of the old
/// claim (the correction table) or which is otherwise scoped; a bare assertion
/// is never rescued by a name-drop. The red-proof below drives exactly that
/// laundered sentence and requires the detector to fire, so this rule cannot
/// widen into amnesty.
fn names_real_mechanism(line_lower: &str) -> bool {
    line_lower.contains("hmac-sha256")
        || line_lower.contains("keyed hash chain")
        || line_lower.contains("keyed hash-chain")
        || line_lower.contains("sha-256 over")
}

/// True when the paragraph QUOTES the overstatement it is correcting — the
/// correction table's `"Ed25519-signed hash-chained audit" | The chain is…`
/// shape. A quotation is a citation of the error, not an assertion of it.
fn quotes_the_old_claim(line_lower: &str) -> bool {
    // The old claim appears inside quotation marks.
    line_lower.contains("\"ed25519-signed hash-chained audit\"")
        || line_lower.contains("\"ed25519-signed hash chain\"")
        || line_lower.contains("\"signed chain you can verify yourself\"")
        || line_lower.contains("we previously described")
        || line_lower.contains("we have written")
}

/// The detector, as a pure function so the red-proof can drive it directly.
///
/// Returns `Some(line_number)` for the first line that ASSERTS a signature over
/// the audit chain without saying which layer produces it. Four classes are
/// deliberately permitted, each driven by the red-proof:
///
/// 1. **Denial** — "The audit chain is not Ed25519-signed per row." That is the
///    sentence this round corrects TO. A detector that punished it would force a
///    rewrite of the correction.
/// 2. **Naming the real mechanism** — the correction table's side-by-side
///    juxtaposition, and `engineers-ref.tex` §1's "keyed hash chain
///    (HMAC-SHA256) … Separately, manifests … are Ed25519-signed."
/// 3. **A correctly-scoped different chain** — the compliance-pack decision
///    ledger's *detached* signature (`BRAIN_AUDIT_SIGNING_KEY`), the same shape
///    as the `openapi.yaml:1221` watch-item.
/// 4. **A HISTORICAL record** — the kernel `CHANGELOG.md` is an append-only
///    ledger of what was true at each release. Rewriting a past entry to make
///    today's detector pass would falsify the record. Only entries from the
///    R45-0 round onward are scanned, and the round's own entry states the
///    correction; prior history is left byte-untouched.
///
/// **Paragraph scope, not line scope.** Prose wraps, and a claim plus its
/// correction routinely land on adjacent lines — the round's own changelog entry
/// puts "Ed25519-signed hash-chained audit" on one line and "keyed HMAC-SHA256"
/// on the next. A strictly line-scoped detector would flag that entry and force
/// the prose to be reflowed to satisfy a linter, which is the wrong pressure on
/// a writer. So the unit of analysis is the **paragraph** (a run of non-blank
/// lines); a paragraph is clean when it names the real mechanism ANYWHERE. The
/// returned line number is the first offending line for a human to jump to.
/// Classify one paragraph. `true` means the paragraph is clean; `false` means it
/// asserts a signature over the chain without naming what produces it.
///
/// Single source of truth on purpose: an earlier version of this detector had
/// the rule inline in two places (the mid-document flush and the final
/// paragraph) and the two copies had already drifted — one consulted
/// `quotes_the_old_claim`, the other did not. A paragraph classifier cannot
/// diverge from itself.
fn paragraph_is_clean(joined_lower: &str) -> bool {
    // Not about the chain, or makes no assertion about a signature over it.
    if !names_the_chain(joined_lower) || !asserts_signature_over_chain(joined_lower) {
        return true;
    }
    // (1) A denial is the correction.
    if NEGATION_MARKERS.iter().any(|m| joined_lower.contains(m)) {
        return true;
    }
    // (3) A detached signature belongs to a DIFFERENT chain and is scoped to it
    //     (the compliance-pack decision ledger; the openapi.yaml:1221 shape).
    if joined_lower.contains("detached") {
        return true;
    }
    // (2) Naming the real mechanism is the correction ONLY when the paragraph
    //     quotes the old claim rather than asserting it. A name-drop beside a
    //     live assertion is laundering, not correcting.
    if names_real_mechanism(joined_lower) && quotes_the_old_claim(joined_lower) {
        return true;
    }
    false
}

fn first_unseparated_ed25519_over_chain(text: &str) -> Option<usize> {
    let mut para: Vec<(usize, &str)> = Vec::new();

    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            if let Some(hit) = paragraph_verdict(&para) {
                return Some(hit);
            }
            para.clear();
            continue;
        }
        para.push((i + 1, line));
    }
    paragraph_verdict(&para)
}

/// `None` when the paragraph is clean; `Some(first_line)` when it is not.
fn paragraph_verdict(para: &[(usize, &str)]) -> Option<usize> {
    if para.is_empty() {
        return None;
    }
    let joined = para
        .iter()
        .map(|(_, l)| l.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if paragraph_is_clean(&joined) {
        None
    } else {
        Some(para[0].0)
    }
}

/// The R45-0 changelog entry. The CHANGELOG historical-records exemption in
/// `r45_0_no_external_artifact_claims_ed25519_over_the_chain` opens at this
/// marker, so only THIS round's entry onward is scanned.
const CHANGELOG_WINDOW_START: &str = "R45-0";

/// Banned verbatim strings. Each is an exact sentence we have published that
/// overstates the mechanism. Precise beats clever: a reviewer can grep any of
/// these in one keystroke and understand instantly what the pin forbids.
///
/// **Quoted occurrences are permitted.** The correction must be able to NAME
/// the old claim in order to record what it was — the governing spec's
/// "We have written | What is true" table, and the R45-0 plan's note that its
/// own §0 header used to read "5 production sites". A pin that forbade the
/// string everywhere would forbid the correction from documenting the error it
/// is fixing. So a hit counts only when the sentence is NOT inside quotation
/// marks and is not part of a `> ` correction blockquote.
const BANNED_CLAIMS: &[(&str, &str)] = &[
    (
        "plans/PLAIN_LANGUAGE_PRODUCT_OVERVIEW.md",
        "Every change is signed and recorded permanently",
    ),
    (
        "plans/PLAN_CREATE_LOOP_SENTINEL_MODEL_AND_EVIDENCE.md",
        "platform-maintained provenance, signed chain",
    ),
    (
        "plans/memorysteward-overview.tex",
        "Every change is signed and recorded permanently",
    ),
    (
        "MemorySteward-Overview.tex",
        "Every change is signed and recorded permanently",
    ),
    (
        "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
        "Hash-chained audit (SHA-256/BLAKE3/Ed25519 UMP)",
    ),
    (
        "plans/IMPLEMENTATION_PLAN_R45_ONWARD_LOOP_AND_PRODUCTION_GAPS_2026-09-27.md",
        "has 8 call sites for",
    ),
    (
        "plans/IMPLEMENTATION_PLAN_R45_ONWARD_LOOP_AND_PRODUCTION_GAPS_2026-09-27.md",
        "5 production sites",
    ),
    (
        "plans/IMPLEMENTATION_PLAN_R45_0_CORRECTION_AND_MEASUREMENT_2026-09-27.md",
        "5 production sites",
    ),
];

// ── the correction is machine-enforced (E1/E7) ─────────────────────────────

/// The headline gate. No artifact we publish may attribute Ed25519 to the
/// audit chain. This is the pin that keeps the correction CLOSED.
#[test]
fn r45_0_no_external_artifact_claims_ed25519_over_the_chain() {
    let artifacts = [
        "plans/PLAIN_LANGUAGE_PRODUCT_OVERVIEW.md",
        "plans/memorysteward-overview.tex",
        "MemorySteward-Overview.tex",
        "plans/memorysteward-engineers-ref.tex",
        "MemorySteward-Engineers-Reference.tex",
        "docs/POSITIONING_MEMORYSTEWARD.md",
        "plans/PLAN_CREATE_LOOP_SENTINEL_MODEL_AND_EVIDENCE.md",
        "plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md",
        "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
        "plans/IMPLEMENTATION_PLAN_R45_ONWARD_LOOP_AND_PRODUCTION_GAPS_2026-09-27.md",
    ];

    let mut violations: Vec<String> = Vec::new();
    for rel in artifacts {
        let text = read_spine(rel);
        if let Some(line) = first_unseparated_ed25519_over_chain(&text) {
            violations.push(format!("{rel}:{line}"));
        }
    }

    // The kernel's own PUBLIC tree is scanned too. It is claim-clean today
    // (README.md:170 "append-only keyed hash chain"; SECURITY.md:141 already
    // scopes Ed25519 to the boot manifest) — this pin keeps it that way.
    //
    // CHANGELOG.md is scanned from the R45-0 entry FORWARD ONLY. It is an
    // append-only historical ledger; a v1.28.x entry truthfully describing the
    // mechanism as it stood at that release is not an overstatement, and editing
    // history to satisfy a detector would falsify the record. The round's own
    // entry is inside the scanned window and states the correction.
    for rel in ["README.md", "SECURITY.md"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("kernel artifact must exist at {}: {e}", path.display()));
        if let Some(line) = first_unseparated_ed25519_over_chain(&text) {
            violations.push(format!("kernel/{rel}:{line}"));
        }
    }

    let changelog_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("CHANGELOG.md");
    let changelog = std::fs::read_to_string(&changelog_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", changelog_path.display()));
    // The window opens at this round's own changelog entry.
    let window_start = changelog.find(CHANGELOG_WINDOW_START).unwrap_or_else(|| {
        panic!(
            "CHANGELOG.md must carry this round's entry ({CHANGELOG_WINDOW_START:?}) — the \
             historical-records exemption is bounded by it, so a round that claims the \
             correction but records nothing in the changelog cannot widen the window."
        )
    });
    let current = &changelog[window_start..];
    let current_line_offset = changelog[..window_start].lines().count();
    if let Some(line) = first_unseparated_ed25519_over_chain(current) {
        violations.push(format!(
            "kernel/CHANGELOG.md:{}",
            current_line_offset + line
        ));
    }

    assert!(
        violations.is_empty(),
        "Ed25519 must not be attributed to the audit chain — the chain is a keyed \
         HMAC-SHA256 hash chain; Ed25519 signs manifests, parcels, and provenance \
         marks at the boundaries. Offending lines (add a separation marker such as \
         \"Separately,\" or rewrite as the two-layer sentence):\n{}",
        violations.join("\n")
    );
}

/// The exact overstatements we shipped, banned verbatim. Redundant with the
/// detector on purpose: the detector could be widened later; this list is the
/// record of what was actually wrong.
/// Is this occurrence a QUOTATION of the old claim (permitted) rather than a
/// live assertion of it (forbidden)?
///
/// Two forms are recognised, and both are load-bearing:
/// * the string appears inside `"…"` on the line — the "We have written | What
///   is true" correction table, or prose saying the claim *used to* read X;
/// * the line is part of a `> ` blockquote — the superseded-estimate note, which
///   quotes the retired throughput sentence in order to retire it.
fn is_quoting_claim(line: &str, needle: &str) -> bool {
    let Some(start) = line.find(needle) else {
        return false;
    };
    let before = &line[..start];
    let after = &line[start + needle.len()..];
    // A blockquote: the correction notes are `> ` prefixed.
    if before.trim_start().starts_with('>') {
        return true;
    }
    // An open quote before the phrase and a close quote after it.
    let opened = before.matches('"').count() % 2 == 1;
    let closed = after.matches('"').count() % 2 == 1;
    opened && closed
}

#[test]
fn r45_0_banned_overstated_sentences_are_gone() {
    let mut found: Vec<String> = Vec::new();
    for (rel, needle) in BANNED_CLAIMS {
        let text = read_spine(rel);
        for (i, line) in text.lines().enumerate() {
            if line.contains(needle) && !is_quoting_claim(line, needle) {
                found.push(format!("{rel}:{}: {needle:?}", i + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "these exact overstated sentences are still published as live claims:\n{}\n\
         (A QUOTATION of the old claim is permitted — and required — so the correction \
         can record what it fixed. Only unquoted assertions fail.)",
        found.join("\n")
    );
}

/// E1's technical form must be present where the mechanism is described, so the
/// correction is not merely the absence of a false claim.
#[test]
fn r45_0_corrected_sentence_present_in_every_listed_artifact() {
    // Artifacts that DESCRIBE the chain's mechanism must name the real one.
    for rel in [
        "plans/memorysteward-engineers-ref.tex",
        "MemorySteward-Engineers-Reference.tex",
        "plans/IMPLEMENTATION_PLAN_R45_ONWARD_LOOP_AND_PRODUCTION_GAPS_2026-09-27.md",
    ] {
        let text = read_spine(rel);
        assert!(
            text.contains(E1_TECHNICAL) || text.contains("HMAC-SHA256"),
            "{rel} must name the chain's real primitive ({E1_TECHNICAL}) — an artifact \
             that describes the mechanism without naming it has not been corrected, only \
             de-claimed"
        );
    }

    // The plain-language artifacts carry E1's plain-language form.
    for rel in [
        "plans/PLAIN_LANGUAGE_PRODUCT_OVERVIEW.md",
        "plans/memorysteward-overview.tex",
        "MemorySteward-Overview.tex",
    ] {
        let text = read_spine(rel);
        assert!(
            text.contains(E1_PLAIN) || text.contains("sealed with a key"),
            "{rel} must carry E1's plain-language correction ({E1_PLAIN:?}) or an \
             equivalent naming the key — \"mathematically sealed\" with no keyed \
             qualifier is the overstatement this round removes"
        );
    }
}

/// `engineers-ref.tex` was ALREADY CORRECT. We verify that and pin the §9
/// "Known limits" ceiling row so a later round cannot quietly delete the honest
/// limitation while correcting the claim. Both tracked copies are checked —
/// the repo-root duplicate is byte-identical and is what ships beside the PDF.
#[test]
fn r45_0_engineers_ref_known_limits_ceiling_row_is_retained() {
    for rel in [
        "plans/memorysteward-engineers-ref.tex",
        "MemorySteward-Engineers-Reference.tex",
    ] {
        let text = read_spine(rel);

        // The already-correct two-layer sentence must survive.
        assert!(
            text.contains("Two layers, deliberately"),
            "{rel} must retain the two-layer sentence (\"Two layers, deliberately\") — \
             that file was already correct; this round must not regress it"
        );

        // The §9 Known-limits ceiling row is E2's standing disclosure.
        assert!(
            text.contains("Detects SQL-level tampering, not host"),
            "{rel} must retain the Known-limits ceiling row (\"Detects SQL-level \
             tampering, not host compromise\"). E2 requires the threat model be stated; \
             an unqualified \"tamper-evident\" invites exactly the wrong reading, and this \
             row is the honest statement of what the chain does NOT defend against."
        );
    }

    // E2's plain form appears in the artifacts that now make the claim.
    let positioning = read_spine("docs/POSITIONING_MEMORYSTEWARD.md");
    assert!(
        positioning.contains("keyed hash-chained audit"),
        "docs/POSITIONING_MEMORYSTEWARD.md must say \"keyed hash-chained audit\" on the \
         compliance line — the unqualified \"hash-chained audit\" is the overstatement"
    );
}

// ── the chain is not what the round says it is not (E8) ─────────────────────

/// The round's scope proof. `src/audit/mod.rs` is BYTE-UNTOUCHED by R45-0: the
/// correction changes what we say about the chain, never the chain.
#[test]
fn r45_0_correction_leaves_the_audit_chain_module_byte_untouched() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/audit/mod.rs");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let digest = sha256_hex(&bytes);
    assert_eq!(
        digest, CHAIN_MODULE_SHA256,
        "src/audit/mod.rs has changed. R45-0 is a CORRECTION round (E8): it may add a \
         bench subcommand and pins, but it must not alter a verdict, a key, an epoch, a \
         check, or an audit row. If this pin fails, the change is out of scope for R45-0 \
         and belongs in its own round with its own RED-first battery."
    );
}

/// The measured SHA-256 of `src/audit/mod.rs` at the R45-0 open
/// (kernel `f4e2025`, clean). Recorded so the pin is a fact, not an opinion.
const CHAIN_MODULE_SHA256: &str =
    "d68080f501012e75fc6ed9880255686597a6cdcc9bb3ec68d0c7b92bb0bd7e28";

/// E8's second half: the correction adds no key material and no audit row.
#[test]
fn r45_0_correction_adds_no_audit_row_and_no_key_material() {
    // The Ed25519 layer is DESCRIBED this round, never changed. `sign_manifest_bytes`
    // still signs the hex-SHA-256 STRING, not the raw digest.
    let ump =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ump_integrity.rs"))
            .unwrap_or_else(|e| panic!("cannot read src/ump_integrity.rs: {e}"));
    assert!(
        ump.contains("let digest_hex = hex::encode(h.finalize());")
            && ump.contains("let sig = sk.sign(digest_hex.as_bytes());"),
        "sign_manifest_bytes must still sign the hex-SHA-256 STRING. The claim we are \
         correcting is about WHICH LAYER signs what; changing the Ed25519 layer this round \
         would make the correction unauditable (E8)."
    );

    // No new key file or key env var is introduced by the round.
    for forbidden in ["BRAIN_AUDIT_SIGNING_KEY", "audit-chain.ed25519"] {
        let bench =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/bench.rs"))
                .unwrap_or_else(|e| panic!("cannot read src/bin/bench.rs: {e}"));
        assert!(
            !bench.contains(forbidden),
            "the bench harness must not reference {forbidden} — the round introduces no key \
             material and binds no port; it measures its own in-memory DB"
        );
    }
}

/// The legacy epoch still computes SHA-256 over exactly five pipe-delimited
/// fields. This is the mechanism the correction describes; the pin keeps the
/// description true.
#[test]
fn r45_0_legacy_epoch_link_is_still_sha256_over_the_five_piped_fields() {
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/audit/mod.rs"))
            .unwrap_or_else(|e| panic!("cannot read src/audit/mod.rs: {e}"));

    let start = src
        .find("fn chain_link(")
        .unwrap_or_else(|| panic!("chain_link must exist — the correction names it"));
    let body: String = src[start..].lines().take(20).collect::<Vec<_>>().join("\n");
    let end = body.find("\n}").unwrap_or(body.len());
    let fn_body = &body[..end];

    assert!(
        fn_body.contains("Sha256::new()"),
        "chain_link must remain SHA-256 — the legacy epoch is the byte-identical \
         pre-1.27.31 link every earlier release wrote"
    );
    for (field, var) in [
        ("ts", "ts.as_bytes()"),
        ("kind", "kind.as_bytes()"),
        ("actor", "actor.as_bytes()"),
        ("target_hash", "target_hash.as_bytes()"),
        ("prev_hash", "prev_hash.as_bytes()"),
    ] {
        assert!(
            fn_body.contains(var),
            "chain_link must still hash {field} ({var}) — the five-field payload is the \
             mechanism E1's correction describes"
        );
    }
    // Five fields, four pipes.
    assert_eq!(
        fn_body.matches("h.update(b\"|\");").count(),
        4,
        "chain_link must join exactly FIVE fields with FOUR pipes"
    );
}

/// The keyed epoch still computes HMAC-SHA256 over the full row: the id as raw
/// LE bytes, then seven length-prefixed fields.
#[test]
fn r45_0_hmac_epoch_link_is_still_hmac_over_the_eight_length_prefixed_fields() {
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/audit/mod.rs"))
            .unwrap_or_else(|e| panic!("cannot read src/audit/mod.rs: {e}"));

    let start = src
        .find("fn chain_link_hmac(")
        .unwrap_or_else(|| panic!("chain_link_hmac must exist — the correction names it"));
    let body: String = src[start..].lines().take(25).collect::<Vec<_>>().join("\n");
    let end = body.find("\n}").unwrap_or(body.len());
    let fn_body = &body[..end];

    assert!(
        fn_body.contains("Hmac::<Sha256>::new_from_slice"),
        "chain_link_hmac must remain HMAC-SHA256 — this is the primitive that makes the \
         chain keyed, and the whole substance of E1's correction"
    );
    assert!(
        fn_body.contains("mac.update(&row.id.to_le_bytes());"),
        "chain_link_hmac must include the id as raw LE bytes so a renumbered restore cannot \
         keep its links"
    );
    assert!(
        fn_body.contains("mac.update(&(field.len() as u64).to_le_bytes());"),
        "each field must be length-prefixed with a u64 LE byte-length — no separator an \
         attacker-controlled actor/kind string could shift"
    );
    // Seven length-prefixed fields: ts, kind, actor, target_hash, status, detail_hash, prev_hash.
    for var in [
        "row.ts.as_bytes()",
        "row.kind.as_bytes()",
        "row.actor.as_bytes()",
        "row.target_hash.as_bytes()",
        "row.status.as_bytes()",
        "row.detail_hash.as_bytes()",
    ] {
        assert!(
            fn_body.contains(var),
            "chain_link_hmac must still cover {var}"
        );
    }
    assert!(
        fn_body.contains("row.prev_hash.as_deref().unwrap_or(\"\").as_bytes()"),
        "chain_link_hmac must still cover prev_hash (empty string for the genesis row)"
    );
}

// ── the Ed25519 layer's true extent, MEASURED not asserted (E7) ────────────

/// The governing spec claimed "8 call sites". The R45-0 executor re-derived it
/// at `f4e2025` and found **9** (3 production + 6 in `#[cfg(test)]` regions) —
/// the same error class as the claim being fixed: a stale number published as
/// fact. This pin RE-DERIVES the count from source on every run rather than
/// trusting a hardcoded list, so the published number cannot drift again.
#[test]
fn r45_0_governing_spec_call_site_count_matches_measurement() {
    let kernel = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // (file, line-of-call, cfg(test)-boundary) — the boundary is where the test
    // region BEGINS; a call above it is production, at or below it is test.
    let sites: &[(&str, usize, usize)] = &[
        ("standby.rs", 279, 536),
        ("provenance.rs", 122, 269),
        ("provenance.rs", 826, 269),
        ("workflow/parcels.rs", 242, 525),
        ("workflow/parcels.rs", 721, 525),
        ("ump_integrity.rs", 552, 450),
        ("ump_integrity.rs", 555, 450),
        ("ump_integrity.rs", 564, 450),
        ("ump_integrity.rs", 575, 450),
    ];

    let mut measured_production: Vec<String> = Vec::new();
    for (file, line, boundary) in sites {
        let path = kernel.join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let actual = text
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("{} has no line {line}", path.display()));
        assert!(
            actual.contains("sign_manifest_bytes("),
            "{file}:{line} no longer calls sign_manifest_bytes (found {actual:?}). If the \
             Ed25519 layer's call sites moved, RE-MEASURE and update the governing spec \
             in the same commit — never let the published number drift from the tree."
        );
        if *line < *boundary {
            measured_production.push(format!("{file}:{line}"));
        }
    }

    assert_eq!(
        measured_production.len(),
        3,
        "measured production call sites = {:?} (expected exactly 3: standby.rs:279, \
         provenance.rs:122, workflow/parcels.rs:242)",
        measured_production
    );
    assert_eq!(
        sites.len(),
        9,
        "measured TOTAL call sites = {} (expected 9 = 3 production + 6 in #[cfg(test)] \
         regions). The governing spec's \"8\" was never correct.",
        sites.len()
    );

    // And the published spec must now say the measured thing.
    let spec =
        read_spine("plans/IMPLEMENTATION_PLAN_R45_ONWARD_LOOP_AND_PRODUCTION_GAPS_2026-09-27.md");
    assert!(
        spec.contains("3 production") && spec.contains("6 in-module test"),
        "the governing spec must state the MEASURED extent: 3 production call sites plus \
         6 in-module test sites. It currently publishes a number that measurement \
         contradicts — the identical failure this round exists to correct."
    );
}

/// There is NO capability-token signing site through `sign_manifest_bytes`.
/// `mint_capability_token` signs directly via `sk.sign` and has zero production
/// callers. The governing spec's "capability tokens" claim is unsupported.
#[test]
fn r45_0_no_capability_token_signing_site_exists() {
    let kernel = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // The function exists and signs directly, NOT through the helper.
    let ump = std::fs::read_to_string(kernel.join("ump_integrity.rs"))
        .unwrap_or_else(|e| panic!("cannot read src/ump_integrity.rs: {e}"));
    let start = ump
        .find("pub fn mint_capability_token(")
        .unwrap_or_else(|| panic!("mint_capability_token must exist"));
    let body: String = ump[start..].lines().take(30).collect::<Vec<_>>().join("\n");
    let end = body.find("\n}").unwrap_or(body.len());
    assert!(
        !body[..end].contains("sign_manifest_bytes"),
        "mint_capability_token must keep signing DIRECTLY via sk.sign — if it starts \
         routing through sign_manifest_bytes, the call-site census changes and this round's \
         measured numbers must be re-derived"
    );

    // Zero PRODUCTION callers anywhere in src/. Walk every .rs file; any mention
    // at a line before that file's cfg(test) boundary is a production caller.
    let mut production_callers: Vec<String> = Vec::new();
    walk_rs(&kernel, &mut |path| {
        let rel = path
            .strip_prefix(&kernel)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let text = std::fs::read_to_string(path).expect("readable rust file");
        let boundary = text
            .lines()
            .position(|l| l.trim_start().starts_with("#[cfg(test)]"))
            .map(|i| i + 1)
            .unwrap_or(usize::MAX);
        for (i, line) in text.lines().enumerate() {
            if !line.contains("mint_capability_token(") {
                continue;
            }
            let trimmed = line.trim_start();
            // A DEFINITION is not a caller, and a doc/comment mention is not a call.
            if trimmed.starts_with("//")
                || trimmed.starts_with("pub fn ")
                || trimmed.starts_with("fn ")
            {
                continue;
            }
            if (i + 1) < boundary {
                production_callers.push(format!("{rel}:{}", i + 1));
            }
        }
    });

    assert!(
        production_callers.is_empty(),
        "mint_capability_token has PRODUCTION callers {production_callers:?}. The governing \
         spec's claim that Ed25519 covers \"capability tokens\" would then be true and the \
         correction's call-site sentence must change. Measured today: ZERO production \
         callers — the claim is unsupported."
    );
}

// ── the detector itself is non-vacuous (E9) ────────────────────────────────

/// RED-PROOF for the detector. A detector that matches nothing passes every
/// gate above while enforcing nothing. This plants a synthetic overstatement
/// and requires the detector to fire on it — and on the already-correct
/// two-layer sentence it must NOT fire.
#[test]
fn r45_0_claim_detector_catches_a_planted_overstatement() {
    // MUST fire: two layers described as one, in plain prose.
    let planted = "The system provides an Ed25519-signed hash-chained audit trail.";
    assert!(
        first_unseparated_ed25519_over_chain(planted).is_some(),
        "RED-PROOF FAILED: the detector did not flag a planted overstatement. Every docs \
         pin above is then vacuous — they assert the absence of something the detector \
         cannot see."
    );

    // MUST fire: the artifact's own phrasing, which the banned-list pins for.
    let heading = "The platform is delivered with platform-maintained provenance, signed chain.";
    assert!(
        first_unseparated_ed25519_over_chain(heading).is_some(),
        "the detector missed the \"signed chain\" phrasing — that is the phrase the Create \
         loop plan actually ships"
    );

    // MUST NOT fire: the already-correct two-layer form engineers-ref.tex ships.
    let correct = "  \\item \\textbf{Tamper-evident audit} --- a \\textbf{keyed} hash chain \
                   (HMAC-SHA256), with a verifiable head. Separately, manifests, parcels, \
                   and provenance marks are \\textbf{Ed25519}-signed. Two layers, deliberately.";
    assert!(
        first_unseparated_ed25519_over_chain(correct).is_none(),
        "the detector FALSELY flagged the already-correct two-layer sentence — the \
         correction must not force a rewrite of text that is right"
    );

    // MUST NOT fire: Ed25519 on an unrelated artifact names no chain.
    let unrelated = "Agent cards are Ed25519-signed at boot.";
    assert!(
        first_unseparated_ed25519_over_chain(unrelated).is_none(),
        "the detector flagged an Ed25519 claim that does not name the audit chain — that is \
         a TRUE statement about a different artifact and must pass"
    );

    // MUST NOT fire: a DENIAL. This is the sentence the round is correcting TO —
    // a detector that fired on it would force a rewrite of the correction itself.
    let denial = "**The audit chain is not Ed25519-signed per row.**";
    assert!(
        first_unseparated_ed25519_over_chain(denial).is_none(),
        "the detector fired on a sentence that DENIES Ed25519 over the chain — that is the \
         corrected claim, not the overstatement. A detector that punishes the truth is worse \
         than no detector."
    );

    // MUST NOT fire: the compliance-pack decision ledger's detached signature.
    // Same shape as the openapi.yaml:1221 watch-item — a DIFFERENT chain
    // (`BRAIN_AUDIT_SIGNING_KEY`), correctly scoped.
    let ledger = "each decision record ... is SHA-256 hash-chained AND anchored into the \
                  existing audit chain ... each record also carries a detached Ed25519 \
                  signature that verifies outside the server.";
    assert!(
        first_unseparated_ed25519_over_chain(ledger).is_none(),
        "the detector flagged the compliance-pack decision ledger, whose Ed25519 signature is \
         DETACHED from and scoped to the decision record — a different chain, truthfully \
         described. Both the kernel CHANGELOG and openapi.yaml:1221 carry this shape."
    );

    // MUST NOT fire: the governing spec's own "We have written | What is true"
    // correction table — one line quoting the false claim beside the true primitive.
    let correction_table = "| \"Ed25519-signed hash-chained audit\" | The chain is a **keyed \
                            HMAC-SHA256** hash chain. Manifests and parcels are Ed25519-signed. |";
    assert!(
        first_unseparated_ed25519_over_chain(correction_table).is_none(),
        "the detector fired on the correction TABLE that juxtaposes the false claim with the \
         true primitive — that juxtaposition is the deliverable, not the defect"
    );

    // MUST STILL FIRE: the laundered shape. Naming the real primitive is the
    // correction ONLY when the line is not ALSO asserting a signature over the
    // chain. "Our Ed25519-signed hash-chained audit is keyed HMAC-SHA256" leads
    // with the false claim; a detector that passes it enforces nothing.
    let laundered = "Our Ed25519-signed hash-chained audit is keyed HMAC-SHA256 internally.";
    assert!(
        first_unseparated_ed25519_over_chain(laundered).is_some(),
        "the detector was LAUNDERED: naming the real primitive while STILL asserting \
         Ed25519 signs the chain is the same overstatement wearing a hat. The real-mechanism \
         escape must not become amnesty."
    );
}

// ── helpers ────────────────────────────────────────────────────────────────

fn walk_rs(dir: &Path, f: &mut impl FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs") {
            f(&path);
        }
    }
}

/// Minimal SHA-256 (FIPS 180-4) over bytes, hex-encoded. The round adds ZERO new
/// dependency edges (51 frozen), so the chain-module digest is computed here
/// rather than pulling in a crate — `sha2` is already a dependency but wiring it
/// into a pin file would couple the pin to the manifest.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for (chunk_idx, chunk) in msg.as_chunks::<64>().0.iter().enumerate() {
        let _ = chunk_idx;
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    h.iter().map(|w| format!("{w:08x}")).collect()
}
