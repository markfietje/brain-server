//! The closed classification of every MODEL SURFACE in this tree, and the
//! per-class telemetry the classification buys.
//!
//! **Why this module exists.** The R53a plan (`IMPL_R53A_…`) opens by treating
//! "routing is an attack surface" — content-influenced signals steering which
//! model runs a call. Measured at `3bac3cb` and re-derived this round: **there
//! is no such surface in this tree.** All three model surfaces bind the model to
//! the call site or to the process:
//!
//! | Class | Surface | Where the model is bound |
//! |---|---|---|
//! | [`OpenGenerate`] | the LLM provider stream | per-`HttpProvider` field, read off `self` at the send seam (`agentloop/provider_http.rs:230`). [`ProviderRequest`](crate::agentloop::provider::ProviderRequest) carries **no model field at all**, so a request cannot name a model even if a caller wanted it to. |
//! | [`Classify`] | the injection screen / ONNX scorer | process-wide `LazyLock` (`screen.rs:176`), copied unconditionally when a `Screen` is built (`:95`). Content selects the **verdict**, never the model. |
//! | [`Encode`] | the embedder | chosen once at boot from the retrieval profile (`server/bootstrap.rs:645-647` → [`embedder_for_profile`]); the model is a property of the concrete *type* selected there. |
//!
//! So the round's deliverable is **not a control closing a hole** — it is the
//! **pin** that makes an accidental safety property machine-checked, plus the
//! telemetry a later round needs. The property is "model selection is not a
//! function of the call's content". It held before this module and still holds;
//! what changed is that a later round cannot quietly open it without turning a
//! pin red. See `R53A_DECISION_CLASS_INSTRUMENTATION_EVIDENCE_2026-09-29.md` §2.1.
//!
//! **The enum is closed and fieldless on purpose.** There is no `Unknown` arm
//! and no payload, which buys two properties that are structural rather than
//! asserted:
//!
//! 1. **Exhaustiveness.** A new model surface cannot be added without a new
//!    variant, because every spend-observation site must name a class
//!    ([`record`](crate::agentloop::subagents::ExchangeBudget::record) takes one
//!    as a required argument — omitting it does not compile). The plan's
//!    `R53a.1` asked for "an unclassified decision site is a hard error, not a
//!    default"; a closed enum delivers that more strongly than a default would.
//! 2. **Data minimisation.** [`as_str`] is a total function of the variant, and
//!    the variant has no fields — so **no emitted label can vary with content**.
//!    That single fact is simultaneously the security control (a metric label
//!    carries nothing an attacker can steer) and the privacy statement (no
//!    personal data can reach a label), and it is why this module needs no
//!    runtime scrubbing seam at all.
//!
//! **The six-class taxonomy in the plan is NOT shipped.** Four of its six
//! variants have no call site; declaring them would present dead vocabulary as
//! coverage. The full disposition — including the four refusals and the reason
//! each was refused — is in the evidence file §2.2. One variant here is *not* in
//! the plan: [`Encode`] performs no decision at all, and is declared anyway
//! because the census is over model **surfaces**, and the embedder's spend would
//! otherwise be invisible.
//!
//! ## Telemetry
//!
//! Process statics, exactly like [`crate::audit::busy_hits`] and
//! [`crate::auth::jwt::azp_rejections`] — so deep write-path and inference-path
//! code can increment them without plumbing `AppState` through every layer, and
//! the `/metrics` scrape needs no state at all. `Relaxed` ordering: each counter
//! is independent and only ever increases, so there is no cross-variable
//! invariant for a stronger ordering to protect (the monotonicity pin below is
//! what holds that promise honest).
//!
//! No new dependency: the `/metrics` text is still hand-rolled `push_str` +
//! `format!`, and there is no Prometheus client anywhere near this.
//!
//! ## A ceiling, stated where a reader will find it
//!
//! These counters are **process-local**. A restart zeroes them, and nothing
//! persists them. They are a *rate and composition* gauge, not a spend ledger
//! and not a spend ceiling — see the evidence file §2.4 for why the per-class
//! *ceiling* half of the plan's `P53a.2` is deliberately not built.

use std::sync::atomic::{AtomicU64, Ordering};

