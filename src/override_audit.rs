//! Override accounting: **a gate override is a decision, and decisions leave a
//! row.**
//!
//! # The defect this closes
//!
//! `brain restore --force` skips the server-liveness probe — and as of the
//! measurement behind this module, it did so **silently**. Measured at the
//! restore call site, the `!force` arm has **two** consequences and the forced
//! arm has neither:
//!
//! * the guard that refuses a split-brain restore is skipped, **and**
//! * the disclosure that names the probe's **blind spot** is *also* suppressed —
//!   it sits inside the same `if !force` block, so an operator who forces a
//!   restore is never told the probe would not have covered their server.
//!
//! So the forced path emits **nothing**: no refusal, no disclosure, no record.
//! The flag was carried in `restore_needs_confirmation(_force, yes)` so a pin
//! could assert that `--force` never skips the *human* gate, and that pin did —
//! but the *record* was missing, and the blind-spot line went with it.
//!
//! That is a **defect class**: a control exists, it is documented, and the
//! moment a human uses it, **no evidence survives the process.** A restore
//! that overwrote the live database while the service held it open is exactly
//! the accident this module records — and the record was missing.
//!
//! # SCOPE — measured, and deliberately narrow
//!
//! This module counts **CLI-side, operator-forced** overrides. It does **not**
//! duplicate the in-transaction governance overrides, which already exist and
//! already audit correctly:
//!
//! * `service::retention::set_overrides` writes its evidence row **inside the
//!   caller's transaction**, with a rollback twin
//!   (`retention_override_rolls_back_with_its_audit`). That path is correct.
//! * The same holds for the other governance overrides the server routes expose.
//!
//! **The gap this module fills is the one place no transaction exists**: a
//! command-line flag, in a process that is about to overwrite a database file.
//! There is no caller-held transaction to ride, and the audit is best-effort by
//! necessity — the same posture `backup::audit_backup` already takes, and for
//! the same reason: *the audit log must never break a restore.*
//!
//! `OverrideGate::{GovernanceThreshold, RetentionWindow}` are therefore
//! **reserved vocabulary, not live sites.** They are declared so the closed set
//! is not an open string, and the zero counts they carry are the honest state —
//! a round that later routes a governance override through here should find the
//! label already waiting rather than inventing a dimension.
//!
//! # What this module does and does not claim
//!
//! It **counts** overrides, per gate, and it is **refused structurally** from
//! authorizing or permitting anything. An override count is evidence, not
//! consent:
//!
//! * the count is per-gate, so a gate that is **never** overridden may be a
//!   wrong gate (nothing needed relaxing) and one that is **always** overridden
//!   may be too strict (the override has become the path). Neither is a verdict
//!   this module reaches.
//! * **`Observed` is not `Approved`.** Nothing here can turn a recorded override
//!   into an allowed one, and a caller that tries is refused at the type level.
//!
//! # The counted gate vocabulary is closed
//!
//! An open string vocabulary would make the metric a dashboard of whatever a
//! caller felt like typing — which is the harness lesson restated: a gate label
//! that cannot be enumerated cannot be reasoned about. [`OverrideGate`] is the
//! closed set, and `parse` refuses everything outside it.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt;

/// The gates whose override this module counts. **Closed vocabulary** — a
/// caller naming a gate outside this set is refused rather than recorded under a
/// new label, because a metric that grows a dimension at runtime is a metric
/// nobody can chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideGate {
    /// `brain restore --force`: skips the server-liveness probe **and its
    /// blind-spot disclosure**. **Only** those. `--yes` is the human gate, and
    /// `--force` never touches it.
    RestoreLiveness,
    /// Reserved. The in-transaction governance overrides already audit inside
    /// the caller's tx (`service::retention`), so this label exists to keep the
    /// set closed, not because a site currently reports here.
    GovernanceThreshold,
    /// Reserved, for the same reason as [`OverrideGate::GovernanceThreshold`].
    RetentionWindow,
}

