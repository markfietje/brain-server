//! II-7a — `POST /verify`: the core's bounds, and its reading of the answer.
//!
//! Division of labour is `recall.rs`'s, unchanged: this file owns BOUNDS and
//! the classification of what the kernel said. It owns no transport. The
//! webview holds the only wire, and the core cannot hold, substitute or
//! fabricate a principal — there is no field through which one could arrive.
//!
//! ## THE AUTHZ SURFACE THIS OPENS, and what a caller can now do that it could not
//!
//! `/recall` was already reachable, so this command opens no NEW reach — it
//! opens a new SHAPE of the reach, and the shape is a content-confirmation
//! oracle. Plan 06 §2's `[correct]` affordance asks "does this claim appear in
//! the chunk that produced this citation", and the kernel answers it
//! deterministically (`handlers/verify.rs::verify_claim`, case-insensitive
//! substring match, no model, no network).
//!
//! What a caller can now do that it could not before, stated plainly because a
//! new command with no stated threat model is a refactor until it says
//! otherwise:
//!
//! 1. **Read a chunk's CONTENT one claim at a time, and learn whether a chosen
//!    substring occurs in it.** Before this, recall returned the content and
//!    the caller could read it; now a caller can turn that content into a
//!    membership oracle over candidate strings without the content ever being
//!    echoed back. That is a real capability, so the route's own gates matter:
//!    the kernel re-authorizes the ROW's `(owner, access_scope)` on top of the
//!    header-resolved domain (`record_read_gate` + `can_read_domain`), and it
//!    misses rather than leaks.
//! 2. **Probe chunk ids.** The core's `ChunkIdNotPositive` arm exists because a
//!    caller with a list of guessed ids gets a 404 for every one, and a 404 is
//!    the kernel's DELIBERATE blur between "no such row" and "a row your record
//!    gate does not admit". This module must not un-blur it: the 404 arm is
//!    named for what the kernel actually said.
//! 3. **Get a `403` that must never render as "that claim is not supported".**
//!    This is the route's confidentiality law and it is the reason the outcome
//!    enum has six states rather than three. A citation the reader may not
//!    read, and a citation that does not contain the claim, are different facts
//!    about the reader and about the world; collapsing them turns a permission
//!    boundary into an information channel.
//!
//! ## WHY THE CLAIM BOUND IS CHARS-AND-AFTER-TRIM (and why bytes would be a bug)
//!
//! `handlers/verify.rs` does, in this order:
//!
//! ```text
//! let claim = req.claim.trim().to_string();
//! if claim.is_empty() { … claim_empty … }
//! if claim.chars().count() > MAX_QUERY { … claim_too_long … }
//! ```
//!
//! `MAX_QUERY` is `2_000` (`src/handlers/mod.rs:88`). So the kernel's ceiling is
//! measured in **characters, on the trimmed string** — not bytes, and not on
//! what arrived. `recall.rs` measures BYTES on the raw string, because
//! `handlers/recall.rs` measures bytes on the raw string; the two commands have
//! two different laws and mirroring recall's here would refuse claims the kernel
//! serves.
//!
//! Three consequences this file's bounds inherit, each of which is a pin:
//!
//! * A claim of 2_000 multibyte characters is LEGAL (up to 8_000 bytes) and a
//!   byte-measuring core would refuse it.
//! * `claim.chars().count()` is a `chars()` walk, not `len()`. `MAX_CLAIM_CHARS`
//!   is a CHAR ceiling and the core's own counter is `chars()`.
//! * `"   "` (whitespace only) is `claim_empty`, not a 2_000-char claim: the
//!   trim precedes the emptiness check on both sides.
//!
//! ## WHY `chunk_id > 0` IS A BOUND AND NOT AN INVENTED ONE
//!
//! `knowledge.id` is `INTEGER PRIMARY KEY` (`src/migration.rs:111`), i.e. the
//! rowid. Every production INSERT omits the column — `service/domains_admin.rs`
//! writes `(title, content, source, content_hash, domain)` — so the rowid
//! sequence starts at 1 and no ingest path can mint 0 or a negative id. The one
//! site that writes `id` explicitly is `service/dsar.rs`'s own test fixture,
//! and it writes 1, 2, 3.
//!
//! So a non-positive id cannot resolve, ever. Refusing it at the boundary
//! converts a guaranteed round-trip-and-404 into a named refusal with no wire
//! traffic. This is the refusal-not-clamp law doing its job in the direction
//! that saves a call, and it is documented here as a shell-side bound because
//! the kernel states none — a bound with no stated provenance is the thing this
//! file exists not to be.

use serde::{Deserialize, Serialize};

use crate::recall::{Refused, Unreachable};

