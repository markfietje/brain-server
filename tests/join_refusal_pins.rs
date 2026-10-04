// R6 — the I58.6 join, refused: three doors, and what it would take to open one.
//
// ## Why this file exists
//
// `I58.6` asks the gold-set gate to cover model bindings: "a binding whose
// measured agreement regresses blocks." R4 made the *key* expressible —
// `delivery_traces` gained `model_registry_id` / `_version` at schema 1.32.25 —
// and R5 built the agreement column that would be compared. **The substrate is
// real and this round does not dispute it.** What the substrate does not do is
// make the JOIN reachable, and that is what these pins measure and hold.
//
// The refusal is not a judgement about whether the gate is a good idea. It is
// that the two populations it would join have no key, no locator, and — the
// part measured here and not stated anywhere upstream — **no seam at all**.
//
// ## The three doors, and why closing two of them is not enough
//
// An implementer reaches a model binding for a census cell through exactly one
// of three routes. Each is shut separately:
//
// 1. **IN, via the corpus.** A pack gains a model field. Refused by P1
//    (`deny_unknown_fields` + an exact field list on each decoder).
// 2. **OUT, via a report type.** A census report gains a model column. Refused
//    by P2 (exact field lists on `CellResult`, `CellLine`, `CensusReport`).
// 3. **OUT, via the database.** The census reads a trace or a registry row.
//    Refused by P3 (the pure domain has no database seam) and P4 (the service
//    layer's table footprint is bounded to the findings write).
//
// A pin set covering only 1 and 2 would leave 3 open, and 3 is the door the
// plan's own SQL sketch (`t.model_registry_id = r.id`) walks through. That is
// why P3 and P4 exist and why they are not redundant with each other either:
// **P3 is about the domain, P4 is about the service core that owns the SQL**, and
// a round could satisfy one while breaking the other.
//
// ## The measurement behind every claim here
//
// Read at `a1037667`, not inherited:
//
// - The union of every field path across all 11 packs is **61**, enumerated by
//   walking every object and array rather than grepping keywords. Four match a
//   model/registry/version pattern; all four are `scorer_version`,
//   `system_version`, and `evidence_refs` — none names a model. A **value-level**
//   sweep on top returns only false positives (case dispositions, prose).
// - All **12** pack run locators (`adm-4471`…`qc-214`) are **absent** from
//   `src/`, `crates/`, `tests/`, `tools/`, `client/`, probed individually and as
//   a class. Nothing maps a corpus case id to a trace row, a registry row, or a
//   run id.
// - `model_registry_id` is non-null on **exactly one of four** `TraceRow`
//   constructions — the phase advance at `delivery.rs:1192`. That is live
//   delivery-run phase advances. The census population is 7 frozen gold cases.
//   **Disjoint.**
//
// ## Why the comment-stripping in P3 is load-bearing, not hygiene
//
// A source-reading pin that greps raw text passes on a COMMENT naming the
// symbol it forbids. That is not hypothetical: it fired last round, flagging
// the author's own explanatory comment for naming the field it explained the
// absence of. A scanner that tolerates a comment mentioning a symbol has
// stopped being a statement about reads.
//
// So P3 matches against `code_only()` output (the `r63a` precedent), and R6-E
// **proves** it: a `delivery_traces` named in a comment must leave P3 GREEN.
// A pin that cannot demonstrate that distinction has not been shown to
// discriminate at all.
//
// ## Why every scan is scoped
//
// Each scan cuts the region it reads — the production region before the
// `#[cfg(test)]` boundary, and code only with comments stripped. An unscoped
// scan finds the pin's own literal strings, which is the vacuous-pass class
// this repository treats as worse than no check. Every pin additionally
// asserts that it **found** its subject, so "found nothing" can never mean
// "looked at nothing."
//
// ## What this file does NOT claim
//
// - Not `I58.6`/`I58.7`/`D58.7`. They are refused, not partially delivered.
// - Not `I58.8`/`D58.8`. No arming, no would-block count.
// - Not R53. The critical path does not move, and a reader who sees R6 land
//   must not infer that it did.
// - Not a control. Seven cases, single-rater, currently unanimous.
// - Not a schema round, so no ceiling pin moves and `openapi.yaml` does not
//   either: `CellLine` carries no `Serialize` and no route emits it.

