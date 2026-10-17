# Agent Execution Log — brain-server

> **R123 "Tally": the suite is measured, and the register says who owes.**
> The test suite is the largest artifact in this repo and nothing had ever
> counted what it is *made of*. `scripts/test-census.py` classifies every
> test in `src/` + `tests/` by five deterministic rules, and
> `docs/test-census.md` is the committed derivation with a `--check` that
> refuses drift. **That table is the authority on the numbers — the figures
> below are this round's measurement as taken, and they move every time a
> pin lands** (they were already one behind by the time this note was
> written; the `the_test_census_table_re_derives_from_the_tree` pin is what
> catches it, and it is a pin to read, not to edit around). As measured:
> **2958 behavioral pins (87.0%), 333
> structural (9.8%), 53 controls (1.6%), 51 residual-documentation (1.5%),
> 6 unclassified (0.2%)** — and the unclassified bucket is reported, not
> hidden. One row of that count is the census's OWN pin, filed as a `control`
> because its doc-comment explains the CONTROLLED-vs-CONTROL distinction and
> therefore contains the word — the marker rule's stated one-test-wide
> ceiling, disclosed rather than reworded away.
>
> **The first classifier was wrong in four ways, and the round's own
> methodology is what found them — not a careful read.** Rule 3 only saw
> reads written IN the test body, so `tests/rbac_evaluation_pins.rs` and
> `src/docs_truth.rs` (source-scanning files whose tests route through a
> `read_repo`/`doc` helper) classified as *behavioral*; rule 4 only saw
> qualified calls, so **261 real pins that call their subject bare** —
> `verify_claim(c, q)`, `parse_flags(args)` — landed in `unclassified`; the
> doc-markers matched SUBSTRINGS, so `CONTROLLED`, `CEILINGS` and
> `STRIP_CONTROL_CHARS` read as labels; and `env!("CARGO_PKG_VERSION")` seeded
> the repo-anchor fixpoint, which swept every helper reaching
> `encrypt_v3_blob` into the structural class — including `backup.rs`'s
> `verify`, which reads an encrypted BACKUP, not the tree. Each was found by
> planting a test and watching the classifier get it wrong. The census now
> carries ten plants, including the three negative arms that matter most (a
> runtime-data read must NOT read as structural; a process spawn must NOT
> either — running a script and reading its exit status DRIVES a subject).
>
> **The register caught up with the tree.** Four guards that shipped in
> v1.29.13 without rows now have them, each with the drill it owes recorded
> (the two-thread env-drill owed on the salt lock; the compile-time fence
> owed on the operator key — a signature change, named not faked). Every
> structural row now carries an explicit **R123 decision**: one converted,
> two already retired, three LANDED-and-kept, seven keep-and-re-date with the
> project named. The law gained its missing clause: an exhaustive row owes
> nothing, because exhaustiveness IS the proof.
>
> **The conversion, and its teeth.** `crates_consumed_set_is_pinned` no
> longer greps manifest text — it asks `cargo metadata` whether a PATH
> dependency resolves, in both workspaces. Red-proof: a COMMENT naming
> `crates/brain-care-core` plus the member moved to the consumed list
> satisfied the old text condition and fails the new one. An anti-vacuity
> arm names two members cargo is known to resolve, so a probe that stopped
> seeing anything fails instead of quietly emptying the island list.
>
> **The register now has a pin.** Nothing read
> `docs/gate-law-register.md` — its first law ("a new guard lands with its
> row") was enforced by nothing, which is how four guards shipped bare.
> `the_gate_law_register_rows_parse_and_carry_their_last_column` parses both
> tables, refuses an empty `Red-proof`/`Owed`, and fails CLOSED on a
> malformed row. Drilled three ways, all reverted: blanked `Owed`, blanked
> `Red-proof`, and a row missing a column separator.
>
> **ponytail:** one script, one committed table, one converted pin, one
> register pin, no new dependency (`cargo metadata` was already there; a
> `serde_json` dev-edge would have re-locked the root for nothing), no new
> test file, no floor written before its population was measured, and no
> test deleted — the round's exit criterion is a measurement plus a
> decision per row, and "we deleted N tests" would have failed it even
> with every lane green.

> **R121 "Second Pass" + R122 "Conduit II": the closures attacked, the owed
> rows landed.** The twelfth pass (a dedicated adversarial lane against
> R115-R120's own diffs) found what a second pass exists to find, and the
> owed-conversions batch landed alongside it.
>
> **The pass's findings, all closed same-session:**
> 1. **The R116 taint inheritance was strippable** — `deep_merge` guarded
>    only `id`/`integrity`, so a `{origin:null}` patch overwrote the emitted
>    base and laundered a captured row in one call. The skip-list now guards
>    `origin`/`untrusted` too (the pin extends the revise drill with the
>    exact patch). 2. **R118's fixture-v2 was one-sided on the PLUGIN lane** —
>    `INVISIBLE_CLASSES` stayed v1 (the "fails CI by design" claim is false
>    there: the plugin's vitest runs nowhere in this repo — S9-07 biting).
>    The lane moved to v2 and the fork suite runs it (72/72). 3.
>    **`console_due` emitted the valet label raw**; it rides the read seam
>    now. 4. **`AGENT_TOKEN_FILE`'s runtime reader bypassed the mode law**
>    (boot enforced, mtime reload did not); it refuses now, and the boot
>    message names the mode before the reader can disguise it as empty. 5.
>    **THREAT_MODEL's "raw stored bytes" digest wording was false** —
>    corrected to the read-canonical truth (the PII ceiling itself stands).
>    6. Verdict asymmetry on `/add` + structured `/ingest` FILED (register
>    line, not silently aged). The other four closures verified HELD.
>
> **The owed rows landed:** the two structural pins' behavioral conversions
> (`the_ingest_lands_its_evidence_row_on_the_chain`,
> `the_edge_history_cap_bounds_and_discloses` — real app, real fixtures);
> the F4-19 digest pins (all three bases pinned against registry-resolved
> digests + a Dependabot `docker` ecosystem); the F4-21 staleness gate
> (`no_ignored_advisory_is_fixed_in_the_current_lock` — each ignore names
> its crate inline); the F4-18 islands census
> (`crates_consumed_set_is_pinned`, per-entry reasons); the F4-15 WCAG
> scope banner; **F4-05's remaining half** (domain create + import each
> write their chain row — the import REPLACES a corpus and wrote zero);
> **the F4-08 census** (the router's 52 rusqlite shapes pinned at EQUALITY —
> any delta is a named act; the migration stays staged in the register).
> The regulatory band: T4-07 reconciled (90 FR 4544/ECCN 4E091 rescinded
> pre-effect — "no rule in force" is the operative truth, the map row names
> the instrument) and L4-15 (the CRA runbook carries an
> APPLICABILITY-FIRST banner; the duty is conditional, the clocks accurate).
>
> **ponytail:** one deep_merge skip-list extension, one regex, two read-seam
> calls, one reader guard, and pins — the owed conversions reuse the
> existing harness; the census is equality-not-floor by design; no new
> knob, no abstraction, no dependency.

> **R120 "Conduit": the CI/supply-chain band lands its gates** The F4-13…23
> row cluster from the tenth pass — mechanical, runner-side, and open for two
> rounds because no round owned the workflows.
>
> 1. **A published release carries `SHA256SUMS`** over every staged file
>    (fail-closed: the manifest must cover the full dist set), generated
>    before the release step — `files: dist/*` ships it. *(F4-13.)*
> 2. **The green gate iterates every publication workflow** (`ci.yml
>    shell.yml codeql.yml fuzz.yml docs.yml`): an existing red run on the
>    tagged SHA refuses the publish; a paths-filtered skip is named and
>    accepted. *(F4-14.)*
> 3. **The shell type-check runs AFTER the supply-chain gates** — one red
>    `svelte-check` used to starve 13 steps including both audits.
>    *(F4-16.)*
> 4. **`check-doc-links.py` is anchored on its own file location** (from a
>    foreign cwd it scanned nothing and exited 0) **and is invoked by the
>    ci.yml docs-gates step**. *(F4-17.)*
> 5. **CodeQL gains the TypeScript lane** over `plugin/` + `shell/src` —
>    the code that parses model output was SAST-invisible. *(F4-22.)*
> 6. **The badges UMP arm reads the case-arm at the gate itself** (fixed
>    string), not the comment above it that a deleted gate left green.
>    *(F4-23; `--selfcheck` green on the new anchor.)*
>
> **Recorded owed, not churned** (the gate-law register's text-bound
> section): F4-19 digest pins (resolving real registry digests is an
> external act), F4-21 audit staleness (the ignore list needs pruning
> first — a lockfile decision), F4-15 WCAG evidence retarget, F4-18 crate
> islands pin. All four are rows with owners now, not silent gaps.
>
> **ponytail:** workflow YAML + one script anchor + one structural pin in
> the existing CI-shape file; no server code, no schema, no dependency.

> **R119 "Gatewright": the process round, two passes overdue.** D8-02
> ("enforcement is the uniform weak link — 3 of the top 10 are guards passing
> while violated") has been OPEN-UNROUTED since the eighth pass; its fix was
> never a patch but a standing artifact.
>
> 1. **`docs/gate-law-register.md` exists.** The law: a guard lands with its
>    row, its red-proof, and its class — behavioral, structural, or
>    exhaustive. A structural row is a regression lock, never proof, and
>    carries an `owed` column naming the behavioral conversion that retires
>    it. Seeded with the ten standing guards (each red-proof recorded) and
>    the four text-bound rows: the two R118 wiring pins, the R9-03
>    rate-limit literal, and the R9-04 file-granular Art 50 check.
>    *(Anti-rot pin `the_gate_law_register_names_the_known_guards`.)*
> 2. **The docs-truth fixes**: the COMPLIANCE.md PHI row now states the
>    read-time truth (raw PHI at rest behind volume encryption; no
>    write-time vault — the sentence the tenth pass called vacuous is
>    gone), and `dsar_deadline` is relabelled as the DSAR processing
>    deadline the operator window governs (Art 17 sets no clock).
> 3. **The register hygiene**: F4-05 corrected to PARTIALLY CLOSED (the
>    `release_quarantine` half silently closed at `memory.rs:2748`; import +
>    create still write zero audit rows), and R111 joined `SHIPPED_ROUNDS`
>    (two commits — the shell rendering steps — that the array had skipped).
>
> **ponytail:** a markdown table and one anti-rot pin — not a framework; the
> R9-03/R9-04 conversions are RECORDED as owed, not churned into the tools
> crate this round; no code behavior change beyond the two doc-truth
> relabels.

> **R118 "Lowtide": the LOW band closes before it ages into ceilings.** Eight
> known-LOW defects, one batch, each a bounded read, a parse, a header, or a
> gate — red-first pins in EXISTING files (no second test file, no knob, no
> abstraction; nothing grew one).
>
> 1. **The invisible set gained its three missing classes** — INVISIBLE PLUS
>    (U+2064), the deprecated format chars (U+206A–206F), and the musical
>    format controls (U+1D173–1D17A) — across the ONE canonical fixture and
>    both code lanes (rust + shell regex) in one change; the exhaustive
>    all-scalars pin forces the sync (a one-sided edit fails CI by design).
>    Fixture → v2. 2. **The Art-19 webhook's `subject` passes the
>    invisible/control strip** before riding to an external receiver.
>    3. **`/ingest/memory`'s audit row moved INSIDE its tx** (the sibling
>    arm's shape) — the post-commit second-connection form was a crash
>    window with no chain row. *(Offset pin
>    `the_ingest_audit_row_precedes_its_commit`.)* 4. **The tokenless
>    bridge-mount arm runs the SAME freshness + replay pair the receiver
>    arm does** — a captured signed envelope previously remounted forever.
>    *(Pins `bridge_mount_arm_refuses_a_stale_signed_timestamp` +
>    `bridge_mount_arm_refuses_a_replayed_webhook_id`, the latter with the
>    first-sight anti-vacuity arm.)* 5. **Per-domain databases join the
>    0600 family** — registry creation AND import, enforced AFTER the pool
>    build (the red-first pin caught the first placement tightening a file
>    that did not exist yet). *(Pin
>    `a_domain_db_created_by_the_registry_is_owner_only`.)* 6. **The
>    documented 0600 law is enforced on `AUTH_TOKEN_FILE`'s read path** —
>    the new secret-file seam now covers the last unguarded secret; boot
>    refuses the world-readable file when no env fallback exists, and the
>    rotation watcher's reload treats the refusal as empty → fail-safe.
>    *(Pin `a_world_readable_auth_token_file_is_refused_not_read` with the
>    compliant-file anti-vacuity arm.)* 7. **The edge-history lineage is
>    bounded** (`EDGE_HISTORY_MAX` = 500) and **names its ceiling**
>    (`truncated: true`). 8. **`brain_memory_proposal_decide` is REMOVED** —
>    the review queue is not the model's to prune: the server role-gates
>    both verbs to humans, and approve demanded a digest the queue list
>    never carried, so the tool advertised a promotion it could never
>    perform (plugin 0.6.14; the anti-comeback assertion pins the absence).
>
> **ponytail:** no new knob, no abstraction, no second test file; the two
> handler-side wiring pins are structural (the R9-03 class) and are owned by
> the gate-law register landing in the next round; zero new dependency
> edges; the wire delta is the additive `truncated` field (openapi not
> required — the schema enumerates the envelope loosely, the field is
> additive JSON).

> **R125 "Bounds" (Phase E, 2026-10-16): the measured GDL case budget.**
> `BRAIN_GDL_CASE_TOKEN_BUDGET` / `config::gdl_case_token_budget()` — the
> default is MEASURED, not chosen: 112 tokens/case p99 over the 12 judged
> `eval_run1` seeds (79 `gdl_gate` usage rows; the row now carries the
> exchange's usage, additive), × MAX_PHASE_ATTEMPTS (3) = **336** — the
> happy-path measurement with full headroom for the bounded retry ladder.
> `the_default_case_budget_is_the_measured_p99` re-runs the seeds and holds
> the const to the measurement. **Scale caveat:** 14 tokens/exchange is the
> loopback provider's own accounting, not a frontier model's — operators on
> real providers set the env var to their own measured ceiling. The budget
> is the CASE's cumulative authority (phases draw it as `fresh_exchange`
> views — the deliberate reversal of F11's per-phase isolation, recorded in
> the `case_budget` doc), and a confirm-exchange interrupt is a receipt-
> backed `Capped`, never the unvalidatable `Routed` it was mapped to before
> (found because the budget path made it reachable).

## Retired rounds — where the older log went

**R117 "Console" (2026-10-12) and everything older** — R116→R68 and the
v1.28.x version notes — moved verbatim to
[`docs/AGENTS_HISTORY.md`](docs/AGENTS_HISTORY.md) on 2026-10-14.

This file had grown to 3,958 lines, 3,717 of them that log: the
per-round history its own footer says it does not keep here. Every round
below is **already** in `CHANGELOG.md`, structured by release rather than
by round — R115–R122 in §[1.29.13], R97–R99/R108–R109/R112 in §[1.29.12],
R95/R104–R107 in §[1.29.11], R93/R100–R103 in §[1.29.10] — so the log was
a second, less retrievable copy of the same reasoning.

**The cut point is R118 "Lowtide".** The five rounds above still name
work owed — R123's census measurement and owed rows, R122/R121's
closures, R120's CI band, R119's gate law, R118's LOW band — so they
stay. Everything from R117 back is closed, shipped, and carried by the
changelog. A round that still owes something re-enters here when it does.

## Architecture Law (the steering)

- **Two layers, one pattern.** Handlers are protocol adapters ONLY: parse →
  OptPrincipal → `authorize`/`authorize_role` → one `spawn_blocking` →
  service-core call → read-seam shaping → response. ALL SQL, bounds/caps,
  FK-children ordering, and invariants live in domain modules
  (`src/workflow/*`; `src/service/*` from v1.28.46) taking `&Connection` /
  `WorkflowTx` — never Pool, AppState, or axum types.
- **Audit-per-write.** Every mutation emits its hash-chained audit row INSIDE
  the caller's transaction (`record_tenant`, SAVEPOINT-nested): a transition
  and its evidence commit or roll back together.
- **Fail-closed everywhere.** Authz gates, poisoned locks, unreadable config,
  quarantine, legal holds, DPO switches: error paths deny loudly; silence is
  never certified (`let _ =` on writes is forbidden).
- **Read seam unconditional.** Every emitted text field passes
  `sanitize_read`; untrusted content is screened + stripped at WRITE time
  (`screen_content` pattern); LLM-facing payloads are fenced.
- **Bounds law.** Every list surface capped, every input bounded, every cap
  pinned by a test; SQL never decides a row's fate (Rust-side pure arbiter).
- **Wire-contract discipline.** Any route change ships openapi.yaml +
  route-coverage + route-authz guard-table entries in the same commit; the
  `x-api-version` stamp moves only when the wire contract moves.
- **Convergence.** New code is ALWAYS a service core; legacy extraction is
  trigger-based (piggyback / pre-storage-adapter deadline); handler-side SQL
  is ENFORCED ZERO by the `no_sql_in_handlers_enforced` guard (Cornerstone —
  any match in `src/handlers/**` fails CI; there is no allowlist). The law's
  public statement lives in `docs/architecture.md` (the two layer rules, the
  request-flow diagram, the seam table); this file is the operational
  summary.
- **The thin binary (Capstone).** main.rs is WIRING ONLY (bootstrap →
  compose → serve), pinned ≤ 300 lines with no cfg(test) region (the mass
  lives in `tests/`); route registrations live ONLY under
  `src/server/router/**`; `server::bootstrap` stays protocol-free (no axum
  types). Each clause is machine-checked by the spire gates in
  `src/spire_inventory.rs` (`route_registrations_live_only_under_router`,
  `bootstrap_stays_protocol_free`,
  `spire_inventory_freezes_the_thin_binary`); the one fenced exception is
  `src/bin/mcp.rs` — a separate binary's single-endpoint /mcp protocol
  edge, pinned at exactly one site. The line's measured record:
  `docs/AUDIT.md` (the Spire Line close-out).

## Service-layer convergence (Foundation Line, v1.28.46+)

- New code ALWAYS builds as a service core (`src/service/<domain>.rs` owning
  SQL + invariants + audit_write inside the caller's tx); handlers are
  protocol adapters only. Templates: `src/workflow/channel.rs` +
  `src/handlers/channel.rs`, and since Plumb `src/service/retention.rs` +
  `src/handlers/govern.rs` (the extraction exemplar).
- Legacy extraction is TRIGGER-BASED, never cosmetic: (1) piggyback — when a
  feature substantively touches `recall/gate/observe/domains/clients`, extract
  that surface's core in the same release (pins first, move second);
  (2) deadline — immediately before any v2.x storage-adapter work.
  (The historical freeze-and-burn machinery — the `sql_inventory_baseline`
  per-file table shipped in Plumb at 445 across 29 files, lowered line-wide
  by Quarry/Masonry/Terrace/Aqueduct/Confluence to 78 — is RETIRED: the
  Cornerstone extraction took gate.rs 78 → 0 and the enforcing flip deleted
  the table. `no_sql_in_handlers_enforced` in `src/service/mod.rs` now fails
  on ANY statement under `src/handlers/**`, recursively. Line plan:
  `IMPLEMENTATION_ROADMAP_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`;
  executor: `EXECUTION_PROMPT_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`.)

## Operational Context (read first)

**Paths**
| What | Where |
|---|---|
| Repo | `/Users/mark/Sites/brain-server` |
| **Live DB** | `~/.openclaw/workspace/brain.db` (override: `BRAIN_DB_PATH` env) |
| Binaries | `~/.local/bin/{brain-server, brain, mcp, bench, brain-connector-stub, brain-migrate-rehearse}` (+ `brain-connector-gh`/`brain-connector-crm` when their features are built) |
| Auth token | `~/.config/brain-server/auth-token` (0600; written by `install-service.sh`) |
| Launchd plist | `~/Library/LaunchAgents/com.brain.server.plist` |
| Logs | `~/Library/Logs/brain-server.{log,err.log}` |
| Config module | `src/config.rs` (all constants + env var resolution) |

**Commands**
```sh
# Build all 4 binaries
 cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench

# Tests + quality gates (always run with --features bench — the bench binary is feature-gated)
# Do NOT hand-type the count. `scripts/badges.sh` derives it and writes the README
# badge; `scripts/badges.sh --verify-count` re-derives and REFUSES on drift. A
# number pasted here is a number nobody diffed against a measurement — the exact
# failure this file's own header documents, repeated. (It happened: the old
# comment here carried a count pinned to a commit twelve releases back.)
cargo test --features bench                                  # count: scripts/badges.sh --verify-count
cargo clippy --all-targets --features bench -- -D warnings   # zero warnings enforced
cargo fmt --check

# CI DRY-RUN — REQUIRED before every push/release (v1.28.29 lesson):
# the local gate alone does not cover the default-feature build or the
# side workspaces CI runs on Ubuntu. Run these before pushing:
export RUSTFLAGS="-D warnings"
cargo clippy --all-targets -- -D warnings                    # job lint-test (default features)
cargo test --all-targets                                     # job lint-test (default features)
cargo test  --manifest-path crates/Cargo.toml --all-targets          # engine-crates
cargo clippy --manifest-path crates/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/steward-harness/Cargo.toml         # steward-harness-gate
cargo clippy --all-targets --features otel -- -D warnings    # otel-gate
cargo test  --all-targets --features otel
# client-gate only when client/ touched (slow: wasm + desktop headers).

# FEATURE LANES — every `[features]` entry has a job in .github/workflows/ci.yml.
# Six did not, and two of those did not compile (`injection-classifier`,
# 9 errors; `neural-embed`, 2 errors), so the breakage accumulated unnoticed.
# A lane that does not exist looks exactly like a lane that passes.
# `tests/feature_lane_pins.rs` derives the feature list from Cargo.toml and
# fails on any feature no lane builds — do NOT delete that pin; it is the only
# thing making the six lanes below durable.
for f in compliance-pack multivec injection-classifier neural-embed loom rerank-tier; do
  cargo clippy --all-targets --features "$f" -- -D warnings
done
# A lane proves a feature COMPILES. It does not prove the feature WORKS — the
# ONNX/model artifacts are not fetched in every lane, and no lane exercises
# layer-2 scoring or a live rerank.

# Repo briefing — ONE-SHOT agent overview before any work (runs <1s):
scripts/repo-brief.sh
# (versions, HEAD, dirty paths, main.rs structure, env/route/CLI counts,
#  which guards exist vs ABSENT, stale-marker probe of living docs)

# v1.28.31 lesson — these two CI jobs are NOT covered by the pre-push hook
# (it fmts only the server tree) and burned the first Charter push. Run them
# locally as part of the gate, ALWAYS before push:
scripts/lipstyk-gate.sh
 cargo fmt --manifest-path client/Cargo.toml -- --check
# lipstyk-gate: the CI watchdog's trap-proof wrapper — the raw
# `lipstyk --diff "$(git rev-parse origin/main)"` form goes VACUOUS once you
# push (origin/main == HEAD → empty diff → exit 0 having scanned nothing) and
# it never sees UNTRACKED new modules (both teeth bit on the Terrace push;
# CI caught what the local run waved through). The wrapper pins the base
# (pre-push merge-base, else HEAD~1, else pass the old tip / release tag),
# marks new files intent-to-add, and REFUSES to pass vacuously. lipstyk:
# changed lines must add no diagnostics (Box<dyn Error> returns in
# tests included — helpers inside #[cfg(test)] mods are still scanned; use
# .expect() like sibling tests instead of Result-returning test bodies).

# CI and releases (2026-10-06 billing law): PRIVATE-repo Actions are
# disabled (the free 2,000 min/month died in six days; CI + CodeQL were the
# burn). PUBLIC-repo Actions are free, so the RELEASE TAG is the public CI
# trigger: a v* tag push runs the FULL ci.yml matrix on public (main is
# still never pushed there — release.sh's law), and release.yml's
# publication step fail-closes unless that matrix is green for the tagged
# SHA: red or absent ⇒ binaries build but nothing publishes. The pre-tag
# discipline is the LOCAL gate suite; release.sh watches the public runs
# after the push and exits non-zero on a not-green verdict. The
# release builds are 4 parallel per-target jobs (linux x86_64/aarch64,
# macOS arm64/x86_64); the verify-required-assets gate refuses to publish if
# any primary brain-server binary is missing.

# README badges — NEVER hand-type them; regenerate from the real build:
scripts/badges.sh                         # prints the version/test/UMP/SBOM badge block
scripts/badges.sh --verify-count          # the REAL count compare (one full cargo test, ~4 min)
scripts/badges.sh --selfcheck             # CHEAP gate: version↔README, UMP, checklist, SBOM
# (derives version from Cargo.toml, test count from cargo test --features bench,migrate)

# Install + restart launchd service (also installs CLI binaries, strips macOS provenance xattr)
scripts/install-service.sh

# Health + stats against running server
brain doctor
brain status
```

**Runtime**
- Server: launchd-managed, `127.0.0.1:8765`, `KeepAlive=true`, `RunAtLoad=true`.
- Auth: bearer token via `AUTH_TOKEN_FILE` → `AUTH_TOKEN` (server) / `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → default file (CLI). Off by default if no token resolves.
- macOS Sonoma+: newly-copied executables get `com.apple.provenance` xattr → Gatekeeper SIGKILLs on first exec (exit 137). `install-service.sh` strips it; manual `cp` does not.

**OpenClaw integration**
- brain-server is the **memory backend** for openclaw; the plugin (`brain-server/plugin/`, TypeScript) calls `/recall` each turn via openclaw's `before_prompt_build` hook.
- openclaw config: `~/.openclaw/openclaw.json` (plugin block at key `brain-server`).
- The `brain` CLI is built from this repo's `Cargo.toml` (`[[bin]] name = "brain"`), **not** from openclaw.

**Customer-domain assets live in a private repo (2026-08-19)**
- The case-classification work is **not** part of brain-server: plan, customer
  taxonomy, routing matrix, playbooks, spine builder + outputs moved to a
  **private** repo (not named here — this file is published to the public
  portfolio, and naming the customer is a confidentiality problem
  independent of the assets themselves). Never commit customer-domain content
  here — `.gitignore` covers `spine/` + the two spine scripts defensively.
- This repo stays the vendor-agnostic product: graph-default-on recall, the
  feature-gated `src/classify` engine (when built), proposals/suggest/audit.
  The taxonomy is *data the engine consumes*, owned by the private repo.

**Steward engine IP lives in a private repo (2026-08-21)**
- The advanced architecture docs (Steward architecture, harness audit, state
  mapping, port specs, rubric pin, diagnostics loop, compliance mapping) are
  **MemorySteward LLC IP** and live in the **private** repo `brain-steward-ip`
  (local checkout `~/Sites/brain-steward-ip`). Never commit them here —
  `.gitignore` defends the names defensively (same pattern as the
  customer-domain repo).
  The public repo ships the substrate only; the engine crates (`brain-engine-sdk`
  5,887 top-level + `host/`/`pure/` lines, 8,352 total / 127 tests at v1.28.23,
  plus the five `*-core` engines) are PUBLIC, REAL code — measured 2026-09-15.
  (The old "scaffolds stay public while empty" claim was stale — do not repeat
  it; the SDK is built, kernel-consumed, and dependency-free by design.)
- **The Steward-line implementation plans (1.27.32 → 1.27.42) live in
  `brain-steward-ip/plans/`** with the master `ROADMAP_1.27.x.md`. This repo
  keeps **no** kernel-root plan files: every `IMPLEMENTATION_PLAN_*`,
  `IMPLEMENTATION_ROADMAP_*`, `ROADMAP.md`, `DESIGN_*`, `CLIENT_ROADMAP.md`,
  `REALITY_CHECK.md`, `MARKETING_PLAN.md`, `TODO.md` and `USE_CASES.md` that
  used to sit here was **moved to `brain-steward-ip/plans/archive/` on
  2026-10-04** — see `plans/archive/KERNEL_ROOT_PLANS.md` and
  `KERNEL_ROOT_NONPLAN.md` there for provenance and the list. None was ever
  tracked (`.gitignore` 55–70), so the public tree carried 222 root `.md` files
  while git held 16; the root is now 19, all of them real project docs.
  **Do not re-add one "for context"** — the private archive is the home, and
  `tests/external_claim_pins.rs` discovers the claim-sensitive ones by CONTENT
  rather than by path, so a copy here is not what any gate reads.

---

## Known issues (open)

- ~~**CI gap — tests run on x86_64 runners only.**~~ **CLOSED 2026-09-15
  as NOT-APPLICABLE:** no Jetson/fleet deployment exists (brain-server is
  not installed on any aarch64 host), so the advisory's precondition —
  "before fleet deploys" — is absent. The release matrix still
  cross-builds aarch64 and CI still executes tests on x86_64 only;
  REOPEN at the first aarch64 fleet deployment, and then as an ARM-hosted
  CI test lane, not a manual smoke.
- ~~**Historical token leak (openclaw-side).**~~ **CLOSED 2026-09-09:** the
  final `transcript_events` copy of the agent token was redacted + VACUUMed
  (backup: `~/Downloads/audit/openclaw-agent-pre-purge-*.sqlite`, 0600), the
  agent token was ROTATED (old value dead → 401; watcher reload audited), and
  the discovery of the night: the openclaw gateway's env carried the OPERATOR
  token — the plugin had been authenticating as full superuser. It now runs
  on the AGENT token (`service-env/ai.openclaw.gateway.env`), so the Twokeys
  agent boundary is ENFORCED live (purge → 403 proven). The operator token is
  unchanged. The `${BRAIN_SERVER_AUTH_TOKEN}` placeholder in
  `openclaw.json` resolves from the same env file. Hygiene:
  `~/.config/brain-server/auth-agent-token` (the installer's designated
  `AGENT_TOKEN_FILE` target, unreferenced by the running service) was
  re-synced to the live agent token so a future installer re-run or
  `AGENT_TOKEN_FILE` adoption cannot resurrect the dead value.

History: per-release detail lives in `CHANGELOG.md` §[version] and
`docs/AGENTS_HISTORY.md` (load on demand). Nothing older than the note above
is kept here.
