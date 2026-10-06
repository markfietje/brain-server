# Domain engine cores

The workspace node in `crates/Cargo.toml` hosts one decision core per
workflow domain. This page is the doc home for the five cores that had
none — `brain-aftersales-core`, `brain-care-core`,
`brain-interview-core`, `brain-evidence-core`, `brain-evolve-core` —
plus short usage pointers for the three cores
[Engine SDK](engine-sdk.md) already covers thinly
(`brain-consensus-core`, `brain-executor-core`,
`brain-troubleshoot-core`). Read that page first for the ABI contract
(`pure` / `policy` / `host`); nothing here repeats it.

Rule of the house, repeated because it is load-bearing: each core is
pure decision logic. The server owns the transaction, the audit row,
and the route. A model proposes; only the gate disposes — including
delivery (see [Architecture](architecture.md)).

Historical notes below are era-pinned to `CHANGELOG.md` versions and
dates. Present-tense facts (crate versions, APIs, constants) are read
from the manifests and sources cited per section. Where `CHANGELOG.md`
names no release for a crate, that is stated rather than filled in.

## brain-aftersales-core (`crates/brain-aftersales-core`, 1.28.33)

**What it is.** The fulfillment-gate core for return / warranty /
repair runs: entitlement → window → disposition, over the same
gate/waterfall shape `brain-troubleshoot-core` obeys, with the
fulfillment domain's own artifact vocabulary. Source:
`crates/brain-aftersales-core/src/lib.rs`, `crates/brain-aftersales-core/src/gates.rs`, `crates/brain-aftersales-core/src/disposition.rs`, `crates/brain-aftersales-core/src/evidence.rs`.
Depends only on `brain-troubleshoot-core` + `serde`
(`Cargo.toml:11-12`). Landed as a crate in v1.28.32 (2026-08-26,
"Frontdesk": care + aftersales crates introduced) and gained its
disposition ranker in v1.28.33 (2026-08-26, "Returns").

