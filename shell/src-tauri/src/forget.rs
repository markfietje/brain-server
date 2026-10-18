//! II-7b — `POST /ump/forget`: the soft path only, and the refusal that is the
//! boundary.
//!
//! Division of labour is `recall.rs`'s, unchanged: this file owns BOUNDS and the
//! classification of what the kernel said. It owns no transport, and it can
//! neither hold nor fabricate a principal.
//!
//! ## THE AUTHZ SURFACE THIS OPENS, and what a caller can now do that it could not
//!
//! `/recall` and `/verify` were already reachable, so this opens no new REACH.
//! It opens a new KIND of act: the first command in the shell that WRITES, and
//! the first one whose consequence is an erasure.
//!
//! The kernel's route is doubly gated (`handlers/ump_ops.rs::forget`): Write
//! scope, the `write` capability, and the `ump.forget` TOOL grant — and then, for
//! `hard: true` only, three more gates (`cap_gate("admin")`, `Action::Admin`,
//! and the `purge` ROLE). `hard: true` is therefore not a flag; it is the
//! `/purge` destructive act wearing a different route's body.
//!
//! What a caller can now do that it could not before: flag a row for exclusion
//! and write a tombstone row naming it. The row is NOT erased — it is
//! quarantined-style `flagged = 1` plus a tombstone plus an audit row, all in
//! the caller's transaction (`service/ump_ops::forget_soft`). That is
//! recoverable in a way `purge_chunk_ids` is not, and the difference is the
//! whole reason the soft path is the one exposed here.
//!
//! ## WHY `hard` IS A REFUSAL AND NOT A BOOLEAN PARAMETER
//!
//! The obvious design is a `hard: bool` field on the request with a bound.
//! That design is wrong here, and it is wrong in a way a bounds table cannot
//! fix: a boolean is a value a caller can flip, and the kernel's dangerous arm
//! is one boolean away. Every unit of authority the shell does hold (the
//! loopback bearer) is enough to reach the soft path and NOT enough to reach
//! the hard path, and a `hard: bool` would put that asymmetry one careless
//! default away from gone.
//!
//! So `hard` is not a parameter. The request type declares the field ONLY to
//! refuse it, `Option<Value>` so that ANY value can be named in the refusal
//! rather than only a well-typed `true`:
//!
//! * absent, or `false` — accepted, and the core still NEVER emits the field.
//!   The kernel's `#[serde(default)] pub hard: bool` supplies `false`, so
//!   omission and explicit-false are the same wire and there is nothing to
//!   authorize.
//! * `null` — accepted. Serde collapses an explicit JSON `null` into `None` for
//!   an `Option` field, so on the wire `{"hard": null}` IS the absent case; an
//!   explicit `false` by contrast stays `Some(Bool(false))`. Both were MEASURED
//!   rather than assumed, and they are pins, because "one collapse for all soft
//!   spellings" is not what serde does and an arm asserting it would have
//!   passed against a core that refused explicit-false — the over-refusal this
//!   module argues against. `Some(Value::Null)` is accepted so a
//!   directly-constructed value behaves like the spelling it imitates; `null`
//!   means "no value" in JSON and cannot mean `true`.
//! * anything else — REFUSED, with the received value in the refusal so the
//!   attempt is legible rather than silent.
//!
//! `HardRefused`'s `Display` names the reason (Admin scope AND the `purge` role,
//! and a capability bearer can never reach it). That sentence is the
//! deliverable: a refusal that says "invalid request" would leave the next
//! reader to guess whether the value was refused for shape or for authority,
//! and the second reading is the one that matters.
//!
//! ## WHY THERE IS NO `reason` FIELD
//!
//! The kernel accepts one (`ForgetRequest.reason`, defaulting to
//! `"ump_forget"`), and `forget_soft` writes it VERBATIM into
//! `tombstones.reason` with no ceiling of its own. So exposing it would mean
//! letting a caller author unbounded free text into a persisted row, and
//! bounding it here would mean CHOOSING a ceiling the kernel does not state —
//! the exact move this phase's bounds law forbids ("the kernel's own values,
//! read from `openapi.yaml` and the handler, never chosen here").
//!
//! Nothing is lost: the audit row the soft path writes carries the
//! `ump-forget-soft` detail regardless of what the caller said, and the human
//! gate is the receipt, not a caller-supplied string. A caller that sends
//! `reason` is refused by `deny_unknown_fields`, which NAMES the field — so the
//! omission is visible rather than silent.
//!
//! ## WHY `id` IS BOUNDED ONLY BY "CAN IT EVER RESOLVE"
//!
//! `resolve_row_id` (`handlers/ump_ops.rs:170-183`) resolves an id three ways:
//! as an `i64`, as a stored `urn:ump:…`, or as any string whose `:`-suffix
//! parses as an `i64`. Anything else is a 404. Inventing a narrower rule here —
//! say, accepting only the two DOCUMENTED forms — would refuse ids the kernel
//! serves, and the prompt's rule is refusal-not-CLAMP: never widen, and never
//! narrow into a fiction.
//!
//! So the core refuses exactly what cannot resolve under ANY of the three:
//! an id that is absent, or blank once trimmed. Resolution against the store is
//! the kernel's 404, and the projector names that as its own state rather than
//! pretending the core checked.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::recall::{Refused, Unreachable, WireExchange};

