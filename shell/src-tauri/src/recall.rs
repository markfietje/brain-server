//! P1 — the recall wire's authz/bounds boundary and its projector.
//!
//! Division of labour (Plan 04 §1): this file owns BOUNDS and the reading of
//! the kernel's answer. It owns no transport: the webview holds the only wire
//! (`shell/src/lib/api/client.ts`, pinned loopback, no http capability in
//! `capabilities/main.json`), and the Swift probe holds no URL at all. That is
//! deliberate and is what makes "no Swift path reaches the kernel except the
//! core" a structural fact rather than a convention.
//!
//! It reads the kernel's ACTUAL response bytes, not a re-serialization by the
//! caller. One reader of the contract, and the `decision` enum is checked
//! against the published vocabulary rather than trusted.
//!
//! Two fields of Plan 04 §2's shape are deliberately ABSENT, and their absence
//! is the point:
//!
//! * `owner` — `RecallHit` carries no owner. The core would have to invent it,
//!   and inventing an authz fact is the one thing a boundary must not do.
//! * `chain_head` — likewise not on the wire. Inventing provenance is the same
//!   defect wearing a different hat.
//!
//! `budget {window, used, remaining}` is absent for a third reason: the model
//! window is the Swift layer's measured quantity, and a core that guessed it
//! would report a budget it never enforced.

use serde::{Deserialize, Serialize};

/// The kernel's own ceilings, at the kernel's own values. `src/handlers/mod.rs`
/// declares `MAX_QUERY = 2_000`, `MIN_LIMIT = 1`, `MAX_LIMIT = 100` and
/// `DEFAULT_RECALL_LIMIT = 5`, and `handlers/recall.rs` REFUSES outside
/// `MIN_LIMIT..=MAX_LIMIT` rather than clamping. These mirror that law so the
/// core never mints a request the kernel will reject, and so a kernel-side
/// change has one visible literal on this side to move with it.
pub const MAX_QUERY_BYTES: usize = 2_000;
pub const MIN_LIMIT: u32 = 1;
pub const MAX_LIMIT: u32 = 100;
pub const DEFAULT_LIMIT: u32 = 5;

/// A domain label is an isolation key, not prose. The kernel stores it as a
/// short label; 64 bytes is generous for one and keeps a caller from turning a
/// filter into a payload.
pub const MAX_DOMAIN_BYTES: usize = 64;

// ─────────────────────────────────────────────────────────────────────
// The gate: what the core will authorize
// ─────────────────────────────────────────────────────────────────────

/// What the caller asks for.
///
/// `deny_unknown_fields` is the authz half of this file. There is deliberately
/// no field through which a principal could arrive: the caller's own bearer
/// rides the webview's existing middleware, and the core can neither hold,
/// substitute nor fabricate one. The consequence is that a caller who tries to
/// smuggle `authorization` (or `token`, or `principal`) into the request is
/// REFUSED rather than quietly ignored — a silently-dropped field is how a
/// synthesized principal rides into production looking like the real one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallQuery {
    pub query: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// The exact body the core authorizes: a `/recall` `QueryDoc` with only the
/// fields the shell is allowed to set. `provenance` is TRUE and not optional —
/// `telemetry.confidence` (the kernel's only confidence number on this route)
/// is present only when provenance is requested, so a client that omitted it
/// would render an abstention with no number attached. Plan 04 §2 lists a bare
/// `confidence: f32` and does not say where it comes from; this is where.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecallDoc {
    pub v: u32,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub limit: u32,
    pub provenance: bool,
}

/// A refusal to mint. Every arm is a bound, and every arm is loud: the core has
/// no silent fallback and no "send it anyway".
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum RecallRefusal {
    QueryEmpty,
    QueryTooLong { bytes: usize, max: usize },
    LimitOutOfRange { requested: u32, min: u32, max: u32 },
    DomainTooLong { bytes: usize, max: usize },
    DomainNotALabel,
}

