# CORDIS_CONTRACT — the normative spec of the SDK's plugin/events/tools surface

Private spec (`docs/private/`, not distributed). Written 2026-09-21. This document is the
matrix's normative reference: `crates/brain-engine-sdk/tests/conformance.rs` pins the laws
below row by row, and its `conformance_all_hooks` check asserts this doc still names every
`@mode`, every frozen `ctx.*` key, and every ceiling in §10 — doc and matrix move together.
Everything here was verified in source on the day it was written; where a claim could not
be verified, the doc says so instead of rounding it up.

## 1. Relationship to Cordis

The SDK's plugin/events/tools surface is a **minimal, hardened reimplementation of the
Cordis/pi shapes** — semantics ported, no code copied, declared in-module (`plugin.rs`
opens: "a minimal Cordis-shaped reimplementation (semantics ported, no code copied)"). The
Cordis kernel is NOT vendored: no `cordis` dependency exists or will enter the tree in this
line. **Vendoring is deferred by choice, not by gap.** The conformance matrix is the gate
that keeps the reimplementation honest and the gate that would catch drift if vendoring
ever happened: every law below is a row that must keep passing against whatever implements
it.

## 2. The `@mode` table

One registry (`Hooks`) owns registration, provenance, and dispatch. Four dispatch modes;
the mode is a property of the DISPATCH call, and a hook registered for one mode never
fires under another (mode isolation is pinned).

| Mode | Registration API | Semantics | Ordering | Panic posture |
| --- | --- | --- | --- | --- |
| `emit` | `Hooks::on` | Broadcast observe. Every registered listener runs; each receives its own clone of the payload. The `Report` records every per-listener outcome (`Ran`/`Denied`/`Panicked`) — the durable per-listener record. | registration order | Contained per listener: a panicking listener is counted as `Panicked` and later listeners ALWAYS run. |
| `waterfall` | `Hooks::on` | Short-circuit policy. Listeners return a verdict; the FIRST denial short-circuits the dispatch (`Err(reason)`), later listeners never run, and the denial cannot be overturned — **monotonic final denial**. | registration order, up to the first deny | A panicking listener is contained and recorded as `Panicked`; it is not a denial and does not stop the dispatch. |
| `serial` | `Hooks::on_mutate` | Ordered mutations: each listener receives `&mut` shared state and applies its mutation. | registration order (strict — the order IS the semantics) | Contained per listener; the mutation chain continues. |
| `parallel` | `Hooks::on_parallel` | Fan-out jobs: each job gets an independent clone of the payload; outcomes aggregate in registration order so reports stay deterministic. | aggregation in registration order | Contained per job. |

Provenance is sidecar metadata keyed by hook id (`Hooks::provenance(id)`); registration
validates loud (empty event or provenance refuses at registration).

## 3. The `ctx.*` keys (frozen ABI)

Typed services are provided and required by concrete type; the KEY is the stable wire
name returned by `Service::key()`. The production key set is CLOSED — exactly these:

| Key | Service | Face |
| --- | --- | --- |
| `ctx.evidence` | `EvidenceSvc` | claim grouping / dedup / contradiction surfacing (delegates to the pure evidence core) |
| `ctx.scoring` | `ScoringSvc` | the scorer family (score_run, classify_cause, override_rate, gap_decision, scoreboard — thin delegates to the pure core) |
| `ctx.systemPrompt` | `SystemPromptSvc` | bounded system-prompt builder + compaction-pressure policy (delegates to `prompt`) |
| `ctx.sandbox` | `SandboxSvc` | the isolation face that is ALWAYS denied today: `require_sandbox()` returns the named `SandboxUnavailable` denial — never a local stand-in; `allows()` delegates exactly to the capability ladder |
| `ctx.workflowEngine` | the engine slot | NOT a `Service`: the context's ONE workflow engine (`Option<Arc<dyn WorkflowEngine>>`), mounted via `Context::mount_workflow_engine` |

Laws: the keys are **frozen ABI** — renaming or removing a key is a breaking release, and
the catalog check fails on any rename/removal before it breaks an engine. A duplicate
claim (same key via `install`, same type via `provide`) fails loud with
`KernelError::Duplicate` — never a silent overwrite. `services::install` mounts exactly
`ctx.evidence` + `ctx.scoring`. A missing service is a loud `KernelError::NotMounted`,
never a default.