/// What the caller asks for.
///
/// `deny_unknown_fields`, for `RecallQuery`'s reason: a silently-dropped field
/// is how a synthesized principal rides into production looking like the real
/// one. `id` is REQUIRED and absent-from-`Option` distinction matters, so it is
/// `Option<String>`: `None` is a missing id and `Some("")` is a malformed one,
/// and those are different operator mistakes.
///
/// `hard` is declared ONLY to refuse it. See the module header: a boolean a
/// caller can flip is not a boundary. `Value` rather than `bool` so the refusal
/// can NAME what arrived — `"true"` as a string, `1`, or `null` are all refused
/// with the received value quoted back, rather than failing as a serde parse
/// error that names nothing.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgetRequest {
    #[serde(default)]
    pub id: Option<String>,
    /// Never honored. Present in the type solely so its presence can be refused
    /// with a reason; see the module header.
    #[serde(default)]
    pub hard: Option<Value>,
}

/// The exact body the core authorizes: `{id}` and nothing else.
///
/// `hard` is structurally absent rather than set to `false`. Both reach the same
/// kernel behaviour (`#[serde(default)] pub hard: bool`), but an absent field
/// is the one a reader can prove: there is no line here that a later edit turns
/// into `true`, because there is no line here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForgetDoc {
    pub id: String,
}

/// A refusal to mint. Every arm is loud, and the `hard` arm carries the reason
/// in its `Display` because the reason is the deliverable.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum ForgetRefusal {
    /// The `id` field was absent entirely.
    IdMissing,
    /// Present, but blank once trimmed — a string `resolve_row_id` cannot
    /// resolve under any of its three rules.
    IdMalformed { id: String },
    /// The destructive path was asked for. `requested` is the received value
    /// rendered as JSON, so the refusal names what arrived.
    HardRefused { requested: String },
}

impl core::fmt::Display for ForgetRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ForgetRefusal::IdMissing => write!(
                f,
                "no id: /ump/forget needs the numeric row id or the urn:ump:… content id"
            ),
            ForgetRefusal::IdMalformed { id } => write!(
                f,
                "id {id:?} is blank, which resolve_row_id cannot resolve under any of its \
                 three rules"
            ),
            ForgetRefusal::HardRefused { requested } => write!(
                f,
                "hard: {requested} is REFUSED, and this command exposes no other arm. \
                 hard:true IS the /purge destructive act: the kernel gates it on Admin scope \
                 AND the purge role (cap_gate admin, Action::Admin, authorize_role purge), \
                 and a capability bearer can never reach it. This shell holds Write scope \
                 only, so it authorizes the soft tombstone path and refuses the erase path — \
                 the gap is the boundary, not an oversight."
            ),
        }
    }
}

/// The authorized request. There is no `hard` field to echo and no `reason`
/// field to echo, because there is no `hard` field and no `reason` field.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthorizedForget {
    pub doc: ForgetDoc,
}

#[tauri::command]
pub fn forget_memory(request: ForgetRequest) -> Result<AuthorizedForget, ForgetRefusal> {
    authorize(request)
}

