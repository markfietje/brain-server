# EXECUTION PROMPT — R76 "Cadence"

> **Theme: the two messaging edges never asked *when* or *how often*.**
>
> S8-02: the valet-relay verifies **who** signed an alert (HMAC, constant-time)
> but never asks **when** it was signed — a captured envelope replays forever.
> S8-04: signal-gateway owns a rate limiter it never calls — `/v2/send` fires
> real Signal-network messages at whatever pace a caller wants. One fix per
> edge; both red-first; both wired to CI that actually runs them.
>
> **No authz change. No route change. No schema change (1.32.26 unchanged).
> Zero new dependency edges: both `tools/*/Cargo.lock` files stay byte-identical
> to HEAD — the R75 lockfile decision is NOT re-opened by this round.**

---

## §0 — Baseline, measured at `ec5b6559` (R75 tip), not transcribed

| Fact | Value | How established |
|---|---|---|
| HEAD | `ec5b655969f09a267df8f3f190491577a299f6f8` (2026-10-06 00:31 +0800) | `git log -1` |
| Working tree | **clean** — working tree == committed tree | `git status --short` (empty) |
| Crate/API version | `1.29.2`; schema **1.32.26** (`src/storage_layout.rs:301`) | grep |
| `CRATE_TEST_FLOOR` | **2 758** (`src/spire_inventory.rs:177`), up-only, **do not raise** | grep |
| Badge | `3160` at `README.md:34` (machine-derived) | grep |
| Register rows | S8-02 at `AUDIT.md:1015`, S8-04 at `AUDIT.md:1016`, both `**OPEN — UNROUTED (registered R75; the code is untouched).**` | read |
| `SHIPPED_ROUNDS` | `[&str; 7]` = R68,R69,R70,R72,R73,R74,R75 at `tests/main_suite.rs:18064` | read |
| Closed-list arm | `:18088-18091`, ends `"S8-01"` | read |
| CI | 19 jobs; `signal-gateway-gate` at `ci.yml:204-224`; **zero jobs touch `valet-relay`** (`grep -rn valet .github/workflows/` → exit 1) | grep |
| `R76` pre-started | **zero** (only `R760` substring in a fixture) | `grep -rnw R76` |

### §0.1 — S8-02 ground truth (all read from the tree)

`tools/valet-relay/relay.js` (227 lines, CommonJS, **no package.json, no tests,
only README + relay.js**):

- `verifyAlert` (`:71-77`): checks `v1,` prefix, recomputes HMAC-SHA256 over
  `${id}.${ts}.${body}`, compares with `crypto.timingSafeEqual` — **correct,
  constant-time, and never asks whether `ts` is recent**.
- The only gate is `:139`; `sendSignal()` fires at `:153`.
- Top-level side effects: `loadConfig()` at `:60` **dies (exit 1) unless a 0600
  config file exists**, `http.createServer(...).listen(CFG.listen_port, '127.0.0.1')`
  at `:128-159`, `setInterval(pollOnce, 15000)` at `:213`.
- Existing test seam: `--selftest` (`:217-228`) — sign/verify round-trip, tamper
  detection, kind mapping — but it exits via `die()` and checks nothing about
  freshness.
- `seenEnvelopes` (`:163`, a Set, bounded at 500/FIFO-half) dedups the **OUTBOUND**
  path only.

**The producer's actual behaviour — verified in `src/alert.rs`, and it changes
the naive fix:**

1. **Retry semantics** (`src/alert.rs:508-535`): the kernel's alert sink sets
   `ts` ONCE (`chrono::Utc::now().to_rfc3339()`), then retries up to 3 times
   **with the same `delivery_id` and the same `ts`** (backoff `2^attempt` s).
   ⇒ A relay-side **id-dedup would eat legitimate retries** (lost-response
   after forward = silently dropped operator alert). **Freshness is the fix;
   id-dedup is explicitly DECLINED** — recorded as a decision, not an omission.
2. **Timestamp format** (`src/alert.rs:510`): the kernel sends
   `webhook-timestamp` as **RFC3339**, while the Standard Webhooks spec
   (verified via Context7, `/standard-webhooks/standard-webhooks`, 2026-10-06)
   defines **epoch seconds**. A freshness check that only parses epoch would
   `NaN` on every real kernel-sent envelope — a green test suite over a fix
   that rejects all legitimate traffic. **Parse both**: all-digits → epoch;
   otherwise `Date.parse` (RFC3339).
