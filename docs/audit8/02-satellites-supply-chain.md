# §3 — SATELLITES, EDGE TOOLS & SUPPLY CHAIN

Leg findings re-verified where decisive. All `cargo audit` runs were **executed**.

---

## S8-01 — MEDIUM-HIGH — `signal-gateway`: a remote bind can be fully unauthenticated

`tools/signal-gateway/src/main.rs:104-119` — two independent `if`s:

- `:104-111` refuses a non-loopback bind unless `SIGNAL_GATEWAY_ALLOW_REMOTE=1`.
- `:113-119` then chooses auth from `config.server.auth_token` **alone**.

`config/mod.rs:45` declares `auth_token: Option<String>` with the doc *"When set, every API
request must carry Authorization: Bearer"* — and `api/mod.rs:54-77`'s `None => router` arm
serves with no auth at all.

**Exploit:** `SIGNAL_GATEWAY_ALLOW_REMOTE=1` with `auth_token` unset ⇒ **`/v2/send`** (send
Signal messages), **`/v1/accounts`** (account/phone enumeration) and **`/api/v1/events`** (SSE
message stream) are served unauthenticated on a public interface. The log line at `:117` reads
`API auth: NONE (loopback-only posture)` — but nothing in that branch *enforces* loopback-only;
the guard already passed. A silent fail-**open** on the one tool controlling a live messaging
identity.

**Fix (minimal, matches the "less is more" mantra):** in the `None` arm, re-assert
`addr.ip().is_loopback()` and `bail!` otherwise — make auth a *function of the bind*, not a
separate flag. Two lines; deletes a state rather than adding a check.

---

## S8-02 — MEDIUM — `valet-relay` alert sink verifies the HMAC but never checks freshness

`tools/valet-relay/relay.js:71-77,139` — `verifyAlert` checks the `v1,` prefix, recomputes the
MAC over `${id}.${ts}.${body}`, and compares with `crypto.timingSafeEqual` (`:76` — correct,
constant-time). It **never validates that `ts` is recent.** Standard Webhooks mandates a
±5-minute tolerance; the kernel-side verifier enforces one, this relay's own `/alert` sink does not.

**Exploit:** capture one correctly-signed envelope → replay indefinitely → `sendSignal()`
(`:153`) re-fires an operator alert forever, until the secret rotates. `seenEnvelopes` (`:163`)
guards the **outbound** path only and is per-process.

**Severity scoping:** the listener binds `127.0.0.1` (`:157`), so remote capture is not
directly available; realistic sources are a co-tenant process, a local log, or a proxy.
Defense-in-depth gap ⇒ MEDIUM, not HIGH.

---

## S8-03 — MEDIUM — `channel-bridge` public webhook has no body cap, concurrency limit, or local replay window

`tools/channel-bridge/src/main.rs:240-245,287-289` — both routers are bare
`Router::new().route(…).with_state(…)`: no `DefaultBodyLimit` layer, no concurrency limit, no
rate limiter. Axum's implicit 2 MB `DefaultBodyLimit` still applies to the `Bytes` extractor,
so **I am not claiming unbounded memory.** The gap is that replay defence is *delegated to the
kernel* (comment at `:374`) on a route sitting behind a public TLS-terminating proxy: a captured
payload is replayable at will, each replay costing a kernel call. Deduped on `external_id`, so
the blast radius is amplification, not duplication.

**Fix:** `DefaultBodyLimit::max(256 * 1024)` + `ConcurrencyLimit` on both routers.

---

## S8-04 — MEDIUM — `signal-gateway`'s rate limiter is a dead module

`tools/signal-gateway/src/ratelimit.rs` — grep across `src/` finds it referenced only by
`main.rs:27` (`mod ratelimit;`, which merely makes it compile) and by `worker.rs:216`, an
unrelated `Arc<Semaphore>` send-concurrency cap. **`RateLimiter::is_allowed` is never called
from any handler or the router.** Its own tests (`:92-105`) pass in isolation — the vacuous-green
class. So `/v2/send`, an outbound-message primitive, has no request-rate control. Amplifies S8-01.

---

## S8-05 — MEDIUM — Plugin config fails **open** on boolean coercion

`plugin/src/config.ts:235` — `resolveConfig(raw: unknown)` does
`const cfg = (raw ?? {}) as Partial<BrainConfig>`: a bare type assertion, zero runtime
validation. The real Typebox schema `brainConfigSchema` (`:16`) is used **only as a type source**
(`Static<typeof …>` at `:78`); grep finds no call site that validates anything.

