//! R63a — determinism contracts: the cross-surface pins.
//!
//! The provider-side pins live in `src/agentloop/provider_http.rs`'s own test
//! module, because they must assert **wire bytes through the real adapter** and
//! that harness is private to it. This file carries the pins that do not need
//! it.
//!
//! **The plan's table was wrong about this surface, and saying so is part of the
//! record.** `IMPL_R63a…` §0 states the injection classifier declares "**No**
//! thread or batch pin". Verified: `src/screen.rs` has set `.with_intra_threads(1)`
//! since the Pores round. The real gap was `with_inter_threads`, unset anywhere
//! in `src/` — inter-op parallelism left to the ONNX Runtime default, which is
//! core-count-derived and therefore host-dependent. That is the deterministic
//! hole, and it is the one now closed.
//!
//! Preregistrations: `R63_R63a_PREREGISTRATION_2026-09-29.md` (P63a.1–P63a.5, drift D-5).

use std::path::{Path, PathBuf};

mod common;

/// Strip `//` and `/* */` comments, string-aware.
///
/// A source pin that greps raw text can pass on a COMMENT naming the symbol —
/// the exact false-pass R51 recorded. Matching is done against code only.
///
/// The body lives in `tests/common/mod.rs`, which **four copies of this
/// stripper previously duplicated and two of them broke**. This one carried the
/// lifetime desync in particular: it measured **14 leaked comment lines** on the
/// two files below, so a pin built on it could scan a comment while believing it
/// had not.
fn code_only(src: &str) -> String {
    common::code_only(src)
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// **A correction to this round's own first draft, kept because the wrong
/// version is more instructive than the right one.**
///
/// The preregistration (drift D-5) reasoned that `with_inter_threads` was the
/// real hole and pinned it. Reading the `ort` 2.0.0-rc.13 source says otherwise:
/// inter-op threads have "no effect when the session execution mode is set to
/// `Sequential`", and parallel execution is **off by default** — nothing in the
/// tree calls `with_parallel_execution(true)`. So the setting would have looked
/// like a determinism control while changing nothing.
///
/// The knob that actually governs reproducible kernels is
/// `with_deterministic_compute`, and it was **off**: the runtime documents the
/// default kernels as faster but "may introduce slight variance".
#[test]
fn classifier_enables_deterministic_compute() {
    let code = code_only(&read("src/screen.rs"));
    assert!(
        code.contains("with_deterministic_compute("),
        "deterministic compute must be configured. `ort` documents the default kernels as \
         faster but liable to 'slight variance' — that is exactly the host-dependent output \
         this round exists to pin, and the build was accepting it."
    );
    assert!(
        code.contains("with_deterministic_compute(SessionThreads::DECLARED.deterministic_compute)"),
        "the builder must read the DECLARED contract, not restate a literal — otherwise the \
         declaration and the setting can drift and nothing notices"
    );
}

/// The negative half of the correction: inter-op threads are NOT set, and the
/// reason is recorded in the code. A future round that enables parallel
/// execution MUST revisit this — and this pin is what makes the revisit
/// mandatory rather than forgotten.
#[test]
fn classifier_does_not_cargo_cult_inert_interop_threads() {
    let code = code_only(&read("src/screen.rs"));
    assert!(
        !code.contains(".with_inter_threads("),
        "inter-op threads are INERT in this session: the runtime defaults to Sequential \
         execution (parallel execution is never enabled here), and `ort` documents inter-op \
         threads as having no effect in Sequential mode. Setting them would look like a \
         determinism control while changing nothing. If you are reading this after enabling \
         `with_parallel_execution`, this assertion is now WRONG and must be revisited WITH \
         that change, not after it."
    );
    assert!(
        !code.contains(".with_parallel_execution("),
        "parallel execution stays off. If it is ever enabled, the inter-op question above \
         becomes real and the Sequential-mode reasoning no longer holds."
    );
}

/// P63a.2 — the convention is copied from `loom.rs`, not invented: declare the
/// contract in ONE named place, and have the builder read it.
#[test]
fn classifier_intra_count_is_pinned_and_read_from_the_declaration() {
    let code = code_only(&read("src/screen.rs"));
    assert!(
        code.contains("with_intra_threads(SessionThreads::DECLARED.intra)"),
        "the intra-op count must stay pinned at 1 (Jetson budget) and be read from the \
         declared contract rather than restated as a literal"
    );
    assert!(
        code.contains("pub const DECLARED: Self = Self"),
        "the contract must be a single named declaration the builder consumes"
    );
}

/// I63a.4 — **the embedder is not edited.** It was already correct, and saying
/// so is a result, not a change. Pinned so a later round cannot "fix" it into a
/// regression by mistake.
#[test]
fn the_embedder_keeps_its_existing_thread_discipline() {
    let code = code_only(&read("src/loom.rs"));
    assert!(
        code.contains(".num_threads(threads)"),
        "the embedder/loom pool must keep declaring its thread count"
    );
    assert!(
        code.contains("current_num_threads()"),
        "the loom pool must keep surfacing its EFFECTIVE thread count"
    );
    assert!(
        !code.contains("with_inter_threads"),
        "I63a.4 is a no-op by design: the embedder was already correct and is not edited. If \
         this ever fires, the round's no-op claim was wrong."
    );
}

/// The ceiling lives in CODE, not only in the evidence file.
///
/// P63a.5 / the plan's own §6: a declared contract is not a guarantee. Anything
/// claiming the system is now deterministic is the overstatement class the
/// claim pins police, so the disclaimer travels with the type.
#[test]
fn the_ceiling_is_stated_beside_the_contract() {
    let src = read("src/agentloop/provider.rs");
    let contract_at = src
        .find("pub(crate) struct SamplingContract")
        .expect("the contract type must exist");
    let ceiling = src
        .find("attribution, not determinism")
        .expect("the ceiling must be stated ON the contract, not only in an evidence file");
    assert!(
        ceiling < contract_at,
        "the disclaimer must sit with the type a future reader meets first"
    );
    // ...and it must say what is NOT claimed, not merely that there is a ceiling.
    let window = &src[ceiling..contract_at + 2000];
    assert!(
        window.contains("batching") && window.contains("not"),
        "the ceiling must name the concrete mechanism (server-side batching, backend model \
         updates, infrastructure) rather than gesturing at 'it might not be deterministic'"
    );
}

/// The open item, named. `P63a.3` wanted the classifier artifact digest
/// recorded per row; drift D-6 established there is **no row to record it in**,
/// and `D63a.7` forbids the schema migration that would create one.
///
/// This pin does not pretend the item is closed. It records that the gap is
/// still open and points at the file that proves it, so the next round inherits
/// a named finding rather than a silently dropped requirement.
#[test]
fn the_classifier_digest_finding_is_still_open_and_named() {
    let screen = code_only(&read("src/screen.rs"));
    assert!(
        !screen.contains("injection_score") && !screen.contains("classifier_score"),
        "if the classifier score gained a persistence seam, D-6 is resolved and this round's \
         open finding should be re-scoped rather than deleted"
    );
    // The evidence file must still be naming it.
    let evidence = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brain-steward-ip/plans/R63a_DETERMINISM_CONTRACTS_EVIDENCE_2026-09-29.md");
    let text = std::fs::read_to_string(&evidence).unwrap_or_else(|e| {
        panic!(
            "the R63a evidence file must exist at {}: {e}",
            evidence.display()
        )
    });
    assert!(
        text.contains("open finding") || text.contains("OPEN"),
        "the evidence file must carry the open finding forward"
    );
}
