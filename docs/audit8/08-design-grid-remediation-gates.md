# §9 — DESIGN CRITIQUE (strategic), COVERAGE GRID, REMEDIATION, GATES, CEILINGS

---

## 9.1 D8-* — Architecture critique

Ranked by **what breaks first at 10× scale**, then by **what breaks silently**.

| ID | Severity | Critique |
|---|---|---|
| **D8-01** | **Strategic** | **The fork is the largest unmanaged risk in the system, and it is not in the repo.** 90 commits ahead, five HIGH silent-regression rows in §5.3, one of which (`hooks.ts`, 24 upstream commits) drops any field upstream adds via an `as TResult` cast **and compiles clean**. A merge does not fail loudly; it quietly un-sanitizes. Every compensating control in §8.3 lives there. The honest mitigation is not more fork code — it is **an owned, generated fixture shared with the fork** (the §6 P8-01 proposal), so drift becomes a red test rather than a review comment. |
| **D8-02** | **Strategic** | **Enforcement is the weak link, uniformly.** Three of this pass's top-10 findings are *gates that pass while their subject is violated* (F8-01 SQL, F8-05 floor, R8-02 dead register reference). The pattern is consistent and diagnosable: the repo invests heavily in writing a gate and lightly in **attacking** it. The mature move is a standing "gate-law register" entry per guard, carrying its own red-proof — and the repo already has the vocabulary for it (`gates_vacuous`, `sql_statement_counter_still_fires`). **The fix is process, not code.** |
| **D8-03** | **HIGH** | **The 30 s timeout means every write route has a success state the client cannot observe** (F8-03). This is an API-design problem, not a bug: there is no idempotency key and no request-id receipt anywhere. At 10× (larger domains, longer cascades) this goes from rare to routine. |
| **D8-04** | **HIGH** | **`audit-per-write` has no stated cost model.** Every mutation writes a hash-chained row inside the caller's transaction. That is the right trade for an erasure/approval product, but nothing measures it: no write amplification figure, no chain-growth rate, no "writes per KB ingested". At 10× this is the most likely silent degradation. **Recommend one measured number** before it becomes an incident. |
| **D8-05** | **MEDIUM** | **The two-layer law's ergonomics are worse than its enforcement reality.** The law is right and it *has* reduced handler SQL to zero-by-declaration — but the enforcement is a keyword scan (F8-01) that two real violations slipped past. A type-level boundary (handlers may only name connection-taking service functions) would make the law *unviolable* rather than *unenforced*. This is the same "less is more" argument the repo makes elsewhere: **one rule beats a keyword list.** |
| **D8-06** | **MEDIUM** | **Single-node ceilings are documented honestly but are now load-bearing.** The chain key and head pin share a host; `.bak` is plaintext; standby chunks are separate. For a **local-first** product this is a defensible trade, and the docs say so precisely. But the product's differentiator is the *tamper-evident* record — so the ceiling is exactly on the property being sold. It should be stated in sales-facing material, not only in THREAT_MODEL. |
| **D8-07** | **MEDIUM** | **Six unconsumed crates (S8-07)** that lint, review, and pass `cargo audit` but cannot execute. This inflates the apparent engineering surface and, worse, the apparent *compliance* surface. Two are outright dead chains. Either wire or delete; a dead crate that passes gates is a small lie the repo does not intend. |
| **D8-08** | **MEDIUM** | **Single-mutex inference serialization** plus a 64-sentence/16 KiB budget is a defensible v1 posture, but it makes the server's throughput ceiling a function of one lock. Not measured in this pass. At 10× this is the first thing that will be noticed by users. |
| **D8-09** | **LOW-MEDIUM** | **Schema migration is append-only and disciplined** (1.32.25, additive, refuse-newer probe) — genuinely good. The cost is a **very long single migration file**; at 1.32.x with dozens of coupled migrations, reviewability is degrading. Not urgent. |
| **D8-10** | **LOW** | **Error taxonomy is consistent** (operator-safe `Display`, 25-row pinned taxonomy) and **API design drift is low** (232 registered routes vs 222 in api.md; 8 documented exclusions). The plugin/fork boundary is the least-documented seam. |

### What the architecture gets right, and should not lose

The **two-layer law**, the **audit-per-write-in-tx** discipline, the **fail-closed** posture on
secrets and egress, the **read seam applied unconditionally**, the **digest-bound approval with
replay receipts**, and above all the **honesty discipline in the docs**. This audit found more
*documentation-truth* defects than *security* defects — which is the profile of a team that is
honest about its limits and imprecise about its numbers. Fix the numbers; the posture is sound.

---

## 9.2 Coverage grid — layer × dimension