/// The kernel's own claim ceiling, at the kernel's own value and in the
/// kernel's own unit: `MAX_QUERY = 2_000` CHARACTERS of the trimmed claim
/// (`src/handlers/mod.rs:88`, enforced at `handlers/verify.rs:76`).
pub const MAX_CLAIM_CHARS: usize = 2_000;

/// The kernel's cap on reported match ranges. `handlers/verify.rs:29` declares
/// `MAX_MATCH_RANGES = 100` and STOPS collecting there — further matches are
/// neither reported nor allowed to change `supported`. The core cannot refuse
/// the kernel's response, so this is not a mint bound: it is a CONTRACT bound,
/// and a body carrying more ranges than the kernel can emit is a
/// `ContractViolation` rather than a `Supported`.
pub const MAX_MATCH_RANGES: usize = 100;

/// The smallest addressable row id, per the rowid argument in the module header.
pub const MIN_CHUNK_ID: i64 = 1;

// ─────────────────────────────────────────────────────────────────────
// The gate: what the core will authorize
// ─────────────────────────────────────────────────────────────────────

/// What the caller asks for.
///
/// `deny_unknown_fields`, for `RecallQuery`'s reason: a silently-dropped field
/// is how a synthesized principal rides into production looking like the real
/// one. This route's body is `{chunk_id, claim}` and NOTHING else — the kernel's
/// own `VerifyRequest` has exactly those two fields, so every additional key is
/// either an error or an invention.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    pub chunk_id: i64,
    pub claim: String,
}

/// The exact body the core authorizes. No `v`, no `domain`, no auth material:
/// `openapi.yaml`'s `/verify` requestBody is `required: [chunk_id, claim]` and
/// nothing else, and the kernel deserializes exactly this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifyDoc {
    pub chunk_id: i64,
    /// TRIMMED, because that is what the core bounded. The kernel trims again
    /// (idempotent), and echoing the raw string while claiming to have bounded
    /// the trimmed one would make the bound a description rather than a fact.
    pub claim: String,
}

/// A refusal to mint. Every arm is a bound, and every arm is loud: there is no
/// silent clamp and no "send it anyway".
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum VerifyRefusal {
    /// Empty AFTER trimming — the kernel's `claim_empty`, at the same place in
    /// the same order (`handlers/verify.rs:69-75`).
    ClaimEmpty,
    /// Over `MAX_CLAIM_CHARS` characters of the trimmed claim.
    ClaimTooLong { chars: usize, max: usize },
    /// Non-positive, and therefore unresolvable (the rowid argument above).
    ChunkIdNotPositive { chunk_id: i64 },
}

impl core::fmt::Display for VerifyRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VerifyRefusal::ClaimEmpty => {
                write!(
                    f,
                    "claim is empty once trimmed — the kernel refuses it as claim_empty"
                )
            }
            VerifyRefusal::ClaimTooLong { chars, max } => {
                write!(f, "claim is {chars} characters, ceiling {max}")
            }
            VerifyRefusal::ChunkIdNotPositive { chunk_id } => write!(
                f,
                "chunk_id {chunk_id} cannot resolve: knowledge ids are rowids minted from 1"
            ),
        }
    }
}

/// The authorized request plus the bound that produced it, so a caller can see
/// which bound fired instead of guessing — the same disclosure `recall.rs`
/// makes with `query_bytes`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthorizedVerify {
    pub doc: VerifyDoc,
    pub claim_chars: usize,
}

#[tauri::command]
pub fn verify_claim_command(request: VerifyRequest) -> Result<AuthorizedVerify, VerifyRefusal> {
    authorize(request)
}

/// The bounds law, as a pure function so it is pinnable without a kernel.
pub fn authorize(request: VerifyRequest) -> Result<AuthorizedVerify, VerifyRefusal> {
    if request.chunk_id < MIN_CHUNK_ID {
        return Err(VerifyRefusal::ChunkIdNotPositive {
            chunk_id: request.chunk_id,
        });
    }
    // Trim FIRST, then measure in CHARS. See the module header: the kernel does
    // exactly this, and a byte measurement here would refuse legal multibyte
    // claims.
    let claim = request.claim.trim().to_string();
    let chars = claim.chars().count();
    if chars == 0 {
        return Err(VerifyRefusal::ClaimEmpty);
    }
    if chars > MAX_CLAIM_CHARS {
        return Err(VerifyRefusal::ClaimTooLong {
            chars,
            max: MAX_CLAIM_CHARS,
        });
    }
    Ok(AuthorizedVerify {
        doc: VerifyDoc {
            chunk_id: request.chunk_id,
            claim,
        },
        claim_chars: chars,
    })
}

// ─────────────────────────────────────────────────────────────────────
// The projector: what the kernel said
// ─────────────────────────────────────────────────────────────────────

