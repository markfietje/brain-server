# EXECUTION PROMPT — R75 "Greenlight"

> **Theme: the tree `main` actually ships must pass the gates that guard it.**
>
> R74 diagnosed six red Rust pins and fixed them, but left `main` **red in CI**
> and shipped **no round notes at all**. R75 finishes what R74 started: it makes
> the committed tree green, and it closes the register rows that let a live
> MED-HIGH defect sit unrouted for a full round.
>
> **No authz change. No route change. No schema change (1.32.26 unchanged). No new
> dependency edge.**

---

## §0 — Baseline, measured at `50406b29` (not transcribed)

Every figure below was **re-measured** at HEAD. Where a predecessor's prose
disagreed with the tree, the tree won and the disagreement is recorded.

| Fact | Value | How established |
|---|---|---|
| HEAD | `50406b29486cc58cbbf68981314826230d087445` | `git log -1 --format=%H` |
| Branch | `main`, **ahead of `origin/main` by 21** | `git branch -vv` |
| Crate / API version | `1.29.2` (`Cargo.toml:3`, `x-api-version` `openapi.yaml:15`) | grep |
| Schema | **`1.32.26`** — `src/storage_layout.rs:301`; written at `src/migration.rs:3188` | grep |
| `CRATE_TEST_FLOOR` | **`2_758`**, measured needle `2_958` (headroom 200) | `src/spire_inventory.rs:177` |
| `ROUTER_SITES_FLOOR` | `255`, measured `258` (**+3**) | `src/spire_inventory.rs:51` |
| `OPENAPI_ROUTE_ROWS_FLOOR` | `214`, measured `217` (**+3**) | `src/spire_inventory.rs:189` |
| `AUTHZ_TABLE_ROWS_FLOOR` | `200`, measured `203` (**+3**) | `src/spire_inventory.rs:199` |
| `MAIN_RS_LINES_MAX` | `300`, measured `124` | `src/spire_inventory.rs:41` |
| Rust suite | **3 160 passed / 0 failed / 3 ignored** across 48 binaries | `badges.sh --verify-count` raw log |
| shell suite | 82 passed / 18 files | `pnpm test` from `shell/` |
| `docs-truth.sh` | `LOW=17`, MED=0, HIGH=0, **exit 0** | `scripts/docs-truth.py:183` |
| `cargo audit` | clean, 514 deps | root `Cargo.lock` |

### §0.1 — THE FINDING THAT DEFINES THIS ROUND

**`main` is red in CI right now.** Two independent gates, both verified in a
pristine `git worktree` of HEAD (not the dirty tree):

```
$ cd /tmp/redcheck/shell
$ pnpm exec openapi-typescript ../openapi.yaml -o "$tmp"; echo "GEN_EXIT=$?"
GEN_EXIT=0
$ cmp src/lib/api/schema.d.ts "$tmp"; echo "CMP_EXIT=$?"
cmp: .../schema.d.ts <tmp> differ: char 215304, line 4605
CMP_EXIT=1
$ pnpm test 2>&1 | tail -6
 Test Files  2 failed | 16 passed (18)
      Tests  1 failed | 78 passed | 3 skipped (82)
TEST_EXIT=1
```

- `.github/workflows/shell.yml:87-94` (`cmp` in job `shell`) → **RED** on `CMP_EXIT=1`.
- `.github/workflows/shell.yml:85` (`pnpm test`) → **RED**. `tests/drift-gate.test.ts`
  fails on the byte diff; `tests/registry-contract.test.ts` fails to even load:
  `Error: Every operation must have a unique 'operationId'.`

**Two independent root causes, not one:**