**What it decides.** `fulfillment_waterfall(has_entitlement_row,
within_window, disposition_is_proposal)` (`crates/brain-aftersales-core/src/lib.rs:19`) runs the
three gates in order and the first rejection is THE answer:
`G_ENTITLEMENT` ("no governed entitlement row grants coverage") beats
`G_WINDOW` ("outside its legal window") beats `G_DISPOSITION` ("a
disposition must be a HITL proposal, never an auto-execution").
`run_waterfall` (`crates/brain-aftersales-core/src/gates.rs:35`) is first-rejection-wins by
construction. `rank_dispositions(&DispositionInput)`
(`crates/brain-aftersales-core/src/disposition.rs:103`) ranks the four candidates deterministically
(same input → same ordering, rank descending, ties by stable kind
name): `ReturnForInspection`, `ReplaceFirst`, `ReturnlessRefund`,
`Deny`. Every candidate cites a closed basis from the anchor table —
`BASIS_WARRANTY_REPLACE` (`2019/771-art.13(2)`),
`BASIS_WITHDRAWAL_REFUND` (`2011/83-art.16`),
`BASIS_GOODWILL_REFUND` (`goodwill-policy`),
`BASIS_INSPECTION_CLAUSE`, `BASIS_FRAUD_SCHEDULE` — never free text.
`FraudSignals::score()` (`crates/brain-aftersales-core/src/disposition.rs:60`) composites
repeat-return rate (halved) + serial mismatch (×3000) + window abuse
(×2000), clamped to 0..=10000. Two named thresholds:
`FRAUD_REVIEW_THRESHOLD_UNITS` (5000) forces fraud review on the
returnless path; `HARD_ESCALATION_UNITS` (9000) escalates every
candidate to the human. A serial mismatch zeroes the returnless rank
(the goods' identity is unproven, so they come back).

**What it proves.** Nothing executes here. Dispositions are HITL
proposals; the gates decide only whether a proposal may exist, and
fraud signals inform — they never autonomously deny. Evidence is cited
by locator + digest through `EvidenceRef { evidence_type, locator,
digest, captured_at }` over five types (`ProofOfPurchase`,
`DiagnosticBundle`, `SerialBatch`, `Photos`, `InspectionReport`;
`EvidenceType::all()` has exactly 5). The `diagnostic_bundle` string
is shared with troubleshoot-core's vocabulary (pinned in
`crates/brain-aftersales-core/src/lib.rs` tests).

**How to use / verify it.** There is no HTTP route on this crate and
no root `Cargo.toml` path edge at HEAD (measured with `rg` over
`src/` and `Cargo.toml`: the aftersales KPI cohort the server does
read flows through `brain_engine_sdk::aftersales` in
`src/workflow/scoreboard.rs` / `src/handlers/workflow.rs`, not through
this crate). Treat it as a library core until a server caller lands:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-aftersales-core
```

**Honest limits.** No server caller, no proposal-table write path, no
dedicated read API at HEAD; the fraud-signal inputs
(`returnless`/`fraud_flagged` state flags) are reserved vocabulary no
run writer populates yet (stated in v1.28.33's own engineering
record). Financial execution never happens here by design, not by
accident of scope.

## brain-care-core (`crates/brain-care-core`, 1.28.33)

**What it is.** A thin, worktype-typed facade over
`brain-interview-core` — inquiry and account-change dialogs with ZERO
new concepts (the crate's own words, `crates/brain-care-core/src/lib.rs:1-4`). Source:
`crates/brain-care-core/src/lib.rs`, `crates/brain-care-core/src/dialog.rs`. Depends only on
`brain-interview-core` (`Cargo.toml:11`). Introduced in v1.28.32
(2026-08-26, "Frontdesk") alongside the aftersales crate.

**What it decides.** Almost nothing of its own — that is the point.
`CareDialog::open(kind)` (`crates/brain-care-core/src/dialog.rs:19`) admits exactly the
closed vocabulary `CARE_KINDS = ["care_inquiry", "account"]` and
refuses anything else loudly (`not_a_care_worktype: {kind}`). An
opened dialog owns a `DraftStore` (`drafts()`); ambiguity scoring,
drafts, and revision-conflict repair are interview-core's machinery
re-exported verbatim (`pub use brain_interview_core::{ambiguity,
draft, repair, state}`, `crates/brain-care-core/src/lib.rs:14`).

**How to use / verify it.** Open a dialog for a care worktype, drive
it with the interview-core functions below, close it. Like its sibling
above it has no `src/` caller and no root path edge at HEAD — a
library core awaiting a server seam:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-care-core
```

**Honest limits.** The 80-line vertical buys vocabulary binding and
nothing else; any claim that care dialogs "reason" beyond
interview-core's math is false. Unknown worktypes deny rather than
degrade, so a renamed intake kind fails closed here until the table is
updated deliberately.

## brain-interview-core (`crates/brain-interview-core`, 1.27.32)

**What it is.** The deep-interview state machine: ambiguity scoring,
revision-guarded answering, drafts, deterministic inspection, and
repair-as-a-mode. Source: `crates/brain-interview-core/src/state.rs`, `crates/brain-interview-core/src/ambiguity.rs`,
`crates/brain-interview-core/src/draft.rs`, `crates/brain-interview-core/src/repair.rs`, `crates/brain-interview-core/src/recorder.rs`,
`crates/brain-interview-core/src/payload.rs`, `crates/brain-interview-core/src/inspect.rs` (plus `crates/brain-interview-core/src/lib.rs` re-exports).
The SDK dependency is the crate's declared ABI contract, intentionally
ahead of the code (`Cargo.toml:18-22`). The crate fill is recorded
under v1.27.32 (2026-08-21); the empty scaffold predates it in v1.27.29
(2026-08-21, "Survey": five intentionally-empty crates).

**What it decides.** Whether an interview may advance, and how
ambiguous it still is:

- **State + revision CAS** (`crates/brain-interview-core/src/state.rs`): `initialize_context`,
  `confirm_topology`, `record_answer`, `apply_round_result` all take an
  `expected_rev`; a stale revision is `DI_STATE_REVISION_CONFLICT`.
  One topology per interview (`DI_TOPOLOGY_CONFLICT`); duplicate round
  ids refuse (`DI_ANSWER_LIFECYCLE_CONFLICT`); scoring applies only to
  `answered` / `pending_scoring` rounds (`DI_ROUND_RESULT_CONFLICT`,
  `DI_ROUND_NOT_FOUND`).
- **Ambiguity** (`crates/brain-interview-core/src/ambiguity.rs`): `weighted_ambiguity_units`
  (greenfield: 3 scores at 40/30/30; brownfield: 4 scores at
  35/25/25/15; anything else is `DI_INVALID_ARGUMENT`),
  `compute_ambiguity_floor` (`min(10000, disputed*1000 +
  unscored*500 + auto_ratio/20)`), `clamp_reported` (the reported
  value never prints below the floor), `derive_milestone` (`Ready` at
  or under threshold, else `Initial` / `Progress` / `Refined` at the
  6000 / 3000 breaks).
- **Drafts** (`crates/brain-interview-core/src/draft.rs`): `DraftStore::{create, update, get}` with
  revision CAS (`DI_DRAFT_REVISION_CONFLICT`), missing drafts
  (`DI_DRAFT_NOT_FOUND`), and a 1-hour TTL (`expires_at = now + 3600`,
  `DI_DRAFT_EXPIRED`).
- **Repair is a mode, not a fork** (`crates/brain-interview-core/src/repair.rs`): the same `DI_*`
  conflict vocabulary, the same revision CAS, the same floor govern the
  repair path exactly as the answering path.
- **Recorder, payloads, inspection**: `recorder::verify_and_apply`
  refuses past 3 auto-answered rounds; `payload::{parse_question,
  parse_answer, parse_result}` are `deny_unknown_fields` parses with
  `DI_INVALID_*_JSON` refusals; `inspect::{summary, pending}` are
  deterministic, digest-pinned reads.

**What it proves.** That no answer, score, topology, or draft lands
without winning its revision race, and that reported ambiguity cannot
be talked below its floor. It does not prove the questions are good,
the scores are fair, or the facts are true — those arrive from outside
the core.

**How to use / verify it.** The persistence adapter alongside it is
`src/workflow/interview.rs` (interview-step outbox writes); the core
itself currently has no `src/` path-dependency edge at HEAD, so drive
it as a library:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-interview-core
```

**Honest limits.** Hard output caps in `validate_limits`
(`crates/brain-interview-core/src/state.rs:68-81`): serialized envelope over 24 KiB or more than
64 rounds refuses with `DI_OUTPUT_LIMIT_EXCEEDED`. The recorder's
auto-answer ceiling (3) is a tripwire, not a policy argument. Payloads
are shape-checked, never semantically checked.

## brain-evidence-core (`crates/brain-evidence-core`, 1.29.0)

**What it is.** The byte-range evidence resolver: does a claim's cited
span resolve to exactly these bytes? Pure, total, I/O-free — no clock,
no store, no network, no model (`crates/brain-evidence-core/src/lib.rs:1-11`). Sole dependency is
`sha2 0.11`, already locked by four sibling cores, so the crate adds
zero new packages (`Cargo.toml:11-28`). `CHANGELOG.md` carries no
named release entry for this crate; 1.29.0 is the manifest version,
not an era claim. The authoritative boundary doc lives in the crate's
own `crates/brain-evidence-core/src/lib.rs:13-79` — this section is the map, not a second copy.

**What it decides.** `resolve(source, refs)` (`crates/brain-evidence-core/src/lib.rs:206`)
returns one of six closed verdicts (`EvidenceVerdict`): `Resolved`,
`UnresolvedSource`, `CidMismatch`, `QuoteMismatch`,
`RangeOutOfBounds`, `EmptyEvidence`. `Resolved` requires every ref to
pass both comparisons — `hash(source_bytes[range]) == hash(quote)`
AND `source_cid == CID(source_bytes)` — with a fixed by-cause
precedence, never by ref order: empty → unresolved-source →
out-of-bounds → CID → quote (`crates/brain-evidence-core/src/lib.rs:190-205`). `failure_cause`
and `describe` (`crates/brain-evidence-core/src/lib.rs:157-181`) publish the five snake_case
reporting strings; adding a variant is a compile error in every
consumer by exhaustiveness. `cid_v1` (`crates/brain-evidence-core/src/cid.rs:79`) mints
`sha256:` + base32(`0x12 0x20` + digest) in the kernel's lowercase
RFC-4648 alphabet; `is_well_formed_cid` is a shape check (prefix,
length `CID_ENCODED_LEN`, alphabet), never a decode. Caller-side
bounds, published not enforced (a pure function cannot be flooded):
`MAX_QUOTE_BYTES` (64 KiB), `MAX_REFS` (256).

**What it proves — and what it does not.** It proves byte-identity at
declared offsets against a caller-committed CID. It does not prove the
quote supports the claim, that the claim is true, that a contradiction
was noticed (each ref resolves independently; semantic contradiction
needs typed disjoint predicates plus, where those do not hold, a model
— stated in `crates/brain-evidence-core/src/lib.rs:27-32`), or that the source was admitted by a
trusted writer at a known time. The CID guarantee is conditional on
the CID being committed at admit time: a caller that recomputes the
CID from the bytes it passes in compares `h(x)` to `h(x)`, and no
check inside the crate can tell the difference — which is why `resolve`
takes the CID as caller-supplied and the crate offers no constructor
that derives one (pinned by
`r46_resolver_never_derives_a_cid_from_the_bytes_it_was_handed`).

**How to use / verify it.** The live callers are the create loop's
gate: `src/workflow/create/verify.rs:426-456` (one `resolve` call per
citation; anything but `Resolved` is `Refusal::EvidenceUnresolvable`)
with CIDs minted by `brain_evidence_core::cid_v1` at
`src/workflow/create/verify.rs:594` and
`src/workflow/create/promote.rs:213`. Verify with the crate battery,
or the full spike report (corpus is private; a public-only checkout
prints a loud `NOT RUN`, never a silent pass):

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-evidence-core
cargo test --manifest-path crates/Cargo.toml -p brain-evidence-core --test r46_spike -- --nocapture
```

Background: [Create loop](create-loop.md), [API reference](api.md).

**Honest limits.** Byte-exact means byte-exact — no case folding, no
trimming, no char-boundary snapping; a containment ("range holds the
quote") still refuses. The bytes are caller-supplied and not yet
guaranteed stable (the admitted-bytes store is future work; the crate
docs say so at `crates/brain-evidence-core/src/lib.rs:36-42`). The crate's own docs warn against
feeding it offsets computed by the verify handler (see the skew note
at `crates/brain-evidence-core/src/lib.rs:62-71`, which names `src/handlers/verify.rs:160-161`):
that surface case-folds before computing ranges, so its offsets skew
past non-trivial Unicode — passing them here yields fail-closed false
refusals. The whole claim reduces to SHA-256 collision resistance.

## brain-evolve-core (`crates/brain-evolve-core`, 1.29.2)

**What it is.** The knowledge-version axis core: the per-domain
version bump at publication and the current-version read a case's
`knowledge_version` is recorded against. Extracted verbatim from the
server's `service::gate`; the migration, the handler wiring, the audit
row, and the base-version constant stayed in the server
(`crates/brain-evolve-core/src/lib.rs:1-20`). Sole dependency is `rusqlite 0.40.1 + bundled`,
matching the workspace pin (`Cargo.toml:10-14`). `CHANGELOG.md`
carries no named release entry for the extraction itself; the KCS
lifecycle it rides on shipped in v1.28.23 (2026-08-24, "Evolve"), and
1.29.2 is the manifest version. The crate holds no DDL — the
`knowledge_domain_versions` table is created by the server migration —
and no copy of the base version: the base is a caller-supplied
parameter (`crates/brain-evolve-core/src/lib.rs:11-15`).

**What it decides.** Two functions, inside the caller's transaction:
`bump_article_knowledge_version(conn, article_id, bumped_by, now,
base)` (`crates/brain-evolve-core/src/lib.rs:62`) resolves the domain from the article's own
`knowledge.domain` row (never from a caller argument — a
caller-supplied domain would let one shared counter wear a per-domain
name), upserts `knowledge_domain_versions` (`base + 1` on first
publication, `version + 1` after), and returns the new current
version; `current_domain_knowledge_version(conn, domain, base)`
(`crates/brain-evolve-core/src/lib.rs:103`) reads it, returning `base` (never 0) when the
domain has no row. `EvolveError::Display` carries the exact pre-move
message text so the server's internal-error mapping is unchanged.

**What it proves.** That a publication moved its domain's basis
exactly once, atomically with the state change it moves. Monotonic by
construction AND by definition: the KCS machine has a backward edge
(`retract`: published → approved), so the bump reads current and
writes current+1 and callers invoke it on publication only — a
retraction must never bump, or a reopened case would be told its basis
moved when the world reverted. Cross-domain comparison is meaningless
by construction; a stored version is comparable to its own domain's
current version and nothing else.

**How to use / verify it.** The server calls both functions: the
publish branch bumps inside the same transaction at
`src/handlers/gate.rs:931` (base passed as
`crate::config::KNOWLEDGE_BASE_VERSION`, `Database`→`Database` mapped
at the call site to avoid a dependency cycle), and case-open stamps
the read at `src/workflow/state.rs:247` (`NULL` keeps its "predates
tracking" meaning):

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-evolve-core
```

The DB-behavioural pins (two domains bumping independently,
monotonicity, retract-does-not-bump, the rollback twin) live
server-side, where the migration lives. Background:
[Architecture](architecture.md) (Evolve row),
[Changelog](CHANGELOG.md) (v1.28.23, 2026-08-24).

**Honest limits.** The crate cannot create its own table, cannot name
the server's error type, and cannot stop a caller from invoking the
bump on retract — the publish-only discipline is enforced at
`src/handlers/gate.rs:924`, not in the core. A domain that never
published reads as `base`, which is honest only while readers honor
the `NULL`-means-untracked contract.

## Short pointers: consensus, executor, troubleshoot

These three are described in [Engine SDK](engine-sdk.md) (with the
machine-checked crate map `engine_sdk_crate_map_is_accurate`); what
follows is usage only, complementing that page.

**`brain-consensus-core` (`crates/brain-consensus-core`, 1.27.29).**
The agreement core: `Artifact::new` (identity is content,
`sha256(content)`), `Review`/`Verdict`, `advance` (capped at
`MAX_ITERATIONS` = 5, fails closed to `Stuck`), `review_join_gate`
(≥2 reviews, one artifact, distinct non-empty reviewers),
`approval_gate` (the single may-execute predicate), `stage_writer`
(total-or-refused: a kind-count mismatch is a named refusal, never a
shorter receipt), `intent_reconciliation`. The delivery phase pass
calls it directly; the typed artifact on
`POST /workflow/delivery/runs/{id}/advance` projects onto the shipped
`Artifact` type (`src/workflow/delivery.rs:148-168`, pinned by
`delivery_typed_artifact_is_a_shipped_type`). Crate fill recorded
under v1.27.33 (2026-08-21). Verify:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-consensus-core
```

Ceiling: decides agreement only — no persistence, no signatures, no
execution, no host contact.

**`brain-executor-core` (`crates/brain-executor-core`, 1.27.29).**
The checkpointed-execution core: `Goal`/`parse_brief`, the
`CheckpointGate` JSON validator (`validate_gate_json` refuses unknown
keys at both levels and demands live-surface evidence — `gui` | `cli`
| `native` | `api` | `algorithm` with a non-empty receipt — unless
top-level `replay_exempt`), `RunState` with the named critic ceiling
(`CRITIC_CEILING` = 5, fifth non-okay pauses), `requires_delegation`
(files ≥ 3, lines ≥ 200, or parallel), `artifact_hash` (sha256 hex;
`src/workflow/releases.rs:158` prefixes it `sha256:`). Consumed on the
delivery phase pass: the build-phase gate runs `validate_gate_json`
before anything is written, and artifact digests ride
`artifact_hash` (`src/workflow/delivery.rs:154-177,1228`). The
v1.29.2 (2026-09-26, "Engines") record is the era pin for the wiring
and the four disclosed fixes (declared-no-op `apply_steering`,
ceiling off-by-one, nested-`replayExempt` false promise,
`stage_writer`-style silent drop in the sibling core). Verify:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-executor-core
```

Ceilings, both pinned: `apply_steering` is a declared infallible
no-op over all six `SteeringKind` values (a round needing real
steering must change the signature deliberately), and
`Goal`/`parse_brief` is the scope engine the design owner assigns to
the delivery phase pass rather than to the interview crate.

**`brain-troubleshoot-core`
(`crates/brain-troubleshoot-core`, 1.27.38).** The universal diagnostic
loop core: `kernel` (step budget `MAX_STEPS_PER_TURN` = 24,
`MAX_STEPS_CEILING` = 1000, steering queue 100 drop-oldest, 3 pause
continuations; `RunState`/`Turn`/`Step`/`SteeringInbox`),
`gates` (nine `GateId`s; `gate_evidence`, `gate_one_variable` — one
mutation per step — `gate_corroborate` — ≥2 supporting lines —,
`gate_bundle`, `gate_approval`, `run_waterfall`), `advisor`
(rate-capped, deduped, disables after 3 consecutive failures; only
`Blocker` pauses), `evidence` (8 artifact types + `VendorProfile`),
`subagents` (`MAX_PARALLEL_TASKS` = 8 reads; mutations strictly
serial, one per step; strict JSON schema check). Shipped in v1.27.38
(2026-08-21). Its live consumer is the reference harness
(`tools/steward-harness/src/engine.rs:17-19`), not `src/` — which is
exactly why [Engine SDK](engine-sdk.md) lists it as Filled *with a
disclosed gap*: decision core with callers, zero tests. Verify:

```sh
cargo test --manifest-path crates/Cargo.toml -p brain-troubleshoot-core
```

(expect a green run over an empty battery — that emptiness is the
finding, not a pass).

## Ceilings and limits (all eight cores)

- **Islands, named.** At HEAD, `src/` wires evidence-core
  (`src/workflow/create/verify.rs`, `src/workflow/create/promote.rs`),
  evolve-core (`src/handlers/gate.rs`, `src/workflow/state.rs`), and
  consensus/executor-core (`src/workflow/delivery.rs`,
  `src/workflow/releases.rs`); the root `Cargo.toml` carries path
  edges for exactly those four (plus delivery-core, evidence of the
  same law). Interview, care, aftersales, and troubleshoot cores have
  no `src/` caller and no root edge — library cores with in-crate
  batteries (troubleshoot-core: not even that). Building a route or
  caller for any of them is a wiring decision with its own review, not
  a discovery that one already exists.
- **Pure means unprivileged.** No core opens a database it does not
  receive, reads a clock it is not handed (evidence-core reads none at
  all), emits an audit row, or reaches a model. A consumer that
  normalises inputs before calling is invisible to the core and can
  only ever produce refusals, never false acceptances (evidence-core
  states this at `crates/brain-evolve-core/src/lib.rs:72-76`).
- **Refusals are the product.** Every core fails closed: closed
  vocabularies, deny-loud unknowns, first-rejection-wins waterfalls,
  fixed by-cause precedence. A gate that refuses everything is not a
  gate — but neither is a core with a permissive arm, and none here
  has one.
- **What no core proves.** Question quality, score fairness, fact
  truth, source trustworthiness, contradiction across independently
  resolving refs, cross-domain version comparability, or anything
  about bytes the caller never showed it. Those are caller, schema,
  admission, and governance obligations — recorded here so they are
  not rediscovered as bugs.
- **History without a pin is not claimed.** Crate manifest versions
  above are file facts; release eras are cited only where
  `CHANGELOG.md` names them (v1.27.29 / v1.27.32 / v1.27.33 /
  v1.27.38 — all 2026-08-21; v1.28.32 / v1.28.33 — 2026-08-26; v1.29.1
  / v1.29.2 — 2026-09-26; v1.28.23 — 2026-08-24). The evidence-core
  and evolve-core extractions have no named `CHANGELOG.md` entry, and
  no date is asserted for either.