use std::path::PathBuf;

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} must be readable: {e}. A read-as-text pin that cannot read its subject passes \
             vacuously, which is worse than no pin.",
            path.display()
        )
    })
}

/// Everything before the file's first `#[cfg(test)]` module.
///
/// Returns an owned `String` so a caller can bind it without holding a
/// temporary alive across the assertion that uses it.
fn production_region(rel: &str) -> String {
    let src = read(rel);
    match src.split_once("#[cfg(test)]") {
        Some((head, _)) => head.to_string(),
        None => src,
    }
}

/// Source with comments removed, preserving string and char literals.
///
/// The `r63a_determinism_pins.rs` precedent, used because the failure it
/// prevents is real and has already fired once: a pin that greps raw text
/// passes on a comment that merely *names* the symbol it forbids. Doc comments
/// explaining a refusal are exactly what this function makes invisible.
///
/// **This body used to live here, and this round's pin caught it still living
/// here.** The machine below is the fix a previous round shipped inside this
/// file while four other copies of the stripper roamed the tree. It is now the
/// ONE shared implementation in `tests/common/mod.rs`, which is a strict
/// superset: it keeps both fixes here made and adds the **escaped quote**
/// (`\"` inside a string), the hazard found by using this machine as the
/// measuring reference for a round whose whole subject was measurement.
///
/// **Two defects in the inherited version, both found by this round's own pins
/// and fixed here.** The `r63a` state machine treats every `'` outside a string
/// as the start of a char literal. Rust lifetimes are written with the same
/// character — `&'static str` at `src/census.rs:90` opens one that never
/// closes, and the machine then believes it is *inside* a char literal for the
/// rest of the file. Every `//` after that point stops being recognised as a
/// comment opener: 6 doc comments survived stripping in that file alone, and
/// the desync continued through string literals too.
///
/// The second defect is in the string arm, and it is the one that bit harder.
/// Rust's line-continuation — `"... \` then the next line — is legal inside a
/// string literal, and this machine did not model it. A `\` before the newline
/// means the string **continues**, so the machine consumed the closing quote of
/// the *following* line as an opener and stayed "inside a string" for the rest
/// of the file.
///
/// **Correction, measured:** the second claim above did not survive
/// re-measurement. The line-continuation defect does **not** leak on this tree —
/// the lifetime fix alone cures 100% of every leak counted (12 on
/// `drift_census.rs`, 7 on `screen.rs`, 12 on `embed.rs`, 91 on `run_loop.rs`),
/// and the continuation fix alone cures **none** of them. The arm is kept
/// because the form is legal Rust, not because it fixed anything, and the
/// "185 doc comments" figure attributed to it is wrong: the true count is 12.
/// See `R7_SCANNERS_AND_ORDER_PREREGISTRATION_2026-10-03.md` §2.3.
fn code_only(src: &str) -> String {
    common::code_only(src)
}

/// The declared field names of `struct <name>`, read from its declaration.
///
/// Driven off the type's **own field list** rather than a hand-chosen string
/// list, so a field this pin has never heard of is still examined — the same
/// reasoning `a_bespoke_per_cell_tolerance_is_refused_structurally` uses on
/// `Cell`, and the reason that pin cannot be defeated by renaming.
///
/// **The visibility prefix is optional, and that is not cosmetic.** The first
/// cut of this helper required `pub(crate) ` or `pub ` and therefore read
/// **zero** fields from `PackCase` and `PackStep`, which are private to the
/// census module. The anti-vacuous assertion in the caller caught it on its
/// first run rather than letting an empty read pass as "the corpus carries
/// nothing" — which is the failure mode this file is about, caught inside
/// itself. Attributes are skipped explicitly so `#[serde(default)]` is never
/// mistaken for a field.
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    let open = format!("struct {name} {{");
    let start = src
        .find(&open)
        .unwrap_or_else(|| panic!("`struct {name} {{` must be declared in the file read"));
    let body = &src[start + open.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("`struct {name}` must close at column 0"));
    body[..end]
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            // Attributes and any residual comment line are not fields. The
            // comment test is belt-and-braces: `code_only` removes them, and a
            // field-list reader that trusted that blindly would mis-read a
            // doc comment containing a colon as a field — which is exactly what
            // it did on this round's first run.
            if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
                return None;
            }
            let rest = t
                .strip_prefix("pub(crate) ")
                .or_else(|| t.strip_prefix("pub "))
                .unwrap_or(t);
            // A declaration without a type is not a field.
            if !rest.contains(':') {
                return None;
            }
            Some(
                rest.split(':')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            )
        })
        .collect()
}