impl core::fmt::Display for RecallRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecallRefusal::QueryEmpty => write!(f, "query is empty"),
            RecallRefusal::QueryTooLong { bytes, max } => {
                write!(f, "query is {bytes} bytes, ceiling {max}")
            }
            RecallRefusal::LimitOutOfRange {
                requested,
                min,
                max,
            } => {
                write!(f, "limit {requested} outside {min}..={max}")
            }
            RecallRefusal::DomainTooLong { bytes, max } => {
                write!(f, "domain is {bytes} bytes, ceiling {max}")
            }
            RecallRefusal::DomainNotALabel => {
                write!(f, "domain must be a single non-empty whitespace-free token")
            }
        }
    }
}

/// The authorized request plus the bounds that produced it. `echo` is the
/// effective query/domain actually authorized, so a caller can see the bound
/// that fired instead of guessing which one it was.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthorizedRecall {
    pub doc: RecallDoc,
    pub query_bytes: usize,
}

#[tauri::command]
pub fn recall_memory(request: RecallQuery) -> Result<AuthorizedRecall, RecallRefusal> {
    authorize(request)
}

/// The bounds law, as a pure function so it can be pinned without a kernel.
pub fn authorize(request: RecallQuery) -> Result<AuthorizedRecall, RecallRefusal> {
    let bytes = request.query.len();
    if bytes == 0 {
        return Err(RecallRefusal::QueryEmpty);
    }
    if bytes > MAX_QUERY_BYTES {
        return Err(RecallRefusal::QueryTooLong {
            bytes,
            max: MAX_QUERY_BYTES,
        });
    }
    if let Some(domain) = request.domain.as_deref() {
        if domain.len() > MAX_DOMAIN_BYTES {
            return Err(RecallRefusal::DomainTooLong {
                bytes: domain.len(),
                max: MAX_DOMAIN_BYTES,
            });
        }
        if domain.is_empty() || domain.split_whitespace().count() != 1 {
            return Err(RecallRefusal::DomainNotALabel);
        }
    }
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT);
    // Inclusive at MAX_LIMIT: the kernel's own range is `MIN_LIMIT..=MAX_LIMIT`
    // and it REFUSES outside it rather than clamping, so an exclusive bound
    // here would refuse a request the kernel serves.
    if !(MIN_LIMIT..=MAX_LIMIT).contains(&limit) {
        return Err(RecallRefusal::LimitOutOfRange {
            requested: limit,
            min: MIN_LIMIT,
            max: MAX_LIMIT,
        });
    }
    Ok(AuthorizedRecall {
        doc: RecallDoc {
            v: 1,
            query: request.query,
            domain: request.domain,
            limit,
            provenance: true,
        },
        query_bytes: bytes,
    })
}

// ─────────────────────────────────────────────────────────────────────
// The projector: what the kernel said
// ─────────────────────────────────────────────────────────────────────

/// One `/recall` exchange as the webview observed it: the status and the
/// response bytes, verbatim. Status is part of the input because the REFUSAL
/// taxonomy is a function of it — a 401 and an empty 200 are different facts,
/// and whoever holds the bytes is the only place both are visible at once.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireExchange {
    pub status: u16,
    pub body: String,
}

/// The kernel's `decision`, verbatim from the published enum
/// (`openapi.yaml` `/recall` 200: `enum: [ok, low_confidence]`).
///
/// The spelling is the wire's, not Plan 04 §2's — that plan writes
/// `"answered" | "low_confidence"`, and the published contract says `ok`. A
/// client-side third spelling would be a contract lie dressed as a
/// convenience, so this enum renames nothing: `Ok` serializes to `ok`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Ok,
    LowConfidence,
}