3. **The tolerance to mirror** — the kernel's own inbound law
   (`src/config.rs:892-896` `WEBHOOK_REPLAY_SECS: u64 = 300`; `src/webhook.rs:41-45`
   `WEBHOOK_TS_FUTURE_SKEW_SECS: u64 = 300`; enforced in `enqueue_ts`,
   `src/webhook.rs:267-275`). The spec's reference implementation agrees:
   `TOLERANCE_IN_SECONDS = 5 * 60`, rejected if `now - ts > 300` or
   `ts > now + 300`. **Constant: 300 both directions. Not a knob** — no env
   var (the repo's env-truth gate treats undocumented knobs as findings).

### §0.2 — S8-04 ground truth (all read from the tree)

`tools/signal-gateway/src/ratelimit.rs` (122 lines):

- Sliding-window log: `HashMap<String, Vec<Instant>>` under
  `Arc<parking_lot::RwLock<...>>`; `is_allowed(&self, ip: &str) -> bool` takes
  the **write** lock; keying is whatever the caller passes.
- `create_rate_limiter()` (`:83-85`) hardcodes `RateLimiter::new(100, 60)`.
- `#![allow(dead_code)]` at `:2` is **why the dead module compiles silently**
  under an otherwise `unwrap_used/expect_used/panic = deny` crate.
- `is_allowed` is referenced **now outside its own file** — sole reference is
  `main.rs:27`'s `mod ratelimit;`.
- Known wart: duplicated `checked_sub` comment lines at `:36-39` and `:58-61`.
- Its 4 existing tests pin allow/block/per-key/remaining, and **nothing
  expires** — there is no clock seam, so no window-passage test exists.

Router/serve ground truth:

- `axum::serve(listener, app)` plain (`main.rs:129-140`) — **no
  `into_make_service_with_connect_info`**; per-IP keying would need new
  plumbing, and in the default loopback posture every client is `127.0.0.1`
  anyway, so per-IP discrimination is illusory (and behind a proxy it is one
  shared IP regardless). **Global keying is the honest choice.**
- The router is built in `api/mod.rs` (`create_router_with_auth`, `:37-80`),
  a **binary-only** tree — `mod api` is declared from `main.rs`, so
  integration tests cannot reach it. The house pattern for provable decisions
  is a pure function in `lib.rs` (S8-01's `resolve_api_auth`).
- `AppState { signal: SignalHandle }` (`state/mod.rs:9-12`) — adding the
  limiter to it is possible but unnecessary (see the closure design in §2).
- Flooding `/v2/send` amplifies into **presage native sends over the live
  identity's websocket** (`worker.rs:662-694`); the existing
  `max_sends_per_second: 5` config name is misleading — the `Arc<Semaphore>`
  at `worker.rs:216` is a **concurrency cap (5 in-flight), not a rate limit**,
  and the command channel queue is bounded (64) but its waiter queue is not.
- Deps (`Cargo.toml`): `reqwest` (main, direct), `tokio` full enough for
  `#[tokio::test]`, `parking_lot`, `axum 0.8.9` — **NO `tower`**, and adding
  it would force a lockfile re-lock (declined). Integration tests CAN use the
  package's regular dependencies, so a real-socket test on `127.0.0.1:0` with
  a `reqwest` client needs **zero new deps**.

---

## §1 — The plan

### W1 — S8-02: freshness at the alert sink (JS)

1. **Extract the pure decision**: add
   `function freshTimestamp(tsHeader, nowMs)` — parses all-digit headers as
   epoch seconds and anything else as RFC3339 via `Date.parse`; returns
   `false` on unparsable input (fail-closed); admits only
   `now - 300 <= t <= now + 300`. Constant `FRESHNESS_TOLERANCE_SECS = 300`
   with a comment citing all three authorities (spec reference lib, kernel
   `WEBHOOK_REPLAY_SECS`, kernel `WEBHOOK_TS_FUTURE_SKEW_SECS`).
2. **Wire it in `verifyAlert`** (after the prefix check, before/after the MAC —
   order documented: MAC first is fine, freshness is a second independent gate;
   both must pass). `verifyAlert` gains an optional `nowMs` parameter
   (defaulting to `Date.now()`) so tests can inject the clock — the same
   seam-and-delegate shape the Rust side uses.
3. **Testability refactor, minimal**: wrap the side effects — the
   `createServer().listen()` block and `setInterval` — behind
   `if (require.main === module)`, and add
   `module.exports = { sign, verifyAlert, freshTimestamp, envelopeToText }`.
   No behaviour change when run as a process. **`--selftest` stays.**
4. **Real tests**: `tools/valet-relay/relay.test.js` using the built-in
   `node:test` runner (zero deps). Cases, all with injected clocks:
   - fresh RFC3339 ts → accepted (and HMAC still valid end-to-end via `sign`);
   - fresh epoch ts → accepted (spec-conformant producer);
   - **RFC3339 exactly 301 s old → REFUSED** (the S8-02 replay, red-first);
   - epoch 301 s old → REFUSED;
   - future-dated +301 s → REFUSED (skew arm, mirrors `enqueue_ts`);
   - unparsable ts string → REFUSED (fail-closed);
   - tampered body still refused (existing guarantee, pinned);
   - wrong-`v1,`-prefix still refused.
   Plus one **integration** test that spins a real loopback HTTP sink for
   `signal_send_url`, posts a stale envelope to a real relay spawned as a
   child process (`spawn` with `--selftest`-style config fixture in a temp
   dir), and asserts 401 + no Signal forward. Child-process route avoids
   depending on the export guard for the integration path.
5. **CI**: new `valet-relay-gate` job in `ci.yml`, slotted after
   `signal-gateway-gate`, before `otel-gate` (~`:226`), reusing the repo's
   pinned action SHAs (`actions/checkout` v7 SHA, `actions/setup-node` — copy
   the exact pinned SHA already used by `ci.yml`'s integration job, which runs
   `node-version: 22`). Steps: checkout, setup-node, `node --test tools/valet-relay/`.
   This is the finding's other half: **the relay's tests ran in no workflow**.
6. **Extend `--selftest`** with a freshness arm (stale ts refused, fresh
   accepted) so the operator-run check grows with the fix.

### W2 — S8-04: wire the limiter (Rust)

1. **Move `ratelimit` into the lib target.** `lib.rs` declares
   `pub mod ratelimit;`; `main.rs` **deletes** its `mod ratelimit;` and the bin
   consumes `signal_gateway::ratelimit::…`. One definition, both consumers,
   integration tests reach the real limiter.
2. **Add the clock seam + fix the warts in `ratelimit.rs`**: make the core
   `fn admit_at(&self, key: &str, now: Instant) -> bool` (prune → decide →
   record, one write-lock critical section), with `is_allowed` delegating
   `admit_at(key, Instant::now())`. **Window expiry becomes testable for
   real** — the gap the auditor named. Delete the duplicated comment lines
   (`:36-39`, `:58-61`). Evict empty per-key vecs after pruning (2 lines;
   removes the unbounded-map hazard class even though the global key makes it
   moot — and the comment says so).
3. **The production constants move to the lib** so prod and tests cannot
   drift: `pub const API_RATE_LIMIT_MAX_REQUESTS: usize = 100;`,
   `pub const API_RATE_LIMIT_WINDOW_SECS: u64 = 60;`,
   `pub const API_RATE_LIMIT_KEY: &str = "api";` and `create_rate_limiter()`
   uses them. Pin the constants in a test.
4. **The layer, in the lib, generic over state**:
   ```rust
   pub fn apply_rate_limit<S>(router: axum::Router<S>, limiter: RateLimiter) -> axum::Router<S>
   ```
   implemented with `axum::middleware::from_fn` **closing over a cloned
   limiter** (no AppState change, no `with_state` coupling). On refusal:
   **429** with a `RETRY-AFTER` header (window secs) and an empty body; one
   `tracing::debug!` per refusal carrying the key and nothing request-derived.
5. **Wire it in `api/mod.rs`**: after the auth `match`, wrap the finished
   router — `let app = signal_gateway::apply_rate_limit(app, limiter);` — so
   the limit sits **outside** auth and throttles unauthenticated floods too.
   `main.rs` builds the limiter once via `create_rate_limiter()` beside the
   auth resolution, and logs the posture once at boot (info, with the
   constants).
6. **Deaden honestly**: remove the module-level `#![allow(dead_code)]`;
   `is_allowed`/`new`/`create_rate_limiter`/`admit_at` are now genuinely used
   (prod or tests), so the compiler polices future deadness. `remaining` and
   `reset` keep narrow `#[allow(dead_code)]` **with a comment** naming who may
   consume them later — or, better, drop them entirely if nothing can justify
   them (decision at implementation; either way the module stops being
   silently-dead).
7. **Real tests** (`tools/signal-gateway/tests/s8_04_rate_limit_wired.rs`):
   - Behavioural, through the lib: budget refusal at the boundary; **window
     expiry admits again** (injected `Instant`s — the red-proof the old
     module could not express); per-key isolation; empty-key eviction;
     constants pinned.
   - **End-to-end, real socket**: build a `Router` in the test with a tiny
     handler, wrap it with `apply_rate_limit`, `tokio::spawn` a serve on
     `127.0.0.1:0`, fire budget+1 requests with `reqwest`, assert N × 200 then
     **429 + `RETRY-AFTER`**. Then a second limiter instance with a fresh
     window passes — proving expiry through the real HTTP path.
   - **Structural pin** (house pattern): read `src/api/mod.rs` and
     `src/main.rs`, assert `apply_rate_limit(` is called in the router path
     and `mod ratelimit;` is GONE from `main.rs` — so a future refactor cannot
     silently unwire the layer (the exact defect class this finding is).
   - **Anti-vacuity**: mutation-proof by construction — the e2e test fails if
     the layer stops rejecting (budget 0 ⇒ all 429; budget ∞ ⇒ no 429), and
     the structural pin fails if the wiring is deleted.

### W3 — Register, pins, round notes

1. `AUDIT.md:1015` S8-02 → `**CLOSED — R76, and the fix is smaller than the
   finding's own remedy suggested.**` … recording: freshness only (both
   directions, 300 s, matching `enqueue_ts`'s two-sided law and the spec's
   `TOLERANCE_IN_SECONDS`); **id-dedup deliberately DECLINED** because the
   producer retries with the same id and a lost response would then silently
   eat a real operator alert (`src/alert.rs:508-535`); RFC3339-vs-epoch dual
   parse because the kernel actually sends RFC3339 (`alert.rs:510`); loopback
   scoping unchanged; residual = a within-window replay fires once more, and
   `node --test` + the new `valet-relay-gate` job now hold the property.
2. `AUDIT.md:1016` S8-04 → `**CLOSED — R76, wired rather than deleted.**` …
   recording: global keying (ConnectInfo plumbing declined — loopback posture
   makes per-IP illusory, proxy collapses it anyway); 100 req/60 s constants
   in the lib, module moved out of the binary-only tree, `#![allow(dead_code)]`
   removed so the compiler now polices deadness; 429+`RETRY-AFTER`, layered
   outside auth; the semaphore's `max_sends_per_second` naming is a misleading
   concurrency cap (recorded, not renamed — config-surface change is a
   decision); residual = a burst of 100 still reaches Signal, and SSE
   long-polls share the budget.
3. `tests/main_suite.rs`: `SHIPPED_ROUNDS` → `[&str; 8]` adding `"R76"`
   (`:18064`); closed-list arm appends `"S8-02", "S8-04"` (`:18090`).
4. Round notes: `CHANGELOG.md` R76 section at line 7; `AGENTS.md` R76 block at
   `:3` with `Predecessor: **R75 "Greenlight"**` demoted into the chain;
   `docs/EXECUTION_PROMPT_R76_Cadence.md` (this file) committed beside its
   predecessors (needs `git add -f` — `.gitignore:107` `EXECUTION_PROMPT_*.md`).
5. Badge: root-workspace test count should NOT move (all new tests live in
   `tools/` and JS). **Re-derive, never hand-type**: if `--verify-count` says
   otherwise, follow the measurement and re-baseline in a dedicated commit.

---

## §2 — Files touched, exactly

| # | File | Change |
|---|---|---|
| W1 | `tools/valet-relay/relay.js` | `freshTimestamp` + wire into `verifyAlert` (`nowMs` seam); `require.main` guard; exports; selftest arm |
| W1 | `tools/valet-relay/relay.test.js` | new — `node:test`, clock-injected |
| W1 | `.github/workflows/ci.yml` | `valet-relay-gate` job (checkout + setup-node + `node --test`) |
| W2 | `tools/signal-gateway/src/lib.rs` | `pub mod ratelimit;` + constants + `apply_rate_limit` |
| W2 | `tools/signal-gateway/src/ratelimit.rs` | `admit_at` clock seam; eviction; comment dedup; allow-scope fix |
| W2 | `tools/signal-gateway/src/main.rs` | delete `mod ratelimit;`; build limiter; boot log |
| W2 | `tools/signal-gateway/src/api/mod.rs` | wrap router with `apply_rate_limit` |
| W2 | `tools/signal-gateway/tests/s8_04_rate_limit_wired.rs` | new — behavioural + real-socket + structural pin |
| W2 | `tools/signal-gateway/README.md` | rate-limit posture paragraph |
| W3 | `AUDIT.md` | two rows → CLOSED — R76 |
| W3 | `tests/main_suite.rs` | `SHIPPED_ROUNDS` 7→8; closed list + S8-02, S8-04 |
| W3 | `CHANGELOG.md`, `AGENTS.md` | R76 notes |
| — | `tools/*/Cargo.lock` | **must stay untouched** (revert before commit if cargo re-dirties) |

**Explicitly NOT touched:** `src/authz/**`, `src/migration.rs`, schema
(1.32.26), `CRATE_TEST_FLOOR` (2 758), `openapi.yaml`, `shell/`, the kernel's
`alert.rs` sink (its retry semantics are the reason the relay fix is
freshness-only), and no new dependency anywhere.

---

## §3 — Implementation notes that will bite if skipped

1. **Test the RFC3339 path first.** A fresh-epoch-only test suite passes while
   the real producer (RFC3339) is rejected wholesale. The first test case must
   use the format `src/alert.rs:510` actually emits.
2. **Do not dedup on `webhook-id`.** The producer reuses ids across retries
   deliberately; dedup trades a duplicate alert for a silently lost one. The
   spec's idempotency-key advice applies to the receiver's *processing*, and
   this relay's processing (a Signal send) must not be deduped.
3. **The limiter wraps the FINISHED router** (post-`with_state`, post-auth
   match), so `apply_rate_limit<S>` stays generic and the layer sits outermost.
   Putting it inside `create_router_with_auth` before the `match` would leave
   one arm unwrapped.
4. **`#[tokio::test]` + real `TcpListener::bind("127.0.0.1:0")` + reqwest** is
   the whole e2e harness — no tower, no new dev-dep, no lockfile touch.
5. **The relay test needs a config fixture**: `loadConfig()` dies without a
   0600 file. Tests write one into a temp dir (mode 0600!) and point
   `BRAIN_CONNECTOR_CONFIG_DIR` at it — per-test isolation, no env race
   (sequential `node --test` default is fine).
6. **Numbers with sources**: the JS tolerance constant cites spec lib
   (`TOLERANCE_IN_SECONDS = 5*60`) + kernel (`WEBHOOK_REPLAY_SECS` /
   `WEBHOOK_TS_FUTURE_SKEW_SECS`, both 300). The Rust constants cite
   `create_rate_limiter()`'s pre-existing hardcoded values — this round makes
   them named, not new.
7. **No recoverable-path panics**: the middleware path returns 429s, never
   unwraps; the JS side returns false / 401, never throws. `clippy -D
   warnings` with the crate's `unwrap_used/expect_used/panic = deny` is the
   gate — and note `arithmetic_side_effects = "warn"` is live for any
   timestamp math in Rust (there is none new; the JS math is `Math.floor`d).

---

## §4 — Verification, exact commands

### Red-first (before any fix)
```
node --test tools/valet-relay/          # expect: freshness cases FAIL pre-fix
cargo test --manifest-path tools/signal-gateway/Cargo.toml --test s8_04_rate_limit_wired
                                        # expect: wiring/structural cases FAIL pre-fix
```

### The full gate
```sh
cd /Users/mark/Sites/brain-server

# — JS edge —
node --test tools/valet-relay/; echo "VALET_EXIT=$?"        # want 0
node tools/valet-relay/relay.js --selftest 2>/dev/null      # needs config fixture; see notes

# — Rust edge —
cargo fmt --manifest-path tools/signal-gateway/Cargo.toml --all -- --check
cargo clippy --manifest-path tools/signal-gateway/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/signal-gateway/Cargo.toml

# — root suite + floors —
export RUSTFLAGS="-D warnings"
cargo fmt --all -- --check
cargo clippy --all-targets --features bench -- -D warnings
cargo test --features bench
cargo test --features bench --test main_suite r73_register          # register pin, extended
cargo test --features bench --lib spire_inventory -- --nocapture    # floors unchanged

# — house gates —
bash scripts/badges.sh --verify-count      # ~5.5 min; re-derives; NEVER hand-type
bash scripts/badges.sh --selfcheck
bash scripts/docs-truth.sh                  # 0 HIGH / 0 MED; LOW=17 pre-existing
bash scripts/env-truth.sh
python3 scripts/check-doc-links.py
bash scripts/lipstyk-gate.sh
cargo audit --file Cargo.lock
```

### Invariants that must hold afterwards
- `git diff --stat src/authz/ src/migration.rs openapi.yaml` → **empty**.
- `git diff --stat -- '**/Cargo.lock' Cargo.lock` → **empty** (the round adds
  zero dependency edges; if a local cargo run re-dirties a tool lockfile,
  `git checkout --` it before committing).
- `grep -n 'CRATE_TEST_FLOOR: usize' src/spire_inventory.rs` → still `2_758`.
- `grep -rnw 'R76' AUDIT.md | head -3` → the new dispositions only.
- `grep -n 'mod ratelimit;' tools/signal-gateway/src/main.rs` → **no match**.

### Rollback
Additive and independently revertable per commit. The JS fix and the Rust fix
share nothing; the register commit is last. Reverting the CI job never breaks
the suite. No migration, no wire change, no irreversible step.

---

## §5 — Named residuals (stated, not silently absorbed)

| Item | Why not in R76 |
|---|---|
| **Id-dedup on the relay sink** | **DECLINED by decision**: the producer retries with the same id (`alert.rs:508-535`); dedup would silently drop a legitimate retry's alert after a lost response. The spec's idempotency-key advice is noted; the kernel's own `webhook_seen` cap guards inbound ingestion, not the sink receiver. A within-window replay therefore still fires once more (bounded: 5 min, one alert per captured envelope per window). |
| **Rate-limit constants not operator-tunable** | 100/60 hardcoded-in-lib; a config surface is a knob requiring env-truth + docs + example-yaml churn, and no deployment evidence demands it. Named, not silent. |
| **`max_sends_per_second` misnomer** | The semaphore is a concurrency cap, not a rate limit (`worker.rs:216`); renaming a config key is a breaking config-surface change — recorded here. |
| **S8-03** (channel-bridge body cap / concurrency limit) | Adjacent finding, different file, its own round. |
| **Stale tool lockfiles** (`--locked` fails, CI regenerates silently) | Pre-existing R75 residual, deliberately untouched: zero-new-deps keeps this round clear of the decision entirely. |
| **S8-05, S8-07, F8-02/F8-03, K8-*, D8-*, L8-*** | Unchanged from R75's register state; none is this round's theme. |

---

## §6 — The law this round must not break

- **Red-first, both edges.** Prove each test fails against the unfixed code
  before proving it green.
- **No new dependency edges.** `tower` stays out; the lockfile decision stays
  closed; `git diff '**/Cargo.lock'` must be empty at every commit.
- **Do not raise `CRATE_TEST_FLOOR`.** New tests live in `tools/`, which the
  floor's needle does not walk — the floor should not move at all.
- **The register closes only what the code proves.** Both CLOSED rows must
  carry their evidence (file:line) and their declined alternatives.
- **Honest scope.** The relay's exposure stays loopback-scoped (MEDIUM,
  defense-in-depth); the limiter's absences (burst of 100, shared SSE budget)
  are stated in the register row, not smoothed away.