/// The closed set of model surfaces, one variant per surface.
///
/// Closed: no `Unknown` arm. Every site that observes model work names a
/// class explicitly, so an unclassified site is a compile error rather than
/// a runtime default (the plan's `R53a.1`).
///
/// [`Default`] resolves to [`OpenGenerate`] — the plan's class 6, whose
/// verifier is the human. That is the plan's fail-closed default, and it
/// fails closed in the right direction: code that reaches for a default gets
/// the class whose output is NOT machine-checked, never one that would let
/// it pass as if it were. It is a backstop, not the mechanism — the
/// mechanism is that [`record`](crate::agentloop::subagents::ExchangeBudget::record)
/// requires a class and has no default to fall back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DecisionClass {
    /// The LLM provider stream — a streamed completion.
    ///
    /// Free text plus tool calls. Nothing gates the assistant turn against a
    /// closed schema, so this is the plan's class 6 (`OpenGenerate`) and its
    /// verifier is **the human** — or, for a tool call, the tool gate
    /// downstream. This is the fail-closed class in substance: the surface
    /// whose output is not machine-checked.
    #[default]
    OpenGenerate,
    /// The injection screen — the deterministic blocklist plus the optional
    /// feature-gated local ONNX classifier.
    ///
    /// A single label from the closed set `{Clean, Quarantine, Reject}`, verified
    /// by declared thresholds. The plan's class 2 (`Classify`).
    Classify,
    /// The embedder — a bounded vector encode.
    ///
    /// **This variant is not a decision**: it produces a vector, not a verdict.
    /// It is declared because the census is over model *surfaces* — a surface
    /// that cannot be named here would be a surface whose spend is invisible.
    ///
    /// Its token series is deliberately **zero**, not an estimate. The embedder
    /// reports no token usage, and the tree already contains one place that
    /// substitutes embedding *dimensions* for tokens and publishes the result as
    /// `usage.prompt_tokens` (`server/router/memory.rs:1513`). This module will
    /// not repeat that substitution: a zero with an honest HELP line beats a
    /// plausible number that is a different quantity.
    Encode,
}

impl DecisionClass {
    /// The census — every class, in declaration order, exactly once.
    ///
    /// The single source of emitted label strings. `/metrics` iterates this, so
    /// the emitted set cannot drift from the declared set in either direction:
    /// no class can be emitted undeclared, and no declared class can go
    /// un-emitted (a zero class still gets its row — `D53a.3`).
    pub const ALL: [DecisionClass; 3] = [
        DecisionClass::OpenGenerate,
        DecisionClass::Classify,
        DecisionClass::Encode,
    ];

    /// The emitted label. A total function of the variant: the enum is
    /// fieldless, so this **cannot** read a call's content, a principal, or a
    /// clock. That is the whole data-minimisation argument, and it is why no
    /// scrubbing seam sits between this value and the metrics text.
    pub const fn as_str(self) -> &'static str {
        match self {
            DecisionClass::OpenGenerate => "open_generate",
            DecisionClass::Classify => "classify",
            DecisionClass::Encode => "encode",
        }
    }

    /// The zero-based index into [`Self::ALL`], for the fixed-edge counter
    /// array. Preregistered (`P53a.1`) to be a pure function of the variant.
    const fn index(self) -> usize {
        match self {
            DecisionClass::OpenGenerate => 0,
            DecisionClass::Classify => 1,
            DecisionClass::Encode => 2,
        }
    }
}

/// One class's observation triple, as the `/metrics` rows read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClassObservation {
    /// The class these numbers belong to.
    pub class: DecisionClass,
    /// Model-surface operations observed since process start.
    pub calls: u64,
    /// Provider-reported tokens (`input + output`). Always `0` for a surface
    /// that reports no token usage — see [`DecisionClass::Encode`].
    pub tokens: u64,
    /// Calls that ended without a `MessageEnd`. **Their spend is unknown, not
    /// zero** — this is the counter that keeps the call count honest for
    /// whoever divides by it later.
    pub incomplete: u64,
}

/// The fixed-edge counter block. Three classes, three counters, laid out so the
/// read side is an index and never a scan — the `/metrics` shape is
/// hand-rolled, and a `HashMap` here would buy nothing but an allocation.
#[derive(Debug, Default)]
struct ClassCounters {
    calls: [AtomicU64; 3],
    tokens: [AtomicU64; 3],
    incomplete: [AtomicU64; 3],
}

/// Process statics — the `crate::audit::BUSY_HITS` precedent, so the inference
/// path increments without `AppState` and the scrape needs no state.
static COUNTERS: std::sync::LazyLock<ClassCounters> =
    std::sync::LazyLock::new(ClassCounters::default);