/// The kernel's `decision`, verbatim from the published vocabulary.
///
/// `openapi.yaml`'s `VerifyResponse` states the code is `"supported"` when
/// `supported` is true and `"unsupported_claim"` otherwise, and
/// `handlers/verify.rs:136` derives one from the other. A third spelling invented
/// here would be a contract lie dressed as a convenience, so this renames
/// nothing: `Supported` serializes to `supported`, `UnsupportedClaim` to
/// `unsupported_claim`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifyDecision {
    Supported,
    UnsupportedClaim,
}

/// The kernel's answer-shaped body, checked for INTERNAL AGREEMENT before it is
/// believed.
///
/// The kernel's invariant is `supported == !match_ranges.is_empty()` with
/// `decision` derived from `supported` (`handlers/verify.rs:131-140`). The core
/// re-checks all three against each other, because a body that violates the
/// invariant is not a verdict the surface may draw — and a surface that drew it
/// would be drawing a contradiction.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireVerify {
    pub chunk_id: i64,
    pub supported: bool,
    pub decision: VerifyDecision,
    #[serde(default)]
    pub match_ranges: Vec<[usize; 2]>,
}

/// An answer, of either verdict. `decision` is present and `ranges` is absent on
/// the unsupported arm — an absence that RENDERS differently, because "the
/// kernel looked and did not find it" and "the kernel returned no ranges" must
/// not look the same as a refusal does.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Verdict {
    pub chunk_id: i64,
    pub decision: VerifyDecision,
    pub supported: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub match_ranges: Vec<[usize; 2]>,
    /// The claim the core authorized and sent, echoed so a receipt states what
    /// was actually checked rather than what the reader assumes.
    pub claim: String,
}

/// The kernel would not serve this chunk.
///
/// NAMED FOR WHAT THE KERNEL SAID, which is deliberately two things at once:
/// `handlers/verify.rs:110-116` returns `Ok(None)` for a foreign row, and
/// `Ok(None)` is also what a miss returns. So this arm does NOT say the id does
/// not exist, and there is no field here a renderer could use to make that
/// claim — un-blurring the kernel's own refusal-to-distinguish would rebuild the
/// enumeration channel the kernel just closed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NoSuchChunk {
    pub chunk_id: i64,
    pub status: u16,
}

/// The core's classification of one `/verify` exchange. Six states.
///
/// The first two are the only ones that are answers, and they are DIFFERENT
/// answers. `Unsupported` is the route working: the kernel read the chunk and
/// the claim is not in it. That is a product feature (Plan 06 §2's `[correct]`
/// exists precisely to catch a hallucinated claim), so it is styled as a result
/// and never as an error.
///
/// The remaining four are the confidentiality law:
///
/// * `Refused` — 401/403. May never be drawn as `Unsupported`.
/// * `NoSuchChunk` — 404, which is the kernel's blur of miss and denial.
/// * `Unreachable` — any other non-2xx, plus an unparseable 2xx.
/// * `ContractViolation` — a 2xx that does not match the published contract, or
///   that contradicts itself. Not `Unreachable`: the kernel answered, and the
///   answer was unreadable, which is worth surfacing rather than calling a
///   transport error.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VerifyOutcome {
    Supported(Verdict),
    Unsupported(Verdict),
    Refused(Refused),
    NoSuchChunk(NoSuchChunk),
    Unreachable(Unreachable),
    ContractViolation(Unreachable),
}

impl VerifyOutcome {
    /// The one question a renderer must be able to ask without inspecting
    /// fields: did the kernel FIND the claim? Deliberately not "was it refused?"
    /// — a refusal and an absence of support are the two states a confused
    /// renderer collapses, and they must stay separable.
    pub fn is_supported(&self) -> bool {
        matches!(self, VerifyOutcome::Supported(_))
    }

    /// True for the states that must never be drawn as "the claim is not in
    /// that chunk". An `Unsupported` is NOT one of them: it is exactly that.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            VerifyOutcome::Refused(_) | VerifyOutcome::NoSuchChunk(_)
        )
    }

    pub fn decision(&self) -> Option<VerifyDecision> {
        match self {
            VerifyOutcome::Supported(v) | VerifyOutcome::Unsupported(v) => Some(v.decision),
            _ => None,
        }
    }

    /// The wire token this arm serializes under.
    ///
    /// It is written out by hand rather than read back from the serializer
    /// because a helper that derived it would be asserting nothing: the pin that
    /// uses it also asserts that each arm's SERIALIZED tag equals this string,
    /// so the two can only agree or the pin is red. That pairing is what makes
    /// the table a bound on the wire rather than a second copy of the enum that
    /// nobody diffs.
    pub fn state_token(&self) -> &'static str {
        match self {
            VerifyOutcome::Supported(_) => "supported",
            VerifyOutcome::Unsupported(_) => "unsupported",
            VerifyOutcome::Refused(_) => "refused",
            VerifyOutcome::NoSuchChunk(_) => "no_such_chunk",
            VerifyOutcome::Unreachable(_) => "unreachable",
            VerifyOutcome::ContractViolation(_) => "contract_violation",
        }
    }
}

