# Model identity — the four seams with no doc home

**Status:** shipped. This page covers the four identity-adjacent modules that
had no full doc home: `src/model_pin.rs` (boot-time artifact pins),
`src/domain_registry.rs` (per-domain pool registry),
`src/profile.rs` (preset knob bundles bound to a domain), and
`src/reg_watch.rs` (the calendar as code, test-only). The digest-pinned
workflow registry itself — registration rules, decision runs, the replay-gated
promotion — lives in [model-governance.md](./model-governance.md) and is not
repeated here; this page names where that page ends and these seams begin.

Era pins (all dates from `CHANGELOG.md`): **1.28.6 — 2026-08-22**
(fail-closed SHA-256 artifact pinning via `BRAIN_MODEL_MANIFEST`);
**1.21.0 — 2026-08-15** (Profiles: `src/profile.rs`, 12 presets,
`GET /profiles`); **1.28.58 — 2026-09-05** (the calendar as code,
the Enterprise Line opens); **1.29.0 — 2026-09-25** (governed model
identity and decision-run surfaces); current tree **1.29.2 — 2026-09-26**
(`Cargo.toml`).

## 1. What model identity is

A model identity is a **digest-pinned registration**, never a bare name. The
workflow registry row keys on `(id, version)` with a closed kind vocabulary
(`deterministic-rules`, `learned`, `reranker`) and a closed output vocabulary
(`choice`, `score`, `noul`); a learned row must carry its lowercase-64-hex
artifact digest, and `config_digest`, when present, is a second sha256 pin.
The full pinning table — which arm must carry what, and every `400`/`409`
refusal code — is stated in [model-governance.md](./model-governance.md)
(`src/workflow/registry.rs`, `src/handlers/model_registry.rs`) and the route
rows in [api.md](./api.md). This page covers what surrounds that row.

Three layers, three different pins, one rule — **a digest is a pin, not a
signature** (the shell boundary states it verbatim in
`shell/src/lib/model-registry.ts:17-19`):

| Layer | Pin | Where it is checked |
|---|---|---|
| Registry row (governed identity) | `artifact_digest` / `config_digest` on the row; `row_digest` (server-computed SHA-256 over the canonical compact `RegistryRow`) | At register and at decision-run execute time — see [model-governance.md](./model-governance.md) |
| Local artifact files (BYO-ONNX dirs, embed models) | `BRAIN_MODEL_MANIFEST`: path → SHA-256 hex | At boot, fail-closed — `src/model_pin.rs`, `src/server/bootstrap.rs:466-472` |
| Marking on emitted artifacts (Art 50(2) posture) | Signed `AIGEN`/`HUMAN` provenance object | At emission, on four classes — asserted by `src/reg_watch.rs`'s deliverable pin (see §4) |

The registry row never carries weights; the listing carries the artifact
digest as a **presence boolean only** (`artifact_digest_present`), digest
values ride the single-row read (`openapi.yaml:8813-8920`,
`shell/src/lib/model-registry.ts:69-70`).

## 2. The registry lifecycle (where governance ends, this page begins)

Governance owns the lifecycle transitions and the gate. What belongs here is
the shape around it:

- Registration lands as `candidate`. There is **no direct status route**:
  promotion and retirement go through the existing human proposal gate as the
  proposal-only `registry_lifecycle` kind (`{"action":"promote"|"retire",…}`
  binding the exact live `row` + `row_digest` copied unchanged from
  `GET /workflow/model-registry/{model_ref}`), which never becomes a
  knowledge `node_kind` (`openapi.yaml:8921-8929`,
  `shell/src/lib/api/schema.d.ts` on the `registry_lifecycle` content shape).
- The console mirrors the kernel's closed transition table rather than
  re-deciding it: `LIFECYCLE_TRANSITIONS` in
  `shell/src/lib/model-registry.ts:54-61` (`promote` from
  `candidate`/`evaluated`, `retire` from `candidate`/`evaluated`/`promoted`),
  with `evaluation_refs` carried as a bounded list of strings that is never
  treated as a status signal (`model-registry.ts:20-22`).
