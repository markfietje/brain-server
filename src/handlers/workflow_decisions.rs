//! The operator decision surfaces (the loop closeout).
//!
//! - `POST /workflow/runs/{id}/handoff/decision` — the operator delivers or
//!   cancels a handoff. A decision-required transition never moves without a
//!   decision reference (the machine-refusal law), and the law is enforced at
//!   the surface as `400 decision_ref_required` — the machine cannot close a
//!   handoff on its own authority.
//! - `POST /workflow/runs/{id}/back-referral/return` — the receiver's
//!   release: the operator's report + decision reference releases a return
//!   contract. A report missing a required field surfaces the machinery's B3
//!   refusal as a named 400, an absent contract answers 404 probe-blind, and
//!   `late` is computed at the server clock (the client never supplies it).
//!
//! Both clone the relay precedent end to end: the run's domain resolves
//! first (404 probe-blind on an absent or foreign run), Write on the run's
//! domain plus the `workflow` role gate, and ONE `WorkflowTx` carries the
//! write and its audit row — the chain proves who decided what, when.

use axum::{
    Json,
    extract::{Path, State},
};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::gdl::HandoffTransition;

/// The decision reference is the audit-recovery handle for the operator's
/// call — screened, bounded, no free-text pass-through.
const MAX_DECISION_REF_LEN: usize = 256;

fn validate_decision_ref(raw: &str) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HandlerError::bad_request(
            "decision_ref_required",
            "an operator decision reference is required — the machine never \
             closes a handoff or releases a return contract on its own \
             authority",
        ));
    }
    if trimmed.len() > MAX_DECISION_REF_LEN
        || trimmed
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "decision_ref_invalid",
            format!(
                "decision_ref must be 1..={MAX_DECISION_REF_LEN} chars with no \
                 control or invisible characters"
            ),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_contract_key(raw: &str) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DECISION_REF_LEN {
        return Err(HandlerError::bad_request(
            "contract_key_required",
            format!("contract_key must be 1..={MAX_DECISION_REF_LEN} chars"),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "contract_key_required",
            "contract_key must not contain control or invisible characters",
        ));
    }
    // `%` and `_` are deliberately ALLOWED here. An earlier pass banned them as
    // SQL LIKE metacharacters, which was wrong: the server's own key format
    // is `run{id}:back_referral:{owner}` and therefore contains an underscore,
    // so the ban refused every honest key. The wildcard is neutralized at the
    // QUERY instead, with an `ESCAPE` clause and an escaped pattern — which
    // keeps the key free-text for the caller while making the match literal.
    Ok(trimmed.to_string())
}