## 4. Mount / HMR semantics

- **install**: `inject` dependencies are enforced BEFORE any mount side effect runs
  (missing dependency refuses, nothing half-mounted); duplicate keys refuse. Audited
  variants (`install_audited`/`uninstall_audited`) write `Workflow` audit rows — a FAILED
  mount leaves its denial on the chain.
- **The effect stack**: `Context::effect` records a reversible registration; the returned
  `EffectHandle` reverses it on `dispose` or drop. Reversal is strict reverse order
  (newest first). The effect is removed from the registry BEFORE its undo closure runs
  (and outside the lock), so an undo that panics cannot re-run or double-count.
- **`reload` (HMR for services)**: unmount then remount the SAME instance — the unload
  half is reversible by construction (effect stack), the remount re-claims registrations.
  Zero residual effects: the undo count equals the mount count at every point in the
  swap, ending with exactly the remounted set.
- **`uninstall` and panicking unloads**: the service is removed from the mounted registry
  BEFORE `unmount()` runs; if the unload hook panics, the key is ALREADY unmounted and the
  service's own `EffectHandle`s reverse during the unwind. A panicking unload therefore
  still reverses — containment by construction, not by catch.
- **The engine slot (`ctx.workflowEngine`)**: ONE engine per context. Mounting REPLACES
  the previous engine (config-driven replacement) — **never parallel providers**. The slot
  has no unload hook: replacement IS the slot's hot-swap.
- **`mounted_keys()`** reports the exact mounted set in install order — the leak probe the
  matrix uses.

## 5. Session minimization

`SanitizedSession` is the ONLY session handle extensions ever see. The raw bytes have
exactly one consumer: the INJECTED sanitizer — there is no method that can return
unsanitized content (misalignment is unrepresentable in the type). Hosts inject their own
redaction posture (PII mask, invisible-strip, markdown-ref strip); the SDK enforces only
the seam. Source errors propagate untouched; clean content passes verbatim. Posture
framing: LLM06 sensitive-information disclosure; GDPR Art 5/6 data-minimization — an
engineering posture, not a legal conclusion.

## 6. Trust and capability (the laws the matrix composes)

- `ExtensionPolicy::decide` precedence: per-engine deny > global deny > per-engine allow >
  global allow > mode fallback. **Deny wins everywhere**; even `Permissive` honors an
  explicit deny. `exec`/`env` are hard-denied in the standard profile (the container is
  the boundary, not a popup).
- `HostCallKind` is a closed vocabulary: an unknown wire class is an ERROR, never a
  guessed default — the dispatcher turns unknown into denial.
- `capability::allows` is the monotonic fail-closed posture ladder (Safe ⊂ Standard ⊂
  Permissive; anything unlisted is denied); `checked_dispatch` audits denials too — a
  denial can never bypass the chain silently.
- Composition law: a denial from the waterfall or the ladder is FINAL — a later allow
  cannot overturn it.

## 7. Tools

The `workflow` tool (`create_workflow_tool`) is bound to ONE engine. Input format
`name\ndescription\nscript`; malformed input is REFUSED BEFORE the engine starts (a
refused tool never produces a run). The dispose guard is the `finally` half of
try/finally: the run is disposed on every path, including panics. A run ending
non-`completed` surfaces as a tool error so the model sees the failure. Awaiting is
bounded (`TOOL_AWAIT_GRACE`). Ceiling: the abort bridge observes only the cooperative
cancel flag — no exec-signal channel exists in this surface.

## 8. The agent lifecycle

Phases: `Idle`/`Running`/`Compact`/`Failed`. Structural operations (compact, set_leaf_id,
tree navigation) require `Idle` — a structural op mid-turn refuses `PhaseBusy`; steering
and abort stay legal mid-turn. `TurnToken` is unforgeable (`Arc` identity + generation):
stale, cross-harness, or retained tokens from earlier turns refuse live — validation
happens under the caller's held lock. Turn-generation overflow refuses (checked increment
BEFORE any host work — no wrap, no audit written, no phase change; pinned by the
same-module test, cited by the matrix). `finish_turn`/`abort_turn` consume the token under
checked settlement. Pending session writes drain FIFO, strictly after the `message_end`
persistence that triggered the flush.

