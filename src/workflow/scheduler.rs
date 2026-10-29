//! The CRANK scheduler core — the dial's flip one (P-4, the absorbed
//! `EXECUTION_PROMPT_CRANK_2026-10-27.md`). THE LAW, unchanged from that
//! spec: *the scheduler adds calls, not decisions; the risk class picks the
//! row; the red flag is the tripwire.*
//!
//! The MEASURE-POINTS resolved against this tree:
//!
//! 1. **Where the risk tier lives.** The DepEd table keys on the case's
//!    risk class; in this tree the class is declared by the DOMAIN'S BOUND
//!    PROFILE — the C3 binding is exactly the deployment's data-class
//!    declaration (education → learner records → assistive; the finance /
//!    health / gov profiles → determinations on people's consequences). A
//!    per-case class does not exist before Triage runs, and the case
//!    machine's freshness law (`state '{}'`, revision 0) leaves no room to
//!    carry one pre-launch — so the row is picked per DOMAIN, by profile,
//!    and an unbound or unknown profile fails CLOSED to the conservative
//!    row (human-cranked). The scheduler never infers a class.
//! 2. **Who cranks today.** For the GDL case machine the human surface is
//!    the case-launch route (`POST /workflow/cases/{id}/gdl`) — the ONE
//!    path into `checkpoint::advance`, whose freshness law already refuses
//!    any second launch (409). The scheduler's crank IS that POST: one
//!    call, the machine's own gates/stops/budgets inside, the outcome
//!    recorded, never retried (a stop or terminal hands the case back to a
//!    human by the route's own law). The harness loop's own scheduler pass
//!    (`brain workflow crank` over WFM runs) is flip-one's SECOND surface
//!    and is deliberately not built here — this round witnesses the dial
//!    against the pilot's live case machine.
//! 3. **Stop respect.** The planner refuses any run whose case state
//!    carries an open red flag (the tripwire — never touched, even to
//!    skip-and-retry) or whose status says the case already ended
//!    (`resolved`); a refused launch (409/4xx) is recorded and the batch
//!    moves on. No retry, no widening, no looping past a stop.
//!
//! NO daemon, NO server change, NO new HTTP surface: the core is pure
//! planning; the CLI verb owns enumeration (local DB read) and the crank
//! (the same launch POST the human uses). `--dry-run` is the DEFAULT —
//! `--yes` is the only writer.

use serde::{Deserialize, Serialize};

/// The DepEd risk tiers (the comparison doc §3's rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrankTier {
    /// Consequences for people — human-cranked, always.
    High,
    /// Assistive, no determinations — scheduler auto-crank within budgets.
    Limited,
    /// Drafting, filtering, IT — already fully automatic.
    Minimal,
}

impl CrankTier {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "limited" => Some(Self::Limited),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Limited => "limited",
            Self::Minimal => "minimal",
        }
    }
}

/// The tier table: the bound profile name picks the row. A name outside
/// this table (or an unbound domain) is `None` — the planner's fail-closed
/// row, never a guess. The mapping IS the declaration: binding a profile to
/// a domain is the operator's risk classification, made once, through the
/// C3 door.
pub fn profile_tier(profile: &str) -> Option<CrankTier> {
    match profile {
        // Assistive, no determinations: learner records (the DepEd
        // beachhead), the C3 posture.
        "education" => Some(CrankTier::Limited),
        // Determinations on people's consequences — human-cranked, always.
        "finance-mifid" | "finance-sox" | "health-hipaa" | "gov-fedramp" => Some(CrankTier::High),
        // Drafting, filtering, IT — already fully automatic.
        "call-center"
        | "engineering"
        | "bpo-multi"
        | "smb-simple"
        | "medium-team"
        | "sales-team"
        | "hr-people"
        | "global-multi-region"
        | "enterprise" => Some(CrankTier::Minimal),
        _ => None,
    }
}

/// One run's crank plan: the row the risk class picked.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrankAction {
    /// The scheduler would POST the case launch (the ONE advance path).
    Crank,
    /// The risk class or the case's own state refuses; the reason is the
    /// record the operator reads.
    Skip { reason: String },
}

/// The plan for one run — pure, no IO, so the pins ride the lib suite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CasePlan {
    pub run_id: i64,
    pub domain: String,
    /// The tier the domain's profile declares; `None` is the fail-closed
    /// unknown (the skip reason says so).
    pub tier: Option<CrankTier>,
    /// The red-flag tripwire, when the case state carries a triage.
    pub red_flag_open: bool,
    pub action: CrankAction,
}