A cell is **done** only with a finding or an evidenced all-clear. **Outstanding cells are marked
and are not claimed as covered.** 15 of 20 dimensions × 5 layer groups were worked; the outstanding
cells are named with their next step.

| Dimension | server core | handlers/router | service/workflow | satellites/tools | supply chain/CI |
|---|---|---|---|---|---|
| 1 Design | D8-05 | F8-01 | D8-04 | D8-07 | — |
| 2 Correctness | F8-03, F8-08 | F8-08 (drill) | F8-09 | S8-06 | — |
| 3 Concurrency | ⚠️ **OUTSTANDING** (lock-ordering cycle check over ~19 Mutex sites) | F8-03 | ⚠️ partial (exec reaping verified at `hostcalls.rs:536-575`) | — | — |
| 4 Resource safety | ⚠️ partial (SSE bounded `sse_reauth.rs:26-30`; webhook queue 503 on Full) | ✅ body caps | ⚠️ **OUTSTANDING** (FTS/vec index bloat) | S8-03 | — |
| 5 Input validation | ✅ `MAX_REQUEST_SIZE` 1 MiB, `MAX_QUERY_LENGTH` 2000, clamps pinned | ✅ `legal_hold` binds all values | ✅ | ✅ `MAX_SECRET_BYTES` etc. | — |
| 6 AuthN/AuthZ | F8-02 | F8-02, F8-06 | — | S8-01 | — |
| 7 Crypto | ⚠️ **PARTIAL** — secret modes verified; `docs/crypto-inventory.md` rows and `ALLOWED_ALGS` not walked | ✅ alg:none/HS* rejected, checked before key lookup | ✅ claim-bound wrapper pinned | — | ✅ audit clean ×4 |
| 8 Data integrity | F8-01, F8-08 | — | ⚠️ **OUTSTANDING** (chain-head discipline on every commit path) | — | — |
| 9 Injection | ✅ path traversal blocked + pinned (`domain_db_rejects_path_traversal`); no shell (argv only) | F8-04 (log) | ✅ `format!`-into-SQL all literals/bound | S8-05 (config coercion) | — |
| 10 Egress/SSRF | F8-07; rest all-clear (redirects, pins, metadata, hex/octal/dword) | — | — | — | — |
| 11 Supply chain | — | — | — | S8-08, S8-10, S8-11 | ✅ **all-clear** (audit ×4, SHA-pinned actions, SBOM current, secrets clean) |
| 12 CI/release | — | — | — | — | S8-09 |
| 13 Secrets | — | — | — | ✅ **all-clear** (one synthetic fixture) | ✅ no `secrets.*` |
| 14 Privacy | F8-08 | — | ✅ purge covers FTS/vec/tombstone/queue/centroids | — | — |
| 15 Observability | F8-04 | F8-04 | ⚠️ **OUTSTANDING** (metric cardinality) | — | — |
| 16 Availability | ⚠️ **OUTSTANDING** (single-mutex inference + ingest storms unmeasured) | F8-03 | ✅ rate limiter outside auth layers (pinned) | S8-04 | — |
| 17 A11y/i18n | ✅ **all-clear** — 7 live gates in `client/src/a11y.rs`; RTL + pseudolocale pinned | — | — | S8-06 | — |
| 18 Config/deploy | — | — | F8-10 (`BIND_PORT` fails open onto 8765) | ✅ installer modes + xattr | ✅ `env-truth.sh` clean |
| 19 Perf/debt | D8-04, D8-08 | F8-05 | F8-01 | — | — |
| 20 Test quality | ⚠️ F8-05 (gameable floor); `client/src/main.rs:2738` fails **open** if the walk path resolves wrong | — | ✅ counter self-pin exemplary | — | — |

**F8-10 — LOW (found during the drill). `BIND_PORT` fails open onto the production port.**
`src/server/bootstrap.rs:1120-1123` — `BIND_PORT` is `.parse().unwrap_or(8765)`. An unparseable
value silently becomes **8765**, the live port, instead of refusing boot. Every other env knob here
fails closed (the `WRITE_POSTURE` pattern this repo prides itself on). *Fix: `.parse().map_err(…)`
and refuse boot on a malformed value.* I hit this live and it is the one place the drill nearly
went wrong.

---

## 9.3 Remediation plan, sequenced as releases

Naming discipline: **one theme per release, one diff, one set of tests.** Every fix names its
**red-first pin** and its `CRATE_TEST_FLOOR` impact. Pins must be built to **fail before** the fix.

### R68 "Silence" — *make the two load-bearing gates actually enforce*

The single highest-leverage release. Three guards that pass while violated.