/// One passage. `valid_from`/`valid_to` are lifted out of the hit's `evidence`
/// object, where the wire actually keeps them: presenting a fact whose
/// `valid_to` has passed as current is the failure this kernel exists to
/// prevent (Plan 06 §2), so validity is a first-class field here rather than
/// something a renderer has to go looking for.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Passage {
    pub id: i64,
    pub title: Option<String>,
    pub text: String,
    pub snippet: Option<String>,
    pub domain: Option<String>,
    pub score: f64,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub flagged: bool,
    pub ingest_kind: Option<String>,
    pub memory_kind: Option<String>,
    pub authority: Option<f64>,
}

/// What was actually searched — the shell's own echo of the authorized request
/// plus the kernel's cross-domain disclosure. `included_global` rides here
/// because `openapi.yaml` states cross-domain mixing is "visible, never
/// silent": a caller handed a global row inside a domain-routed answer has to
/// be able to see that it happened.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Searched {
    pub query: String,
    pub domain: Option<String>,
    pub limit: u32,
    pub domains_searched: Vec<String>,
    pub included_global: Option<bool>,
}

/// An answer, of either confidence. `decision` is NOT optional and has no
/// default anywhere in this file: an outcome that cannot say how sure it is
/// is not an outcome, it is a bug with a 200 on it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallAnswer {
    pub decision: Decision,
    pub passages: Vec<Passage>,
    pub confidence: Option<f32>,
    pub searched: Searched,
}

/// A refusal to answer. `status` is kept because "403" and "401" are different
/// operator actions, and `scope` is kept because Plan 04 §3's law is that an
/// absent authority renders as a refusal — never as an empty list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Refused {
    pub status: u16,
    pub scope: &'static str,
}

/// A kernel that did not answer. Plan 04 §5's law: degraded recall with a
/// clear banner, NEVER a silent empty result. This is its own state precisely
/// so it cannot be drawn as "no results".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Unreachable {
    pub status: u16,
    pub detail: String,
}

/// The core's classification of one exchange. Six states, and the reason there
/// are six is that Plan 04 §5 names five while the wire admits a sixth:
///
/// * `Answered` — `decision: ok` with at least one passage.
/// * `Absent` — `decision: ok` with none. An ABSENCE, named as one.
/// * `Abstained` — `decision: low_confidence`. An answer that says "I could not
///   find that", and emphatically not an error.
/// * `Refused` — 401/403. Confidentiality: this must never render like
///   `Absent`, because "you may not read this" and "there is nothing here"
///   are different facts about the reader.
/// * `Unreachable` — any other non-2xx, plus an unparseable 2xx.
/// * `ContractViolation` — a 2xx whose body does not match the published
///   contract. It is not `Unreachable`: the kernel answered, and the answer was
///   unreadable, which is a fact worth surfacing rather than a transport error.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecallOutcome {
    Answered(RecallAnswer),
    Absent(RecallAnswer),
    Abstained(RecallAnswer),
    Refused(Refused),
    Unreachable(Unreachable),
    ContractViolation(Unreachable),
}

impl RecallOutcome {
    /// The one question a renderer must be able to ask without inspecting
    /// fields: did we get passages? Note that this is deliberately NOT the
    /// question "was it refused?" — an abstention also has no passages, and
    /// the two must stay separable.
    pub fn has_passages(&self) -> bool {
        matches!(self, RecallOutcome::Answered(answer) if !answer.passages.is_empty())
    }

    /// True only for the state that must never be drawn as "no results".
    pub fn is_refusal(&self) -> bool {
        matches!(self, RecallOutcome::Refused(_))
    }

    pub fn decision(&self) -> Option<Decision> {
        match self {
            RecallOutcome::Answered(a) | RecallOutcome::Absent(a) | RecallOutcome::Abstained(a) => {
                Some(a.decision)
            }
            _ => None,
        }
    }
}

// ── the wire shapes, read exactly as the kernel writes them ──────────