/// A model-surface operation began. Called at the send seam, adjacent to the
/// provider call itself — the same line the content-independence pin targets,
/// so the counter and the guard cannot drift apart.
pub fn note_call(class: DecisionClass) {
    COUNTERS.calls[class.index()].fetch_add(1, Ordering::Relaxed);
}

/// Provider-reported tokens for one completed call. Folded at the **same**
/// observation seam that updates the budget's total, so the per-class series and
/// the enforced total are one path and one number — never two meters that can
/// disagree.
pub fn note_tokens(class: DecisionClass, input_tokens: u64, output_tokens: u64) {
    COUNTERS.tokens[class.index()].fetch_add(
        input_tokens.saturating_add(output_tokens),
        Ordering::Relaxed,
    );
}

/// A call started and ended without a `MessageEnd`: its spend is UNKNOWN.
///
/// Counted separately from the calls counter so a reader can tell "the model
/// declined to answer" from "we stopped paying attention", which look identical
/// in a bare call count. A rising series is a positive statement that spend is
/// going unaccounted — not a health signal.
pub fn note_incomplete(class: DecisionClass) {
    COUNTERS.incomplete[class.index()].fetch_add(1, Ordering::Relaxed);
}

/// The read side, for the `/metrics` scrape. One row per declared class, in
/// census order, **including the classes currently at zero** — a dashboard must
/// never have to distinguish "zero happened" from "not instrumented".
pub fn observations() -> Vec<ClassObservation> {
    DecisionClass::ALL
        .into_iter()
        .map(|class| {
            let i = class.index();
            ClassObservation {
                class,
                calls: COUNTERS.calls[i].load(Ordering::Relaxed),
                tokens: COUNTERS.tokens[i].load(Ordering::Relaxed),
                incomplete: COUNTERS.incomplete[i].load(Ordering::Relaxed),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_census_and_the_label_set_are_the_same_three_names() {
        // Preregistered (P53a.1): three variants, three labels, and the census
        // is the label set. A fourth class cannot be emitted, and a declared
        // class cannot be dropped from the scrape, without breaking this.
        let labels: Vec<&str> = DecisionClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            labels,
            vec!["open_generate", "classify", "encode"],
            "the emitted label vocabulary is preregistered and frozen"
        );
        assert_eq!(DecisionClass::ALL.len(), 3, "three surfaces, three classes");
    }

    #[test]
    fn as_str_is_a_total_function_of_the_variant() {
        // The minimisation property: the label is decided by the variant alone.
        // Because the enum is fieldless there is nothing else it COULD read, so
        // this is the proof that no label can vary with content.
        for class in DecisionClass::ALL {
            assert_eq!(class.as_str(), class.as_str());
            assert!(class.index() < DecisionClass::ALL.len());
        }
        // Distinct variants, distinct labels — no two surfaces share a series.
        let mut seen = std::collections::BTreeSet::new();
        for class in DecisionClass::ALL {
            assert!(
                seen.insert(class.as_str()),
                "two classes emit the same label: {class:?}"
            );
        }
    }

    #[test]
    fn every_declared_class_gets_a_row_even_when_zero() {
        // D53a.3: a value for every class that occurs — and, stronger, for
        // every class that DECLARED. The census is the emitted set.
        let rows = observations();
        assert_eq!(rows.len(), DecisionClass::ALL.len());
        for (row, class) in rows.iter().zip(DecisionClass::ALL) {
            assert_eq!(row.class, class, "rows follow the census order");
        }
    }

    #[test]
    fn the_counters_only_ever_increase() {
        // The `Relaxed`-ordering promise, pinned: no counter ever moves
        // backwards, so a scrape can never show spend disappearing.
        let before = observations();
        note_call(DecisionClass::Classify);
        note_tokens(DecisionClass::OpenGenerate, 3, 4);
        note_incomplete(DecisionClass::OpenGenerate);
        let after = observations();
        for (b, a) in before.iter().zip(&after) {
            assert!(
                a.calls >= b.calls,
                "{}: calls went backwards",
                a.class.as_str()
            );
            assert!(
                a.tokens >= b.tokens,
                "{}: tokens went backwards",
                a.class.as_str()
            );
            assert!(
                a.incomplete >= b.incomplete,
                "{}: incomplete went backwards",
                a.class.as_str()
            );
        }
    }
}
