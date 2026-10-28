# PIA — the finance vertical (MiFID II / FINRA / GDPR / EU AI Act Annex III)

> A Data Protection Impact Assessment **mapping**, in plan 28's method:
> every duty below names the tree surface that owns it and the pin that
> proves it. It is a documented engineering posture, not a certification —
> the same standing as [compliance.md](compliance.md). Written by the C3
> round (2026-10-28) against the surfaces as they land; each section cites
> its proof test by name.

## The system in one paragraph

A per-tenant, loopback-first memory kernel: an advisory practice runs its
own brain-server per domain; advisor notes and KYC **context** (summaries,
attributes) live in the advisor's own private memory. The memory is
**context, not a ledger** — that refusal is structural, below.

## Data-class discipline — the deny-list (C3)

Plan 28 §3.3: never credentials, never card numbers, never full government
IDs. The `finance-mifid` profile (C3) is the kernel's first **refuse-at-
ingest** posture: `deny_patterns: ["pan","iban"]` — a Luhn-valid PAN and
IBAN-shaped identifiers are **refused at ingest naming the class**, before
any masking, through the single funnel (`ingest_one`) every write path
crosses.

| Arm | Pin |
|---|---|
| PAN refused naming the class | `service::ingest::tests::the_deny_list_refuses_under_the_finance_profile_only` |
| IBAN refused naming the class | same pin |
| A Luhn-failing digit string passes (the false-positive arm) | same pin |
| The delta IS the profile: standard mode byte-unchanged, deny-list-free strict mode still masks | same pin + `profile::tests::finance_mifid_carries_the_deny_list_and_validation_is_closed` |
| The refusal is audited (kind ingest, status denied, the class named — never the content) | `handlers::ingest::tests::the_deny_list_refusal_arm_audits_before_the_400`; witnessed live in the C3 rehearsal |

The vocabulary is **closed** (`DENY_PATTERNS`); `validate()` refuses an
unknown class fail-closed. Witnessed live 2026-10-28: finance → 400
`deny_list_refused`, support (standard) → 200, the audit row `denied`
(`plans/brain-shell/c3-edu-fin-fixture/`, private side).

## MiFID II / FINRA retention × GDPR Art 17 — the tension, encoded as data

- **The clocks are rule rows, not code.** The MiFID II client-records clock
  (5y/7y, Art 16(7)/Art 72), the FINRA books-and-records clock (6y,
  17a-4/4511), and the FINRA communications clock (3y, 2210) land through
  the **C2 rules door** (`brain rules insert`, `src/service/rules.rs`) —
  jurisdiction DATA on the private side, one audit row per insert, revised
  only forward. Witnessed live (the C3 rehearsal, 2026-10-28).
- **Holds suspend erasure lawfully.** `POST /legal-hold`
  (`src/legal_hold.rs`) freezes rows; the retention sweep honors holds —
  measured, not assumed: `src/service/lifecycle/decay.rs` filters held ids
  before the arbiter ("a held id never shows up in the decay registry"),
  and the supersession/replace sweeps gate on `refuse_if_held` (the R2-02
  pin). **Witnessed live**: a decayed row is listed by `/decayed` → a hold
  empties the listing → release resumes the clock (the C3 rehearsal §4).
- **Art 17 erasure** rides the standing DSAR machinery (`/dsar` locate →
  export → purge → tombstone → certificate); a hold refuses the purge for
  the rows it freezes, which is the lawful suspension the clock rows
  state.
- **The mechanical backstop** is the profile's retention map (7-year shape,
  `finance-mifid`), applied at query time by the retriever — never a
  sweeper (`src/handlers/govern.rs`'s own header states it; the human
  purges).

## EU AI Act Annex III — creditworthiness scoring is HIGH-RISK

The duty mapping is education's table (audit chain, review queue, eval
harness, provenance-sealed binaries, the Art 30 register) — see
[pia-education.md](pia-education.md) for the row-per-duty mapping. The
finance-specific boundary: **the answer surface never presents a generated
number as a decision.** The chat spine's tier/citation/abstention posture
is the control — a generated answer carries its tier and citations, and
calibrated abstention withholds answers the evidence does not support
(the model-tiering + GDL grounding rounds' pins; the citation requirement
is pinned in the chat layer's contract tests).

## Art 50 transparency

The chat spine's tier/citation posture (above) plus the public notices
(`/.well-known/ai-notice`, `ai-literacy`, `cop-notice`) carry the
system's self-description; the notices are held to one truth by a
doc-truth pin (`docs_truth.rs`).

## What this vertical REFUSES (plan 28 §3.3, written down)

1. **The memory as a system of record** — it is context; the ledger of
   authority stays in the books-and-records system. The retention clocks
   above exist so the memory's contents survive as long as the law
   requires — not so it can replace the record system.
2. **Cross-client pooling** — per-domain isolation is the migration law;
   an `advisor` preset that reads the shared pool IS the pooling the plan
   refuses, so the capability's absence is a recorded reason row on
   `SHARED_POOL_ROLES`, pinned by `only_declared_roles_widen_the_owner_fence`.
3. **Any write path that bypasses the review queue** — made structural in
   C3: the `advisor`'s writes propose on every seam
   (`REVIEW_POSTURE_ROLES` in `effective_write_posture`; pin:
   `handlers::tests::advisor_writes_ride_the_proposal_queue`), and the
   deny-list's refusals mean the forbidden data classes cannot enter by
   any path at all.