#[derive(Debug, Deserialize)]
struct WireEvidence {
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireHit {
    id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: String,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    evidence: Option<WireEvidence>,
    #[serde(default)]
    flagged: bool,
    #[serde(default)]
    ingest_kind: Option<String>,
    #[serde(default)]
    memory_kind: Option<String>,
    #[serde(default)]
    authority: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WireTelemetry {
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct WireRecall {
    #[serde(default)]
    hits: Option<Vec<WireHit>>,
    /// REQUIRED, with no `#[serde(default)]`: a 200 without it is a contract
    /// violation and lands in `ContractViolation`, not in a best-effort answer.
    decision: Decision,
    #[serde(default)]
    domains_searched: Option<Vec<String>>,
    #[serde(default)]
    included_global: Option<bool>,
    #[serde(default)]
    telemetry: Option<WireTelemetry>,
}

impl From<WireHit> for Passage {
    fn from(hit: WireHit) -> Passage {
        let (valid_from, valid_to) = match hit.evidence {
            Some(evidence) => (evidence.valid_from, evidence.valid_to),
            None => (None, None),
        };
        Passage {
            id: hit.id,
            title: hit.title,
            text: hit.content,
            snippet: hit.snippet,
            domain: hit.domain,
            score: hit.score,
            valid_from,
            valid_to,
            flagged: hit.flagged,
            ingest_kind: hit.ingest_kind,
            memory_kind: hit.memory_kind,
            authority: hit.authority,
        }
    }
}

/// The IPC surface of [`shape_recall`]. Tauri commands take owned arguments,
/// so the borrow stays inside the core and the classifier below stays a pure
/// function — which is the only reason its six arms are pinnable at all.
///
/// The caller hands the authorized doc BACK, so the `searched` echo can report
/// what was really sent. That is a hole unless it is closed HERE: a caller
/// could otherwise hand back a doc the core never authorized, and the echo
/// would report a request that was never made. So the doc is re-authorized
/// against the same bounds before it is trusted, and one that would not pass
/// the gate is refused rather than echoed.
#[tauri::command]
pub fn shape_recall_command(
    exchange: WireExchange,
    requested: RecallDoc,
) -> Result<RecallOutcome, RecallRefusal> {
    authorize(RecallQuery {
        query: requested.query.clone(),
        domain: requested.domain.clone(),
        limit: Some(requested.limit),
    })?;
    Ok(shape_recall(exchange, &requested))
}

/// Reads one exchange and classifies it. Pure: no I/O, so every arm below is
/// pinnable without a running kernel.
pub fn shape_recall(exchange: WireExchange, requested: &RecallDoc) -> RecallOutcome {
    match exchange.status {
        401 | 403 => {
            return RecallOutcome::Refused(Refused {
                status: exchange.status,
                scope: "recall",
            });
        }
        status if !(200..300).contains(&status) => {
            return RecallOutcome::Unreachable(Unreachable {
                status,
                detail: "the kernel answered with a non-success status".to_string(),
            });
        }
        _ => {}
    }

    let wire: WireRecall = match serde_json::from_str(&exchange.body) {
        Ok(wire) => wire,
        Err(error) => {
            return RecallOutcome::ContractViolation(Unreachable {
                status: exchange.status,
                detail: format!("the recall body is not the published contract: {error}"),
            });
        }
    };

    let passages: Vec<Passage> = wire
        .hits
        .unwrap_or_default()
        .into_iter()
        .map(Passage::from)
        .collect();
    let answer = RecallAnswer {
        decision: wire.decision,
        passages,
        confidence: wire.telemetry.and_then(|telemetry| telemetry.confidence),
        searched: Searched {
            query: requested.query.clone(),
            domain: requested.domain.clone(),
            limit: requested.limit,
            domains_searched: wire.domains_searched.unwrap_or_default(),
            included_global: wire.included_global,
        },
    };

    match answer.decision {
        Decision::LowConfidence => RecallOutcome::Abstained(answer),
        Decision::Ok if answer.passages.is_empty() => RecallOutcome::Absent(answer),
        Decision::Ok => RecallOutcome::Answered(answer),
    }
}