/// Plan one run. `state_json` is the run's persisted state; `status` is the
/// run row's status. The filters, in order — every refusal is a NAMED
/// reason:
///
/// 1. a case that already ended (`resolved`) — the machine owns its close;
/// 2. the red-flag tripwire — an open flag is NEVER touched;
/// 3. the fail-closed tier row — no profile, or a profile outside the
///    table, is human-cranked;
/// 4. the selected tiers (`--tier`, default `limited,minimal`) — High and
///    unselected tiers are never auto-cranked;
/// 5. a case whose state is no longer the fresh `{}` — the case machine
///    owns that run (launched, routed or escalated); a second launch would
///    be refused by the route's freshness law, and the scheduler does not
///    spend a call to be refused.
pub fn plan_case(
    run_id: i64,
    domain: &str,
    status: &str,
    state_json: &str,
    selected: &[CrankTier],
    tier: Option<CrankTier>,
) -> CasePlan {
    let red_flag_open = red_flag_open_in(state_json);
    let plan = |action| CasePlan {
        run_id,
        domain: domain.to_string(),
        tier,
        red_flag_open,
        action,
    };
    if status == "resolved" {
        return plan(CrankAction::Skip {
            reason: "case resolved — the machine owns its close".into(),
        });
    }
    if red_flag_open {
        return plan(CrankAction::Skip {
            reason: "red flag open — the tripwire; human-cranked, always".into(),
        });
    }
    let Some(tier) = tier else {
        return plan(CrankAction::Skip {
            reason: "unknown tier (no bound profile, or a profile outside the table) — \
                     fail-closed human-cranked"
                .into(),
        });
    };
    if !selected.contains(&tier) {
        return plan(CrankAction::Skip {
            reason: format!(
                "tier {} not in the selected set — human-cranked",
                tier.as_str()
            ),
        });
    }
    if state_json.trim() != "{}" {
        return plan(CrankAction::Skip {
            reason: "case state is the case machine's (launched/routed/escalated) — \
                     the freshness law owns re-launches"
                .into(),
        });
    }
    plan(CrankAction::Crank)
}

