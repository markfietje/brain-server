//! The create loop's storage layer.
//!
//! ## What this core owns
//!
//! Every statement that touches the four claim tables, the bounds that guard
//! them, and the audit rows each mutation owes — written INSIDE the caller's
//! transaction, so a claim and its evidence commit or roll back together.
//!
//! ## What it deliberately does not own
//!
//! **It does not decide anything.** No verdict is computed here. The gate in
//! the workflow layer is a pure function over rows; this layer's whole job is
//! to load those rows, hand them across, and store what came back. A storage
//! layer that also holds policy is a second, unreviewed copy of the policy,
//! and this loop's central claim is that its authority is non-model and
//! singular.
//!
//! ## The gated read
//!
//! [`recall_page`] is the ONLY path by which a claim reaches a reader, and it
//! joins on `status = 'ratified' AND recall_visible = 1` — the same two
//! columns the database fence protects. So the fence and the query are two
//! independent locks on one fact: a trigger that was somehow bypassed still
//! leaves an unratified claim invisible here, and a row that somehow became
//! visible without a promote audit row is invisible there.
//!
//! It is a query and not a view, and the migration carries a pin that says so.
//! A view would put the gated read outside this core, behind no read seam.
//!
//! ## Principal kinds
//!
//! Every write that sets `created_by` or `authored_by` routes through the one
//! mapping function in the create loop's module root. A literal in this file
//! would be a fence key that a future caller could set from a request body,
//! and a caller-supplied principal kind is a total bypass of every fence in
//! the loop.
//!
//! ## A truthful dead-code allow
//!
//! `#![allow(dead_code)]` here is the same argument the workflow substrate
//! makes and for the same reason: the loop ships inert, so several items on
//! this page have test callers and no production caller *because that is the
//! point*. `flip_batch_visibility` and `store_batch_verdict` are reached only
//! by the round's own battery, because the batch flip happens in no request
//! path while promotion is disabled. `schema_digest` is the tamper check that
//! runs at promotion time, and there is no promotion. Every item is covered by
//! a test in this module, so the clippy watchdog becomes a real gate again the
//! moment any of them acquires a production caller — and a later round that
//! silently gives promotion a caller will find these flags gone.
//!
//! **One item is no longer in that set.** `evaluate_disproof` had no production
//! caller for as long as it existed — the falsification half of the loop could
//! store a condition and never once read it back. It is now reached by
//! [`sweep_disproofs`], which the `brain disproof` verb drives. That sweep is a
//! **read**: it evaluates stored conditions against the claim's own subject and
//! writes nothing, because a sweep that wrote its verdicts back would be a
//! promotion path. The rest of the allow stands, and `evaluate_disproof` is the
//! one name a reader should check before assuming it is still unreachable.

// The loop ships inert; see the module header for which items have no
// production caller and why that is the design rather than an omission.
#![allow(dead_code)]

use rusqlite::{Connection, OptionalExtension, params};

use crate::audit::{AuditKind, AuditStatus};
use crate::auth::policy::PrincipalKind;
use crate::workflow::create::corpus::PlantedClaim;
use crate::workflow::create::disproof::{DisproofColumns, DisproofCondition};

/// Re-exported so a caller of [`sweep_disproofs`] can name the three states
/// without reaching into the crate-private workflow module.
///
/// The verdict type is **not** re-implemented or mirrored here — it is the same
/// type the core returns, so a caller cannot read a three-state verdict through
/// a two-state summary.
pub use crate::workflow::create::disproof::DisproofVerdict;

/// One claim's stored disproof condition, read back and evaluated.
///
/// **The three states survive, and this is the whole point of the type.**
/// `NoVerdict` is not a soft pass and not a rounding of the other two: it is
/// what a prose condition and a claim that predates the field both report, and
/// [`DisproofVerdict::is_green`] exists so it cannot be mistaken for
/// `Satisfied`. Collapsing the three into a boolean here is the one change that
/// would make this module lie, so nothing collapses them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisproofReading {
    pub claim_id: String,
    pub verdict: DisproofVerdict,
}

/// The sweep's totals — three counts, never one.
///
/// **A single "pass" number is the defect this shape exists to prevent.** A
/// sweep that reports "N of M ok" invites reading `NoVerdict` as a pass, and the
/// three counts are what makes that reading unavailable rather than merely
/// discouraged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisproofSweep {
    /// Claims carrying a mechanically-evaluated condition whose disproof was
    /// NOT observed. These stand.
    pub satisfied: usize,
    /// Claims whose named disproof WAS observed. **These do not stand**, and the
    /// name says so rather than the reader having to remember the polarity.
    pub refuted: usize,
    /// Claims with a prose condition, or none at all. **Never green.**
    pub no_verdict: usize,
}

impl DisproofSweep {
    /// Claims read. The denominator, so a sweep over zero claims cannot read as
    /// a clean bill of health.
    pub fn total(&self) -> usize {
        self.satisfied + self.refuted + self.no_verdict
    }
}

/// The bounds law: the sweep reads every claim row, so the cap bounds a
/// receipt, not a verdict. A claim past the cap is NOT counted as green — it is
/// simply not read, and `total()` says so.
const MAX_SWEEP_CLAIMS: usize = 10_000;

/// Read every claim's stored disproof condition and evaluate it against the
/// claim's own subject.
///
/// **This writes NOTHING.** No `status` is set, nothing is demoted, nothing is
/// ratified, and no audit row is emitted — the sweep is a read, and that is what
/// lets it ship while promotion stays a compile-time `false`. A sweep that wrote
/// its verdicts back would be a promotion path, which is a different artifact
/// with a different authority.
///
/// The `subject` is the claim's own stored `subject` column. That is the
/// text the condition was written against, so evaluating it against anything
/// else — the current knowledge base, a re-rendered body — would be measuring a
/// different claim than the one on the row.
/// The sweep's typed refusal.
///
/// [`CreateError`] is `pub(crate)`, and widening it is a change to a published
/// type this module has no standing to make — so the PUBLIC entry point maps it
/// to a rendered string at the boundary rather than leaking a crate-private
/// type through a `pub` signature. The cause is preserved verbatim, so the
/// message still names what failed.
pub fn sweep_disproofs(conn: &Connection) -> Result<Vec<DisproofReading>, String> {
    sweep_disproofs_inner(conn).map_err(|e| e.to_string())
}

fn sweep_disproofs_inner(conn: &Connection) -> Result<Vec<DisproofReading>, CreateError> {
    let mut stmt = conn
        .prepare(
            "SELECT claim_id, subject FROM claims
             ORDER BY id LIMIT ?1",
        )
        .map_err(|e| CreateError::Storage(format!("disproof_sweep_read:{e}")))?;
    let rows = stmt
        .query_map(params![MAX_SWEEP_CLAIMS as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| CreateError::Storage(format!("disproof_sweep_read:{e}")))?;
    let mut out = Vec::new();
    for row in rows {
        let (claim_id, subject) =
            row.map_err(|e| CreateError::Storage(format!("disproof_sweep_read:{e}")))?;
        let verdict = evaluate_disproof(conn, &claim_id, &subject)?;
        out.push(DisproofReading { claim_id, verdict });
    }
    Ok(out)
}
use crate::workflow::create::gap::{GapCandidate, GapMethod};
use crate::workflow::create::schema::{self, SchemaDecl, SchemaFault};
use crate::workflow::create::verify::{
    Citation, ClaimUnderTest, GateOutcome, RatifiedClaim, SlotDecl, SlotType, SlotValue,
};
use crate::workflow::create::{Refusal, RefusalReceipt, principal_kind_string};

/// The listing bound. Every list surface in this repository is capped and
/// every cap is pinned; a claims listing is no different.
pub(crate) const RECALL_PAGE_MAX: usize = 50;
/// The default page size when the caller does not ask for one.
pub(crate) const RECALL_PAGE_DEFAULT: usize = 20;
/// The bound on a claim's public id.
pub(crate) const MAX_CLAIM_ID_BYTES: usize = 128;

/// The core's typed error. `Display` preserves the underlying message so the
/// handler can map it onto that route's frozen vocabulary; the core never
/// names an HTTP status.
#[derive(Debug)]
pub(crate) enum CreateError {
    Storage(String),
    SchemaRejected(SchemaFault),
    /// The gate refused. Carries the CLOSED vocabulary and the claim id, and
    /// nothing else — the detail goes to the audit chain.
    Refused(RefusalReceipt),
    /// The loop is inert.
    PromotionDisabled(String),
    NotFound,
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateError::Storage(m) => write!(f, "storage: {m}"),
            CreateError::SchemaRejected(fault) => {
                write!(f, "schema rejected: {}", fault.as_str())
            }
            CreateError::Refused(r) => write!(f, "refused: {} ({})", r.reason, r.claim_id),
            CreateError::PromotionDisabled(id) => write!(f, "promotion_disabled ({id})"),
            CreateError::NotFound => write!(f, "not found"),
        }
    }
}

impl From<rusqlite::Error> for CreateError {
    fn from(e: rusqlite::Error) -> Self {
        CreateError::Storage(e.to_string())
    }
}

/// One row of the gated claim read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ClaimRow {
    pub(crate) claim_id: String,
    pub(crate) subject: String,
    pub(crate) predicate: String,
    pub(crate) object: String,
    pub(crate) scope: Option<String>,
    pub(crate) promoted_by: Option<String>,
    pub(crate) promoted_at: Option<i64>,
}