- The three API routes are `GET /workflow/model-registry` (bounded listing,
  `limit` 1..=50 default 20, optional closed `status`/`kind` filters,
  `operationId: listModelRegistry`, Admin plus DPO role, audited),
  `GET /workflow/model-registry/{model_ref}` (whole-segment `id@version`,
  `400 model_ref_invalid`, probe-blind 404, `operationId:
  getModelRegistryEntry`, Read on global, audited), and
  `POST /workflow/model-registry/register`
  (`operationId: registerModel`, Admin on global, audited)
  (`openapi.yaml:8813-8921`, [api.md](./api.md)).

## 3. Seam one: boot-time artifact pins (`src/model_pin.rs`)

Operator-trusted local files stay trusted only when pinned.
`verify_configured_models()` reads `BRAIN_MODEL_MANIFEST` (a JSON object
mapping file path → SHA-256 hex); `verify_manifest_file()` is the env-free
core. Every listed artifact is verified at boot: a missing file, a hash
mismatch, or a malformed entry **refuses boot** (`src/server/bootstrap.rs`
returns `fatal model manifest`). Absent env is the documented unpinned
posture (`Ok(0)`).

Refusals, each load-bearing: want must be 64 hex chars; a relative entry
containing `..` refuses (`escapes its directory`); on unix a symlinked entry
refuses even when the destination bytes hash correctly (`symlinks are not
pinnable` — `fs::read` follows links, so the pin covers the file itself, not
its destination). Generate the file with `scripts/gen-model-manifest.sh`;
`scripts/install-service.sh` wires it into the service plist; the knob is
documented in [configuration.md](./configuration.md) and the deploy step in
[deployment.md](./deployment.md).

## 4. Seam two: the per-domain pool registry (`src/domain_registry.rs`)

`DomainRegistry` maps a domain name to a SQLite pool. Two modes, one flag:

- **Shim mode (default, `BRAIN_MULTI_DB` off):** every domain resolves to the
  shared global pool — byte-for-byte legacy single-DB behavior. The domain is
  a label, not a boundary (see [architecture.md](./architecture.md)
  multi-domain and [features.md](./features.md)).
- **Multi-db mode (`BRAIN_MULTI_DB=true`):** each non-`global` domain gets its
  own `brain-<domain>.db` file + pool in the same directory as `brain.db`.
  `global` always resolves to the legacy `brain.db`, so an upgrade never
  redistributes existing rows.

Gated creation, the point of the module: `pool_for` resolves only
**registered** names and never creates a file for anything else (`Unknown` —
an unauthenticated-probeable surface cannot fill the disk); `register` is the
one creation path, idempotent, cap-bounded by `BRAIN_MAX_DOMAIN_DBS`
(default 256, `src/config.rs:44`); `seed_registered` adds the name without
opening a pool (boot-time seed, lazy first-open; a fresh boot rescans
`brain-<domain>.db` files). Names are filename-safe by construction
(`^[a-z0-9][a-z0-9_-]{0,62}$`, shared with the handler regex — no separators,
no `..`). Poisoned locks fail closed; `DELETE /domains` clears data but keeps
the file (the audit segment survives) and the registered slot.

## 5. Seam three: profiles — defaults, never primitives (`src/profile.rs`)

A Profile is a typed JSON bundle of **existing** knob defaults
(`default_access_scope`, `pii_mode`, per-kind `retention`, `audit_level`,
`kinds`, `connectors_allowed`, `legal_hold_default`), one row per name bound
to a domain (`profiles` + `domain_profiles` tables, global DB). Shipped
**1.21.0 — 2026-08-15** with 12 presets (`presets()`, seeded with
`INSERT OR IGNORE` so operator edits survive re-migration); every field is
optional (absent = the server default applies) and editable via
`POST /profiles/{name}`.