/// The IPC surface of [`shape_verify`].
///
/// The caller hands the authorized doc BACK, exactly as `shape_recall_command`
/// does, and that is a hole unless it is closed here: a caller could hand back
/// a doc the core never authorized and the echo would report a request that was
/// never made. So the doc is re-authorized against the same bounds before it is
/// trusted, and one that would not pass the gate is refused rather than echoed.
#[tauri::command]
pub fn shape_verify_command(
    exchange: WireExchange,
    requested: VerifyDoc,
) -> Result<VerifyOutcome, VerifyRefusal> {
    authorize(VerifyRequest {
        chunk_id: requested.chunk_id,
        claim: requested.claim.clone(),
    })?;
    Ok(shape_verify(exchange, &requested))
}

/// The one exchange type, shared with `recall.rs`. Both commands are handed the
/// same `{status, body}` pair and both need status to be part of the input
/// because their refusal taxonomies are functions of it.
pub use crate::recall::WireExchange;

/// Reads one exchange and classifies it. Pure: no I/O, so every arm is pinnable
/// without a running kernel.
pub fn shape_verify(exchange: WireExchange, requested: &VerifyDoc) -> VerifyOutcome {
    match exchange.status {
        401 | 403 => {
            return VerifyOutcome::Refused(Refused {
                status: exchange.status,
                scope: "verify",
            });
        }
        404 => {
            // NOT `Unsupported`. The kernel answered "I will not serve this
            // chunk", and a surface that reads it as "the claim is not in that
            // chunk" has turned a permission decision into a fact about the
            // text. The kernel deliberately does not let these two be told
            // apart (a foreign row and a miss are both `Ok(None)`), so neither
            // does this arm.
            return VerifyOutcome::NoSuchChunk(NoSuchChunk {
                chunk_id: requested.chunk_id,
                status: exchange.status,
            });
        }
        status if !(200..300).contains(&status) => {
            return VerifyOutcome::Unreachable(Unreachable {
                status,
                detail: "the kernel answered with a non-success status".to_string(),
            });
        }
        _ => {}
    }

    let wire: WireVerify = match serde_json::from_str(&exchange.body) {
        Ok(wire) => wire,
        Err(error) => {
            return VerifyOutcome::ContractViolation(Unreachable {
                status: exchange.status,
                detail: format!("the verify body is not the published contract: {error}"),
            });
        }
    };

    // The kernel echoes the id it answered for. A body naming a DIFFERENT chunk
    // is not this request's answer — without this check a stale or misrouted
    // response would attach its ranges to the wrong citation, which is the data
    // -integrity hazard the recall transport already documents.
    if wire.chunk_id != requested.chunk_id {
        return VerifyOutcome::ContractViolation(Unreachable {
            status: exchange.status,
            detail: format!(
                "the kernel answered for chunk {} but {} was requested",
                wire.chunk_id, requested.chunk_id
            ),
        });
    }
    // More ranges than the kernel can emit (`MAX_MATCH_RANGES`), which means the
    // body is not this kernel's body.
    if wire.match_ranges.len() > MAX_MATCH_RANGES {
        return VerifyOutcome::ContractViolation(Unreachable {
            status: exchange.status,
            detail: format!(
                "the verify body carries {} match ranges, above the kernel's {MAX_MATCH_RANGES}",
                wire.match_ranges.len()
            ),
        });
    }
    // The kernel's own invariant, re-checked here:
    //   supported == !match_ranges.is_empty()
    //   decision == (supported ? "supported" : "unsupported_claim")
    // (`handlers/verify.rs:131-140`). A body that breaks either is a
    // contradiction, not a verdict.
    let ranges_present = !wire.match_ranges.is_empty();
    if wire.supported != ranges_present
        || (wire.supported && wire.decision != VerifyDecision::Supported)
        || (!wire.supported && wire.decision != VerifyDecision::UnsupportedClaim)
    {
        return VerifyOutcome::ContractViolation(Unreachable {
            status: exchange.status,
            detail: format!(
                "the verify body contradicts itself: supported={} decision={:?} ranges={}",
                wire.supported,
                wire.decision,
                wire.match_ranges.len()
            ),
        });
    }

    let verdict = Verdict {
        chunk_id: wire.chunk_id,
        decision: wire.decision,
        supported: wire.supported,
        match_ranges: wire.match_ranges,
        claim: requested.claim.clone(),
    };

    if wire.supported {
        VerifyOutcome::Supported(verdict)
    } else {
        VerifyOutcome::Unsupported(verdict)
    }
}
