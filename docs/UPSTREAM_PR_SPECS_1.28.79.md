# UPSTREAM PR SPECS — v1.28.79 (for `openclaw/openclaw`, no fork code)

Each spec is file:line-anchored against `upstream/main`, with a minimal diff
sketch, a red-first test name, and a conflict-risk note. Filing is operator
action; the fork carries disclosures (THREAT_MODEL) until merged.

## U1 — Multi-block MCP envelope skips neutralization (HIGH)

- **Upstream file:** `src/agents/mcp-content.ts:161`
  (`wrapMcpToolResultContent` multi-block branch splices raw `...content`
  between prefix/suffix; only the single-text path calls
  `wrapExternalContent` → `sanitizeExternalContentText`).
- **Sketch:** map text blocks through `sanitizeExternalContentText` before
  splicing (mirror the single-text path; ~5 lines).
- **Red-first test:** `multi_block_mcp_result_neutralizes_markers`
  (2-block result with `<<<EXTERNAL_UNTRUSTED_CONTENT>>>` + role tags →
  neutralized inside the envelope).
- **Conflict risk:** low — one function, fork's hard-block work is adjacent
  (pins/materialize are fork-only files) but does not overlap these lines.

## U2 — `systemPrompt` bypasses the Meridian seam (HIGH)

- **Upstream file:** `src/plugins/hooks.ts:511` (`systemPrompt:
  firstDefined(...)` raw beside sanitized `prepend/appendContext`).
- **Sketch:** run the merged `systemPrompt` through `sanitizePluginContext`
  at the same seam (~2 lines).
- **Red-first test:** `system_prompt_hook_input_is_sanitized`.
- **Conflict risk:** low — single expression; prompt-composition churn is
  nearby but this line is stable vocabulary.

## U3 — Pin hard-block is opt-in (HIGH)

- **Upstream file:** `src/agents/agent-bundle-mcp-materialize.ts:489`
  (`catalogPinsPath?` optional; reconcile early-returns empty without it —
  only `attempt-bundle-tools.ts:149` passes it).
- **Sketch:** default to `${agentDir}/mcp-catalog-pins.json` when the param
  is omitted and `agentDir` is known; document the first-use-flagged
  posture for the no-agentDir case.
- **Red-first test:** `materialize_defaults_pins_path_from_agent_dir`.
- **Conflict risk:** medium — the file churns upstream; file against a
  fresh `upstream/main`.

## U4 — Replay-prefix spoof (MEDIUM)

- **Upstream file:** `src/auto-reply/reply/strip-inbound-meta.ts:19,158-163`
  (`$1` passthrough; exact `includes("[memory | ")` gate bypassable).
- **Sketch:** sanitize `$1` (strip newlines/markers) + case/space-tolerant
  prefix match.
- **Red-first test:** `replay_prefix_cannot_smuggle_instructions`.
- **Conflict risk:** low — small pure helpers.