The invariant: **the profile sets defaults, the row wins** — an explicit
per-row value is never overridden, and an unbound domain is byte-identical to
pre-v1.21 (test-pinned). Routes: `GET /profiles` (the wizard pick list),
`GET /profiles/{name}`, `POST /profiles/{name}` (Admin, audited), and the
binding `GET|POST /domains/{name}/profile` (`{"profile": "health-hipaa"}`
binds, `null` unbinds) (`src/server/router/memory.rs:105-113`,
`src/handlers/profiles.rs`, [api.md](./api.md)). The client Health panel
shows the active profile and its effective knobs (`client/src/api.rs`
`GET /profiles`, `GET /domains/{domain}/profile`; `client/src/panels/system.rs`).

Layering that matters: `audit_read_events_for` resolves explicit
`BRAIN_AUDIT_READ_EVENTS` (deployer kill-switch) over the bound profile's
`audit_level` over the default; `retention_map` drops `null` (no-decay) kinds
while the map's **presence** stays authoritative (an empty block means nothing
decays for the bound domain); `kinds` is a 422 constraint; `pii_mode: strict`
masks at the write boundary one-way (never a vault); `connectors_allowed` is
stored and surfaced with family-prefix matching (`crm` grants `crm-*`) and an
explicit-empty air-gap (`[]` allows nothing) — the file's own note marks
wider enforcement as later work, so read it as stored posture, not a gate.

## 6. Seam four: the calendar as code (`src/reg_watch.rs`)

The whole module is `#[cfg(test)]` by construction: each pinned deadline
carries its source URL, and until the date the pin is a WATCH, after it the
pin asserts the **deliverable exists** — a passing date without the artifact
fails CI. Provenance is labelled, never laundered: where no primary fetch is
reachable from a build, the file records `audit-asserted, not source-verified`
in code, doc, and assertion.

| Deadline | What the pin asserts |
|---|---|
| CRA Art 14 reporting live 2026-09-11 | `docs/cra-reporting-runbook.md` carries the 24-hour / 72-hour / final-report sections, ENISA + CSIRT channels, the stamped date, the drill script, and the split final-report clocks (vuln: 14 days after the corrective/mitigating measure is available, Art 14(2)(c); severe incident: one month, Art 14(4)(c)) |
| AI Act general application 2026-08-02; legacy-marking grace ends 2026-12-02 | [compliance.md](./compliance.md) states **both** dates (December alone misreads as the start); the transitional period is cited to its operative provision **Article 111(4)** — recital 38 is the recited reason, never the granting instrument — with the audit-asserted provenance recorded; the Art 50(2) deliverable pins the provenance module (`src/provenance.rs`: `MARK_AIGEN`, `MARK_HUMAN`, `attach_aigen`, `verify`, `NOT C2PA`) wired on all four emission classes with its meta-tests |
| PQC key-establishment horizon 2030-12-31 | [crypto-inventory.md](./crypto-inventory.md) in SP 1800-38B shape (algorithm inventory, HNDL verdicts, the JWT `auth/jwt.rs` `ALLOWED_ALGS` landing procedure, the UMP did:key version-prefix rule) plus the closed-both-ways crypto-crate census |
| MGF for Agentic AI published 2026-01-22, updated 2026-05-20; CETS 225 in force 2025-09-01 | [compliance.md](./compliance.md) carries the stamps with their posture: MGF **VOLUNTARY**, four dimensions (buyer evidence, never a duty claim); CETS 225 **party-facing duties only** |

Deployer horizons from the same instruments (Annex III from 2027-12-02, Annex
I from 2028-08-02) are tracked in docs, not in code — by the file's own
stated design.

## 7. Operator runbook

1. **Pin local artifacts.** Emit the manifest (`scripts/gen-model-manifest.sh`),
   set `BRAIN_MODEL_MANIFEST` to its path, restart. Any mismatch refuses boot
   naming the pinned vs found hash — fix the file or the manifest, never
   bypass: absent env means unpinned, and unpinned is the ceiling (§8).