| Fix | Red-first pin | Floor impact |
|---|---|---|
| **F8-01** — invert `no_sql_in_handlers_enforced` to deny any direct rusqlite surface in `src/handlers/**` | Plant `PRAGMA journal_mode=WAL` + `VACUUM INTO` + `REPLACE INTO` in a handler → guard must FAIL. Also plant `Connection::open` alone. Then migrate the **two existing violations** (`govern.rs:417-419`, `domains.rs:261`) into `src/service/`. | **+2** (one guard self-pin + one migration pin) |
| **F8-05** — count `#[test]` with the comment-stripping scanner; add `MIN_*` floors so an empty walk fails | Plant 10 `#[test]` in a doc comment → floor must still FAIL | +1 |
| **F8-02** — correct `auth.rs:49-54` to say "coverage only" **and** delete the self-asserting pin at `gates.rs:206-214`, replacing it with a pin that fails when `required_action` has no enforcement use | Pin: *if `required_action` is unread by the oracle, fail* — i.e. make the current state RED | +1 |

`ponytail:` **does not** make the authz oracle enforce actions — that is a design decision with a
second-opinion surface the repo already reasoned about and declined. This release makes the *prose*
true and the *pin* honest; it does not change runtime authorization.

### R69 "Erasure" — *close the gap the drill found*

| Fix | Red-first pin | Floor impact |
|---|---|---|
| **F8-08** — carry the approved chunk ids the erasure just deleted and delete their proposals by `id IN (…)` | Approve a proposal whose body does **not** contain its owner's string → purge the owner → assert **zero** surviving rows matching the body. **Fails today.** | +1 |

`ponytail:` **does not** add an owner column to `proposals`; the join is reachable from the
knowledge row, which *is* owner-attributed, so no migration is needed.

### R70 "Seams" — *the cheap enforcement wins*

| Fix | Red-first pin | Floor impact |
|---|---|---|
| **F8-04** — a `LogValue` newtype only constructible via `sanitize_log_value`; pin failing `tracing::*!` in `src/handlers/**` that interpolates a request-derived id | Plant `tracing::warn!("… {domain} …")` → pin FAIL | +1 |
| **F8-06** — replace the `/webhooks/` prefix rule with an explicit `WEBHOOK_PATHS` const; pin that every registered webhook route's body calls a verifier | Add a `/webhooks/noverify` route → pin FAIL | +1 |
| **F8-03** — move the deadline **inside** the `spawn_blocking` closure (the `hostcalls.rs` pattern) so writes refuse to begin rather than being abandoned mid-commit | A write that exceeds the timeout must leave **no** committed row | +1 |
| **F8-10** — `BIND_PORT` refuses boot on a malformed value | `BIND_PORT=abc` → boot must FAIL, not bind 8765 | +1 |
| **F8-07** — add `224.0.0.0/4` and `192.88.99.0/24`; unwrap `::/96` before the v4 arm | Feed each address to `validate_public_addrs` → must be denied | +1 |
| **F8-09** — DSAR roster sweep collects `Result`s, mapping to `DsarError::Database` | Corrupt a `roster_json` → certificate must FAIL, not under-count | +1 |

### R71 "Fork" — *in the openclaw repo, not this one*

`K8-01` (anchored-regex strip, copying `web-search-output.ts:114-115` — a ~3-line diff that removes a
real bypass) · `K8-03` (`<img` + reference-style forms) · `K8-06` (`agentDir` absent ⇒ hard error,
not empty map) · `K8-02` (route link-reader through the image gate) · `K8-05` (move the ack off an
agent-settable env var) · `K8-07` (reconcile typebox to one value so `--frozen-lockfile` passes).
`K8-04` deserves a **decision**, not a patch: either default the toggles on for non-loopback binds,
or record the posture as a declared non-claim.

### R72 "Truth" — *the numbers and the register*

`R8-01` re-baseline AGENTS.md's test count to the measured 3,122 (and stop hand-typing it) ·
`R8-02` repoint or remove the two dead `IMPLEMENTATION_PLAN_v1.11.0_HippoRAG.md` references ·
`R8-03` make `--selfcheck` actually verify the count, or stop calling it a drift check ·
`S8-11` re-baseline "three Cargo.lock files" to eight · `L8-01` re-arm the CT clock ·
`L8-07` fix the two OWASP dates and the internal contradiction · `L8-05` add the two federal EOs ·
`L8-06` run the quarterly check the map instructs · `P8-01` emit the invisible-set fixture for
client + fork · `K8-15` note in the fork docs that fork builds track upstream's appcast.

---

## 9.4 Gate results — every gate executed, red/green reported