/// The bounds law, as a pure function so it is pinnable without a kernel.
pub fn authorize(request: ForgetRequest) -> Result<AuthorizedForget, ForgetRefusal> {
    // `hard` is checked FIRST, and the order is load-bearing rather than
    // incidental: a caller who asked for the destructive path should be told so
    // even if their id was also malformed. Answering `id_malformed` to a
    // `hard: true` request would hide the one refusal that matters behind an
    // unrelated one — the same shape as a 403 that leaks nothing, applied to a
    // refusal that should leak nothing but must still be the loud one.
    if let Some(value) = request.hard.as_ref() {
        // `null` is soft because it means "no value" in JSON and cannot mean
        // `true`; see the module header for what serde does with each spelling.
        let is_soft = matches!(value, Value::Bool(false) | Value::Null);
        if !is_soft {
            return Err(ForgetRefusal::HardRefused {
                requested: value.to_string(),
            });
        }
    }
    let id = match request.id {
        None => return Err(ForgetRefusal::IdMissing),
        Some(id) => id,
    };
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(ForgetRefusal::IdMalformed { id });
    }
    Ok(AuthorizedForget {
        doc: ForgetDoc {
            id: trimmed.to_string(),
        },
    })
}

// ─────────────────────────────────────────────────────────────────────
// The projector: what the kernel said
// ─────────────────────────────────────────────────────────────────────

/// The kernel's `result` string, verbatim.
///
/// `openapi.yaml`'s `/ump/forget` documents `{result: "tombstoned"}` and the
/// handler returns `"erased"` on the hard path and `"tombstoned"` on the soft
/// one (`handlers/ump_ops.rs:844`). The soft path is the only path this core
/// authorizes, so `Erased` is not an outcome — it is a CONTRACT VIOLATION, and
/// `shape_forget` treats it as one. See the arm's own comment.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetResult {
    Tombstoned,
    Erased,
}

/// The kernel's success body. `deny_unknown_fields`, so a body carrying a
/// certificate id (the fabricated one `forget.ts` exists to prevent) or any
/// other invented field is a contract violation rather than a receipt source.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireForget {
    pub result: ForgetResult,
}

/// What the kernel confirmed.
///
/// `result` is carried faithfully rather than assumed, and `chain_head` is
/// deliberately ABSENT — the route emits none, and inventing one is the same
/// defect as inventing an owner (`recall.rs`'s header). The chain row exists and
/// is read through `/ump/audit`; this response is not where it lives.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tombstoned {
    pub id: String,
    pub result: ForgetResult,
    pub dsar_certificate: Option<String>,
}

/// The kernel would not serve this row.
///
/// NAMED FOR WHAT THE KERNEL SAID, which `openapi.yaml` states as two things at
/// once: "missing id, or a target row the caller's record gate does not admit".
/// The kernel deliberately conflates them, so this arm does not un-conflate
/// them, and it carries no field from which a caller could learn that a row
/// EXISTS but is not theirs — that would be the enumeration channel the
/// target-row gate exists to close.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NoSuchRow {
    pub id: String,
    pub status: u16,
}

/// The core's classification of one `/ump/forget` exchange.
///
/// The law, which is II-6's `forget.ts` taxonomy carried down into the core:
///
/// * `Tombstoned` — the kernel flagged the row, wrote the tombstone and the
///   audit row. A RESULT, not a success/failure pair: the receipt says what
///   happened and claims nothing beyond it.
/// * `Refused` — 401/403, INCLUDING the destructive path's refusals and the
///   legal-hold 409. A refusal is a result: the row was NOT erased, and saying
///   so is the whole point.
/// * `NoSuchRow` — 404, the kernel's blur of absent and not-admitted.
/// * `Unavailable` — any other non-2xx, plus an unparseable 2xx. Erasure
///   degraded, never silently not-done.
/// * `ContractViolation` — a 2xx that is not the published contract, or that
///   reports an ERASE. The last is the backstop for this whole module: a
///   `hard: true` that somehow reached the kernel would come back `erased`,
///   and this arm refuses to present it as a tombstone. The refusal vocabulary
///   is the boundary; this is the arm that notices if the vocabulary is
///   bypassed.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ForgetOutcome {
    Tombstoned(Tombstoned),
    Refused(Refused),
    NoSuchRow(NoSuchRow),
    Unavailable(Unreachable),
    ContractViolation(Unreachable),
}

impl ForgetOutcome {
    /// The one question a renderer must be able to ask without inspecting
    /// fields: did a row get tombstoned? Deliberately NOT "was it refused?" —
    /// a refusal and a completed tombstone are different facts about the
    /// operator's data.
    pub fn is_tombstoned(&self) -> bool {
        matches!(self, ForgetOutcome::Tombstoned(_))
    }