2. **Register the model identity.** `POST /workflow/model-registry/register`
   as Admin; read back `GET /workflow/model-registry/{id}@{version}` and keep
   the returned `row_digest` — lifecycle proposals must copy `row` +
   `row_digest` unchanged. A `409 model_already_registered` means the
   `(id, version)` is taken; pick a new version, never reuse one.
3. **Inspect from the console.** The `models` route
   (`shell/src/routes/models/+page.svelte`) lists via
   `GET /workflow/model-registry` and reads detail via
   `GET /workflow/model-registry/{model_ref}` through the bounded parsers in
   `shell/src/lib/model-registry.ts` (limits at `MODEL_REGISTRY_LIMITS`;
   unknown values refused, never invented). Lifecycle moves go through the
   human proposal gate, not a status button.
4. **Scope the domain.** `POST /domains` to create/warm, then bind posture:
   `POST /domains/{name}/profile` with `{"profile": "<preset>"}`. For true
   file isolation set `BRAIN_MULTI_DB=true` **before first use** (`global`
   stays on the legacy file either way); watch the `BRAIN_MAX_DOMAIN_DBS` cap
   (default 256) and remember deletes keep the file and the slot until the
   operator removes the files and reboots.
5. **Rehearse the calendar.** Read [compliance.md](./compliance.md) for both
   AI Act dates, `docs/cra-reporting-runbook.md` for the three Art 14 clocks,
   and [runbooks.md](./runbooks.md) for the dated standby/revocation drill
   records; re-verify each `reg_watch` source URL at its stated stamp rather
   than trusting the constant.

## 8. Honest limits (ceilings, not footnotes)

- Absent `BRAIN_MODEL_MANIFEST` is **unpinned, not safe-by-default**:
  `verify_configured_models` returns `Ok(0)` and boot proceeds. The pin also
  covers only listed files — an unlisted artifact is unverified, and the
  manifest is a boot check, not runtime re-verification.
- Shim-mode domains are **labels sharing one pool**, not isolation; per-file
  isolation exists only under `BRAIN_MULTI_DB=true`, and cross-domain
  federation/centroid routing remain next-phase work per the module header.
- Profiles configure existing seams; they add no new governance primitive.
  `connectors_allowed` here is stored + surfaced posture; strict-mode masking
  runs after auto-routing, so the quantized embedding and caller-declared
  entity names derive from raw text; the HITL `/ingest/proposal` flow keeps
  its pre-profile posture.
- `reg_watch` is **test-only** — it gates CI, never the request path — and
  its corrected cites are audit-asserted, not primary-verified. No EUR-Lex
  fetch is reachable from a build; a session with source access should
  confirm article numbers.
- The console never computes a digest: `row_digest` is carried and forwarded,
  `artifact_digest_present` is display-only, and there is no model-registry
  panel in the `client/` crate (domains/profiles only) — the shell `models`
  route is the console surface.
- Nothing here claims model quality, out-of-sample accuracy, or
  false-promotion rates; evaluation records stay explicitly non-authoritative
  and lifecycle status moves only through the human gate — the non-claim
  posture of [model-governance.md](./model-governance.md) governs.

## See also

- [model-governance.md](./model-governance.md) — the digest-pinned registry
  core, decision runs, and the replay-gated promotion (the doc home this page
  defers to)
- [api.md](./api.md) — route rows for every surface named here
- [configuration.md](./configuration.md) — `BRAIN_MODEL_MANIFEST`,
  `BRAIN_MULTI_DB`, and the full env reference
- [compliance.md](./compliance.md) — both AI Act dates, the MGF/CETS stamps
- [crypto-inventory.md](./crypto-inventory.md) — the PQC seam the watch guards
- [runbooks.md](./runbooks.md) — standby and kill-switch drill records
- [architecture.md](./architecture.md) / [features.md](./features.md) —
  shim vs multi-db and the profile system in context