| Gate | Command | Result |
|---|---|---|
| **Full suite** | `cargo test --features bench,migrate` | 🟢 **3122 passed / 0 failed / 3 ignored** |
| **clippy (bench)** | `cargo clippy --all-targets --features bench -- -D warnings` | 🟢 exit 0 |
| **clippy (default)** | `cargo clippy --all-targets -- -D warnings` | 🟢 exit 0 |
| **clippy (otel)** | `cargo clippy --all-targets --features otel -- -D warnings` | 🟢 exit 0 |
| **fmt** | `cargo fmt --check` | 🟢 exit 0 |
| **client fmt** | `cargo fmt --manifest-path client/Cargo.toml -- --check` | 🟢 exit 0 |
| **crates workspace** | `cargo test --manifest-path crates/Cargo.toml --all-targets` | 🟢 **308 passed / 0 failed** |
| **steward-harness** | `cargo test --manifest-path tools/steward-harness/Cargo.toml` | 🟢 **44 passed / 0 failed** |
| **lipstyk** | `scripts/lipstyk-gate.sh 43fe5304^` | 🟢 **exit 0** (`changed: src/bin/brain.rs`) |
| **badges selfcheck** | `bash scripts/badges.sh --selfcheck` | 🟢 `OK badges + release checklist self-check clean` |
| **env truth** | `bash scripts/env-truth.sh` | 🟢 `OK env truth clean` |
| **doc links** | `python3 scripts/check-doc-links.py` | 🟢 `checked 404 relative .md links — all resolve` |
| **docs truth** | `bash scripts/docs-truth.sh` | 🟢 exit 0 (LOW=17, documented exclusions) |
| **`cargo audit`** | 4 lockfiles | 🟢 exit 0 ×4 (one yanked-crate warning, S8-08) |

**On lipstyk, precisely:** run against the default base it **refused to pass vacuously** — *"no
changed lines under src client plugin crates."* That refusal is **correct and is the gate working**:
the last eight commits are docs-only. I then ran it against `43fe5304^`, which spans a real code
change, and it returned **exit 0**. I am reporting both because the first result alone would
misrepresent the gate as failing.

**Not run:** the six feature lanes (`compliance-pack`, `multivec`, `injection-classifier`,
`neural-embed`, `loom`, `rerank-tier`), `cargo test --all-targets` under default features, and the
client-gate (wasm + desktop). These are the AGENTS.md pre-push set; this audit changed no code, so
they were not required to validate a no-op.

---

## 9.5 Honest ceilings — what this audit could NOT verify

Stated because a coverage claim is only worth its weakest cell.

1. **Concurrency is the weakest dimension.** I did **not** complete a lock-ordering cycle analysis
   over the ~19 `Mutex`/`RwLock` sites, nor verify poisoning posture per site. F8-03 is about
   cancellation, not lock order. *Next: enumerate holders from the `LockWaitHistogram` registrations
   and check for a cycle.*
2. **FTS/vec index bloat** (dim 4) was not measured.
3. **Metric cardinality** (dim 15) was not audited.
4. **Single-mutex inference serialization and ingest-storm behaviour** (dim 16) were not measured —
   the 64-sentence/16 KiB budget's pre-lock enforcement was not confirmed.
5. **Crypto**: `docs/crypto-inventory.md`'s algorithm rows were not walked against
   `src/ump_integrity.rs` / `src/secrets.rs` / `src/secret_file.rs`; `ALLOWED_ALGS`' exclusion of
   HS256 for asymmetric keys was **not** confirmed beyond the two existing pins.
6. **Chain-head discipline and migration ordering** were not walked end to end.
7. **The fork's red-proofs were not executed.** Every "test that fails on deletion" cell in §5.1 is
   derived from reading test names and assertions — **not** from deleting the hardening and running
   the test. The fork leg's terminal wedged before it could. This is the single largest gap in §5.
8. **`S8-05`'s reachability is host-dependent** — whether openclaw enforces
   `openclaw.plugin.json`'s `configSchema` before `safeParse` decides MEDIUM vs LOW.
9. **`cargo audit` ran against the locally cached advisory DB**; one fetch-enabled run would confirm currency.
10. **Regulatory: see §7.8 in full.** The CRA Art 14 clocks — the most-cited legal claim in the repo —
    were **not verified**. No US statute text was read. Eleven named items are unverified with the
    next check recorded for each.
11. **No live-DB behaviour was tested** — `.bak` plaintext, standby promote RTO, and real chain
    tamper were taken as documented ceilings, not exercised.
12. **Prior audit reports were deliberately not read**, per the brief's "fresh eyes" instruction.
    This report neither inherits nor contradicts their conclusions; where a leg's claim was
    decisive I re-ran the attack myself at final HEAD.

**One methodological note worth recording.** Two legs reported different HEADs because the tree
moved mid-audit (`39443a87` → `e9c71919`). Per the stop rule I re-ran every decisive attack at final
HEAD rather than reconciling by assertion — and in one case (**F8-01**) that re-run produced the
single most important piece of evidence in this report: a guard passing green with two violations
live.