/// Does this field name name a model, a binding, or the key to one?
///
/// Deliberately a **field-name** test and nothing more: it is applied to
/// declared struct fields, where a name is all there is. Pinned as a claim
/// about report/decoder SHAPES, not about values — the census's arithmetic is
/// P3's subject, not this predicate's.
fn is_model_binding_field(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    f.contains("model") || f.contains("registry") || f.contains("binding")
}

/// Does this SQL name a table the census has no business reading?
///
/// The census writes exactly one table (`findings`) and reads one back for its
/// own idempotency check. `audit_events` is the second: it is how a finding row
/// is recognised as already recorded. Anything else is a census reaching past
/// the corpus it measures.
fn is_out_of_corpus_table(table: &str) -> bool {
    !matches!(table, "findings" | "audit_events")
}

const CENSUS_CORE: &str = "src/workflow/drift_census.rs";
const CENSUS_SHELL: &str = "src/census.rs";
const CENSUS_STORE: &str = "src/service/drift_census.rs";

// ─────────────────────────────────────────────────────────────────────────────
// DOOR 1 — IN, via the corpus
// ─────────────────────────────────────────────────────────────────────────────

/// **No corpus pack can carry a model binding, and the decoder refuses one twice
/// over.**
///
/// Two independent controls, because either alone is insufficient:
///
/// 1. **`deny_unknown_fields`** on all three decoder structs. A pack that grows
///    an unrecognised key becomes a **loud decode failure** rather than a field
///    the census silently ignores. A census quietly measuring less than the
///    corpus holds is the failure this whole module exists to prevent, so the
///    strictness is the control and it is asserted on each struct by name.
/// 2. **An exact field list** per decoder. Strictness alone would permit a
///    *recognised* field — someone could add `model_registry_id: String` to
///    `PackCase` and the decoder would accept it happily, because by then it is
///    a known field. The field list is what refuses the second attempt.
///
/// **Read as a field list, not a name list**, so a binding named something this
/// pin never anticipated is still caught by `is_model_binding_field` *and* by
/// the equality assertion below.
#[test]
fn no_pack_field_can_carry_a_model_binding() {
    let production = code_only(&production_region(CENSUS_CORE));

    // Anti-vacuous: the region must have been read, and it must be the real one.
    assert!(
        production.contains("include_str!"),
        "anti-vacuous: the census core must compile its packs in with `include_str!`. The region \
         read here is empty or wrong, so 'no pack field names a model' would be true for the \
         wrong reason."
    );

    // The three decoders, each with the shape it is pinned to.
    //
    // These are the EXACT lists, not the ones this pin would accept. A field
    // added to any of them fails here — which is the point: the corpus is
    // frozen and its provenance was never recorded, so a pack amendment is a
    // separate decision that a pin firing loudly is what should force someone
    // to state.
    let expected: [(&str, &[&str]); 3] = [
        (
            "PackCase",
            &[
                "id",
                "family",
                "system_version",
                "scorer_version",
                "kappa_units",
                "ambiguity_register",
                "evidence_refs",
                "human_pass",
                "artifacts",
            ],
        ),
        (
            "PackArtifacts",
            &[
                "steps",
                "findings",
                "contradictions",
                "audit_ok",
                "repeat_contact",
                "handoff_complete",
                "verified",
                "escalation_honored",
            ],
        ),
        (
            "PackStep",
            &[
                "expected",
                "actual",
                "skipped_verify",
                "abstained",
                "guidance_accepted",
            ],
        ),
    ];

    for (ty, fields) in expected {
        let declared = struct_fields(&production, ty);
        assert!(
            !declared.is_empty(),
            "anti-vacuous: `struct {ty}` must declare at least one field. An empty read means the \
             field-list scan found nothing, which is indistinguishable from a corpus that carries \
             nothing."
        );

        // Control 1: the strict decoder is still strict.
        //
        // **Corrected after R6-B proved the first version vacuous.** That version
        // took the text between the struct and the nearest preceding
        // `deny_unknown_fields` and asserted another `#[serde(` appeared there —
        // so it confirmed the attribute it had just found, and removing the
        // attribute from `PackCase` left `PackArtifacts`' copy inside the window
        // and the check went green. A guard that passes when the control it
        // names is deleted is the vacuous-pass class, and R6-B exists to say it
        // fired.
        //
        // The fix is to slice the **contiguous attribute block immediately above
        // the declaration** — walking backwards over lines only while they are
        // attributes or docs — and require the attribute to be *in that block*.
        // A neighbouring struct's attribute is outside it by construction, so
        // deleting this one cannot borrow the next.
        //
        // **The first attempt at this fix was itself wrong, and the anti-vacuous
        // assertion caught it on the next run.** `find("struct PackArtifacts
        // {")` stops mid-line, so the line immediately above the declaration
        // point is the declaration's own remainder — `pub(crate) ` — which is
        // not an attribute, so the backwards walk broke on its first step and
        // every decoder read as having no attribute block at all. The walk
        // therefore starts from the **start of the declaration's line**, not
        // from the match offset. Recorded because the symptom (an empty block,
        // asserted) looked like a missing attribute rather than a walker that
        // never started.
        let decl_at = production
            .find(&format!("struct {ty} {{"))
            .expect("struct must be declared");
        let line_start = production[..decl_at].rfind('\n').map_or(0, |n| n + 1);
        let before = &production[..line_start];
        let mut attrs = String::new();
        for line in before.lines().rev() {
            let t = line.trim();
            if t.starts_with("#[") || t.starts_with("///") || t.starts_with("/*") {
                attrs.insert_str(0, &format!("{t}\n"));
                continue;
            }
            if t.is_empty() {
                // A blank line still separates the block from a neighbour.
                break;
            }
            break;
        }
        assert!(
            !attrs.trim().is_empty(),
            "anti-vacuous: `struct {ty}` must have a readable attribute block above it. An empty \
             block means the backwards walk started at the wrong offset, not that the attribute is \
             missing — the declaration at {decl_at} is preceded by {} bytes.",
            decl_at - line_start
        );
        assert!(
            attrs.contains("#[serde(deny_unknown_fields)]"),
            "`struct {ty}` must still carry `#[serde(deny_unknown_fields)]` on its OWN declaration. \
             The attribute block directly above it is:\n{attrs}\nWithout it a pack could grow a \
             key the census silently ignores — a census measuring less than the corpus holds, \
             quietly, which is the failure this module exists to prevent. R6-B caught the first \
             version of this check passing with the attribute deleted: it searched backwards to \
             the nearest copy anywhere above, so a neighbouring struct's attribute satisfied it."
        );

        // Control 2: the exact field list.
        let want: Vec<String> = fields.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(
            declared, want,
            "`struct {ty}` declares {declared:?}, expected {want:?}. The gold corpus is FROZEN \
             under a standing refusal whose provenance was never recorded: nothing in this tree \
             writes a pack, bumps a scorer version, or re-serialises one. A field here is a corpus \
             amendment, which is a separate decision and not one a census decoder makes by growing \
             a struct."
        );

        // And the predicate, so a field named something unexpected cannot slip
        // past an equality assertion that someone later edits.
        let binding: Vec<&String> = declared
            .iter()
            .filter(|f| is_model_binding_field(f))
            .collect();
        assert!(
            binding.is_empty(),
            "`struct {ty}` declares a model-binding field {binding:?}. A corpus pack that names a \
             model is the only way I58.6's join becomes expressible from the corpus side, and the \
             corpus carries no model — measured across all 61 field paths of all 11 packs. The \
             near-misses are `scorer_version` (\"1\"), `system_version` (\"1.28.6\"), and inert \
             `evidence_refs` labels; none names a model."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DOOR 2 — OUT, via a report type
// ─────────────────────────────────────────────────────────────────────────────

/// **No census report type can carry a model binding.**
///
/// This is the door the agreement column itself did not open: `CellResult`
/// gained `agreement_units`, which is measured over the corpus and names no
/// model. A model binding is a different quantity — it would be the same field
/// carrying a **per-model** attribution the corpus has no vocabulary for.
///
/// **`CellLine` and `CensusReport` are new coverage.** Measured at `a1037667`:
/// zero references to either anywhere in `tests/`. The R5 round pinned
/// `CellResult`'s field list and left the CLI's own projection unpinned, so a
/// model column could have arrived on `CellLine` without any pin noticing. That
/// gap closes here, and it is the reason this pin is not a restatement of
/// `the_census_reports_agreement_and_carries_no_verdict_column`.
#[test]
fn no_census_report_type_can_carry_a_model_binding() {
    // The pure domain's report type.
    let core = code_only(&production_region(CENSUS_CORE));
    let results = struct_fields(&core, "CellResult");
    assert!(
        !results.is_empty(),
        "anti-vacuous: `CellResult` must declare fields, or the scan read nothing."
    );
    assert_eq!(
        results,
        vec![
            "id",
            "verdict",
            "observed_units",
            "delta_units",
            "agreement_units"
        ],
        "`CellResult`'s fields are {results:?}. The agreement column is the fifth and is measured \
         over the frozen corpus alone — it names no model, and a sixth field is either a second \
         vocabulary or the per-model attribution I58.6 would need and the corpus cannot supply."
    );

    // The CLI's projection of it. Both are in `src/census.rs`, and both are
    // unpinned by any other suite — measured, 0 references in `tests/`.
    let shell = code_only(&read(CENSUS_SHELL));
    for (ty, want) in [
        (
            "CellLine",
            vec![
                "id",
                "verdict",
                "observed_units",
                "delta_units",
                "agreement_units",
            ],
        ),
        (
            "CensusReport",
            vec![
                "cells",
                "breaches",
                "unbaselined",
                "orphaned",
                "rows_written",
            ],
        ),
    ] {
        let declared = struct_fields(&shell, ty);
        assert!(
            !declared.is_empty(),
            "anti-vacuous: `struct {ty}` must declare fields in {CENSUS_SHELL}, or the scan read \
             nothing and would pass on any edit."
        );
        assert_eq!(
            declared, want,
            "`struct {ty}`'s fields are {declared:?}, expected {want:?}. This is the CLI's own \
             projection and no other pin in the tree covers it — measured at 0 references in \
             `tests/` — so a model binding added here would have drifted in silently."
        );
        let binding: Vec<&String> = declared
            .iter()
            .filter(|f| is_model_binding_field(f))
            .collect();
        assert!(
            binding.is_empty(),
            "`struct {ty}` declares a model-binding field {binding:?}. A census cell cannot report \
             per-model agreement: the census reads no delivery trace and no registry row, so there \
             is no model for a cell to be attributed to."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DOOR 3 — OUT, via the database
// ─────────────────────────────────────────────────────────────────────────────

/// **The pure census domain has no database seam at all.** The load-bearing pin.
///
/// Measured: a case-insensitive scan of `drift_census.rs` for
/// `connection|rusqlite|registry|delivery_traces|model_ref|model_registry|
/// resolve_for_execution` returns **nothing**. All 1,237 lines of it are a pure
/// function of `&[Cell]`, `&BTreeMap<String, i32>`, and `&BTreeMap<String,
/// bool>`.
///
/// **This is the measurement the refusal actually rests on.** The plan's join
/// needs the census to read a trace row. It cannot — not because a key is
/// missing, but because **no code path exists that could read one**. Disjoint
/// populations are therefore structural rather than accidental, and a future
/// round that "fixes the missing key" without this seam still has no join.
///
/// **Scanned against `code_only()`, and R6-E proves that matters.** A doc comment
/// naming `delivery_traces` in order to explain this refusal is invisible to
/// this pin — which is the correct behaviour, and the distinction the R57b
/// lesson turned on. A scanner that fired on such a comment would be flagging
/// the explanation rather than the code.
#[test]
fn the_census_core_has_no_database_seam() {
    let production = code_only(&production_region(CENSUS_CORE));

    // Anti-vacuous: the region read must be the real, substantial one.
    assert!(
        production.contains("include_str!") && production.len() > 5_000,
        "anti-vacuous: the census core's production region must be read in full ({} bytes after \
         comment-stripping). A scan that looked at nothing reports 'no database seam' for the \
         wrong reason.",
        production.len()
    );

    // The forbidden vocabulary. A census that reads a trace row would name one
    // of these in CODE; a census that only explains why it does not, does not.
    for forbidden in [
        "Connection",
        "rusqlite",
        "delivery_traces",
        "decision_model_registry",
        "model_registry_id",
        "model_registry_version",
        "model_ref",
        "resolve_for_execution",
    ] {
        assert!(
            !production.contains(forbidden),
            "the pure census domain now names `{forbidden}` in CODE. The census is the subject I58.6 \
             would join a model binding onto, and it has no database seam: no Connection, no trace \
             read, no registry read. Adding one changes what the census measures — from a fixed \
             corpus re-scored against one tolerance, to something whose population depends on live \
             delivery runs. That is a different instrument and needs its own round, its own \
             preregistration, and the answer to 'which seven cases did this pass even read?'."
        );
    }

    // The signature is the seam in its most direct form: three pure arguments.
    // Pinned structurally (sliced from the declaration) so `cargo fmt` reflowing
    // the parameter list cannot break it — the r66a lesson.
    let sig = production
        .split("pub(crate) fn census(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .expect("`census(` must be declared");
    assert!(
        sig.contains("cells: &[Cell]") && sig.contains("verdicts: &BTreeMap<String, bool>"),
        "`census()` must keep its pure three-argument signature, found: {sig}"
    );
    assert!(
        !sig.contains("Connection") && !sig.to_lowercase().contains("conn"),
        "`census()` grew a connection parameter ({sig}). Taking a connection is the seam, whatever \
         it is later used for."
    );
}

/// **The census's database seam is bounded to the findings write.**
///
/// P3 is about the *domain*; this is about the *service core that owns the
/// SQL*, and the two are deliberately not redundant — a round could leave the
/// pure domain untouched while adding a trace read beside it, which would pass
/// P3 and fail here.
///
/// Measured footprint across both census files:
/// `FROM findings · INTO findings · FROM audit_events`. Nothing else. The
/// census writes a breach as a hash-chained row and reads it back to stay
/// idempotent; that is the entire persistence story.
#[test]
fn the_census_database_seam_is_bounded_to_the_findings_write() {
    for rel in [CENSUS_STORE, CENSUS_SHELL] {
        let code = code_only(&read(rel));
        assert!(
            !code.is_empty(),
            "anti-vacuous: {rel} must be readable and non-empty."
        );

        // Every table named after a SQL keyword, in any case.
        let mut out_of_corpus: Vec<String> = Vec::new();
        let bytes = code.as_bytes();
        for kw in ["FROM ", "INTO ", "UPDATE ", "JOIN "] {
            let mut i = 0usize;
            while let Some(found) = code[i..].find(kw) {
                let at = i + found + kw.len();
                let table: String = code[at..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !table.is_empty()
                    && is_out_of_corpus_table(&table)
                    && !out_of_corpus.contains(&table)
                {
                    out_of_corpus.push(table);
                }
                i = at.min(bytes.len());
            }
        }
        assert!(
            out_of_corpus.is_empty(),
            "{rel} reaches tables outside the census's own write: {out_of_corpus:?}. The census's \
             persistence is a hash-chained `findings` row plus the `audit_events` read that makes \
             it idempotent, and nothing else. `delivery_traces` or `decision_model_registry` here \
             would be the I58.6 seam opening — and it would join a live delivery population to a \
             seven-case frozen corpus, which are disjoint by measurement."
        );
    }

    // And the seam that IS sanctioned, asserted so this pin is not merely
    // "found nothing": the findings write is real and is the census's evidence.
    let store = code_only(&read(CENSUS_STORE));
    assert!(
        store.contains("INSERT INTO findings"),
        "the census must still write its breach as a `findings` row. A census with no persistence \
         at all reports drift that leaves no evidence, which is the failure `a_breach_is_a_\
         hash_chained_row_not_a_log_line` exists to prevent."
    );
}