impl OverrideGate {
    /// The closed persistence vocabulary. `GATE_OVERRIDE_LABELS` is the
    /// single authority for the labels; nothing writes a literal.
    pub fn as_str(self) -> &'static str {
        match self {
            OverrideGate::RestoreLiveness => "restore_liveness",
            OverrideGate::GovernanceThreshold => "governance_threshold",
            OverrideGate::RetentionWindow => "retention_window",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "restore_liveness" => Ok(OverrideGate::RestoreLiveness),
            "governance_threshold" => Ok(OverrideGate::GovernanceThreshold),
            "retention_window" => Ok(OverrideGate::RetentionWindow),
            other => Err(format!("DI_OVERRIDE_GATE_UNKNOWN:{other}")),
        }
    }
}

/// Every label, in a stable order — so the metric surface and the tests both
/// have one list to agree with.
pub const GATE_OVERRIDE_LABELS: [&str; 3] = [
    "restore_liveness",
    "governance_threshold",
    "retention_window",
];

/// What an override is allowed to do.
///
/// **There is no `Approved` variant, and that is the point.** The whole reason to
/// record an override is that no path may treat recording it as permission. A
/// future round that needs one must add the variant deliberately and name the
/// governance that authorises it — it cannot arrive by making the counter stop
/// complaining.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideStanding {
    /// Recorded. The counter saw it. Nothing else follows.
    Observed {
        gate: OverrideGate,
        /// Who asked. Ref-only, never free prose that could carry content.
        actor: String,
        /// Why, as a **closed justification label** — not a free-text reason.
        /// A free-text justification is an unaudited content channel attached to
        /// a security control, which is the worse of both.
        justification: OverrideJustification,
    },
}

impl OverrideStanding {
    pub fn gate(&self) -> OverrideGate {
        match self {
            OverrideStanding::Observed { gate, .. } => *gate,
        }
    }
    pub fn actor(&self) -> &str {
        match self {
            OverrideStanding::Observed { actor, .. } => actor,
        }
    }
    pub fn justification(&self) -> OverrideJustification {
        match self {
            OverrideStanding::Observed { justification, .. } => *justification,
        }
    }
}

/// Why an override was taken. **Closed vocabulary, again** — and every variant
/// carries its own meaning so the counter can distinguish "operator knew" from
/// "the flag was reflex".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideJustification {
    /// The probe's subject is known-absent: the service is stopped, and the
    /// operator has said so.
    ServiceKnownStopped,
    /// The split-brain was understood and accepted for this one operation.
    SplitBrainUnderstood,
    /// Recovering from a failed prior restore.
    RestoreRecovery,
}

impl OverrideJustification {
    pub fn as_str(self) -> &'static str {
        match self {
            OverrideJustification::ServiceKnownStopped => "service_known_stopped",
            OverrideJustification::SplitBrainUnderstood => "split_brain_understood",
            OverrideJustification::RestoreRecovery => "restore_recovery",
        }
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "service_known_stopped" => Ok(OverrideJustification::ServiceKnownStopped),
            "split_brain_understood" => Ok(OverrideJustification::SplitBrainUnderstood),
            "restore_recovery" => Ok(OverrideJustification::RestoreRecovery),
            other => Err(format!("DI_OVERRIDE_JUSTIFICATION_UNKNOWN:{other}")),
        }
    }
}

pub const JUSTIFICATION_LABELS: [&str; 3] = [
    "service_known_stopped",
    "split_brain_understood",
    "restore_recovery",
];

/// One override, as it reaches the audit row. **Refs only** — the actor and the
/// two closed labels. No free text, so this struct cannot become a content
/// channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideRecord {
    pub gate: OverrideGate,
    pub actor: String,
    pub justification: OverrideJustification,
}

impl fmt::Display for OverrideRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gate={} actor={} justification={}",
            self.gate.as_str(),
            self.actor,
            self.justification.as_str()
        )
    }
}

