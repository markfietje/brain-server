# THIRD-PASS AUDIT — 2026-09-10 (fork-vs-upstream diff, all layers)

**Scope:** the 92-file `upstream/main...fork` delta (`openclaw/openclaw` vs
`markfietje/openclaw-fork`) plus brain-server seams the wiring touches.
Three lanes (auth/secrets, content-trust, egress/persistence) + direct
verification of every load-bearing claim against live source. Research-only;
no edits in the pass itself. Disposition per finding: **fixed** (fork-only
file, F-lane), **spec'd** (upstream file, U-lane), **accepted** (residual).

## 1. Transport layer

- **M1 redirect bearer leak (fixed, F2).** `fetchJson` pinned pre-request;
  `fetch` follows cross-origin redirects resending `Authorization`.
  Fix: refuse when `new URL(res.url).origin !== pinned`.
- **Origin pin (prior release, verified).** Pre-request pin + scheme gate
  hold; `res.url` re-pin closes the redirect half.
- **DNS-rebind of pinned host (accepted).** Needs resolved-IP plumbing;
  Loop line. Loopback-first deployments only until then.

## 2. AuthN/Z + secrets

- **H2 token-file multiline (fixed, F1).** Whole-file-as-Bearer vs
  second-line-agent-token docs. Refuse multiline files naming the agent line.
- **M3 contradiction gate dead by default (fixed, F4).** Untrusted-source
  verdict replaces silent ok.
- **M4 null-Origin pre-pass (fixed, F4).** Missing `Origin` faces the
  allowlist; loopback/CLI shape unaffected.
- **M5 comma-reject availability pressure (fixed, F4).** Multi-entry
  `X-Forwarded-For` parses; strict stays on.
- **Refresh-family burn (accepted).** OWASP reuse pattern, documented.
- **`safeEqualSecret("","")==true` (upstream, reported).** Low today
  (callers guard); fix belongs upstream.

## 3. Content-trust / injection

- **H1 multi-block envelope (spec'd, U1).** `mcp-content.ts:161` splices raw
  blocks; single-text path sanitizes. 5-line sketch in spec doc.
- **H3 `systemPrompt` seam bypass (spec'd, U2).**
  `hooks.ts:511` rides `firstDefined` raw beside sanitized siblings.
- **F5 brain-fence pass-through (fixed).** `context-hygiene.ts` (fork-only)
  neutralizes brain sentinels like existing markers.
- **M7 replay-prefix (spec'd, U4).** `$1` passthrough + exact-gate bypass in
  `strip-inbound-meta.ts`.
- **`INJECTION_POLICY=allow` (accepted).** Loud + health-echoed by design.
- **New-tool pre-ack execution (accepted ceiling + spec'd U3).** First-use
  not gated by design; default-`catalogPinsPath` sketched for upstream.

## 4. Persistence / integrity

- **DSAR completeness (closed .78).** Certificate excepts backups,
  audit-chain rows, logs. Chain deletion would break verification —
  documented tradeoff, not a gap.
- **Procedure-write posture (fixed-doc, F6).** `procedural.ts` posture note.
- **Pins file trust (accepted).** Writable ack state; perms + loud-rebuild
  mitigate; signed ack is Loop line.

## 5. Tenancy

- **M6 team-bridge chat gates (fixed, F3).** `allowedChatTypes` conjointed.
- **Shim tenancy (accepted).** Row-level `WHERE`; true isolation needs
  `BRAIN_MULTI_DB=true`. One `format!` predicate away — reviewer guidance
  in spec doc.

## 6. Config

- **Token ladder fail-closed (closed .78).** Plugins + server posture verified.
- **Gateway strict flags opt-in (accepted).** Correct for loopback; exposed
  PoCs must enable — deployment-doc note.

## 7. Supply chain

- `pnpm-lock` delta: typebox + typescript only. No new fetch/http libs.
- No snapshot/backup/restore code in the delta. SBOM per release intact.

## 8. Wiring (fork ↔ brain-server)

- Parity sync direction healthy; byte-identity holds except oxfmt fallout.
- Token flow, baseUrl parity, recall/ingest/proposal shapes, `untrusted`
  chain, timeout asymmetry all coherent (2nd-pass record stands).
- `fetchJson` error mapping closed .78; redirect re-pin closes the half.

## Disposition count

Fixed 6 (F1–F6) · Spec'd 4 (U1–U4) · Accepted 8 · Prior-release 6.
No critical (unexploitable-alone RCE/priv-esc) found; two highs needed
only a malicious MCP server or operator misconfig to trigger.
