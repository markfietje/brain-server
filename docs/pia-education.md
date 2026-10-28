# PIA — the education vertical (FERPA / COPPA / EU AI Act Annex III)

> A Data Protection Impact Assessment **mapping**, in plan 28's method:
> every duty below names the tree surface that owns it and the pin that
> proves it. It is a documented engineering posture, not a certification —
> the same standing as [compliance.md](compliance.md). Written by the C3
> round (2026-10-28) against the surfaces as they land; each section cites
> its proof test by name.

## The system in one paragraph

A per-tenant, loopback-first memory kernel: an institution runs its own
brain-server per domain (`docs/compliance.md` "What the system is";
per-domain DB isolation is migration law). Learner records enter only by
explicit ingest calls; nothing is inferred, scraped, or profiled. The
education vertical composes the kernel's existing governance surfaces — it
adds no new authority model.

## EU AI Act Annex III — education uses (admission, evaluation) are HIGH-RISK

The plan-28 mapping (§3.2) is favorable because the duties map to features
that already ship:

| Duty | Tree surface | Pin |
|---|---|---|
| Logging | the hash-chained audit chain (`src/audit/`) — every ingest, hold, approval, refusal | `audit/verify` + the per-round audit pins |
| Human oversight | the review queue (`src/handlers/gate.rs` proposals → approve, digest-bound) | the approve-action round's pins; the advisor's posture arm `advisor_writes_ride_the_proposal_queue` |
| Accuracy / testing | the eval harness (plan 08) + calibrated abstention | the eval harness's own target family |
| Supply chain | provenance-sealed binaries (the updater chain) | the III-7 distribution pins |
| Recordkeeping | the Art 30 register projection (`GET /art30`, `src/service/art30.rs`) | the govern handlers' pins |

## FERPA — the education records are the memory

- **Access scope + owner filters**: a learner's records are readable only
  through the data gate — `src/role.rs` `effective_filter`; the educator
  preset (`educator`, C3) is the clinician pattern: own private rows,
  read/write, **no review powers**. Pin:
  `role::tests::the_vertical_presets_staff_the_verticals`; the shared-pool
  absence is recorded on `SHARED_POOL_ROLES` (a cross-owner read of learner
  records is the FERPA isolation this vertical exists to hold).
- **School-official legitimacy**: memory is context, not a system of record
  (the plan-28 refusal below); the education record of authority stays in
  the SIS.
- **DSAR / export / targeted erasure**: the standing DSAR machinery
  (`/dsar` locate → export → purge → tombstone → certificate,
  `src/service/dsar/`) — FERPA amendment requests ride the same
  operator doors; no education-specific path exists or is needed.
- **Retention**: the education profile (`education`, C3) keeps the
  permanent record decay-free (`retention_map()`'s `fact: null`);
  curriculum artifacts and evaluation decisions ride long clocks. Pin:
  `profile::tests::education_holds_the_learner_records_arms`. Jurisdiction
  clocks land as **rules rows** through the C2 door (`brain rules insert`),
  never code — the finance fixture demonstrates the mechanism
  (`plans/brain-shell/c3-edu-fin-fixture/`, private side).
- **Breach**: the breach workflow (`AuditKind::Breach`, the DPO incident
  ledger) mirrors the chain.

## COPPA — consent before collection (under-13): the honest sentence

**The consent artifact door does not exist yet.** The C3 round measured the
mechanism against the escape hatch it stated in advance and took the hatch:
ingest names no data subject (`apply_profile_ingest` carries
title/content/scope/kind — no subject key), and the only consent store
(`consent_registry`) keys domain+subject_hash+channel+purpose with
outreach-vocabulary enums. "Check an artifact at ingest" is therefore a new
ingest seam (wire field + service check + operator CLI door), not the small
gate the round budgeted. The finding is recorded on the **gate-law
register** (`docs/gate-law-register.md`, C3 row) and on the profile itself
(`src/profile.rs`'s `education` preset comment).

What an operator binding `education` gets **today**: strict write-time
masking (witnessed live: `[redacted:email]` stored — the C3 rehearsal),
verbose read auditing, per-educator isolation, the permanent-record
retention shape. What they do **not** get: automated consent-before-
collection. Until the consent round lands, an institution collecting
under-13 data must run consent through the structural door that DOES exist —
the review queue (nothing enters the store without a human act under
`BRAIN_WRITE_POSTURE=review`, `effective_write_posture`) — or not bind the
profile for those collections. This document refuses to dress that gap as a
feature.

## Purpose limitation

`lawful_basis`/`purpose` are **write-once at ingest** (`src/ph.rs`'s
scrape posture + the knowledge columns, `storage_layout.rs`) — learner
profiling for ad-tech cannot re-purpose stored rows, because the purpose
travels with the row and nothing rewrites it.

## What this vertical REFUSES (plan 28 §3.2, written down)

1. **Autonomous evaluation decisions with no human in the loop** — the HITL
   gate law; admissions/evaluation outputs ride the review queue or do not
   land (`effective_write_posture`; the approve action is digest-bound).
2. **Learner profiling for ad-tech purposes** — purpose limitation above;
   the write-once `purpose` column + the profile's kinds vocabulary
   (`IngestError::KindNotAllowed`).
3. **Cross-owner reads of learner records** — the educator carries no
   `shared_pool` and the absence is a recorded reason row
   (`SHARED_POOL_ROLES`), pinned by
   `only_declared_roles_widen_the_owner_fence`.
