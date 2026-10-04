# §6 — FOUR-TREE PARITY MATRIX (server ↔ plugin ↔ client ↔ fork host)

**Rebuilt from scratch.** The brief says Parity claimed the gap ledger was "zero"; **that claim
was already retracted upstream** (v1.28.87 → "gap ledger balanced (4 known residuals with owners)").
`CHANGELOG.md:1523-1536` documents the overstatement and a gate now enforces the corrected
wording: `grep -rn "gap ledger zer[o]" CHANGELOG.md docs/` must return zero hits.

**Rule applied:** *parity enforced only by humans re-reading comments is a FINDING, not parity.*

| # | Surface | Server Rust | Plugin TS | Client | Fork host | Pin | Verdict |
|---|---|---|---|---|---|---|---|
| 1 | **Invisible-Unicode set** | `src/strip_invisible.rs:16` `is_invisible` | `plugin/src/format.ts:192` `INVISIBLE_CLASSES` | `client/src/main.rs:325-338` | `src/infra/unicode-visibility.ts:16-17` | `plugin/fixtures/invisible-classes.json` | **ALIGNED — 3 of 4 pinned** ⚠️ |
| 2 | **Hostile-element mirror** | `src/gate.rs:430` `HOSTILE_ELEMENTS` (26) | `format.ts:209` `HOSTILE_ELEMENTS` + 30-name MathML appendix | n/a (Dioxus text nodes) | n/a | `plugin/fixtures/hostile-elements.json` v1 | **ALIGNED — pinned both ways** |
| 3 | **Read-seam fixed point** | `src/gate.rs` bounded fixpoint | `format.ts` `STRIP_FIXPOINT_PASSES` | n/a | `external-content.ts` | `hostile_elements_fixture_pins_server_set` | **ALIGNED** |
| 4 | **Truncation head+tail** | n/a (server-side caps) | n/a | n/a | `tool-result-truncation.ts` (single entry) | `tool-result-truncation.test.ts` +133 ln | **FORK-ONLY — single-implementation, no cross-tree fixture** ⚠️ |
| 5 | **Fences + origin labels** | n/a | `format.ts` `UNTRUSTED_BEGIN/END`, `untrustedOrigins` label\|exclude | n/a | `context-hygiene.ts:38-65` ZWSP-splits 5 sentinels | `hooks.prompt-build-hygiene.test.ts` | **ALIGNED** (see drill §7) |
| 6 | **Digest-bound approvals** | `handlers/gate.rs:675-686` | bridge-side refuse-and-log | console renders digest | `infra/plugin-approvals.ts:255,337` | drill-verified live (§7) | **ALIGNED — verified end to end** |
| 7 | **`untrusted: true` parity** | 14 sites in `handlers/recall.rs` | `format.ts:74` only on `=== true` | n/a | external-content wrap | 3-surface parity pin | **ALIGNED** |
| 8 | **MCP scope ↔ catalog pins** | `src/bin/mcp.rs:742` `x-brain-scope` | plugin tool usage | n/a | `agent-bundle-mcp-catalog-pins.ts` | `…pins.test.ts` | **ALIGNED in intent, BYPASSABLE in practice** (K8-05/06) |
| 9 | **Token handling** | `AUTH_TOKEN_FILE` ladder fail-closed | `config.ts:145-189` multi-line throws | `shell/src-tauri` 1 IPC cmd | gateway auth | `config.test.ts:120` | **ALIGNED — no URL tokens anywhere** ✅ |
| 10 | **SSE close codes / 403-before-stream** | `sse_reauth.rs` | n/a | console | gateway | re-auth pins | **ALIGNED** |
| 11 | **Blocklist families / typoglycemia / bounded decoding** | `screen.rs` | — | — | — | `encoding_scan_bounded`, `blocklist_families_cover_five_languages` | **SERVER-ONLY — no cross-tree duplication found** ✅ |
| 12 | **Invisible set at the CLIENT** | — | — | `main.rs:325-338` mirrors server | — | **none** | **GAP P8-01** |

---

## P8-01 — MEDIUM — The invisible-Unicode set is pinned for only **two** of four trees

The brief flagged this specifically and **it is correct.** The fixture
`plugin/fixtures/invisible-classes.json` is consumed by `plugin/src/format.test.ts`. Grepping
for its consumers returns **plugin only** — there is no equivalent fixture lane in `client/`
or in the fork.

I diffed all four implementations by hand:

- **Server** `src/strip_invisible.rs:25-46` — tag block, variation selectors, ZWSP/ZWNJ/ZWJ/
  word-joiner, `0x2061-2063`, `FEFF`, `00AD`, `034F`, Mongolian `180E`, Hangul fillers
  `115F`/`1160`, interlinear `FFF9-FFFB`, plus bidi `202A-202E` and isolates `2066-2069`.
- **Client** `client/src/main.rs:325-338` — an **exact character-for-character match** with the
  server set, including the "Mirror of the server's residual-class additions" comment.
- **Plugin** `format.ts:192` — server set **plus** a deliberate superset (`\u2060-\u2063` widened
  to include `2064`, `2066-2069`).
- **Fork** `unicode-visibility.ts:16-17` — the largest superset (`\u2060-\u206F`), documented in
  a comment as *"SUPERSET of the canonical fixture … Extra stripping is invisible-chars only, so
  user-visible drift is nil by construction."*

**Verdict: the implementations are currently CORRECT.** All drift is *additive and in the safe
direction* (more stripping), and the fork's superset is explicitly documented and intentional.

**But the correctness is a coincidence of four hand-written copies, not an enforced property.**
The client copy is a hand-transcription of the Rust set that happens to match today; nothing
fails if someone adds a class to the server and forgets the client. Given the system's first
mantra runs through this set, **this is the one place where "parity enforced by humans
re-reading" is currently doing the work.**

**Cross-tree fixture proposal (the `format.test.ts` pattern, extended):**

1. Emit the canonical set as **data** from the Rust side — `plugin/fixtures/invisible-classes.json`
   already exists; add `client/fixtures/invisible-classes.json` generated by the same test.
2. One fixture, three consumers: plugin vitest, client Rust test, and (via `include_str!`) a
   fork-side assertion that its superset ⊇ the canonical set.
3. A **red-proof**: add a class to the server, watch all three lanes fail, then propagate.
4. Assert **subset-or-equal in the stripping direction** (`canonical ⊆ implementation`), never
   exact equality — the fork legitimately carries extras, and the plugin comment already says so.
   Exact equality would punish the safer direction and teach contributors to *narrow* the sets.

**Why this is MEDIUM and not HIGH:** no current drift, and every divergence is fail-safe.
It is filed as a finding because the property is unowned, not because it is currently broken.

---

## P8-02 — LOW — Truncation head+tail arithmetic exists in exactly one tree

`tool-result-truncation.ts` is the sole implementation. No server, plugin, or client counterpart
exists, so there is nothing to drift *from* — but there is also **no shared boundary-budget
fixture**. The arithmetic's hardest cases (tight budgets shrinking the tail, aggregate elision
marker count-first, "a notice larger than the result it replaces is skipped") are pinned only in
the fork's own test file.

**Proposal:** lift the boundary-budget vectors into `plugin/fixtures/truncation-budgets.json`
so the server's own caps and the fork's truncation are checked against one table.

---