    /// True for the states that must never be drawn as "forgotten". A refusal
    /// is not a tombstone: the row is still there and still retrievable.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            ForgetOutcome::Refused(_) | ForgetOutcome::NoSuchRow(_)
        )
    }

    pub fn result(&self) -> Option<ForgetResult> {
        match self {
            ForgetOutcome::Tombstoned(t) => Some(t.result),
            _ => None,
        }
    }

    /// The wire token this arm serializes under. Tied to the serialization by
    /// `forget_tests.rs`, so it cannot drift from the serde tag without the pin
    /// going red — see `verify.rs`'s `state_token` for the same arrangement.
    pub fn state_token(&self) -> &'static str {
        match self {
            ForgetOutcome::Tombstoned(_) => "tombstoned",
            ForgetOutcome::Refused(_) => "refused",
            ForgetOutcome::NoSuchRow(_) => "no_such_row",
            ForgetOutcome::Unavailable(_) => "unavailable",
            ForgetOutcome::ContractViolation(_) => "contract_violation",
        }
    }
}

/// The IPC surface of [`shape_forget`].
///
/// The caller hands the authorized doc BACK, exactly as the other two commands
/// do, so the same hole exists and is closed the same way: re-authorize before
/// trusting the echo. That re-authorization is ALSO the `hard` backstop at the
/// IPC seam — a caller that hands back a doc it was never granted cannot smuggle
/// an erase through the echo, because there is no field to smuggle it in.
#[tauri::command]
pub fn shape_forget_command(
    exchange: WireExchange,
    requested: ForgetDoc,
) -> Result<ForgetOutcome, ForgetRefusal> {
    authorize(ForgetRequest {
        id: Some(requested.id.clone()),
        hard: None,
    })?;
    Ok(shape_forget(exchange, &requested))
}

/// Reads one exchange and classifies it. Pure: no I/O, so every arm is pinnable
/// without a running kernel.
pub fn shape_forget(exchange: WireExchange, requested: &ForgetDoc) -> ForgetOutcome {
    match exchange.status {
        401 | 403 => {
            return ForgetOutcome::Refused(Refused {
                status: exchange.status,
                scope: "ump.forget",
            });
        }
        404 => {
            // NOT a tombstone, and NOT "it was already gone". `openapi.yaml`
            // states this status covers a missing id AND a target row the
            // caller's record gate does not admit; the kernel returns one 404
            // for both. Un-blurring it would rebuild the enumeration channel
            // the target-row gate closes.
            return ForgetOutcome::NoSuchRow(NoSuchRow {
                id: requested.id.clone(),
                status: exchange.status,
            });
        }
        status if !(200..300).contains(&status) => {
            // 409 (legal hold) lands here too, and that is CORRECT: a legal hold
            // is a refusal of the erasure, and the operator's next action
            // ("release the hold, then retry") is not "retry harder". It is
            // `Unavailable` rather than `Refused` because it is the kernel's
            // state machine declining rather than a permission decision — and
            // the register records which of the two a reader is looking at.
            return ForgetOutcome::Unavailable(Unreachable {
                status,
                detail: "the kernel answered with a non-success status".to_string(),
            });
        }
        _ => {}
    }

    let wire: WireForget = match serde_json::from_str(&exchange.body) {
        Ok(wire) => wire,
        Err(error) => {
            return ForgetOutcome::ContractViolation(Unreachable {
                status: exchange.status,
                detail: format!("the forget body is not the published contract: {error}"),
            });
        }
    };

    // THE BACKSTOP. `erased` on a request this core authorized soft is a
    // contradiction with a cause, and the only cause that matters is that a
    // `hard: true` reached the kernel somehow. Presenting it as a tombstone
    // would tell the operator "flagged, still retrievable" about a row that has
    // been destroyed — so it is refused, loudly, and never rendered.
    if wire.result != ForgetResult::Tombstoned {
        return ForgetOutcome::ContractViolation(Unreachable {
            status: exchange.status,
            detail: format!(
                "the kernel reported `{}` for a request authorized as a SOFT tombstone. The \
                 erase path is the /purge act and this core does not authorize it; treat this \
                 as a contract violation and do not report a tombstone.",
                match wire.result {
                    ForgetResult::Tombstoned => "tombstoned",
                    ForgetResult::Erased => "erased",
                }
            ),
        });
    }

    ForgetOutcome::Tombstoned(Tombstoned {
        id: requested.id.clone(),
        result: wire.result,
        // Always None, always named. `/ump/forget` emits no certificate;
        // certificates belong to the Admin-only, subject-addressed `/dsar`
        // workflow. This field exists so a receipt cannot be built from this
        // outcome by omission — see `forget.ts`'s `certificateAbsentBecause`.
        dsar_certificate: None,
    })
}