**(A) A duplicate `operationId` — invalid OpenAPI, and the harder failure.**
`openapi.yaml:1525` (`POST /verify`) and `openapi.yaml:9163`
(`POST /workflow/claims/{id}/verify`) both declare `operationId: verifyClaim`.
`openapi-typescript`'s **CLI** does not validate uniqueness (`GEN_EXIT=0`), which
is why this went unnoticed; its **library** entry point does, which is why
`registry-contract.test.ts` hard-fails. **Regression archaeology: the string
`verifyClaimGate` has NEVER existed in `openapi.yaml` on any branch** —
`git log -S'verifyClaimGate' -- openapi.yaml` returns **nothing**. The duplicate
was born in `37212f57` (which added the second path reusing the first's id).
**The claim at `docs/EXECUTION_PROMPT_R70_Seams.md:323-325` that this rename
"is already fixed in R69's follow-up" is FALSE** — that fix was never committed.

**(B) `openapi.yaml` moved ahead of `schema.d.ts`.** `schema.d.ts` was last
regenerated at `2a775811` (R67D); `openapi.yaml` was last touched at `43fe5304`,
which shipped two hunks and did **not** regenerate. Cause (B) alone made `main`
red *before* the duplicate existed.

**The working tree already holds the complete fix and is verified green:**
`DIRTY_GEN_EXIT=0`, `DIRTY_CMP_EXIT=0`, `DIRTY_TEST_EXIT=0`
(`Test Files 2 passed (2)`). **The rename is safe**: `grep -rn 'verifyClaim'`
over `shell/src shell/e2e shell/tests` returns **only** the generated
`schema.d.ts` itself; no consumer references the symbol, and `client.ts:14` keys
off `paths`, not operation ids. `x-api-version` correctly stays `1.29.2` — a
symbol rename on an unconsumed id is not a contract move.

### §0.2 — A second red gate, and it blocks the push too

```
$ bash scripts/badges.sh --verify-count; echo "EXIT=$?"
ERR: README test-count badge drifts from the build — badge says 3158, the run derives 3160.
EXIT=1
```
Wired at `.github/workflows/ci.yml:64-85`, step *test-count badge drift gate*,
inside job **`lint-test`** — so **CI is red on two independent jobs.**

The +2 is **not** a failing test. `CARGO_EXIT=0`, all 49 result lines are
`test result: ok. … 0 failed`. `scripts/badges.sh:38-47` scrapes **only**
`[0-9]+ passed`, so a failure cannot inflate the count and the 3 ignored tests
are excluded. The +2 is exactly R73's two new `#[test]`s
(`src/reg_watch.rs:328` art50 citation pin, `tests/main_suite.rs:17915` register
disposition pin) landing after the badge was last baselined at `77eb2aa5`.
`--selfcheck` is **green** (exit 0).

### §0.3 — Three pieces of uncommitted pollution, not the fix

| Path | Nature | Action |
|---|---|---|
| `tools/signal-gateway/Cargo.lock` | **13 lines moved, 6 packages** (`clap` 4.6.6→4.6.7, `reqwest` 0.13.4→0.13.5, `base64` 0.22.1→0.23.1, `tokio` 1.53.1→1.53.2, `uuid` 1.25.0→1.27.0, `clap_derive` 4.6.4→4.6.7) — an artifact of a `cargo test` run in that sub-tree. `Cargo.toml` unchanged, so **no new declared edge**, but it contradicts the rounds' "all `Cargo.lock` byte-identical" figure. | **revert** |
| `./.svelte-kit/` at repo **root** | untracked, **NOT gitignored** (`shell/.svelte-kit` is ignored at `shell/.gitignore:3`; the root one is not) — would ride into any `git add -A` | **delete** |
| `/private/tmp/r74head` | a registered `git worktree` at `ed9c24ab`, left by a prior agent | **remove** |

---

## §1 — Gaps, prioritized, each with the evidence that it is real

### P0 — `main` does not pass its own CI (two jobs red)
- **W1** commit the wire fix: `openapi.yaml` `verifyClaim` → `verifyClaimGate` at
  `:9163` **plus** the regenerated `shell/src/lib/api/schema.d.ts`. Fixes §0.1 (A)
  and (B) together — the regeneration is what picks up `43fe5304`'s two hunks.
- **W2** re-baseline the badge: `README.md:34`, `3158` → `3160`, in **both** the
  `alt` text and the `src` URL.

### P1 — S8-01: a live MED-HIGH defect, unrouted, with no test and no CI
`tools/signal-gateway/src/main.rs:104-119` holds **two independent `if`s**:
```rust
if !addr.ip().is_loopback()
    && std::env::var("SIGNAL_GATEWAY_ALLOW_REMOTE").as_deref() != Ok("1")
{ anyhow::bail!(...) }                       // consumes `addr`, returns

let app = if let Some(token) = &config.server.auth_token {
    api::create_router_with_auth(state, Some(token.clone()))
} else {
    info!("API auth: NONE (loopback-only posture)");   // asserts a posture
    api::create_router(state)                            // nothing enforces
};
```
The second branch **never reads `addr`**. `api/mod.rs:30-32` makes `create_router`
= `create_router_with_auth(state, None)`, and `:37-55` adds the bearer middleware
**only** in the `Some` arm. So `SIGNAL_GATEWAY_ALLOW_REMOTE=1` with no
`auth_token` serves **10 routes unauthenticated on a public interface**, including
`POST /v2/send` (outbound messaging), `GET /api/v1/accounts` (enumeration) and
`GET /api/v1/events` (SSE). The log line asserts a "loopback-only posture" that
no code in that branch establishes.

Nothing can catch it: **`tools/signal-gateway/tests/` does not exist**, no
`#[cfg(test)]` module touches `auth_token` / `create_router` / `ALLOW_REMOTE` /
`is_loopback`, and `is_loopback` occurs **exactly once** in the whole crate.
`AUDIT.md:1014` records it as `OPEN — UNROUTED`.

The register already names the remedy (`AUDIT.md:1014`): *"in the `None` arm,
re-assert `addr.ip().is_loopback()` and `bail!` otherwise — make auth a
function of the bind."* **That is the whole fix.**

- **W3** implement the bind-coupled auth guard.
- **W4** add `tools/signal-gateway/tests/` with real tests for it.
- **W5** add the missing CI job — the crate's **20 tests never run in CI**
  (`grep -rn 'signal-gateway' .github/workflows/` hits a *comment only*, at
  `ci.yml:186`). Model it on `channel-bridge-gate` (`ci.yml:184-202`).

**Honest scoping.** This is a genuine fail-open defect in shipped code, but
**latent, not deployed**: `grep -rn 'signal-gateway' scripts/ deploy/ *.sh`
returns nothing, so no in-repo installer deploys it. The round's claim is
"unguarded and ungated", **not** "actively exploited". **S8-04** (the rate
limiter is a dead module, so `/v2/send` has no request-rate control, amplifying
S8-01) is **NOT fixed here** — wiring a rate limiter is a design decision with
its own config surface, and it is registered as a residual in §5 rather than
silently absorbed.

### P2 — R74 shipped two commits and zero round notes
`15964613` and `50406b29` are on `main`. There is **no** `CHANGELOG.md` R74
section (`grep -c R74` → 0), **no** `docs/EXECUTION_PROMPT_R74_*.md`, and
`AGENTS.md:3` still reads `Current release (unreleased): **R73 "Receipts"**`.
`tests/main_suite.rs:18064` reads
`const SHIPPED_ROUNDS: [&str; 5] = ["R68", "R69", "R70", "R72", "R73"];` —
**R74 absent**, so the anti-drift guard cannot catch a row naming it. The most
recent *complete* round is R73.

- **W6** add `"R74"` (and `"R75"`) to `SHIPPED_ROUNDS`; write R74's and R75's
  round notes.

### P3 — The register is blind to live in-repo findings, and its enforcement map is stale
- **S8-02** (`docs/audit8/02-satellites-supply-chain.md:31-40`) is **absent from
  `AUDIT.md` entirely** (`grep -c 'S8-02' AUDIT.md` → 0), and it is a live MEDIUM
  in-repo defect: `tools/valet-relay/relay.js:71-77` verifies the MAC with
  `crypto.timingSafeEqual` but **never checks that `ts` is recent**, and `:139`
  is the only gate. A captured, correctly-signed envelope replays indefinitely,
  re-firing an operator alert via `sendSignal()`.
  **NOT fixed in R75** (it changes a live wire listener; that is its own round) —
  but it is **registered**, which is the gap being closed.
- **S8-04** likewise absent; registered as the residual amplifier of S8-01.
- **D8-02** claims no enforcement map exists. One **does** —
  `AUDIT.md:466-483` — and **4 of its 9 figures are stale** against
  `src/spire_inventory.rs` (claims `CRATE_TEST_FLOOR` 1 196 / router 199 /
  openapi 161 / authz 145; the code says **2 758 / 255 / 214 / 200**). It also
  carries no per-guard red-proof column. Repairing it is cheap and it is
  *itself* a register-disagrees-with-the-code instance.

---

## §2 — Files touched, exactly

| # | File | Change |
|---|---|---|
| W1 | `openapi.yaml` | `:9163` `operationId: verifyClaim` → `verifyClaimGate` |
| W1 | `shell/src/lib/api/schema.d.ts` | regenerate via `openapi-typescript` (never hand-edit) |
| W2 | `README.md:34` | badge `3158` → `3160` (alt text **and** src URL) |
| W3 | `tools/signal-gateway/src/main.rs` | bind-coupled auth guard in the `None` arm |
| W4 | `tools/signal-gateway/tests/s8_01_bind_coupled_auth.rs` | new — real behavioural tests |
| W5 | `.github/workflows/ci.yml` | new `signal-gateway-gate` job |
| W6 | `tests/main_suite.rs:18064` | `SHIPPED_ROUNDS` 5 → 7 |
| W6 | `AUDIT.md` | add `S8-02`, `S8-04` rows; repair the §466-483 enforcement map; correct `S8-05`'s routing |
| W6 | `CHANGELOG.md`, `AGENTS.md` | R74 + R75 round notes |
| — | `tools/signal-gateway/Cargo.lock` | **revert** (pollution, §0.3) |
| — | `./.svelte-kit/` | **delete** (pollution, §0.3) |
| — | `/private/tmp/r74head` | `git worktree remove --force` |

**Explicitly NOT touched:** `src/authz/**` (zero diff), `src/migration.rs` (zero
diff), schema stays **1.32.26**, `shell/pnpm-lock.yaml`,
`shell/pnpm-workspace.yaml`, `CRATE_TEST_FLOOR` (**stays 2 758** — `AUDIT.md:1001`
explicitly forbids re-baselining it), and the shell `pnpm` pin (verified already
correct and committed: `packageManager: pnpm@12.3.4` at `shell/package.json:11`,
`pnpm/action-setup` `version: 12.3.4` at `.github/workflows/shell.yml:39`, both
introduced by `d1c4a586` / `0c3c0794` and ancestors of HEAD).

---

## §3 — Implementation notes that will bite if skipped

1. **Regenerate, never hand-edit `schema.d.ts`.** CI byte-compares it. The pin's
   whole value is that it is not a human's transcription.
2. **The `None` arm must refuse, not warn.** `anyhow::bail!` before
   `TcpListener::bind`, so no socket is ever opened. Mirroring R70's
   `write_deadline` shape: the check is a **precondition**, not a log line.
3. **Do not touch the `Some` arm.** A non-loopback bind **with** a token is the
   intended remote posture and must keep working — `SIGNAL_GATEWAY_ALLOW_REMOTE=1`
   plus a configured token stays valid. Over-refusing would break a deployment.
4. **Remove or reword the lying log line.** `info!("API auth: NONE (loopback-only
   posture)")` is currently a false statement; after the fix it becomes true, and
   the wording must not claim a posture the code no longer guarantees.
5. **Do not log the token.** The `Some` arm's message must stay as-is.
6. **The S8-01 test must drive the real decision, not a re-implementation.**
   Extract the bind/auth decision into one small pure function both `main.rs` and
   the test call — otherwise the test asserts a copy and the seam is unforced.
   This is exactly the defect class R73 was filed about.

---

## §4 — Verification, exact commands

### Red-first proof (required before the W3 fix lands)
```
cd tools/signal-gateway
# 1. the guard does not exist yet → the test must FAIL
cargo test --manifest-path tools/signal-gateway/Cargo.toml --test s8_01_bind_coupled_auth
# 2. then land the fix and re-run → must PASS
```

### The full gate
```sh
cd /Users/mark/Sites/brain-server

# — the two red gates —
bash scripts/badges.sh --verify-count; echo "BADGES_EXIT=$?"     # want 0
cd shell && pnpm test; echo "SHELL_TEST_EXIT=$?"                  # want 0
cd shell && tmp="$(mktemp)" && pnpm exec openapi-typescript ../openapi.yaml -o "$tmp" \
  && cmp src/lib/api/schema.d.ts "$tmp"; echo "DRIFT_EXIT=$?"    # want 0
cd shell && pnpm check; echo "SVELTE_CHECK_EXIT=$?"              # want 0
cd shell && pnpm exec tsc --noEmit -p tsconfig.json; echo "TSC_EXIT=$?"

# — the Rust gate —
export RUSTFLAGS="-D warnings"
cargo fmt --all -- --check; echo "FMT_EXIT=$?"
cargo clippy --all-targets --features bench -- -D warnings; echo "CLIPPY_EXIT=$?"
cargo test --features bench; echo "TEST_EXIT=$?"

# — the new crate gate (mirrors ci.yml signal-gateway-gate) —
cargo fmt --manifest-path tools/signal-gateway/Cargo.toml --all -- --check
cargo clippy --manifest-path tools/signal-gateway/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/signal-gateway/Cargo.toml

# — house gates —
bash scripts/badges.sh --selfcheck
bash scripts/env-truth.sh
bash scripts/docs-truth.sh                       # LOW=17 pre-existing, MED/HIGH must be 0
python3 scripts/check-doc-links.py
bash scripts/lipstyk-gate.sh
cargo audit --file Cargo.lock

# — spire floors, must print the §0 figures —
cargo test --features bench --lib spire_inventory 2>&1 | grep -E 'spire:|test result'
```

### Invariants that must hold afterwards
- `git diff --stat src/authz/ src/migration.rs` → **empty**.
- `grep -n 'SCHEMA_VERSION_V1_32_26' src/storage_layout.rs` → still `1.32.26`.
- `grep -n 'CRATE_TEST_FLOOR: usize' src/spire_inventory.rs` → still `2_758`.
- `git diff --stat -- '**/Cargo.lock' Cargo.lock` → **empty** (no new dep edges).
- `grep -c 'badge/tests-3160' README.md` → `1`.

### Rollback
Every change is additive and independently revertable. `git revert <sha>` per
commit. The only shared-state risk is `schema.d.ts`: reverting `openapi.yaml`
without reverting the regenerated `schema.d.ts` **re-breaks `main`**, so those two
must move in the **same commit**. No migration, no schema bump, no irreversible
change in this round.

---

## §5 — Named residuals (stated, not silently absorbed)

| Item | Why not in R75 |
|---|---|
| **S8-04** — dead rate limiter; `/v2/send` has no rate control | wiring one is a design decision with its own config surface; recorded as a register row |
| **S8-02** — `valet-relay` never checks `ts` freshness (replayable operator alerts) | registered; fixing it changes a live wire listener and deserves its own round |
| **S8-05** — plugin `resolveConfig` bare type assertion (`"false"` resolves truthy) | the file is **in this repo** (`plugin/src/config.ts:234-235,254`) but the register routes it to R71; R75 corrects the routing and records that the audit's own open question ("does the host validate?") is answered: the plugin's exported entry points do **not** — `brainConfigSchema` is referenced only at its own `:16` declaration and `:78` type alias, never to validate |
| **S8-07** — 4 of 13 `crates/` members are unconsumed islands | needs a wire/delete **decision**; also corrected (`brain-troubleshoot-core` **is** consumed by `tools/steward-harness`, so "6 of 13" over-counts) |
| **F8-02** | `decide_gate_verdict` still does not read `required_action` — a **declined** decision (`src/authz/policy.rs:205-220`), not a bug. Recording it as closed would repeat the defect the finding was filed about |
| **F8-03** | no idempotency/receipt registry; ~50 other `spawn_blocking` write handlers still admit the window |
| **L8-04, L8-06, L8-07, L8-11, L8-05** | external acts (deployer identity, publisher fetch, BIS/ECFR analysis) or correctly deferred; **no legal conclusion added** |
| **K8-01…K8-15, D8-01, K8-04** | the **openclaw fork** at `~/Sites/openclaw` @ `1d2d29b22`, a different repository. K8-04 is a **decision** |
| **`skipLibCheck: true`** in `shell/tsconfig.json` suppresses `tsc` diagnostics **inside** `schema.d.ts` — which is why a duplicate identifier was invisible to the type-checker and surfaced only through the generator's own validation | flagged, not changed: flipping it is a separate decision with its own blast radius |

---

## §6 — The law this round must not break

- **Do not "helpfully" re-baseline `CRATE_TEST_FLOOR`.** `AUDIT.md:1001`: raising
  it would spend the guard's headroom on a measurement rather than on a round.
- **Do not soften or delete a test to reach green.** Every figure here is measured.
- **Do not claim conformance the repo cannot support.** `docs/cra.md:53` already
  warns against it; this round adds no legal claim.
- **Red-first.** Each fix is proven able to fail before it is proven green.
- **Three spire table floors have +3 headroom each.** Any route addition this
  round would consume it. R75 adds **no route**, which is why it stays green.