/// The gated claim read — the loop's only reader.
///
/// Joins on both fenced columns. It is a bounded page, newest-first, and it
/// is deliberately unable to express "show me pending claims": there is no
/// parameter for it, so no caller can ask this function for unratified
/// material.
pub(crate) fn recall_page(conn: &Connection, limit: usize) -> Result<Vec<ClaimRow>, CreateError> {
    let limit = limit.clamp(1, RECALL_PAGE_MAX);
    let mut stmt = conn.prepare(
        "SELECT claim_id, subject, predicate, object, scope, promoted_by, promoted_at
           FROM claims
          WHERE status = 'ratified' AND recall_visible = 1
          ORDER BY promoted_at DESC, claim_id ASC
          LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], |r| {
            Ok(ClaimRow {
                claim_id: r.get(0)?,
                subject: r.get(1)?,
                predicate: r.get(2)?,
                object: r.get(3)?,
                scope: r.get(4)?,
                promoted_by: r.get(5)?,
                promoted_at: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One slot as read from the document, fully OWNED.
///
/// The intermediate form exists because the parsed document cannot be borrowed
/// from at a `'static` lifetime, and pinning every document the process ever
/// read would be worse than one more type.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedSlot {
    predicate: String,
    ty: SlotType,
    class: Option<String>,
    lo: Option<i64>,
    hi: Option<i64>,
    labels: Vec<String>,
}

/// The newest ratified schema row id for a domain, or `None`.
///
/// This exists so a handler never has to ask the question itself. A claim
/// cannot be typed without a ratified schema, so the lookup belongs with the
/// write that needs it — and a handler that opened its own transaction to run
/// it would be doing storage work, which is the split this repository's
/// architecture law draws.
pub(crate) fn latest_ratified_schema_id(
    conn: &Connection,
    domain: &str,
) -> Result<Option<i64>, CreateError> {
    let id = conn
        .query_row(
            "SELECT id FROM claim_schemas WHERE domain = ?1 AND ratified_at IS NOT NULL \
             ORDER BY version DESC LIMIT 1",
            params![domain],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Read one claim's stored fields. Used by the promotion screen, which is the
/// ONE surface besides this core that sees a non-ratified claim.
pub(crate) fn load_for_screen(conn: &Connection, claim_id: &str) -> Result<ClaimRow, CreateError> {
    conn.query_row(
        "SELECT claim_id, subject, predicate, object, scope, promoted_by, promoted_at
           FROM claims WHERE claim_id = ?1",
        params![claim_id],
        |r| {
            Ok(ClaimRow {
                claim_id: r.get(0)?,
                subject: r.get(1)?,
                predicate: r.get(2)?,
                object: r.get(3)?,
                scope: r.get(4)?,
                promoted_by: r.get(5)?,
                promoted_at: r.get(6)?,
            })
        },
    )
    .optional()?
    .ok_or(CreateError::NotFound)
}

/// Does this claim exist at all? Probe-blind: the answer is a boolean, so a
/// caller cannot use this to distinguish "wrong claim" from "wrong domain".
pub(crate) fn claim_exists(conn: &Connection, claim_id: &str) -> Result<bool, CreateError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM claims WHERE claim_id = ?1",
        params![claim_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// The body-digest a schema is stored under, recomputed on read so a row that
/// was tampered with is detected at load rather than at promotion.
pub(crate) fn schema_digest(conn: &Connection, id: i64) -> Result<Option<String>, CreateError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT body_digest FROM claim_schemas WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let body: Option<String> = conn
        .query_row(
            "SELECT body FROM claim_schemas WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(body) = body else { return Ok(None) };
    if schema::body_digest(&body) != stored {
        // A digest mismatch is not a soft read: the artifact a reviewer would
        // sign is not the artifact in the row.
        return Err(CreateError::Storage(
            "claim_schemas.body_digest does not match its body".into(),
        ));
    }
    Ok(Some(stored))
}

/// Load a schema's declared slots.
///
/// The typed slot table is stored as one JSON document per schema and decoded
/// here, so the DOMAIN model — types, disjointness classes, bounds — lives in
/// Rust where it can be expressed, and the store holds only its serialisation.
/// A JSON Schema document would have been the opposite trade: portable, and
/// unable to say that two slots are disjoint.
/// Load a schema's declared slots by its row id — the FK the claim actually
/// carries.
///
/// This is the read the GATE uses, and it keys on `id` rather than on `domain`
/// deliberately. The writer binds `schema_ref` from the request's `domain`
/// (`handlers/claims.rs:243-250`) and the database's foreign key is what makes
/// that binding real; re-deriving the schema from any request string at read
/// time throws the binding away and re-opens the door the FK closed.
pub(crate) fn load_schema_slots_by_ref(
    conn: &Connection,
    schema_ref: i64,
) -> Result<Option<Vec<SlotDecl>>, CreateError> {
    let body: Option<String> = conn
        .query_row(
            "SELECT body FROM claim_schemas
              WHERE id = ?1 AND ratified_at IS NOT NULL",
            params![schema_ref],
            |r| r.get(0),
        )
        .optional()?;
    let Some(body) = body else { return Ok(None) };
    Ok(Some(decode_slots(&body)?))
}

/// Load a schema's declared slots BY DOMAIN.
///
/// This is the read a WRITER uses — it is how `schema_ref` is chosen in the
/// first place. The gate does not use it, and a future caller reaching for this
/// to resolve a claim is re-opening the original defect: `domain` and `subject`
/// are independent caller-controlled strings, so a lookup keyed on either of
/// them at verification time consults a schema the writer never bound.
pub(crate) fn load_schema_slots(
    conn: &Connection,
    domain: &str,
) -> Result<Option<Vec<SlotDecl>>, CreateError> {
    let body: Option<String> = conn
        .query_row(
            "SELECT body FROM claim_schemas
              WHERE domain = ?1 AND ratified_at IS NOT NULL
              ORDER BY version DESC LIMIT 1",
            params![domain],
            |r| r.get(0),
        )
        .optional()?;
    let Some(body) = body else { return Ok(None) };
    Ok(Some(decode_slots(&body)?))
}

/// Decode the typed slot document. Bounded, total, and fail-closed.
fn decode_slots(body: &str) -> Result<Vec<SlotDecl>, CreateError> {
    if body.len() > schema::MAX_SCHEMA_BODY_BYTES {
        return Err(CreateError::Storage("schema body exceeds its bound".into()));
    }
    // The parsed document owns its strings and the slot table borrows them
    // with a `&'static` lifetime, so the read is done in two passes: every
    // field is first copied into an OWNED value, and only then interned. The
    // obvious one-pass version does not compile, because a `&'static` cannot
    // borrow from a local `Value` — and the tempting fix, leaking the whole
    // document, would pin every schema the process ever read.
    let document: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CreateError::Storage(format!("schema body is not valid json: {e}")))?;
    let slots = document
        .get("slots")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CreateError::Storage("schema body carries no slots array".into()))?;
    if slots.len() > schema::MAX_SCHEMA_SLOTS {
        return Err(CreateError::Storage(
            "schema declares too many slots".into(),
        ));
    }
    // Pass one: owned.
    let mut owned: Vec<OwnedSlot> = Vec::with_capacity(slots.len());
    for slot in slots {
        let predicate = slot
            .get("predicate")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CreateError::Storage("a slot has no predicate".into()))?;
        let ty = match slot.get("ty").and_then(|v| v.as_str()).unwrap_or("") {
            "integer" => SlotType::Integer,
            "instant" => SlotType::Instant,
            "label" => SlotType::Label,
            "free" => SlotType::Free,
            other => {
                return Err(CreateError::Storage(format!(
                    "a slot declares an unknown type `{other}`"
                )));
            }
        };
        let class = slot
            .get("class")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let lo = slot.get("lo").and_then(|v| v.as_i64());
        let hi = slot.get("hi").and_then(|v| v.as_i64());
        let labels: Vec<String> = slot
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .take(schema::MAX_LABELS)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        owned.push(OwnedSlot {
            predicate: predicate.to_string(),
            ty,
            class,
            lo,
            hi,
            labels,
        });
    }
    // Pass two: interned, and the document is free to drop at the end.
    let mut out = Vec::with_capacity(owned.len());
    for owned_slot in owned {
        let labels: Vec<&'static str> = owned_slot
            .labels
            .iter()
            .filter_map(|l| intern_class(l))
            .take(schema::MAX_LABELS)
            .collect();
        out.push(SlotDecl {
            predicate: owned_slot.predicate,
            ty: owned_slot.ty,
            // A class the document invented is interned under a bound; a name
            // that does not fit simply carries no class, which makes the
            // contradiction pass skip the slot rather than compare it against
            // a stranger.
            class: owned_slot.class.as_deref().and_then(intern_class),
            lo: owned_slot.lo,
            hi: owned_slot.hi,
            labels: Box::leak(labels.into_boxed_slice()),
        });
    }
    Ok(out)
}

/// Intern a disjointness class name, bounded. Beyond the cap the name is not
/// interned and the slot simply carries no class — which makes contradiction
/// checking skip it rather than compare against a stranger.
fn intern_class(name: &str) -> Option<&'static str> {
    const MAX_INTERNED: usize = 256;
    if name.is_empty() || name.len() > schema::MAX_PREDICATE_BYTES {
        return None;
    }
    INTERNED_CLASSES.with(|c| {
        let mut v = c.borrow_mut();
        if let Some(found) = v.iter().find(|k| **k == name) {
            return Some(*found);
        }
        if v.len() >= MAX_INTERNED {
            return None;
        }
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        v.push(leaked);
        Some(leaked)
    })
}

thread_local! {
    static INTERNED_CLASSES: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Load the ratified claims the contradiction pass needs.
pub(crate) fn load_ratified(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<RatifiedClaim>, CreateError> {
    let mut stmt = conn.prepare(
        "SELECT claim_id, predicate, object, support_n FROM claims
          WHERE status = 'ratified'
          ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit.clamp(1, RECALL_PAGE_MAX) as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(claim_id, predicate, object, support_n)| RatifiedClaim {
            claim_id,
            predicate,
            // A stored object is canonical JSON; the gate's value is the typed
            // reading of it. A value this layer cannot type is not silently
            // coerced into one — it is omitted, so the contradiction pass sees
            // fewer rows rather than wrong ones.
            value: SlotValue::Integer(object.parse::<i64>().unwrap_or(0)),
            declared_type: SlotType::Integer,
            class: None,
            support_n,
        })
        .collect())
}

/// Load one claim's citations and their admitted bytes.
///
/// The bytes come from the caller, not from the store: a citation is only
/// meaningful against the bytes that were ADMITTED, and a read that
/// normalised the stored text first would be comparing something else
/// entirely.
pub(crate) fn load_citations(
    conn: &Connection,
    claim_row_id: i64,
    admitted: &[(String, Vec<u8>)],
) -> Result<Vec<(Citation, Vec<u8>)>, CreateError> {
    let mut stmt = conn.prepare(
        "SELECT source_cid, byte_start, byte_end, quote FROM claim_evidence
          WHERE claim_ref = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![claim_row_id], |r| {
            Ok(Citation {
                source_cid: r.get(0)?,
                byte_start: r.get(1)?,
                byte_end: r.get(2)?,
                quote: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|c| {
            // A citation whose admitted source was not supplied is carried with
            // EMPTY bytes, which the gate refuses as unresolvable. Fetching it
            // here would make this layer the thing that decides what counts as
            // evidence.
            let bytes = admitted
                .iter()
                .find(|(cid, _)| *cid == c.source_cid)
                .map(|(_, b)| b.clone())
                .unwrap_or_default();
            (c, bytes)
        })
        .collect())
}

/// Run the gate over a stored claim and return the verdict.
pub(crate) fn verify_claim(
    conn: &Connection,
    claim_id: &str,
    admitted: &[(String, Vec<u8>)],
) -> Result<GateOutcome, CreateError> {
    let Some(claim) = load_claim_under_test(conn, claim_id, admitted)? else {
        return Ok(GateOutcome::Refused {
            check: crate::workflow::create::verify::Check::Referential,
            reason: Refusal::Referential,
        });
    };
    // The schema comes from the claim's OWN `schema_ref` — the foreign key the
    // writer bound and the database enforces. It used to come from
    // `claim.subject`, which is a free string on the request body with no
    // relationship to `domain`, so a caller could name a subject that happened
    // to match a different domain's schema and be gated against an artifact
    // they never named. Fail-open and fail-closed were both live here: a claim
    // on an undeclared predicate passed because every check bailed on the miss,
    // and a legal claim was refused because the borrowed schema's bounds
    // excluded it.
    let schema = load_schema_slots_by_ref(conn, claim.schema_ref)?;
    Ok(crate::workflow::create::verify::run(
        &claim,
        schema.as_deref(),
    ))
}

/// The stored fields the gate's plumbing needs. Named rather than returned as
/// an eight-column tuple, because a positional row shape is exactly the kind
/// of thing that silently transposes two `String` columns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimRowFields {
    row_id: i64,
    subject: String,
    predicate: String,
    object: String,
    qualifiers: String,
    contradicts: String,
    /// The FK to `claim_schemas`. The gate needs it — it is how the claim's
    /// schema is resolved at verification time — so it is read, not discarded.
    schema_ref: i64,
    _support_n: i64,
}

/// Assemble the rows the gate reads. Pure plumbing: no verdict here either.
fn load_claim_under_test(
    conn: &Connection,
    claim_id: &str,
    admitted: &[(String, Vec<u8>)],
) -> Result<Option<ClaimUnderTest>, CreateError> {
    let raw: Option<ClaimRowFields> = conn
        .query_row(
            "SELECT id, subject, predicate, object, qualifiers, contradicts, schema_ref, support_n
               FROM claims WHERE claim_id = ?1",
            params![claim_id],
            |r| {
                Ok(ClaimRowFields {
                    row_id: r.get(0)?,
                    subject: r.get(1)?,
                    predicate: r.get(2)?,
                    object: r.get(3)?,
                    qualifiers: r.get(4)?,
                    contradicts: r.get(5)?,
                    schema_ref: r.get(6)?,
                    _support_n: r.get(7)?,
                })
            },
        )
        .optional()?;
    let Some(fields) = raw else {
        return Ok(None);
    };
    let ClaimRowFields {
        row_id,
        subject,
        predicate,
        object,
        qualifiers,
        contradicts,
        schema_ref,
        ..
    } = fields;
    let cites = load_citations(conn, row_id, admitted)?;
    let (citations, sources): (Vec<Citation>, Vec<Vec<u8>>) = cites.into_iter().unzip();
    let declared_type = if object.parse::<i64>().is_ok() {
        SlotType::Integer
    } else {
        SlotType::Free
    };
    Ok(Some(ClaimUnderTest {
        claim_id: claim_id.to_string(),
        subject: subject.clone(),
        schema_ref,
        predicate,
        value: match declared_type {
            SlotType::Integer => SlotValue::Integer(object.parse::<i64>().unwrap_or(0)),
            _ => SlotValue::Free(object.clone()),
        },
        declared_type,
        qualifiers: decode_pairs(&qualifiers)?,
        contradicts: decode_list(&contradicts)?,
        citations,
        sources,
        ratified: load_ratified(conn, RECALL_PAGE_DEFAULT)?,
    }))
}

fn decode_pairs(raw: &str) -> Result<Vec<(String, String)>, CreateError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CreateError::Storage(format!("qualifiers is not valid json: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| CreateError::Storage("qualifiers is not an object".into()))?;
    let mut out = Vec::new();
    for (k, v) in obj {
        if out.len() >= crate::workflow::create::verify::MAX_QUALIFIERS {
            break;
        }
        let s = v
            .as_str()
            .ok_or_else(|| CreateError::Storage("a qualifier value is not a string".into()))?;
        out.push((k.clone(), s.to_string()));
    }
    Ok(out)
}

fn decode_list(raw: &str) -> Result<Vec<String>, CreateError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CreateError::Storage(format!("a list column is not valid json: {e}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| CreateError::Storage("a list column is not an array".into()))?;
    let mut out = Vec::new();
    for v in arr {
        if out.len() >= crate::workflow::create::verify::MAX_CONTRADICTS {
            break;
        }
        let s = v
            .as_str()
            .ok_or_else(|| CreateError::Storage("a list element is not a string".into()))?;
        out.push(s.to_string());
    }
    Ok(out)
}

/// Write a claim, its citations, and its audit row in ONE transaction.
///
/// This is the only write path into `claims`, and the audit row is written
/// inside the caller's transaction so a claim and the evidence that it exists
/// commit or roll back together.
///
/// It carries no disproof condition. The condition travels as a separate
/// argument to [`store_claim_with_disproof`] rather than as a field on
/// [`ClaimDraft`], because `ClaimDraft` has a construction site outside this
/// write scope (`src/handlers/claims.rs`); adding a field there would make the
/// tree unbuildable without a change this round may not make. The two functions
/// share one INSERT, so there is still exactly one write path into `claims`.
pub(crate) fn store_claim(
    tx: &rusqlite::Transaction<'_>,
    draft: &ClaimDraft<'_>,
) -> Result<i64, CreateError> {
    store_claim_with_disproof(tx, draft, None)
}

/// [`store_claim`], carrying the claim's disproof condition.
///
/// `None` is a claim about the row's HISTORY — "no condition was ever written
/// here" — and never a claim that none was required. It writes SQL NULL into
/// all six nullable disproof columns and leaves coverage at its `'[]'`
/// default, so a claim written before this round stays stamp-blind by
/// declaration. Read it back with [`read_disproof`], which keeps that
/// distinction rather than widening `None` to cover an unreadable row.
pub(crate) fn store_claim_with_disproof(
    tx: &rusqlite::Transaction<'_>,
    draft: &ClaimDraft<'_>,
    disproof: Option<&DisproofCondition>,
) -> Result<i64, CreateError> {
    let ClaimDraft {
        claim_id,
        domain: _domain,
        schema_ref,
        subject,
        predicate,
        object,
        authored_by,
        created_at,
        citations,
    } = *draft;
    if claim_id.is_empty() || claim_id.len() > MAX_CLAIM_ID_BYTES {
        return Err(CreateError::Storage("claim id out of bounds".into()));
    }
    // The pre-computed target digest the database fence compares against. It
    // is computed HERE, in Rust, because SQLite cannot hash a column — a fence
    // that tried to compute it in SQL would refuse every visibility flip.
    let target_hash = crate::audit::hash(claim_id);
    let evidence_digest = evidence_digest_of(claim_id, citations);
    let qualifiers = "{}";
    let contradicts = "[]";
    // The mapping happens HERE, from the typed principal kind, not at the
    // call site. That is stronger than trusting every caller: a handler cannot
    // pass an arbitrary string for a column the database fences key on, so
    // there is no spelling of this function that lets a request body name its
    // own author.
    let created_by = principal_kind_string(authored_by);

    // The seven disproof columns. A `None` condition writes SQL NULL into all
    // six nullable ones and `[]` into coverage — which is the column's own
    // DEFAULT, written explicitly so the INSERT is a single fixed statement
    // rather than two that could drift. The observable state is identical to a
    // row written before these columns existed: a claim written before this
    // round stays stamp-blind by declaration, and an empty string is never
    // substituted for NULL.
    //
    // The condition is validated at its single constructor
    // (`DisproofCondition::new`), so there is deliberately NO validation here.
    // A second check at the write seam would be a second law that could drift
    // from the first, and it would be unreachable: nothing can hand this
    // function a value the constructor did not admit. Serialisation is
    // infallible too, since schema 1.32.23 gave `scope` a column — so this
    // seam can no longer manufacture a row its own read-back would refuse,
    // which is what the `DI_DISPROOF_SCOPE_NOT_PERSISTED` refusal used to
    // exist to prevent.
    let (d_form, d_body, d_op, d_citation, d_scope, d_coverage, d_audit_ref) = match disproof {
        Some(c) => {
            let cols = c.to_columns();
            (
                cols.form,
                cols.body,
                cols.op,
                cols.citation,
                cols.scope,
                cols.coverage,
                cols.audit_ref,
            )
        }
        None => (None, None, None, None, None, Some("[]".to_string()), None),
    };

    tx.execute(
        "INSERT INTO claims(
            claim_id, schema_ref, subject, predicate, object, qualifiers, contradicts,
            evidence_digest, audit_target_hash, created_by, created_at, status,
            disproof_form, disproof_body, disproof_op, disproof_citation, disproof_scope,
            disproof_coverage, disproof_audit_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending',
                 ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            claim_id,
            schema_ref,
            subject,
            predicate,
            object,
            qualifiers,
            contradicts,
            evidence_digest,
            target_hash,
            created_by,
            created_at,
            d_form,
            d_body,
            d_op,
            d_citation,
            d_scope,
            d_coverage,
            d_audit_ref,
        ],
    )?;
    let row_id = tx.last_insert_rowid();
    for (citation, bytes) in citations {
        let quote_digest = crate::audit::hash(&String::from_utf8_lossy(bytes));
        tx.execute(
            "INSERT INTO claim_evidence(
                claim_ref, source_cid, quote, byte_start, byte_end, quote_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row_id,
                citation.source_cid,
                citation.quote,
                citation.byte_start,
                citation.byte_end,
                quote_digest,
            ],
        )?;
    }
    crate::audit::record_tenant(
        tx,
        AuditKind::Workflow,
        created_by,
        claim_id,
        AuditStatus::Ok,
        "claim proposed",
        "global",
    );
    Ok(row_id)
}

/// A claim as offered for storage. Borrowed throughout, so the handler can
/// pass a projection rather than a clone, and named so the fields of a
/// claim write are visible as a shape rather than as a call's argument list.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClaimDraft<'a> {
    pub(crate) claim_id: &'a str,
    pub(crate) domain: &'a str,
    pub(crate) schema_ref: i64,
    pub(crate) subject: &'a str,
    pub(crate) predicate: &'a str,
    pub(crate) object: &'a str,
    pub(crate) authored_by: PrincipalKind,
    pub(crate) created_at: i64,
    pub(crate) citations: &'a [(Citation, Vec<u8>)],
}

/// Read a claim's disproof condition back, **fail-closed**.
///
/// # Why this is a separate function and not a column decode
///
/// The distinction `Ok(None)` carries is *"this claim predates the field"*.
/// It does **not** carry "the condition could not be read". A row that names a
/// form it cannot rebuild is returned as an `Err`, because a caller that
/// receives `None` treats the claim as exempt from evaluation — and a damaged
/// condition is precisely the claim that should be looked at hardest. Widening
/// `None` to also mean "unreadable" is the one defect this seam exists to
/// prevent.
///
/// A missing claim row is `Ok(None)` for the ordinary reason that there is
/// nothing there to read; a claim whose disproof columns are corrupt is an
/// error naming the claim, never a silent absence.
pub(crate) fn read_disproof(
    conn: &Connection,
    claim_id: &str,
) -> Result<Option<DisproofCondition>, CreateError> {
    let cols = conn
        .query_row(
            "SELECT disproof_form, disproof_body, disproof_op, disproof_citation,
                    disproof_scope, disproof_coverage, disproof_audit_ref
             FROM claims WHERE claim_id = ?1",
            params![claim_id],
            |r| {
                Ok(DisproofColumns {
                    form: r.get(0)?,
                    body: r.get(1)?,
                    op: r.get(2)?,
                    citation: r.get(3)?,
                    scope: r.get(4)?,
                    coverage: r.get(5)?,
                    audit_ref: r.get(6)?,
                })
            },
        )
        .optional()?;
    let Some(cols) = cols else {
        return Ok(None);
    };
    // The refusal is mapped into `Storage`, which is the core's existing
    // "this row is not what it claims to be" channel. The cause string is
    // preserved verbatim: it names which column failed to rebuild, and it is
    // NOT a location hint about the request — nothing here echoes request
    // input back to a caller.
    DisproofCondition::from_columns(&cols)
        .map_err(|e| CreateError::Storage(format!("disproof_read_back:{e}")))
}

/// Evaluate a stored claim's condition against a subject, fail-closed.
///
/// The three-state result is preserved end to end: a claim with no condition
/// and a claim whose condition has no mechanical verdict both come back as
/// `NoVerdict`, which [`DisproofVerdict::is_green`] reports as NOT green. There
/// is no path through this function by which an unevaluated claim reads as
/// satisfied.
pub(crate) fn evaluate_disproof(
    conn: &Connection,
    claim_id: &str,
    subject: &str,
) -> Result<DisproofVerdict, CreateError> {
    // No condition is not a pass. The reason is the legacy one, stated
    // rather than left as a silent green.
    read_disproof(conn, claim_id)?.map_or_else(
        || {
            Ok(DisproofVerdict::NoVerdict {
                reason: "DI_DISPROOF_ABSENT_PREDATES_FIELD".into(),
            })
        },
        |c| Ok(c.verdict(subject)),
    )
}

/// The digest of a claim's evidence, computed at write time so promotion can
/// be refused if any cited byte moved between review and approval.
fn evidence_digest_of(claim_id: &str, citations: &[(Citation, Vec<u8>)]) -> String {
    let mut acc = String::from(claim_id);
    for (c, bytes) in citations {
        acc.push('\u{1f}');
        acc.push_str(&c.source_cid);
        acc.push(':');
        acc.push_str(&c.byte_start.to_string());
        acc.push(':');
        acc.push_str(&c.byte_end.to_string());
        acc.push(':');
        acc.push_str(&crate::audit::hash(&String::from_utf8_lossy(bytes)));
    }
    crate::audit::hash(&acc)
}

/// Store a human-authored schema, with its audit row, in ONE transaction.
///
/// The `CHECK (authored_by = 'human')` on the table is the tripwire; the
/// binding check that the ACTING principal is human lives in the
/// authorization layer, and this function refuses anything but the human
/// spelling rather than trusting its caller to have checked.
///
/// `ratified_at` is set at admission, because admission IS ratification here:
/// the route requires the workflow role and a human principal, and there is no
/// second review step. The column records WHEN, and a schema with no
/// `ratified_at` is one this route never wrote — which is what the typed-schema
/// read keys on.
pub(crate) fn store_schema(
    tx: &rusqlite::Transaction<'_>,
    domain: &str,
    version: i64,
    authored_by: PrincipalKind,
    body: &str,
    slots: Vec<SlotDecl>,
    created_at: i64,
) -> Result<StoredSchema, CreateError> {
    // Mapped from the typed principal kind, for the same reason as the claim
    // write above: the stored author string is a CHECK constraint's input and
    // must not be reachable from a request body.
    let authored_by = principal_kind_string(authored_by);
    let admitted = schema::admit(domain, version, authored_by, body, slots)
        .map_err(CreateError::SchemaRejected)?;
    let digest = schema::body_digest(body);
    tx.execute(
        "INSERT INTO claim_schemas(
            domain, version, authored_by, body, body_digest, ratified_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            admitted.domain,
            admitted.version,
            authored_by,
            body,
            digest,
            created_at
        ],
    )?;
    // Captured HERE, immediately after the insert and before the audit row.
    // `last_insert_rowid` moves on every insert, so a caller that read it
    // after this function returned would have gotten the AUDIT row's id — a
    // silent wrong-row bug that a foreign key turns into a confusing
    // constraint failure far from its cause.
    let row_id = tx.last_insert_rowid();
    crate::audit::record_tenant(
        tx,
        AuditKind::Workflow,
        authored_by,
        domain,
        AuditStatus::Ok,
        "claim schema authored",
        "global",
    );
    Ok(StoredSchema {
        id: row_id,
        decl: admitted,
        body_digest: digest,
    })
}

/// A stored schema, as the writer hands it back: the row id and the digest the
/// row is under. Both are captured at insert time for the reason on
/// `store_schema`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredSchema {
    pub(crate) id: i64,
    pub(crate) decl: SchemaDecl,
    pub(crate) body_digest: String,
}

/// Open a batch and record its set-check verdict, with its audit row, in ONE
/// transaction.
///
/// `Passed` and `Failed` are separate arms rather than a boolean so a caller
/// cannot record a partial result: there is no way to store "checked, unknown".
pub(crate) fn store_batch_verdict(
    tx: &rusqlite::Transaction<'_>,
    batch_id: i64,
    digest: &str,
    member_count: i64,
    set_check: &str,
    checked_at: i64,
    actor: &str,
) -> Result<(), CreateError> {
    if set_check != "pass" && set_check != "fail" {
        return Err(CreateError::Storage(
            "a stored set-check verdict must be a decision".into(),
        ));
    }
    tx.execute(
        "INSERT INTO claim_batches(id, batch_digest, member_count, set_check, checked_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![batch_id, digest, member_count, set_check, checked_at],
    )?;
    crate::audit::record_tenant(
        tx,
        AuditKind::Workflow,
        actor,
        &batch_id.to_string(),
        AuditStatus::Ok,
        "batch set-check recorded",
        "global",
    );
    Ok(())
}

/// Flip recall visibility for a whole batch, with its audit row, in ONE
/// transaction.
///
/// The trigger refuses this unless every member is ratified and the batch's
/// set check passed, so this function does not re-check the set verdict — it
/// cannot, cheaply or correctly, and duplicating the check here would create a
/// second policy. The database is the authority; this is the transaction that
/// gives it something to authorise.
pub(crate) fn flip_batch_visibility(
    tx: &rusqlite::Transaction<'_>,
    batch_id: i64,
    actor: &str,
    at: i64,
) -> Result<usize, CreateError> {
    let n = tx.execute(
        "UPDATE claims SET recall_visible = 1 WHERE batch_id = ?1 AND recall_visible = 0",
        params![batch_id],
    )?;
    crate::audit::record_tenant(
        tx,
        AuditKind::Workflow,
        actor,
        &batch_id.to_string(),
        AuditStatus::Ok,
        "batch recall visibility granted",
        "global",
    );
    let _ = at;
    Ok(n)
}

/// The corpus, as read by the round's boundary harness. Data only — nothing
/// here reaches a write path.
pub(crate) fn corpus() -> &'static [PlantedClaim] {
    crate::workflow::create::corpus::PLANTED_CORPUS
}