/// The red-flag tripwire, read straight out of the case state: a triage
/// artifact whose `red_flag` is present and open. This is the planner's OWN
/// read — `gdl::red_flag_is_open` needs a typed artifact; the state JSON
/// carries the same shape (`red_flag.open`/`ruled_out`), and the pin holds
/// the two readings together.
fn red_flag_open_in(state_json: &str) -> bool {
    #[derive(Deserialize)]
    struct CaseState {
        #[serde(default)]
        triage: Option<TriagePeek>,
    }
    #[derive(Deserialize)]
    struct TriagePeek {
        #[serde(default)]
        red_flag: Option<RedFlagPeek>,
    }
    #[derive(Deserialize)]
    struct RedFlagPeek {
        #[serde(default)]
        open: bool,
        #[serde(default)]
        ruled_out: bool,
    }
    let Ok(state) = serde_json::from_str::<CaseState>(state_json) else {
        return false;
    };

    state
        .triage
        .and_then(|t| t.red_flag)
        .map(|rf| rf.open && !rf.ruled_out)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected() -> Vec<CrankTier> {
        vec![CrankTier::Limited, CrankTier::Minimal]
    }

    /// THE TIER TABLE, fail-closed: the known profiles land on their rows;
    /// anything else is `None` — never a guess, never a default row.
    #[test]
    fn the_tier_table_maps_declared_profiles_and_fails_closed() {
        assert_eq!(profile_tier("education"), Some(CrankTier::Limited));
        for high in [
            "finance-mifid",
            "finance-sox",
            "health-hipaa",
            "gov-fedramp",
        ] {
            assert_eq!(profile_tier(high), Some(CrankTier::High), "{high}");
        }
        for minimal in ["call-center", "engineering", "smb-simple"] {
            assert_eq!(profile_tier(minimal), Some(CrankTier::Minimal), "{minimal}");
        }
        for unknown in ["", "school-district", "acme", "EDUCATION"] {
            assert_eq!(profile_tier(unknown), None, "{unknown:?} must fail closed");
        }
        assert_eq!(CrankTier::parse("high"), Some(CrankTier::High));
        assert_eq!(CrankTier::parse("HIGH"), None, "the parse is exact");
    }

    /// THE FRESH, LIMITED CASE IS THE CRANK.
    #[test]
    fn a_fresh_limited_case_is_planned_for_crank() {
        let plan = plan_case(
            1,
            "education",
            "active",
            "{}",
            &selected(),
            profile_tier("education"),
        );
        assert_eq!(plan.action, CrankAction::Crank);
        assert_eq!(plan.tier, Some(CrankTier::Limited));
        assert!(!plan.red_flag_open);
    }

    /// THE RED-FLAG TRIPEWIRE: a state carrying an OPEN red flag is never
    /// planned — not for any tier, not by any selection. (Red-proof: plant
    /// a crank past the filter, the pin reds.)
    #[test]
    fn a_red_flagged_case_is_never_planned() {
        let state = r#"{"phase":"Triage","ticket":"t","triage":{"red_flag":{"open":true,"ruled_out":false,"worst_case":"x"}}}"#;
        for tier in [None, Some(CrankTier::Limited), Some(CrankTier::Minimal)] {
            let plan = plan_case(2, "education", "active", state, &selected(), tier);
            assert!(
                matches!(plan.action, CrankAction::Skip { .. }),
                "tier {tier:?}: an open red flag is never cranked"
            );
            assert!(plan.red_flag_open);
        }
        // And a RULED-OUT flag is closed — the tripwire reads the whole
        // law (the skip below is the freshness arm's, never the tripwire's:
        // the state carries a case, so the machine owns it either way).
        let ruled_out = r#"{"triage":{"red_flag":{"open":true,"ruled_out":true}}}"#;
        let plan = plan_case(
            3,
            "education",
            "active",
            ruled_out,
            &selected(),
            profile_tier("education"),
        );
        assert!(!plan.red_flag_open, "a ruled-out flag reads closed");
        let CrankAction::Skip { reason } = plan.action else {
            panic!("the case-machine-owned state still skips");
        };
        assert!(reason.contains("freshness"), "{reason}");
    }

    /// A HIGH-TIER (or unselected-tier) case is never auto-cranked.
    #[test]
    fn a_high_tier_case_is_never_planned() {
        let plan = plan_case(
            4,
            "health",
            "active",
            "{}",
            &selected(),
            Some(CrankTier::High),
        );
        assert!(matches!(plan.action, CrankAction::Skip { .. }));
        // And the unknown row fails closed even when the selection is wide.
        let plan = plan_case(5, "acme", "active", "{}", &selected(), None);
        let CrankAction::Skip { reason } = plan.action else {
            panic!("an unknown tier must skip");
        };
        assert!(reason.contains("fail-closed"), "{reason}");
    }

    /// STOP RESPECT: a resolved run is skipped (the machine owns its close);
    /// a launched/routed/escalated state is the machine's — the planner
    /// spends no call that the freshness law would refuse.
    #[test]
    fn terminal_and_owned_cases_are_never_planned() {
        let plan = plan_case(
            6,
            "education",
            "resolved",
            "{}",
            &selected(),
            profile_tier("education"),
        );
        assert!(matches!(plan.action, CrankAction::Skip { .. }));
        let routed_state = r#"{"phase":"Triage","ticket":"t"}"#;
        let plan = plan_case(
            7,
            "education",
            "active",
            routed_state,
            &selected(),
            profile_tier("education"),
        );
        let CrankAction::Skip { reason } = plan.action else {
            panic!("a case-machine-owned state must skip");
        };
        assert!(reason.contains("freshness"), "{reason}");
    }

    /// THE SCHEDULER CANNOT DISPOSE (source scan): this module's code opens
    /// no approve/promote/reject path — the tokens are assembled here so
    /// the scan cannot hit its own literals.
    #[test]
    fn the_scheduler_core_has_no_dispose_path() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/workflow/scheduler.rs"
        ))
        .expect("this module is in the tree");
        let code: String = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !(trimmed.starts_with("//") || trimmed.starts_with("*"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let banned = [
            ["ap", "prove"].concat(),
            ["pro", "mote"].concat(),
            ["re", "ject"].concat(),
            ["le", "gal-hold"].concat(),
        ];
        for banned in banned {
            assert!(
                !code.contains(banned.as_str()),
                "the scheduler must never touch a dispose surface: found `{banned}`"
            );
        }
    }
}