Every boolean gate then resolves with `??`, which does not coerce: `autoCapture: "false"`
(string) → non-nullish → the resolved config holds the **truthy string** ⇒ **auto-capture turns
ON when the operator wrote `"false"`.** Same for `enabled`, `teamBridge`, `proposalTools`,
`strictDomain`. The server-side injection screen becomes the only barrier instead of
proposal-review — the plugin's own documented LLM01/LLM06 posture.

**Honest mitigation:** `plugin/openclaw.plugin.json:29` declares a correctly-typed JSON schema
with `additionalProperties:false`. **If the host enforces it, this is unreachable.** The plugin
itself does not. (Flagged as an outstanding cross-tree check, not claimed as a live exploit.)

**Plugin all-clear, evidenced:** token ladder fails closed — unreadable `BRAIN_TOKEN_FILE`
throws, multi-line values throw rather than transmitting the operator token down the agent path
(`config.ts:145-189`); `brain-client.ts:878-880` re-pins origin every request, `:907-914`
refuses redirects and re-pins the response origin; **bearer never in a URL** (header-only).
All 11 `registerTool` sites pair `tool.name` with the `{name}` options exactly — no tool-name
shadowing. `format.ts:102` runs `stripSentinels` on the **composed** text at the assembly
point, not per field, closing the title|body marker-split.

---

## S8-06 — MEDIUM — Client export seam: a live escape mismatch, and a latent injection sink

`client/src/download.rs:37` — two defects in one `format!`:

1. `a.download='{safe}'` — `safe` (from `safe_filename`, `:9`) is spliced into a
   **single-quoted JS string with no escaping.** `safe_filename` refuses control chars, `/`,
   `\`, `..` — it does **not** refuse `'`. A filename containing `'` closes the literal and
   injects arbitrary JS into `document::eval`.
2. `new Blob([{body:?}], …)` — `{body:?}` is Rust's `Debug`, which emits `\u{...}`.
   `\u{2028}` is **not a valid JavaScript escape sequence**: a SyntaxError in strict mode,
   otherwise an identity escape.

**Severity scoping, honestly:** defect 1 is **latent, not currently exploitable** — all three
callers pass safe values (`audit.rs:203`, `data.rs:145-149` are string literals; `recall.rs:351`
is `format!("trace-{trace_id}.json")` where `trace_id: i64`, digits only). I am **not claiming
a working XSS.** Defect 2 **is** live and reachable and corrupts exports today.

**Fix (one line, and the codebase already demonstrates it):** `serde_json::to_string(&body)` —
exactly what `client/src/panels/mod.rs:66` correctly does.

---

## S8-07 — MEDIUM — Six of thirteen `crates/` members are unconsumed islands

`cargo tree --manifest-path crates/Cargo.toml --invert <crate>` returns a **bare node** for
`brain-care-core`, `brain-aftersales-core`, `gold-sets`, `brain-fuzz`; and two chains are dead
at the root (`brain-interview-core → brain-care-core`, `brain-troubleshoot-core →
brain-aftersales-core`). Root `Cargo.toml` has path edges to only 7 members (`:91,100,103,110,119,120,128`).

**Impact:** dead security-relevant code that reviews, lints, and passes `cargo audit` but can
never execute — inflating the apparent coverage of the compliance story. `gold-sets` is reachable
only via `include_str!`, a build-time text read, so even `cargo tree` cannot see the edge.

**Fixture integrity IS enforced — a genuine all-clear:** `tests/agreement_path_pins.rs:35-47`
lists all 11 gold fixtures by explicit path (deliberately not a directory walk — *"a walk would
silently absorb a new file"*) and hashes each against digests recorded at the 2026-09-30
preregistration.

---

## S8-08 — LOW — A yanked crate in the Tauri shell lockfile

`cargo audit` over `shell/src-tauri/Cargo.lock` reports `yoke-derive 0.8.3` as **yanked**
(RUSTSEC unmaintained `proc-macro-error1.0.4` also warned). A yanked version is one the
upstream author withdrew. **`cargo audit` exits 0 because yanked ≠ advisory** — so CI's audit
gate cannot catch this class at all.

---

## S8-09 — LOW — The release gate is documentary; the script prints its own bypass

`scripts/release.sh` is well-built: refuses a pre-existing tag (`:33`), refuses unless
`HEAD_SHA == origin/main` (`:41`), fails closed if no CI run registers (`:64`), refuses after
60 min (`:80`), refuses any conclusion ≠ `success` (`:84`). **But line 54**, inside the
`gh`-not-found error, prints: *"Install + auth gh, verify the Actions tab yourself, **or push a
tag manually at your own judgement**."* And `.github/workflows/release.yml:4-5` triggers on
`push: tags: ['v*']` with **no CI re-run and no verification**.

**Honest severity:** this requires push access, which is already fully trusted — it is not a
privilege-escalation finding. It matters because the control is **documentary, not enforced**,
and the repo points at the bypass. **Fix:** have the release job re-assert green CI on
`github.sha`, fail-closed if not found.

**Remote hygiene all-clear (verified):** `origin → markfietje/brain-server-private.git`;
`public → markfietje/brain-server.git` with push URL literally **`DISABLED`**. The public mirror
physically cannot be published to by accident.

---

## Supply chain — verified all-clears

| Check | Result |
|---|---|
| `cargo audit`, root `Cargo.lock` | **514 deps, exit 0**, no vulnerabilities |
| `cargo audit`, `crates/Cargo.lock` | 68 deps, exit 0 |
| `cargo audit`, `tools/steward-harness/Cargo.lock` | 162 deps, exit 0 |
| `cargo audit`, `shell/src-tauri/Cargo.lock` | 417 deps, exit 0 + 2 warnings (see S8-08) |
| GitHub Actions pinning | **All 79 `uses:` across 6 workflows resolve to 40-hex commit SHAs.** No `pull_request_target`, no `issue_comment`, no `workflow_run`. |
| Workflow permissions | Least-privilege declared everywhere; `contents: write` scoped to the single release job, `pages/id-token` to deploy only. |
| Secrets exposure | **No `secrets.*` reference anywhere** in workflows — nothing to leak. |
| Committed secrets | `git grep` for `sk-`/`ghp_`/`AKIA`/`xox*-`/`BEGIN * PRIVATE KEY`/`eyJ…` over tracked non-doc files: **one** hit — `src/audit/mod.rs:1491`, a synthetic fixture whose value literally reads `ghp_verysecrettokenvalue1234567890ABCDEF`. Not a credential. |
| SBOM currency | `sbom/brain-server-1.29.2.cdx.json` **matches** `Cargo.toml` 1.29.2. `bash scripts/badges.sh --selfcheck` → `OK`, exit 0. |
| SBOM scope | 365 of 514 packages — **runtime closure only**, honestly disclosed (dev/build tree absent). `crates/`, `client/`, `fuzz/`, `tools/*` have **no SBOM** (S8-10). |
| CI audit coverage | `ci.yml:630` loops `find . -name Cargo.lock` — matches all 8. |
| Env truth | `bash scripts/env-truth.sh` → `OK env truth clean`, exit 0. |
| Installer | `chmod 700` token dir, `chmod 600` token/audit-key/agent-token; strips **both** `com.apple.provenance` **and** `com.apple.quarantine`. Correct. |

## S8-10 — LOW — SBOM coverage gap

Only the root server has an SBOM. `crates/`, `client/`, `fuzz/`, `tools/*` are inventoried only
by the CI audit loop. Documented and intentional, recorded so a future reader does not infer
full-tree coverage from a populated `sbom/` directory.

## S8-11 — LOW — `AGENTS.md`'s "all three `Cargo.lock` files byte-identical" is false — there are 8

The same "figure shipped without anyone diffing it against a measurement" pattern AGENTS.md
itself calls out in its own header. Re-baseline to 8, or scope the claim to the audited workspaces.

## S8-12 — INFO — Edge-tool HMAC verification is genuinely correct

`hubsig.rs:48-60`: verify-first, raw-body HMAC-SHA256, `"sha256="` prefix required,
**length-checked to exactly 64 hex chars before any MAC compare** (`:52`), folded constant-time
compare with no early-out. `main.rs:341-353`: the `Bytes` extractor is the raw buffer and
`verify_hub_signature` runs **before** `translate::project`. `teams.rs:399-422`: Bot Framework
JWT verified at `:414` and **returns before** `serde_json::from_slice` at `:419`. Config bounds
real (`MAX_SECRET_BYTES:256`, `MAX_CHANNELS:16`, `MAX_ENVELOPE_TEXT_CHARS:4000`). This is the
best-executed edge code in the tree.

---