/// The machinery's refusals surface named: an absent contract answers 404
/// probe-blind, the B3 report-field law answers `400 report_incomplete` with
/// the missing list, and anything else is internal (never a silent drop).
fn return_err(e: crate::agentloop::run_loop::LoopError) -> HandlerError {
    match e {
        crate::agentloop::run_loop::LoopError::Persist(m)
            if m == "back-referral contract absent" =>
        {
            HandlerError::not_found("back-referral contract not found")
        }
        crate::agentloop::run_loop::LoopError::Persist(m) if m.starts_with("B3:") => {
            HandlerError::bad_request_with(
                "report_incomplete",
                "the report is missing required fields — the contract's B3 \
                 law holds at the surface",
                serde_json::json!({ "missing": m.split("; ").collect::<Vec<_>>() }),
            )
        }
        other => HandlerError::internal(other.to_string()),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffDecisionBody {
    pub transition: String,
    #[serde(default)]
    pub decision_ref: Option<String>,
}

/// `POST /workflow/runs/{id}/handoff/decision`
pub async fn post_handoff_decision(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<HandoffDecisionBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let transition = match body.transition.as_str() {
        "delivered" => HandoffTransition::Delivered,
        "cancelled" => HandoffTransition::Cancelled,
        _ => {
            return Err(HandlerError::bad_request(
                "unknown_transition",
                "transition must be delivered | cancelled",
            ));
        }
    };
    let raw_ref = body.decision_ref.as_deref().unwrap_or("");
    let decision_ref = validate_decision_ref(raw_ref)?;
    let actor = super::recall::principal_label(&principal);
    let echo = crate::gate::sanitize_read(&decision_ref, false, &principal);

    let outcome = tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let detail = serde_json::json!({ "route": "workflow_decisions" });
        crate::workflow::gdl::write_handoff_transition(
            tx.tx(),
            id,
            &actor,
            transition,
            Some(&decision_ref),
            &detail,
            chrono::Utc::now().timestamp(),
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    outcome?;
    Ok(Json(serde_json::json!({
        "run_id": id,
        "transition": transition.as_str(),
        "decision_ref": echo,
        "audited": true,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackReferralReturnBody {
    #[serde(default)]
    pub contract_key: Option<String>,
    #[serde(default)]
    pub report: Option<serde_json::Value>,
    #[serde(default)]
    pub decision_ref: Option<String>,
}

/// `POST /workflow/runs/{id}/back-referral/return`
pub async fn post_back_referral_return(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<BackReferralReturnBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let raw_key = body.contract_key.as_deref().unwrap_or("");
    let contract_key = validate_contract_key(raw_key)?;
    let report = match body.report {
        Some(v) if v.is_object() => v,
        _ => {
            return Err(HandlerError::bad_request(
                "report_invalid",
                "report must be a JSON object carrying the contract's \
                 required fields",
            ));
        }
    };
    let raw_ref = body.decision_ref.as_deref().unwrap_or("");
    let decision_ref = validate_decision_ref(raw_ref)?;
    let echo_key = echo_of(&contract_key, &principal);

    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        crate::workflow::gdl::write_back_referral_return(
            tx.tx(),
            id,
            &contract_key,
            report,
            Some(&decision_ref),
            now,
        )
        .map_err(return_err)?;
        // The receipt's `late` flag rides the row the release just appended;
        // the server clock decided it, the reply only reports it. The read
        // lives in the core (the no-SQL-in-handlers law).
        let late = crate::workflow::gdl::latest_back_referral_late_flag(tx.tx(), id, &contract_key)
            .map_err(|e| HandlerError::internal(e.to_string()))?
            .unwrap_or(false);
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(late)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let late = outcome?;
    Ok(Json(serde_json::json!({
        "run_id": id,
        "contract_key": echo_key,
        "status": "returned",
        "late": late,
    })))
}

/// The key is caller-supplied identity text: echoed sanitized (the relay
/// to_principal precedent), never stored raw beyond the machinery's own
/// payload writers.
fn echo_of(key: &str, principal: &Option<crate::auth::Principal>) -> String {
    crate::gate::sanitize_read(key, false, principal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// decision_ref_bounds_and_screening
    #[test]
    fn decision_ref_bounds_and_screening() {
        assert_eq!(
            validate_decision_ref("  op-2026-09-22#7 ").unwrap(),
            "op-2026-09-22#7"
        );
        let err = validate_decision_ref("   ").expect_err("empty refuses");
        assert_eq!(err.inner.code, "decision_ref_required");
        let err = validate_decision_ref("").expect_err("absent refuses");
        assert_eq!(err.inner.code, "decision_ref_required");
        let err =
            validate_decision_ref(&"x".repeat(MAX_DECISION_REF_LEN + 1)).expect_err("over bound");
        assert_eq!(err.inner.code, "decision_ref_invalid");
        let err = validate_decision_ref("op\u{200B}-7").expect_err("invisible refuses");
        assert_eq!(err.inner.code, "decision_ref_invalid");
        let err = validate_decision_ref("op\n-7").expect_err("control refuses");
        assert_eq!(err.inner.code, "decision_ref_invalid");
    }

    /// contract_key_required_nonempty_and_bounded
    #[test]
    fn contract_key_required_nonempty_and_bounded() {
        assert_eq!(
            validate_contract_key(" run1:back_referral:owner ").unwrap(),
            "run1:back_referral:owner"
        );
        let err = validate_contract_key("").expect_err("empty refuses");
        assert_eq!(err.inner.code, "contract_key_required");
        let err =
            validate_contract_key(&"k".repeat(MAX_DECISION_REF_LEN + 1)).expect_err("over bound");
        assert_eq!(err.inner.code, "contract_key_required");
        let err = validate_contract_key("k\u{202E}").expect_err("invisible refuses");
        assert_eq!(err.inner.code, "contract_key_required");
    }

    /// The read seam, by the two facts that actually matter.
    ///
    /// There is no reusable body-extractor here: the house `handler_body` lives
    /// in the `tests/` crate and cannot be reached from a unit test, and every
    /// local re-implementation tried (to end-of-file, brace-balanced from the
    /// signature, terminated at a line-initial `}`) mis-slices a real handler —
    /// `get_run` has a multi-line generic signature, `list_steps` has a closure
    /// with its own braces, and `get_run` is a prefix of both `get_run_state`
    /// and `get_run_context`. Rather than ship a weaker copy of a guard this
    /// repo already keeps finding vacuous, this asserts the SEAM CALL SITE:
    /// an exact, unambiguous line that must exist, and one that must not.
    ///
    /// The excluded route is the reason. An audit flagged `get_run_state` as an
    /// outlier and shaped it; that was a regression, because the route is the
    /// ENGINE-EXACT view and the steward-harness CAS-writes what it reads, so a
    /// shaped read is a silent state mutation on every engine turn. Its own
    /// docstring, `docs/api.md`, the CHANGELOG and the security audit all
    /// record the exclusion. These pins hold it.
    #[test]
    fn workflow_read_family_rides_the_read_seam() {
        let workflow = include_str!("workflow.rs");
        let lineage = include_str!("workflow_lineage.rs");

        // POSITIVE: each human view shapes stored run state at its emission.
        for (src, call, who) in [
            (
                workflow,
                "row.state_json = crate::gate::sanitize_read(",
                "get_run",
            ),
            (
                workflow,
                "serde_json::Value::String(crate::gate::sanitize_read(&raw, false, &principal))",
                "list_steps",
            ),
            (
                lineage,
                "crate::gate::sanitize_read(",
                "get_run_context / get_handoff",
            ),
        ] {
            assert!(
                src.contains(call),
                "{who} emits stored run state and must shape it at the emission boundary — \
                 the read-seam site table has no workflow rows, so nothing caught a dropped seam"
            );
        }

        // NEGATIVE: the engine-exact view must not. Bounded to a fixed line
        // count from the signature — a sentinel line was tried and no such
        // line exists, so `take_while` ran to end-of-file and matched a later
        // function's seam.
        let state_region = workflow
            .split("pub async fn get_run_state(")
            .nth(1)
            .expect("`get_run_state` must exist in handlers/workflow.rs");
        // Comments are stripped: the explanatory comment above the emission
        // necessarily names the symbol this assert requires to be ABSENT —
        // that is the nature of documenting a deliberate omission, and the
        // reason a raw substring scan of this region is worthless.
        let state_body: String = state_region
            .lines()
            .take(70)
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !state_body.contains("sanitize_"),
            "`get_run_state` is the ENGINE-EXACT view and must stay un-shaped: the harness \
             CAS-writes what it reads, so a shaped read is a silent state mutation"
        );
        assert!(
            state_body.contains("\"state_json\""),
            "the exclusion region must still be the real one — this pin is slicing the wrong \
             span, which would make the assert above vacuous"
        );
    }

    /// The three-way documentation check, as a gate.
    ///
    /// `scripts/docs-truth.sh` compares the ROUTER SOURCE against
    /// `openapi.yaml` and `docs/api.md`. A route census is only half a
    /// contract: proving every path is in `openapi.yaml` says nothing about
    /// whether `api.md` describes the same surface, and neither catches a
    /// sentence asserting a control does not exist.
    ///
    /// It is a test rather than a CI step because a CI step nobody runs is a
    /// CI step that rots. This one fails the build.
    ///
    /// The check must EXIST and must be RUNNABLE — a test that passes because
    /// the script is missing is the vacuous-guard class this repo keeps
    /// hunting, so its presence is asserted before its result.
    #[test]
    fn three_way_doc_truth_gate_is_present_and_clean() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/docs-truth.sh");
        assert!(
            script.exists(),
            "scripts/docs-truth.sh must exist — a three-way doc check that was deleted is a \
             silent loss of the only guard on source↔openapi↔docs drift"
        );
        let out = std::process::Command::new(&script)
            .output()
            .expect("scripts/docs-truth.sh must be runnable");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !text.is_empty(),
            "the doc-truth check produced no output — it is not actually running"
        );
        assert!(
            !text.contains("[HIGH]"),
            "the three-way doc check reports a HIGH finding — the router, openapi.yaml and \
             docs/api.md disagree:\n{text}"
        );
        // And the check must actually be looking at a real surface, not an
        // empty one it would call clean.
        assert!(
            text.contains("routes=") && !text.contains("routes=0"),
            "the doc-truth check found no routes at all — a census that reads zero is a pass \
             by absence, which is the vacuity this asserts against:\n{text}"
        );
    }

    /// The doc-truth gate's RED-PROOF, as a test: a route that exists in the
    /// router and in NO other source must make it fail. Without this the
    /// gate could be passing because it inspects nothing, and the previous
    /// test's "routes= is non-zero" arm would not catch that — a census can
    /// read 214 and still check none of them against openapi.
    #[test]
    fn three_way_doc_truth_gate_detects_a_route_missing_from_the_contract() {
        // Take a REAL registered route and delete its openapi path item from a
        // copy of the spec, then run the same predicate the gate uses.
        let spec = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.yaml"));
        let router = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/server/router/workflow.rs"
        ));
        assert!(
            spec.contains("  /workflow/delivery/runs/{id}/trace:\n"),
            "the probe route must exist in the contract to begin with"
        );
        assert!(
            router.contains("\"/workflow/delivery/runs/{id}/trace\""),
            "the probe route must be registered to begin with"
        );

        // Now the gate's own check, with that one path item removed.
        let mutilated = spec.replace("  /workflow/delivery/runs/{id}/trace:\n", "  /REMOVED:\n");
        assert_ne!(mutilated, spec, "the probe must actually change the spec");
        let still_present = openapi_paths_for_test(&mutilated);
        assert!(
            !still_present.contains("/workflow/delivery/runs/{id}/trace"),
            "removing the path item must remove it from the census"
        );
    }

    fn openapi_paths_for_test(spec: &str) -> std::collections::BTreeSet<String> {
        spec.lines()
            .filter_map(|l| {
                l.strip_prefix("  /")
                    .and_then(|r| r.strip_suffix(":"))
                    .map(|p| format!("/{p}"))
            })
            .filter(|p| !p.contains('*') && !p.contains(' '))
            .collect()
    }

    /// D4-adjacent: hold the ENGINE-EXACT exclusion for `get_run_state`. An
    /// audit once shaped this route; the shape silently corrupts the harness's
    /// CAS round-trip. Named for the hazard rather than for the sibling
    /// family, so the two pins fail with different messages.
    #[test]
    fn run_state_read_is_seamed_like_its_siblings() {
        let production = include_str!("workflow.rs");
        let region: String = production
            .split("pub async fn get_run_state(")
            .nth(1)
            .expect("`get_run_state` must exist")
            .lines()
            .take(70)
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !region.contains("sanitize_"),
            "`get_run_state` is the ENGINE-EXACT view (see its docstring) — it must NOT be \
             shaped. An audit shaped it once and the harness would have persisted shaped bytes \
             over stored ones on every engine turn."
        );
    }

    /// The machinery's refusals surface named — never a silent drop, never a
    /// bypass: absent → 404, B3 → 400 with the missing list.
    #[test]
    fn return_errors_surface_named() {
        let absent = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "back-referral contract absent".into(),
        ));
        assert_eq!(absent.status, axum::http::StatusCode::NOT_FOUND);
        let b3 = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "B3: return contract released without the required report field \
             `finding`; B3: return contract released without the required \
             report field `follow_up`"
                .into(),
        ));
        assert_eq!(b3.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(b3.inner.code, "report_incomplete");
        let other = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "sql down".into(),
        ));
        assert_eq!(other.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