/// The refusal. Constructing an `OverrideStanding` cannot produce it, and the
/// one function that hands out `Observed` takes no capability — so "an override
/// approved something" is not a reachable state, not a guarded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideRefusal {
    /// An unknown gate label.
    UnknownGate { given: String },
    /// An unknown justification label.
    UnknownJustification { given: String },
    /// An empty actor. An override with no actor is a control with no subject.
    NoActor,
    /// The caller tried to obtain a *permission* from the accounting layer.
    /// Named explicitly so the attempt is a compile- or test-visible event
    /// rather than a shrug.
    NotAnAuthorisation,
}

impl fmt::Display for OverrideRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverrideRefusal::UnknownGate { given } => {
                write!(f, "unknown override gate {given:?}")
            }
            OverrideRefusal::UnknownJustification { given } => {
                write!(f, "unknown override justification {given:?}")
            }
            OverrideRefusal::NoActor => write!(f, "an override must name an actor"),
            OverrideRefusal::NotAnAuthorisation => write!(
                f,
                "the override accounting layer records; it does not authorise"
            ),
        }
    }
}

impl std::error::Error for OverrideRefusal {}

/// Record an override. **The only producer of `OverrideStanding`.**
///
/// Takes the three closed labels as strings (what a CLI flag or a config value
/// actually is) and refuses anything outside the vocabularies, so a typo is a
/// refusal at the boundary rather than a new metric dimension discovered later.
pub fn observe(
    gate: &str,
    actor: &str,
    justification: &str,
) -> Result<OverrideStanding, OverrideRefusal> {
    let gate = OverrideGate::parse(gate).map_err(|_| OverrideRefusal::UnknownGate {
        given: gate.to_string(),
    })?;
    let justification = OverrideJustification::parse(justification).map_err(|_| {
        OverrideRefusal::UnknownJustification {
            given: justification.to_string(),
        }
    })?;
    if actor.trim().is_empty() {
        return Err(OverrideRefusal::NoActor);
    }
    Ok(OverrideStanding::Observed {
        gate,
        actor: actor.to_string(),
        justification,
    })
}

/// The refusal that makes "the counter authorizes" impossible to write by
/// accident. It is total and always an `Err`, and it is the only function here
/// that mentions authorisation — so a reader looking for the permission path
/// finds exactly one place, and that place refuses.
///
/// The `Ok` type is [`Infallible`]: "an override was authorised" is **not a state
/// this signature can represent**, which is the same guarantee the revision
/// routing in the admission suite rests on. Adding a permission would mean
/// changing the return type, and that edit would be visible in a diff — which
/// is the point of spending a type on it.
pub fn authorise(_standing: &OverrideStanding) -> Result<Infallible, OverrideRefusal> {
    Err(OverrideRefusal::NotAnAuthorisation)
}

/// Per-gate counts, plus the interpretation that is explicitly *not* drawn.
///
/// A `BTreeMap` rather than a `HashMap`: the census precedent — a hash seed would
/// make the rendered metric order vary between runs, and a metric whose order
/// changes is a diff nobody can read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverrideCounts {
    by_gate: BTreeMap<OverrideGate, u32>,
}

