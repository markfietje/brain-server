# Changelog — @markfietje/brain-server-openclaw

All notable changes to the plugin. Semantic-versioned (patch = behavioral
fix/security, minor = feature, major = breaking). Mirror of the OpenClaw
extension at `extensions/brain-server`.

## [0.6.0] — 2026-09-08

**Origin** — taint labels survive the whole trip; shipped with brain-server
v1.28.74.

- Hit lines inside the UNTRUSTED fence prefix ` [memory | <origin>]` for any
  non-`owner` origin (`[memory | channel-capture]` for group/channel
  captures; owner memories stay untagged). The label passes through
  `sanitizeForBlock` — it is displayed, never executable.
- New config `untrustedOrigins: "label" | "exclude"` (default `label`):
  `exclude` drops channel-captured hits from AUTO-INJECT entirely. The
  `memory_recall` TOOL path always labels, whatever the setting — a tool
  consumer always sees the taint.
- `autoCapture` sends `origin_context: "channel"` when the gating chat-type
  is a group/channel chat, so the server stores the capture with origin
  `channel-capture` (absent = `owner`; unknown values 400 at the server).
- No new deps; wire additive only. Does NOT add a taint lattice, policy
  engine, or server-side exclusion — ONE boolean-grade label, and promotion
  stays human-gated.

## [0.5.1] — 2026-09-07

**Meridian M2 (X-R5)** — strip-set sync; shipped with brain-server v1.28.65.

- `sanitizeForBlock`'s invisible-Unicode class is now the Rust canonical set
  (`src/strip_invisible.rs` `is_invisible`), exported as `INVISIBLE_CLASSES`
  so the next sync is a one-line diff. Adds the members the old set lacked:
  U+061C (ALM), U+E0100–E01EF (supplemental variation selectors), U+FE00–FE0F
  (BMP variation selectors), U+180E, U+115F/U+1160, U+FFF9–FFFB, and the
  U+2060–2063 / U+00AD / U+034F legacy members.
- Parity fixture `plugin_invisible_set_matches_rust_canonical` in
  `format.test.ts`: one probe char per Rust-set class + the survivor vectors —
  the drift pin (either side changing without the other fails CI).
- Zero deps; prompt-bytes change is invisible-class only. Server strip is the
  primary path; this is belt-and-braces when the plugin runs first.

## [0.5.0] — 2026-08-26

**Team Bridge** — mirror OpenClaw agent activity onto brain-server's governed
workflow dashboards (shipped with brain-server v1.28.34+). Off by default.

- New `src/team-bridge.ts`: on `before_agent_run`, ensure a signed mesh card
  (once per agent, 409-tolerant), open a governed run for the session
  (`POST /workflow/runs`), and append exactly-once lineage events
  (`workflow/openclaw/start|beat|done|failed|paused`). On `agent_end`, close
  via CAS (`status done|failed`); on `session_end`, pause still-open runs.
- Heartbeat rides the existing `before_prompt_build` handler (one handler per
  hook), throttled by `teamHeartbeatMs` (default 60s, seeded at run-open).
- Config: `teamBridge` (off), `teamDomain` (defaults to `defaultDomain`),
  `teamHeartbeatMs`. Gated by the existing per-agent allowlist.
- Privacy: only a 200-char intent label enters run state; prompts/messages
  never leave the host process. Observation-only and fail-open by contract;
  17 new tests pin the wire shapes against the Rust handlers'
  `deny_unknown_fields` contracts.

## [0.4.7] — 2026-08-23

Hardening patch; shipped with brain-server `v1.28.14 "Parity"`.

- **Drift reconciled:** the deployed extension was at 0.4.6 while the mirror
  lagged at 0.4.5 (`tools.ts` import order + `format.test.ts` formatting);
  the mirror is byte-identical again before any fix landed.
- **F-I4 — sanitize every interpolation inside the fence:** `hit.domain`,
  and `edgePath`/`domain`/`kind`/`screenVerdict` in the traverse + proposal
  renderers run through the existing `sanitizeForBlock` (tag-block chars,
  zero-width, bidi cannot reach the prompt or forge the fence).
- **F-E3 — bound the error seam:** server error bodies reaching the agent are
  routed through the same strip (invisible + markdown-ref), length cap kept;
  hook-side `String(err)` log lines stripped too.
- **F-E4 — scheme gate:** `baseUrl` must be `https:` or a loopback `http:`
  host; anything else throws at registration with a clear message (the bearer
  plus user-turn text never transits remote cleartext).
- **Boundary provenance:** the per-hit label carries `origin:<origin>` when
  the server ships it (absent field omits the segment — back-compat).
- **Docs:** the two-token pattern (operator token vs agent token) documented
  in README + the config ladder.
- `scripts/sync-plugin.sh` (in brain-server) makes the mirror discipline
  permanent.

## [0.4.5] — 2026-08-18

Security + privacy + fence hardening; shipped with brain-server `v1.27.21`.

- **Token resolution (M7 / S2-54):** the plugin never writes a token. The
  bearer resolves via a ladder mirroring the `brain` CLI —
  `BRAIN_TOKEN_FILE` (a 0600 file; the token never appears in config or env
  dumps) → `BRAIN_TOKEN` (env) → `authToken` in the plugin config (legacy
  fallback). Config wins only when no env source is set; an unreadable token
  file degrades loudly (console.warn) to the next rung, never silently to a
  weaker source. Closes the documented openclaw-config token leak on the
  plugin side.
- **Privacy (query log):** the per-turn abstention log now carries the query
  **length only**, never the query text — openclaw's log is persistent and a
  recall query is user text. The injection path logs the same length-only blob.
- **Fence (S2-01):** the `UNTRUSTED_*` sentinel strip runs on the **composed**
  inner text (banner + lines) at assembly time, not per-field — a marker split
  across the title|body boundary (title ending `…UNTRUSTED_CONTEXT`, body
  starting `END ===`) can no longer forge the fence close. Per-field
  `sanitizeForBlock` remains as defense-in-depth.
- **Lint:** the attribution-line `if` guards now use braces (openclaw oxlint
  `curly` rule). Test harness clears env vars with `delete` (portable across
  Node versions that stringify `= undefined`).

Tests: **144** extension tests (openclaw vitest) + oxlint + `tsc --noEmit`
clean.

## [0.4.4] — 2026-08-16

Shipped with brain-server `v1.27.14` ("Fencepost2").

- **Sentinel order (F-01):** `sanitizeForBlock` moves the sentinel strip to the
  END of the pipeline so a near-marker a transform later synthesizes (NBSP/TAB/
  zero-width split across the `CONTEXT|END` boundary) cannot forge the fence
  close after it was stripped.
- **Invisible strip order:** the `U+E0000–U+E007F`-inclusive invisible strip now
  runs BEFORE the `\s` collapse so `U+FEFF` (which JS `\s` treats as
  whitespace) is removed, not widened to a space.
- **Snippet boundary:** the recall `snippet` is routed through the same block
  boundary (was the one raw field).

## [0.4.3] — 2026-08-15

First release; shipped with brain-server `v1.27.13` ("Contract").

- **Provenance labels:** the deterministic `[src: · mk: · lb: · reg:]`
  attribution line (source / memory kind / lawful basis / region) now runs
  through `sanitizeForBlock` like hit bodies, so recalled content can neither
  forge its attribution line nor the `UNTRUSTED_*` fence markers.