## 9. The Stores analogue (`workflow_state`)

Four frozen routing keys (`status`, `pending_question`, `next_step`, `next_state`) with
precedence terminal > pending question > step > advance > fallthrough `Done`; the key set
may only GROW, never reshape (a rename is a breaking release). `PublicStatus` is a closed
six-word customer vocabulary. The context window: `EventRow` chain → derivation is pure —
latest checkpoint at-or-before the anchor + delta after it + FNV-1a findings digests +
open question. Appending events never changes an earlier window (**prefix stability** —
consumers may cache derived slices). Field budget: the OLDEST delta events drop first
until the budget fits; the `truncated` flag marks exactly the drops; the checkpoint, the
notes (digests), and the open question are never truncated away. FNV-1a is a
fingerprint for dedupe, NOT a security primitive; `count_fields` is a documented
one-field-≈-one-token approximation.

## 10. Honest ceilings

1. **Single-process kernel.** Cross-process HMR, nested-fiber lifecycles, and remote
   session transport are out of scope in this line.
2. **No CBOR anywhere in 1.32.x.** This tree carries ZERO CBOR code, crate, or lock entry
   (verified tree-wide on 2026-09-21). The remote CBOR boundary is **experimental** and
   **deferred to 2.0 Cortex**. This corrects the v1.32.6 contract's sentence "CBOR lives
   in `crates/bench` harness only" to the verified truth: the bench harness carries no
   CBOR today either — that sentence was aspirational, and stating it as fact would have
   been a fabrication. The boundary stays explicit and unimplemented, by choice.
3. **The trust layer gates the hostcall boundary only** — it is not a sandbox for hostile
   code running inside an engine.
4. **The abort bridge is cooperative** (see §7).
5. **FNV-1a is not cryptographic**; `count_fields` is an approximation (see §9).

## 11. The anchor

`crates/brain-engine-sdk/tests/conformance.rs` (`conformance_all_hooks`) fails if this
document loses the `@mode` names (emit, waterfall, serial, parallel), any frozen key
(`ctx.evidence`, `ctx.scoring`, `ctx.systemPrompt`, `ctx.sandbox`,
`ctx.workflowEngine`), or the §10 ceiling markers ("single-process", "No CBOR anywhere in
1.32.x", "experimental", "2.0 Cortex", "deferred by choice"). The doc and the matrix move
together or the gate is red.

## 12. The 1.32.7 stamp — Diagnostic Closure (2026-09-21)

The loop line's 1.32.7 label stamps here, in the in-tree doc that names the line's
versioned state. 1.32.7 shipped in two preregistered parts, both recorded against
hash-pinned preregs with executed-command evidence:

- **Part 1 — TreeHandoff (R17):** the gajae session tree as `harness::tree`, the pi
  admission vocabulary as `harness::compaction`, the GDL handoff pipeline (lifecycle
  named + audited, HITL at generated, persist asymmetry), and the CETS 225 stamp
  correction.
- **Part 2 — Diagnostic Closure (R18):** the triage duty (ESI/MTS acuity + the red-flag
  forcing function with the monotonic escalate-first lock and the per-domain must-miss
  catalog), the closure artifact (NAM 2015 step 6 as gate law: A8/A9, mandatory before
  any resolution), and the back-referral return contract (B1, atomic rows, the
  escalation exception law, the overdue HITL discipline) — the three re-slotted gaps
  landing as typed artifacts through the EXISTING gate machinery, additive
  `#[serde(default)]` only, zero new dependency edges, zero `unsafe`.

Kernel gates at close: **1744/1768/1772/1751** (R17 floors 1720/1744/1748/1727 plus
exactly the 24 new spawn-free kernel tests, up-only). The 1.32.8 System-One lane remains
**opener-gated — not prebuilt**: its gates (the labeled corpus + the separately
preregistered eval) stay the operator's standing inputs, as do the κ labeling round,
τ²-bench, and the live configured case for any ship decision. One operator action item
rides the close: the foreign-operator WIP file `src/handlers/case_run.rs` carries
pre-1.32.7 fixture copies whose route tests need the new mandatory triage fields
re-synced (the round left the file untouched by rule; the plain case_run lane is the
only red readout and is owned by that re-sync). This stamp is a version-label record,
not an evaluation result, release, or compliance claim.