impl OverrideCounts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one observation. Takes the `OverrideStanding` rather than the
    /// labels, so a caller cannot count something the validator would have
    /// refused.
    pub fn record(&mut self, standing: &OverrideStanding) {
        *self.by_gate.entry(standing.gate()).or_insert(0) += 1;
    }

    pub fn count_for(&self, gate: OverrideGate) -> u32 {
        self.by_gate.get(&gate).copied().unwrap_or(0)
    }

    pub fn total(&self) -> u32 {
        self.by_gate.values().sum()
    }

    /// Gates never overridden. **Not a verdict** — the caller may read it as
    /// "this gate is never needed", which is the opposite of what a zero means
    /// on a control that exists.
    pub fn never_overridden(&self) -> Vec<OverrideGate> {
        GATE_OVERRIDE_LABELS
            .iter()
            .filter_map(|l| OverrideGate::parse(l).ok())
            .filter(|g| self.count_for(*g) == 0)
            .collect()
    }

    /// The metric line. One per gate, in label order, so the output is stable
    /// and diffable across runs.
    pub fn render(&self) -> String {
        let mut out = String::from("brain_override_total{gate=\"");
        out.push_str(GATE_OVERRIDE_LABELS[0]);
        out.push_str("\"} ");
        out.push_str(&self.count_for(OverrideGate::RestoreLiveness).to_string());
        for label in GATE_OVERRIDE_LABELS.iter().skip(1) {
            let gate = match OverrideGate::parse(label) {
                Ok(g) => g,
                Err(_) => continue,
            };
            out.push_str("\nbrain_override_total{gate=\"");
            out.push_str(label);
            out.push_str("\"} ");
            out.push_str(&self.count_for(gate).to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observing_a_known_gate_yields_observed_and_nothing_more() {
        let s = observe("restore_liveness", "operator", "service_known_stopped")
            .expect("three closed labels");
        assert_eq!(s.gate(), OverrideGate::RestoreLiveness);
        assert_eq!(s.actor(), "operator");
        assert_eq!(
            s.justification(),
            OverrideJustification::ServiceKnownStopped
        );
    }

    #[test]
    fn the_gate_vocabulary_is_closed() {
        assert_eq!(
            OverrideGate::parse("restore_liveness"),
            Ok(OverrideGate::RestoreLiveness)
        );
        assert_eq!(GATE_OVERRIDE_LABELS.len(), 3);
        // Every label round-trips, so the metric and the parser cannot drift.
        for l in GATE_OVERRIDE_LABELS {
            let g = OverrideGate::parse(l).expect("label parses");
            assert_eq!(g.as_str(), l, "label round-trips");
        }
        assert!(
            OverrideGate::parse("Restore_Liveness").is_err(),
            "case-sensitive"
        );
        assert!(
            OverrideGate::parse("whatever").is_err(),
            "no open vocabulary"
        );
        assert!(OverrideGate::parse("").is_err(), "empty is not a gate");
    }

    #[test]
    fn the_justification_vocabulary_is_closed_and_round_trips() {
        for l in JUSTIFICATION_LABELS {
            assert_eq!(OverrideJustification::parse(l).expect("parses").as_str(), l);
        }
        assert!(OverrideJustification::parse("because").is_err());
    }

    #[test]
    fn an_unknown_label_is_refused_rather_than_recorded() {
        assert_eq!(
            observe("not_a_gate", "operator", "service_known_stopped"),
            Err(OverrideRefusal::UnknownGate {
                given: "not_a_gate".into()
            })
        );
        assert_eq!(
            observe("restore_liveness", "operator", "no_reason"),
            Err(OverrideRefusal::UnknownJustification {
                given: "no_reason".into()
            })
        );
    }

    #[test]
    fn an_override_with_no_actor_is_refused() {
        // A control with no subject is not a control.
        assert_eq!(
            observe("restore_liveness", "   ", "service_known_stopped"),
            Err(OverrideRefusal::NoActor)
        );
    }

    #[test]
    fn the_accounting_layer_never_authorises() {
        let s = observe("restore_liveness", "operator", "service_known_stopped").expect("ok");
        assert_eq!(authorise(&s), Err(OverrideRefusal::NotAnAuthorisation));
        // And the refusal says so in its own text, so an operator reading a log
        // learns why the flag did not unlock anything.
        assert!(
            OverrideRefusal::NotAnAuthorisation
                .to_string()
                .contains("does not authorise")
        );
    }

    /// **THE load-bearing structural pin.** The pin above pins the RETURN VALUE
    /// of `authorise`, and a red-proof showed it passes green even when this
    /// module grows an `Approved` permission variant — the doc's central promise
    /// was convention, not gate. This pin closes that.
    ///
    /// Written as a non-exhaustive match rather than a string scan, so a
    /// comment cannot satisfy it and a new variant is a COMPILE error.
    #[test]
    fn override_standing_has_exactly_one_variant_and_it_is_observed() {
        let s = observe("restore_liveness", "operator", "service_known_stopped").expect("ok");
        // Adding an `Approved { .. }` variant to `OverrideStanding` makes this
        // match non-exhaustive, which under `RUSTFLAGS="-D warnings"` is a
        // build failure — not a green run with a quietly-broken promise.
        let mut variants: Vec<&'static str> = Vec::new();
        match &s {
            OverrideStanding::Observed { .. } => variants.push("Observed"),
        }
        assert_eq!(
            variants,
            vec!["Observed"],
            "OverrideStanding must carry ONLY Observed. An approval variant here would make the \
             module's promise false, and the pin above would not catch it."
        );
    }

    #[test]
    fn the_authorisation_path_is_a_compile_time_refusal_not_a_runtime_branch() {
        // The `Ok` type is `Infallible`, so "an override was authorised" is not a
        // state the signature can represent. Binding the fn to a pointer of the
        // exact type makes any future widening a COMPILE error — the technique
        // the revision routing in the admission suite already relies on.
        let inf: fn(&OverrideStanding) -> Result<Infallible, OverrideRefusal> = authorise;
        let s = observe("restore_liveness", "operator", "service_known_stopped").expect("ok");
        assert!(inf(&s).is_err(), "total, and always an Err");
    }

    #[test]
    fn counts_accumulate_per_gate_and_total() {
        let mut c = OverrideCounts::new();
        assert_eq!(c.total(), 0);
        for _ in 0..3 {
            let s = observe("restore_liveness", "operator", "service_known_stopped").expect("ok");
            c.record(&s);
        }
        let s = observe("retention_window", "operator", "restore_recovery").expect("ok");
        c.record(&s);
        assert_eq!(c.count_for(OverrideGate::RestoreLiveness), 3);
        assert_eq!(c.count_for(OverrideGate::RetentionWindow), 1);
        assert_eq!(c.count_for(OverrideGate::GovernanceThreshold), 0);
        assert_eq!(c.total(), 4);
    }

    #[test]
    fn a_never_overridden_gate_is_reported_but_not_judged() {
        let c = OverrideCounts::new();
        let never = c.never_overridden();
        assert_eq!(never.len(), 3, "an untouched counter reports every gate");
        // The point of the test: the report is a LIST, and nothing in this
        // module turns it into a verdict about the gate being unnecessary.
        assert!(never.contains(&OverrideGate::GovernanceThreshold));
    }

    #[test]
    fn the_rendered_metric_is_stable_and_complete() {
        let mut c = OverrideCounts::new();
        let s = observe("restore_liveness", "operator", "service_known_stopped").expect("ok");
        c.record(&s);
        let text = c.render();
        // One line per gate, always all three, in label order — so a scrape
        // never sees a dimension appear and disappear.
        assert_eq!(text.lines().count(), 3);
        assert!(
            text.lines()
                .next()
                .expect("first")
                .contains("\"restore_liveness\"} 1")
        );
        assert!(text.contains("\"governance_threshold\"} 0"));
        assert!(text.contains("\"retention_window\"} 0"));
        // Rendering twice is byte-identical: no hash seed, no ordering luck.
        assert_eq!(text, c.render());
    }

    #[test]
    fn a_record_carries_refs_only_and_no_free_text() {
        let r = OverrideRecord {
            gate: OverrideGate::RestoreLiveness,
            actor: "operator".into(),
            justification: OverrideJustification::ServiceKnownStopped,
        };
        // The rendered form is exactly three closed fields.
        assert_eq!(
            r.to_string(),
            "gate=restore_liveness actor=operator justification=service_known_stopped"
        );
    }
}