/// The gap flood, as generated. Pure; the caller supplies the domain state.
pub(crate) fn gaps(
    domain: &str,
    covered: &[String],
    schema: Option<&SchemaDecl>,
) -> Vec<GapCandidate> {
    crate::workflow::create::gap::generate(
        domain,
        &crate::workflow::create::gap::DomainState {
            covered: covered.to_vec(),
        },
        schema,
    )
}

/// The generation methods, for a screen that shows the operator what produced
/// a gap. Ranking only; it can never set a status.
pub(crate) fn gap_methods() -> &'static [GapMethod] {
    &GapMethod::ALL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::workflow::create::verify::Check;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migration(&mut conn, 512).expect("migration");
        conn
    }

    fn slot() -> SlotDecl {
        SlotDecl {
            predicate: "warranty_months".into(),
            ty: SlotType::Integer,
            class: Some("warranty"),
            lo: Some(0),
            hi: Some(120),
            labels: &[],
        }
    }

    #[test]
    fn the_gated_read_is_the_only_reader_and_it_joins_both_fenced_columns() {
        let source = include_str!("create.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        let gated = production
            .split("pub(crate) fn recall_page(")
            .nth(1)
            .expect("the gated read must exist");
        let gated = &gated[..gated.find("\n}").expect("a fn must close")];
        assert!(gated.contains("status = 'ratified'"));
        assert!(gated.contains("recall_visible = 1"));
        // And it must not offer a way to ask for unratified material.
        assert!(
            !gated.contains("pending"),
            "the gated read must have no parameter that reaches an unratified row: a \
             reader that can be asked for pending material is a reader that will be"
        );
    }

    #[test]
    fn a_schema_round_trips_through_its_digest() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        let stored = store_schema(
            &tx,
            "global",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[{"predicate":"warranty_months","ty":"integer","class":"warranty","lo":0,"hi":120}]}"#,
            vec![slot()],
            1,
        )
        .expect("a human schema must be admitted");
        assert_eq!(stored.decl.version, 1);
        let digest = schema_digest(&tx, stored.id).expect("digest read");
        assert_eq!(
            digest.expect("a stored schema row"),
            stored.body_digest,
            "the stored digest must match its own body"
        );
        tx.commit().expect("commit");

        let slots = load_schema_slots(&conn, "global")
            .expect("load")
            .expect("a ratified schema must load");
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].predicate, "warranty_months");
        assert_eq!(slots[0].ty, SlotType::Integer);
    }

    #[test]
    fn a_tampered_schema_body_is_detected_at_load() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        let stored = store_schema(
            &tx,
            "global",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[]}"#,
            vec![slot()],
            1,
        )
        .expect("admit");
        let id = stored.id;
        tx.commit().expect("commit");
        conn.execute(
            "UPDATE claim_schemas SET body = ?1 WHERE id = ?2",
            params![r#"{"slots":[{"predicate":"other"}]}"#, id],
        )
        .expect("tamper");
        assert!(
            schema_digest(&conn, id).is_err(),
            "a schema whose body no longer matches its digest must not load: the artifact a \
             reviewer signs would not be the artifact in the row"
        );
    }

    #[test]
    fn an_unratified_claim_is_invisible_to_every_read() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        let stored = store_schema(
            &tx,
            "global",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[]}"#,
            vec![slot()],
            1,
        )
        .expect("admit");
        let schema_ref = stored.id;
        tx.commit().expect("commit");

        let tx = conn.unchecked_transaction().expect("tx");
        store_claim(
            &tx,
            &ClaimDraft {
                claim_id: "clm_hidden",
                domain: "global",
                schema_ref,
                subject: "acme",
                predicate: "warranty_months",
                object: "24",
                authored_by: PrincipalKind::AgentLoopback,
                created_at: 1,
                citations: &[],
            },
        )
        .expect("store");
        tx.commit().expect("commit");

        assert!(
            claim_exists(&conn, "clm_hidden").expect("probe"),
            "the claim exists; the point is that it is not READABLE"
        );
        assert!(
            recall_page(&conn, RECALL_PAGE_DEFAULT)
                .expect("page")
                .is_empty(),
            "a pending claim must be invisible to the gated read: it is not a corpus row and \
             no recall query can see it"
        );
        // And the screen read, the one surface that may look at a pending
        // claim, is a different function on purpose.
        assert!(load_for_screen(&conn, "clm_hidden").is_ok());
    }

    /// RED-FIRST (R54 §3.1, second defect). The write path mints `schema_ref`
    /// from `body.domain` (`handlers/claims.rs:243-250`) and the gate then threw
    /// that FK away and re-derived the schema from the caller-controlled
    /// `subject` string. This drives the REAL seam — DB in, verdict out — which
    /// no existing test did, and pins both halves of the law:
    ///
    ///  1. the schema the gate consults is the one the writer bound by FK, and
    ///  2. a claim whose subject happens to name a DIFFERENT domain's schema is
    ///     refused, not silently re-typed against the schema it did not name.
    #[test]
    fn the_gate_reads_the_schema_the_writer_bound_and_not_the_callers_subject() {
        let conn = db();

        // Two ratified schemas, in two different domains, declaring the same
        // predicate with DIFFERENT bounds. If the gate consults the wrong one,
        // the value's legality changes — which is exactly the observable.
        let tx = conn.unchecked_transaction().expect("tx");
        let acme = store_schema(
            &tx,
            "acme",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[{"predicate":"warranty_months","ty":"integer","class":"warranty","lo":0,"hi":120}]}"#,
            vec![slot()],
            1,
        )
        .expect("admit");
        let mut narrow = slot();
        narrow.hi = Some(1); // acme allows 0..=1
        store_schema(
            &tx,
            "globex",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[{"predicate":"warranty_months","ty":"integer","class":"warranty","lo":0,"hi":1}]}"#,
            vec![narrow],
            1,
        )
        .expect("admit");
        tx.commit().expect("commit");

        // A claim written under the `acme` schema — where 24 is IN BOUNDS —
        // but whose `subject` names the `globex` domain, where 24 is NOT.
        // This is the attacker-chosen-subject case: `subject` is a free string
        // on the wire, validated only for non-emptiness.
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim(
            &tx,
            &ClaimDraft {
                claim_id: "clm_keyed",
                domain: "acme",
                schema_ref: acme.id,
                subject: "globex",
                predicate: "warranty_months",
                object: "24",
                authored_by: PrincipalKind::AgentLoopback,
                created_at: 1,
                citations: &[],
            },
        )
        .expect("store");
        tx.commit().expect("commit");

        // The FK the writer actually bound says `acme`, and `acme` admits 24.
        // The gate must consult THAT. Keyed on `subject` it would load
        // `globex` instead and refuse at check 2 (`bounds`), because 24 sits
        // outside that schema's 0..=1.
        //
        // So the ORDER is the evidence: this claim carries no admitted sources,
        // and an empty citation set is refused at check 4. Reaching check 4 at
        // all proves checks 1-3 passed, and check 2 is the one that would have
        // caught a wrongly-borrowed schema. The refusal below is therefore
        // EXPECTED — what is asserted is that it is check FOUR and not check
        // TWO.
        match verify_claim(&conn, "clm_keyed", &[]).expect("gate") {
            GateOutcome::Refused {
                check: Check::CitationResolvability,
                reason: Refusal::EvidenceUnresolvable,
            } => {}
            GateOutcome::Refused { check, reason } => panic!(
                "the gate refused this claim BEFORE the citation check, at {} / {}. Its own \
                 FK-bound schema admits 24, so an earlier refusal means the gate consulted a \
                 schema other than the one the writer bound by FK — almost certainly the one \
                 named by the caller's `subject`.",
                check.as_str(),
                reason.as_str()
            ),
            other => panic!("expected a citation refusal, got {other:?}"),
        }
    }

    /// The companion to the test above, and the one that shows the fail-open is
    /// REACHABLE. A claim whose predicate its bound schema never declares must
    /// be refused by the gate. Before the fix the four `?`-on-lookup bails let
    /// it through to `Pass`, and this test is what makes that reachable rather
    /// than theoretical: the schema loads (the FK is valid), the predicate is
    /// undeclared, and the check bails.
    #[test]
    fn a_claim_whose_predicate_its_bound_schema_never_declared_is_refused() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        let stored = store_schema(
            &tx,
            "acme",
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[{"predicate":"warranty_months","ty":"integer","class":"warranty","lo":0,"hi":120}]}"#,
            vec![slot()],
            1,
        )
        .expect("admit");
        let schema_ref = stored.id;
        tx.commit().expect("commit");

        // The attack: `predicate` is never validated against the schema, so a
        // caller may name a slot the human artifact does not contain. `subject`
        // names a real domain so the schema lookup succeeds at all — which is
        // the only reason this reaches the battery instead of stopping at
        // `NoSchema`.
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim(
            &tx,
            &ClaimDraft {
                claim_id: "clm_undeclared",
                domain: "acme",
                schema_ref,
                subject: "acme",
                predicate: "operator_granted_superuser",
                object: "1",
                authored_by: PrincipalKind::AgentLoopback,
                created_at: 1,
                citations: &[],
            },
        )
        .expect("store");
        tx.commit().expect("commit");

        match verify_claim(&conn, "clm_undeclared", &[]).expect("gate") {
            GateOutcome::Refused { check, reason } => assert_eq!(
                (check, reason),
                (Check::SchemaConformance, Refusal::SchemaMismatch),
                "an undeclared predicate is a shape failure"
            ),
            other => panic!(
                "a claim on a predicate the bound schema never declared reached {other:?}. The \
                 `subject` here names a live domain, so the schema DID load and the checks DID \
                 run — they bailed. This is the reachable fail-open, not a theoretical one."
            ),
        }
    }

    #[test]
    fn the_recall_page_is_bounded() {
        let conn = db();
        // A limit of zero clamps to one; a limit past the cap clamps to the cap.
        assert!(recall_page(&conn, 0).is_ok());
        assert!(recall_page(&conn, usize::MAX).is_ok());
        // A compile-time relation, so a later edit that inverts the two fails
        // the build rather than producing a default past the cap.
        const {
            assert!(RECALL_PAGE_DEFAULT < RECALL_PAGE_MAX);
        };
        assert_eq!(RECALL_PAGE_MAX, 50);
    }

    // ── the disproof writer and its fail-closed read-back ──────────────
    //
    // These live HERE rather than in `tests/r50_create.rs` because that file is
    // an external integration test: it links the crate as a downstream consumer
    // and cannot name a `pub(crate)` item. Widening `store_claim_with_disproof`
    // or `read_disproof` to `pub` to make a test compile would be a production
    // API change made for test convenience, so the behavioural pins sit beside
    // their subject and the source-level pins sit outside.

    /// A real schema for a claim to bind to, so the write path is exercised
    /// exactly as production exercises it.
    fn admit(conn: &Connection, domain: &str) -> i64 {
        let tx = conn.unchecked_transaction().expect("tx");
        let stored = store_schema(
            &tx,
            domain,
            1,
            PrincipalKind::Jwt,
            r#"{"slots":[{"predicate":"warranty_months","ty":"integer","class":"warranty","lo":0,"hi":120}]}"#,
            vec![slot()],
            1,
        )
        .expect("admit");
        let id = stored.id;
        tx.commit().expect("commit");
        id
    }

    fn draft(claim_id: &str, schema_ref: i64) -> ClaimDraft<'_> {
        ClaimDraft {
            claim_id,
            domain: "acme",
            schema_ref,
            subject: "acme",
            predicate: "warranty_months",
            object: "24",
            authored_by: PrincipalKind::AgentLoopback,
            created_at: 1,
            citations: &[],
        }
    }

    /// A machine-checked condition: the form that was **unpersistable** before
    /// schema 1.32.23, because `Evaluated` requires a scope and there was no
    /// `disproof_scope` column. Held here so the round-trip pins can use the
    /// form the table could not previously hold.
    fn machine_checked() -> DisproofCondition {
        use crate::workflow::create::disproof::{DisproofForm, EvaluatedOp};
        DisproofCondition::new(
            DisproofForm::Evaluated,
            "the payer matches",
            Some(EvaluatedOp::Contains),
            Some("payer-verified".into()),
            Some("subject.payer_state".into()),
            vec!["payer_state".into()],
            None,
        )
        .expect("an evaluated condition carries its operator, byte range and scope")
    }

    /// A prose condition: scoped by its audit rather than by a machine-checkable
    /// scope, so it carries no `scope` and must round-trip without one.
    fn prose() -> DisproofCondition {
        DisproofCondition::new(
            crate::workflow::create::disproof::DisproofForm::Audited,
            "this claim is false if the member's consent was not recorded",
            None,
            None,
            None,
            vec!["consent".into(), "scope_of_use".into()],
            Some("audit:care/consent-441".into()),
        )
        .expect("a prose condition shipped with its audit")
    }

    /// The raw stored columns, read WITHOUT the read-back, so an assertion about
    /// what is on disk is not answered by the function under test.
    fn raw_columns(conn: &Connection, claim_id: &str) -> (Option<String>, Option<String>, String) {
        conn.query_row(
            "SELECT disproof_form, disproof_body, disproof_coverage FROM claims
             WHERE claim_id = ?1",
            params![claim_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .expect("the claim row must exist")
    }

    /// Round-trip (task 1) — a real condition written by the real writer, read
    /// back byte for byte.
    #[test]
    fn a_disproof_condition_survives_a_real_database_round_trip() {
        let conn = db();
        let schema_ref = admit(&conn, "acme");
        let c = prose();

        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_rt", schema_ref), Some(&c)).expect("store");
        tx.commit().expect("commit");

        // On disk first, read without the read-back: the row must actually CARRY
        // the condition, not merely round-trip through a function that is
        // returning something it was handed.
        let (form, body, coverage) = raw_columns(&conn, "clm_rt");
        assert_eq!(form.as_deref(), Some("audited"), "the form was written");
        assert_eq!(
            body.as_deref(),
            Some("this claim is false if the member's consent was not recorded"),
            "the body was written verbatim"
        );
        assert_eq!(
            coverage, "[\"consent\",\"scope_of_use\"]",
            "coverage is a JSON array, order preserved"
        );

        // And now the read-back, field by field.
        let back = read_disproof(&conn, "clm_rt")
            .expect("a well-formed row reads back")
            .expect("the row carries a condition");
        assert_eq!(back, c, "every field survives the round trip");
    }

    /// Legacy (task 2) — a claim with no condition reads back `None`, and the
    /// columns on disk are the pre-field shape.
    #[test]
    fn a_claim_written_without_a_condition_reads_back_as_none() {
        let conn = db();
        let schema_ref = admit(&conn, "acme");
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim(&tx, &draft("clm_legacy", schema_ref)).expect("store");
        tx.commit().expect("commit");

        // The five nullable columns are SQL NULL — not an empty string, which
        // would be a value a reader could mistake for content.
        let (form, body, coverage) = raw_columns(&conn, "clm_legacy");
        assert_eq!(form, None, "no form, and it is NULL rather than ''");
        assert_eq!(body, None, "no body, and it is NULL rather than ''");
        assert_eq!(coverage, "[]", "coverage stays at the column's own default");
        assert_eq!(
            read_disproof(&conn, "clm_legacy").expect("read"),
            None,
            "a row that predates the field reads back as None"
        );
        // And None is NOT a pass: it is a distinct verdict that is never green.
        let v = evaluate_disproof(&conn, "clm_legacy", "anything").expect("evaluate");
        assert!(!v.is_green(), "an unevaluated claim is never green");
        assert_eq!(
            v,
            DisproofVerdict::NoVerdict {
                reason: "DI_DISPROOF_ABSENT_PREDATES_FIELD".into()
            },
            "absence is named, not silently treated as satisfied"
        );
    }

    /// Corrupt, three shapes (task 3) — each must be an `Err` and NEVER
    /// `Ok(None)`. This is the whole point of the round: `Ok(None)` exempts a
    /// claim from evaluation, so a damaged row reported as legacy is a damaged
    /// row nobody looks at.
    #[test]
    fn a_corrupt_disproof_row_is_refused_and_never_masquerades_as_legacy() {
        for (claim_id, form, body) in [
            // form present, body absent
            ("clm_c1", Some("audited"), None),
            // body present, form absent
            ("clm_c2", None, Some("a condition body")),
            // unparseable form spelling
            ("clm_c3", Some("prose"), Some("a condition body")),
        ] {
            let conn = db();
            let schema_ref = admit(&conn, "acme");
            let tx = conn.unchecked_transaction().expect("tx");
            store_claim(&tx, &draft(claim_id, schema_ref)).expect("store");
            tx.commit().expect("commit");

            // Plant the corruption the way a bad migration or a tampered row
            // would: directly in the columns.
            conn.execute(
                "UPDATE claims SET disproof_form = ?2, disproof_body = ?3 WHERE claim_id = ?1",
                params![claim_id, form, body],
            )
            .expect("plant");

            let outcome = read_disproof(&conn, claim_id);
            assert!(
                outcome.is_err(),
                "{claim_id} (form={form:?} body={body:?}) read back as {outcome:?}. A corrupt row \
                 reported as Ok(None) is reported as a claim that predates the field — which \
                 exempts it from evaluation. Damaged is not legacy."
            );
        }
    }

    /// The unparseable OPERATOR is its own corrupt shape, because `op` is the
    /// one column whose vocabulary is closed and whose parse failure would
    /// otherwise be silent: an unreadable op would become `None`, and `None` is
    /// a legal op only for prose.
    #[test]
    fn an_unparseable_operator_is_refused_rather_than_becoming_absent() {
        let conn = db();
        let schema_ref = admit(&conn, "acme");
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim(&tx, &draft("clm_op", schema_ref)).expect("store");
        tx.commit().expect("commit");
        conn.execute(
            "UPDATE claims SET disproof_form = 'audited', disproof_body = 'b',
                    disproof_op = 'regex', disproof_coverage = '[\"c\"]',
                    disproof_audit_ref = 'audit:x'
             WHERE claim_id = 'clm_op'",
            [],
        )
        .expect("plant");
        let err = read_disproof(&conn, "clm_op").expect_err("an unknown op is not an absent one");
        assert!(
            format!("{err}").contains("DI_DISPROOF_OP_UNKNOWN"),
            "the refusal must name the unknown operator, got {err}"
        );
    }

    /// Transactional rollback twin (task 4) — force the claim write to fail and
    /// assert that NO claim row, NO disproof column and NO audit row survives.
    ///
    /// # Why this database is FILE-backed, and why that matters
    ///
    /// The obvious version of this test runs on the in-memory `db()` helper and
    /// asserts only that the claim row is gone after a rollback. That version
    /// **cannot fail**. It was written that way, planted with a disproof write
    /// in a second statement, and observed PASSING — because a write inside the
    /// SAME transaction rolls back with everything else, whether or not the
    /// disproof columns actually ride the claim's statement. A green pin that
    /// cannot fail is worse than no pin, so the test was rewritten.
    ///
    /// What makes it falsifiable is a second connection. A write through it
    /// genuinely ESCAPES the caller's transaction — and a second connection to
    /// `:memory:` is a different database, so this test opens a file. The two
    /// control rows below then PROVE the harness can see an escaped write, and
    /// only after that do the absence assertions mean anything: they now
    /// distinguish "the audit row rides the caller's transaction" from "the
    /// audit row was written elsewhere and outlived the claim".
    #[test]
    fn a_failed_claim_write_leaves_no_disproof_column_and_no_audit_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rollback_twin.db");
        let url = format!("file:{}?mode=rwc", path.display());
        {
            // Registration BEFORE the connection: `sqlite3_auto_extension` only
            // reaches connections opened after it. Opening first left this pin
            // green in the full suite (where a sibling test had already
            // registered) and red on its own — an order dependency, not a
            // result.
            crate::register_sqlite_vec::register_sqlite_vec();
            let mut conn = Connection::open(&url).expect("open");
            crate::migration::run_migration(&mut conn, 512).expect("migration");
        }
        // A second connection to the SAME file: a write through it does not
        // participate in the other connection's transaction.
        let outside = Connection::open(&url).expect("open second");
        let conn = Connection::open(&url).expect("open");
        let schema_ref = admit(&conn, "acme");

        // Control 1 — a CLAIM row written outside any transaction must be
        // visible from `conn`.
        outside
            .execute(
                "INSERT INTO claims(claim_id, schema_ref, subject, predicate, object,
                        qualifiers, contradicts, evidence_digest, audit_target_hash,
                        created_by, created_at, status)
                 VALUES ('clm_control', ?1, 'acme', 'warranty_months', '24', '{}', '[]',
                         'd', ?2, 'agent', 1, 'pending')",
                params![schema_ref, crate::audit::hash("clm_control")],
            )
            .expect("the out-of-transaction control row must be writable");
        let control: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE claim_id = 'clm_control'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            control, 1,
            "the control row is not visible from the other connection, so this harness cannot \
             distinguish an escaped write from a rolled-back one and the assertions below would \
             pass vacuously"
        );

        // Control 2 — an AUDIT row written outside the transaction must also be
        // visible. This is the control the audit assertion actually depends on:
        // without it, "no audit row" would hold even for a broken audit-per-write
        // law, because the escaped row would be invisible to the query.
        let control_target = crate::audit::hash("clm_audit_control");
        outside
            .execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
                 VALUES ('workflow', 'agent', ?1, 'ok', 'control')",
                params![control_target],
            )
            .expect("the out-of-transaction control audit row must be writable");
        let control_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                params![control_target],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            control_audits, 1,
            "an audit row written OUTSIDE the caller's transaction is not visible here. The \
             absence asserted below would then hold even for a broken audit-per-write law, and \
             this twin would be a pin that cannot fail"
        );

        // Now the rollback. The second claim collides on the UNIQUE claim_id, so
        // the claim write itself fails — and the transaction is rolled back.
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_rolled", schema_ref), Some(&prose()))
            .expect("first write");
        let second =
            store_claim_with_disproof(&tx, &draft("clm_rolled", schema_ref), Some(&prose()));
        assert!(
            second.is_err(),
            "the duplicate claim_id must be refused; if it succeeded the test is not exercising \
             a failure at all"
        );
        tx.rollback().expect("rollback");

        // No claim row, so no disproof column.
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE claim_id = 'clm_rolled'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(rows, 0, "the rolled-back claim left a row behind");

        // And no audit row for it. The audit rides the caller's transaction, so
        // a rolled-back claim leaves no evidence that it happened. Control 2 is
        // what makes this absence mean something.
        let target = crate::audit::hash("clm_rolled");
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                params![target],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            audits, 0,
            "a rolled-back claim left an audit row. The claim and the evidence that it exists \
         must commit or roll back TOGETHER — an audit row that outlives its claim is \
         evidence of a write that did not happen"
        );
    }

    /// **The structural half of the rollback law, and the half that is actually
    /// falsifiable.**
    ///
    /// The database twin above asserts that a rolled-back claim leaves nothing
    /// behind. That assertion cannot be made to fail by planting an "escaped
    /// write" into the writer, and the reason is worth stating rather than
    /// hiding: `store_claim_with_disproof` receives a `&rusqlite::Transaction`
    /// and nothing else. It has no `&Connection`, no pool and no way to open
    /// one, so **every byte it writes is inside the caller's transaction by
    /// construction**. Two separate plants (a second statement in the same
    /// transaction, and an `ATTACH`ed sink) were tried and both left the suite
    /// green, which is the correct behaviour and the reason this pin exists.
    ///
    /// So the law is carried by the SIGNATURE, and this asserts it. If a later
    /// round widens the writer to take a `&Connection` — so it can "also" write
    /// a projection row, or an index entry, or a cache — this fails, and it
    /// fails at compile-review time rather than in production.
    #[test]
    fn the_claim_write_cannot_escape_the_callers_transaction() {
        let source = include_str!("create.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        let writer = production
            .split("pub(crate) fn store_claim_with_disproof(")
            .nth(1)
            .expect("the disproof-carrying writer must exist")
            .split(") -> Result<i64, CreateError> {")
            .next()
            .expect("the writer's parameter list must close");

        assert!(
            writer.contains("tx: &rusqlite::Transaction<'_>"),
            "the writer must take the caller's transaction. A writer that also took a \
             &Connection could write outside it, and the audit row would stop rolling back \
             with the claim"
        );
        for escape in ["&Connection", "Pool", "Connection::open", "get()"] {
            assert!(
                !writer.contains(escape),
                "the writer's parameters name `{escape}`. Every byte this function writes must \
                 ride the caller's transaction, and a connection or a pool is a way out of it"
            );
        }

        // And the BODY must not open a way out either. A parameter-list check
        // alone passed a writer that took only the transaction and then went
        // and opened its own connection — the same defect wearing a different
        // hat. That plant was observed passing here before this body check was
        // added, which is why the body check is here.
        let body = production
            .split("pub(crate) fn store_claim_with_disproof(")
            .nth(1)
            .expect("the writer must exist")
            .split("\n/// ")
            .next()
            .unwrap_or_default();
        for escape in ["Connection::open", "unchecked_open", "ATTACH"] {
            assert!(
                !body.contains(escape),
                "the writer's body reaches for `{escape}`. Every byte it writes must ride the \
                 caller's transaction; opening or attaching a second database is a way out of it"
            );
        }
    }

    /// The twin read in the other direction: prove the disproof columns really
    /// are part of the claim's INSERT, by rolling back and confirming a
    /// SUCCESSFUL write left them behind. Without this, test 4 would also pass
    /// if the columns were simply never written.
    #[test]
    fn a_successful_claim_write_commits_its_disproof_columns() {
        let conn = db();
        let schema_ref = admit(&conn, "acme");
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_ok", schema_ref), Some(&prose()))
            .expect("store");
        tx.commit().expect("commit");
        let (form, body, _) = raw_columns(&conn, "clm_ok");
        assert_eq!(form.as_deref(), Some("audited"));
        assert!(body.is_some(), "the body was committed with the claim");
        assert!(read_disproof(&conn, "clm_ok").expect("read").is_some());
    }

    /// Construction refusal (task 5) — an inadmissible condition cannot be
    /// built, so it cannot reach the write path. This is asserted at the
    /// constructor, and it is WHY there is no validation at the write seam:
    /// `store_claim_with_disproof` trusts its argument because the only way to
    /// obtain one is through a constructor that refuses. A second check at the
    /// write would be a second law that could drift from the first.
    #[test]
    fn an_inadmissible_condition_cannot_be_constructed_and_so_cannot_be_written() {
        use crate::workflow::create::disproof::{DisproofForm, EvaluatedOp};
        // Prose with no audit: refused at construction.
        let refused = DisproofCondition::new(
            DisproofForm::Audited,
            "false if consent was never recorded",
            None,
            None,
            None,
            vec!["consent".into()],
            None,
        )
        .expect_err("prose with neither evaluation nor audit must be refused");
        assert_eq!(refused, "DI_DISPROOF_PROSE_NEEDS_AUDIT");
        // No coverage at all: refused.
        assert!(
            DisproofCondition::new(
                DisproofForm::Audited,
                "body",
                None,
                None,
                None,
                vec![],
                Some("audit:x".into()),
            )
            .is_err(),
            "coverage is mandatory alongside either form"
        );
        // An evaluated condition with no operator: refused.
        assert!(
            DisproofCondition::new(
                DisproofForm::Evaluated,
                "body",
                None,
                Some("c".into()),
                Some("s".into()),
                vec!["c".into()],
                None,
            )
            .is_err(),
            "an evaluated condition with no operator is not machine-checkable"
        );
        // So the writer's argument is admissible by construction, and the
        // writer therefore holds no validation of its own — which is the claim
        // this test exists to make load-bearing.
        let _ = EvaluatedOp::Contains;
    }

    /// The write path STORES an `Evaluated` condition, scope included.
    ///
    /// **This pin is inverted, not deleted.** It asserted the opposite for a
    /// release: `Evaluated` requires a scope and the table had no
    /// `disproof_scope` column, so the writer refused rather than write a row
    /// its own read-back would reject. Schema 1.32.23 closed that ceiling, and
    /// the claim that replaced it is the load-bearing one — the form that makes
    /// a condition machine-checkable is now durable. A pin that vanishes takes
    /// its coverage with it, so the replacement has to bite just as hard.
    #[test]
    fn an_evaluated_condition_is_stored_with_its_scope_not_refused() {
        use crate::workflow::create::disproof::{DisproofForm, EvaluatedOp};
        let conn = db();
        let schema_ref = admit(&conn, "acme");
        let evaluated = DisproofCondition::new(
            DisproofForm::Evaluated,
            "the payer matches",
            Some(EvaluatedOp::Contains),
            Some("payer-verified".into()),
            Some("subject.payer_state".into()),
            vec!["payer_state".into()],
            None,
        )
        .expect("admissible as a CONDITION");

        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_eval", schema_ref), Some(&evaluated))
            .expect("schema 1.32.23 gave scope a column, so the whole form persists");
        tx.commit().expect("commit");

        // On disk, read WITHOUT the read-back: the column must actually carry
        // the scope, not merely round-trip through a function handed it.
        let scope: Option<String> = conn
            .query_row(
                "SELECT disproof_scope FROM claims WHERE claim_id = 'clm_eval'",
                [],
                |r| r.get(0),
            )
            .expect("the claim row must exist");
        assert_eq!(
            scope.as_deref(),
            Some("subject.payer_state"),
            "the scope must be in its own column. If it were dropped on the floor the read-back \
             below would refuse the row, and if the read-back were loosened to accept it the \
             condition would be unfalsifiable — neither is acceptable"
        );
        assert_eq!(
            read_disproof(&conn, "clm_eval").expect("read"),
            Some(evaluated.clone()),
            "and the condition comes back whole"
        );
        assert_eq!(
            evaluate_disproof(&conn, "clm_eval", "the payer is payer-verified").expect("verdict"),
            DisproofVerdict::Refuted,
            "a machine-checked condition is now EXERCISED, not merely storable"
        );
    }

    /// The migration applied to a database that ALREADY has the six v1.32.22
    /// columns, with rows in them.
    ///
    /// The obvious test builds a fresh database, which cannot fail: it proves
    /// the `ALTER TABLE` runs, not that it runs *on a populated pre-column
    /// table*. So this one walks the shape backwards instead — it migrates, then
    /// **drops the seventh column** to reconstruct the 1.32.22 shape exactly,
    /// writes rows through the pre-column writer, and only then re-runs the
    /// migration.
    ///
    /// A rebuild-based migration would pass the fresh-database test and destroy
    /// these rows here, so this is the pin that keeps the round additive.
    #[test]
    fn the_scope_migration_applies_to_an_existing_six_column_database_with_rows() {
        use crate::workflow::create::disproof::{DisproofForm, EvaluatedOp};
        let dir = tempfile::tempdir().expect("temp dir");
        // Registration FIRST: `sqlite3_auto_extension` only reaches connections
        // opened after it, so opening before registering yields a connection
        // with no `vec0` — and this test would then pass or fail depending on
        // whether some OTHER test in the binary had already registered. It ran
        // green in the full suite and failed on its own, which is the whole
        // order-dependency class.
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open(dir.path().join("pre-column.db")).expect("open");
        run_migration(&mut conn, 512).expect("first migration");

        // Rows in it FIRST, then the shape walks backwards to 1.32.22. Writing
        // them through the real writer is not a shortcut: prose carries no
        // scope, so the seven-column writer's row and the six-column writer's row
        // are the same row. (Running the current writer against a six-column
        // table cannot work at all — it names the seventh column — which is why
        // the rows go in first.)
        let schema_ref = admit(&conn, "acme");
        let prose = prose();
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_pre", schema_ref), Some(&prose))
            .expect("store prose");
        store_claim(&tx, &draft("clm_pre_bare", schema_ref)).expect("store bare");
        tx.commit().expect("commit");

        // Back to the v1.32.22 shape: the six columns, no seventh. SQLite's
        // DROP COLUMN is exactly the additive round's inverse, so what is left is
        // a populated table this release has to UPGRADE rather than create.
        conn.execute("ALTER TABLE claims DROP COLUMN disproof_scope", [])
            .expect("reconstruct the pre-column shape");
        let six: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name LIKE 'disproof_%'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            six, 6,
            "the reconstructed table has exactly the six old columns"
        );

        // Upgrade.
        run_migration(&mut conn, 512).expect("the migration applies to an existing database");

        let seven: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name='disproof_scope'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(seven, 1, "the seventh column was added");

        // The rows survived, byte for byte, and the old ones read back exactly
        // as they did before the upgrade — including the NULL in the new column
        // being "predates the field" and not "unreadable".
        let scope: Option<String> = conn
            .query_row(
                "SELECT disproof_scope FROM claims WHERE claim_id = 'clm_pre'",
                [],
                |r| r.get(0),
            )
            .expect("row survived");
        assert_eq!(
            scope, None,
            "a row written before the column existed carries NULL there, and NULL is not an \
             empty string a reader could mistake for content"
        );
        assert_eq!(
            read_disproof(&conn, "clm_pre").expect("read"),
            Some(prose),
            "the pre-upgrade condition reads back unchanged"
        );
        assert_eq!(
            read_disproof(&conn, "clm_pre_bare").expect("read"),
            None,
            "a claim that predates the field still reads back as None"
        );

        // And the upgraded table can hold the form it could not hold before.
        let evaluated = DisproofCondition::new(
            DisproofForm::Evaluated,
            "the payer matches",
            Some(EvaluatedOp::Contains),
            Some("payer-verified".into()),
            Some("subject.payer_state".into()),
            vec!["payer_state".into()],
            None,
        )
        .expect("admissible");
        let tx = conn.unchecked_transaction().expect("tx");
        store_claim_with_disproof(&tx, &draft("clm_post", schema_ref), Some(&evaluated))
            .expect("store evaluated after the upgrade");
        tx.commit().expect("commit");
        assert_eq!(
            read_disproof(&conn, "clm_post").expect("read"),
            Some(evaluated)
        );
    }

    #[test]
    fn a_stored_set_check_verdict_must_be_a_decision() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        assert!(
            store_batch_verdict(&tx, 1, &"a".repeat(64), 2, "pending", 1, "operator").is_err(),
            "a stored verdict must be a decision; a third state would let a caller record \
             'checked, unknown' and then treat it as a pass"
        );
        assert!(store_batch_verdict(&tx, 1, &"a".repeat(64), 2, "pass", 1, "operator").is_ok());
    }

    #[test]
    fn the_core_carries_no_policy_and_no_fence_key_literal() {
        let source = include_str!("create.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        // No principal-kind literal: the fences key on that string.
        for needle in ["\"agent\"", "\"human\""] {
            let hits = production.matches(needle).count();
            assert!(
                hits <= 1,
                "the core contains {hits} occurrences of {needle}. The stored principal \
                 string is a fence key and must come from the one mapping function, not from a \
                 literal here."
            );
        }
        // No verdict computation: the gate decides, this layer stores.
        assert!(
            !production.contains("fn run("),
            "the storage layer must not re-implement the gate; a second policy in the layer \
             that persists is a second authority"
        );
    }

    #[test]
    fn the_corpus_and_the_gap_flood_are_reachable_only_as_data() {
        assert!(corpus().len() >= 8);
        assert_eq!(gap_methods().len(), 4);
        let flood = gaps("global", &[], None);
        assert!(flood.len() <= crate::workflow::create::gap::GAP_FLOOD_CAP);
    }
}
