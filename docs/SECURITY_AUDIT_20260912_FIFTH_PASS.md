# Fifth-Pass Full-Spectrum Audit — 2026-09-12 (evening)

**Scope:** brain-server v1.28.82 "Vigil" + post-release closure commits
(HEAD `cb189f3`, clean tree) × openclaw fork (`~/Sites/openclaw`,
HEAD `bbd1d6aca13`, plugin 0.6.7).
**Mode:** dual — Mode A forward sweep + Mode B claims falsification.
**Relation to the fourth pass:** the morning fourth-pass report
(`docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md`) found 6 findings and
closed all 6 same-day. This pass RE-VERIFIES those closures, then hunts
what the fourth pass missed. Result: **all six closures HELD in code
and live — but the F4-S-01 closure introduced a HIGH availability
regression (A5-01) that turned the release gate red**, plus 10 further
server findings, 3 reverse-audit findings, 4 satellite findings, and
7 regulatory gaps.
**Method laws honored:** evidence-or-it-didn't-happen (every finding
carries file:line or an executed command's output); anti-vacuity
(new pins checked for revert-failure); copies-only (fresh DB
`/tmp/audit5/brain.db`, test port 9879; the live launchd server on 8765
and `~/.openclaw/workspace/brain.db` never touched — the drill server
died on EADDRINUSE once, proving the live port was never bound by us).

Execution: orchestrator ground truth + live drill + synthesis; five
parallel subagent lanes (server / satellites / claims / fork /
regulatory) all COMPLETED this pass (the fourth pass's failed spawns
did not recur).

---

## §0 — Executive summary (top 10 by risk × likelihood)

| # | ID | Sev | One line |
|---|---|---|---|
| 1 | A5-01 | **HIGH** | F4-S-01's `unknown_principal` refusal broke the kill-switch for every JWT identity with no DB row — 7/22 `authz_matrix` tests RED, main is unreleasable |
| 2 | T5-01 | MED | Fourth-pass closure gates never ran the `authz_matrix` binary — the "all green" record missed the red it created |
| 3 | A5-02 | MED | `DELETE /memory/{id}` writes vec0+knowledge+tombstone with NO in-tx audit row — audit-per-write violation on the certified-erasure path |
| 4 | A5-03 | MED-LOW | Forget cascade narrower than purge: `suggest_feedback`-by-chunk + trace/evidence soft refs survive a single-chunk forget |
| 5 | L5-07 | LOW-MED | Single-forget default retains proposal copies (disclosed, not scrubbed) — no Art 17(3) basis recorded at this seam; needs a directive |
| 6 | L5-01/L5-02 | LOW-MED | US map misses two 2026 laws: CO HB26-1263 (chatbot safety, eff Jan-2027) + IL SB315 (frontier audits, eff Jan-2027/2028) |
| 7 | S5-02 | LOW | Second fork merge seam (`mergeAgentTurnPrepare`) concatenates turn context raw — X-S1 bypass unless downstream re-flow is proven |
| 8 | R5-01 | LOW | CSP middleware still `starts_with("/app")` — future `/app-*` routes get the loose client CSP (defense-in-depth) |
| 9 | R5-02 | LOW/INFO | `no_sql_in_handlers` matches `"select "`-with-space only — tab/newline/paren/concat variants score 0; "ANY statement" overclaims |
| 10 | A5-08 | LOW | `BRAIN_INJECTION_THRESHOLD_HIGH/LOW` fail OPEN on typo (`.parse().ok()` + default) while every sibling env refuses boot |

No new CRITICAL. No bypass of the fixed-point read-seam strips, the
quarantine-everywhere posture, digest-bound approvals, provenance marks,
the egress IANA table, or the Twokeys/agent boundary was found this
pass (Mode B verdict table §5).

---

## §1 — Ground truth (verified)

| Claim | Evidence |
|---|---|
| HEAD `cb189f3`, clean tree | `git log -1` + `git status --short` (empty before this report) |
| v1.28.82 + 6 closure commits on top | CHANGELOG §§[1.28.77]–[1.28.82] read (Erasure/Unconditional/Parity/Lockdown/AgBOM/Vigil); closures `aacee4d` (revoke), `b36a603` (forget), `518c9fd` (get-source pin), `947c531` (Art 50 clock), `5e7d503` (four-tree fixture), `d63ddcb` (plugin 0.6.6/0.6.7) all present in code (grep-verified §2) |
| main.rs thin (124 lines, 203 routes, 86 env, 40 CLI) | `scripts/repo-brief.sh` |
| Fourth-pass closures present | `principal_known` (workflow/mesh.rs:148), `unknown_principal` 400 (handlers/mesh.rs:401-410), `retained_proposal_copies` + `scrub_proposals` (handlers/forget.rs:16-95, service/forget.rs:123-172), `AI_ACT_APPLICATION` (reg_watch.rs:79+199), `invisible_set_fixture_is_exhaustive_truth` (strip_invisible.rs:141), `get_sanitizes_source_label_behaviorally` (main_suite.rs:9128) |
| Fork topology today | HEAD `bbd1d6aca13`; **111 behind / 78 ahead** of upstream tip `045a9de3d2b` (fresh fetch; velocity, not regression); tree clean; merge-base with the fourth-pass tip `785266d3af1` confirms the rebase happened |
| Plugin parity + typebox | `diff -rq plugin/src` ↔ fork extension → exit 0 (11 files); manifests both `1.3.26`, lockfile aligned — K4-02 CLOSED by 0.6.7 |

---

## §2 — Live drill (fresh DB `/tmp/audit5/brain.db`, port 9879, debug binary)

| Leg | Command / observation | Verdict |
|---|---|---|
| Boot | `BRAIN_DB_PATH=/tmp/audit5/brain.db AUTH_TOKEN_FILE=/tmp/audit5/tokens BRAIN_REQUIRE_AUTH=1 BIND_PORT=9879` → `health:200`, opaque-token mode, Twokeys scoped | ✅ clean boot |
| F4-S-01 closure | `revoke {"principal":"agent"}` → **400 `unknown_principal`** naming `agent@loopback` | ✅ HELD live |
| **A5-01 regression** | `revoke {"principal":"user:ghost"}` → **400 `unknown_principal`** — a live identity the middleware WOULD honor cannot be revoked | ❌ **HIGH, live-proven** |
| Kill-switch (known) | `revoke {"principal":"agent@loopback"}` → 200 `revoked:true`; agent token → **401** on next call | ✅ HELD live |
| F4-S-02 closure | `DELETE /memory/1` → `{"deleted":true,"retained_proposal_copies":[],"scrubbed":false}` | ✅ HELD live |
| Live-DB safety | Drill server died once on `EADDRINUSE` against the live :8765 before the port move; all drill traffic went to :9879 | ✅ live untouched |

---

## §3 — Findings

### A. Server bugs (A5-*)

| ID | Sev | File:line | Narrative | Fix |
|---|---|---|---|---|
| A5-01 | **HIGH** | `src/workflow/mesh.rs:148-171`, `src/handlers/mesh.rs:401-402`, `tests/authz_matrix.rs:923-932` | `principal_known` matches only cards/presence/skills/delegations/prior-revocations + `agent@loopback`. A JWT `sub` (e.g. `user:ghost`) with no DB row is LIVE (middleware `is_revoked` honors its row) but its revoke now 400s. `revoke_via_route` sends no `allow_unknown` → **7/22 authz_matrix tests fail** (incl. the probe-blindness pin that revoking never-seen `user:ghost` was INTENTIONAL). Typo-confusion traded for kill-switch refusal — the worse direction. Full-binary run: 15 passed / 7 failed; subset re-run by orchestrator: 1 passed / 5 failed (same mechanism); live drill: `user:ghost` → 400 | **Warn-not-refuse:** always write the revocation, return 200 with `"known":false` + hint naming `agent@loopback`. Typo-safety becomes advisory; availability unconditional. Red-first pin: revoke-then-401 for a never-seen JWT sub |
| A5-02 | MED | `src/handlers/forget.rs:40-78` (zero `audit::` calls), `src/service/forget.rs:60-104` | The tx deletes vec0 + knowledge + tombstone (+ per-proposal scrub audits) but the erasure itself emits NO audit row — the one mutation family without in-tx evidence. Contrast purge (`service/lifecycle/purge.rs:206`), quarantine-delete (`server/router/memory.rs:2707-2716`) | One `audit::record(tx, …, "chunk:{id}", Ok, "forget")` inside the tx before commit |
| A5-03 | MED-LOW | `src/service/forget.rs:12-18,80-91` | Forget cascade narrower than purge: relationships SET NULL (orphan edges), `suggest_feedback`-by-`chunk_id` untouched (purge deletes them explicitly), `recall_traces`/`evidence_links`/`case_articles` soft refs outlive the tombstone | Same `suggest_feedback`-by-chunk delete purge uses, in-tx; document-or-delete the orphans |
| A5-04 (=R5-03) | LOW | `src/service/forget.rs:123-136`, `src/handlers/forget.rs:62-71,86-95` | (a) Correlation is `WHERE content = ?1` exact-only — sound TODAY (approve promotes the identical `&content` bytes, `gate.rs:557-571` → `:1286-1302`) but any non-approve writer silently breaks the link with no signal. (b) No LIMIT — N same-content proposals materialize unbounded. (c) `"scrubbed"` echoes the request flag, not the scrub count | (a) `content_hash` second arm or surface exactness in openapi; (b) `LIMIT 500` + `truncated` bit; (c) echo `scrubbed_count` |
| A5-05 | LOW | `src/handlers/mesh.rs:401-402`, `src/workflow/mesh.rs:192-197` | 5-table `principal_known` UNION runs on the raw unbounded body string; the `1..=256` refusal sits inside `revoke_principal`, after. No trim/case-fold — `" agent@loopback"` creates junk rows under `allow_unknown:true` | Length/charset gate in handler BEFORE the probe; `trim()` + loud whitespace refusal |
| A5-06 | LOW | `src/workflow/mesh.rs:229-236` | Drain covers `from_principal` only; a revoked delegatee's `requested` rows wedge `active` forever (result path refuses post-revocation). Pin even asserts the 0-drain | Cancel-or-terminal the wedged delegations + lineage event, or return `wedged_delegations:[ids]` |
| A5-07 | LOW | `src/handlers/transfers.rs:95` (`let _ = crate::audit::record(…)`) | Certified silence on an audit write — the Architecture Law's forbidden shape. The loud-warn pattern exists in-tree (`server/router/memory.rs:3046-3057`) | Mirror it: `if record(…).is_none() { tracing::warn!(…) }`. Also check `workflow/host.rs:251` |
| A5-08 | LOW | `src/config.rs:660-675` | `injection_threshold_high/low` use `.parse().ok()` + default — `=banana` silently becomes 0.9 while every sibling env refuses boot | Fail-closed parse + boot-time `validate_*` |
| A5-09 | INFO | `src/server/router/memory.rs:2367` vs `:2479-2492` | `get_chunk` returns sanitized `source`; `multi_get` omits `source` (no leak, verified field-by-field — contract skew only) | Emit sanitized `source` per row or document the projection in openapi |
| A5-10 | LOW (test) | `src/workflow/mesh.rs:1153-1171`, `tests/main_suite.rs:9988-10031` | Two vacuous-adjacent pins: (a) `cross_agent_recall_shows_origin_labels` asserts `sanitize_read_opt("agent")=="agent"` — deleting the labeling still passes; (b) `poisoned_lock_denies_every_gate` asserts message substrings — dropping arms stays green. The three closure pins (revoke/forget/get-source) were all revert-tested NOT vacuous | (a) Seed agent chunk, drive the real hit builder, assert prefix; (b) drive poisoned-store behaviorally |
| A5-11 | LOW (code) | `src/workflow/mesh.rs:289-325` | Drain `remaining` bookkeeping is dead logic (`let _ = remaining`); the `drain_incomplete` row still fires correctly — works, but misleads the next editor | Delete `remaining`, branch on `pages >= DRAIN_MAX_PAGES` |

### B. Docs-truth (T5-*)

| ID | Sev | Finding | Evidence |
|---|---|---|---|
| T5-01 | MED | **Closure gates omitted the `authz_matrix` binary.** The fourth-pass closure record lists lib 1204 + main_suite 199 + client 240 — all green — but never ran `cargo test --test authz_matrix`, the binary that owns the kill-switch contract. Main is RED at HEAD (`cb189f3`) and `release.sh` blocks tags on red: the tree is unreleasable and the record says green | `cargo test --test authz_matrix` → 7 failed (lane) / subset re-run 5 failed (orchestrator); closure record gates list in AUDIT.md has no authz_matrix row |

### C. Reverse-audit new findings (R5-*)

| ID | Sev | File:line | Narrative | Fix |
|---|---|---|---|---|
| R5-01 | LOW | `src/server/router/mod.rs:89` vs `route_guards.rs:52-55` | CSP middleware still `starts_with("/app")` while D12 made only the guard table segment-exact. A future `/app-*`/`/apple` route (or its error body) gets `CLIENT_CSP` (`wasm-unsafe-eval`) instead of `API_CSP` (`default-src 'none'`). No live `/app-*` route (`core.rs:35-40`) — defense-in-depth | Reuse the segment-exact predicate in the middleware + pin `/apple` → `API_CSP` |
| R5-02 | LOW/INFO | `src/service/mod.rs:92-98` | `no_sql_in_handlers` counter matches `"select "`-with-space only; `SELECT\t*`, `SELECT\n*`, `SELECT(*)`, `"SEL"+"ECT "` score 0 (demonstrated). No live violation (`rg` clean) — regression lock HELD, anti-malice/"ANY statement" wording overclaims | Broaden needles to `select[\s\(]` class + flag `+`-concatenation, or downgrade the claim to "regression lock" |

### D. Satellite findings (S5-*)

| ID | Sev | File:line | Narrative | Fix |
|---|---|---|---|---|
| S5-01 | LOW | fork `src/infra/unicode-visibility.ts:8-10` | Host set is a deliberate SUPERSET (`\u2060-\u206F` keeps U+2064/U+206A–206F the other three trees strip). Extra stripping is invisible-chars-only (nil user impact) but the fixture's exact-parity comment overstates | Narrow host RE to canonical ranges, or rename the contract to "canonical-subset" with extras pinned separately |
| S5-02 | LOW | fork `src/plugins/hooks.ts:535-552` vs `:479-534` | `mergeAgentTurnPrepare` concatenates `prepend/appendContext` raw; only `mergeBeforePromptBuild` sanitizes. X-S1 bypass IF turn-prepare output reaches the model without re-flowing through prompt-build (likely it does re-flow — both runners ride prompt-build — but unproven at this seam) | Route the joined result through `sanitizePluginContext`, or comment + test proving the re-flow |
| S5-03 | LOW | `scripts/install-service.sh:139,159,226` | Secret-parent dirs `mkdir -p` without mode (0755 via umask); files correctly 0600. Filenames/plist presence world-visible; contents protected | `mkdir -p -m 0700` for the three secret parents |
| S5-04 | INFO | fork `src/plugins/tool-metadata.ts:18` | "Signed ack" wording: ack is `pendingAck` + operator file-touch (v1.28.67 design), no cryptographic ack signature. Matches the disclosed ceiling (surfaced, not gated) — wording only | Keep labeled as ceiling; or sign pins via `sign_manifest_bytes` |

### E. Fork findings (K5-*)

| ID | Sev | Finding | Evidence |
|---|---|---|---|
| K5-01 | INFO | Behind-gap re-opened 0→**111** within the day (upstream lands dozens of commits/day). Structural velocity, not a regression; merge-tree exit 0, zero conflict markers (141 greps all prose) | `rev-list --count HEAD..upstream/main` → 111 (fresh fetch); `merge-tree` clean |
| K5-02 | CLOSED | Typebox four-way truth aligned by 0.6.7 (manifests 1.3.26 + lockfile) — closes K4-02 | grep-verified all three manifests + lock |
| K5-04 | INFO | `docker-compose.yml:68-71` publishes 18789/18790/3978 on all interfaces; default `--bind lan`; `fly.toml` bind lan | Operator-exposure note, was §5 OUTSTANDING, now partially verified |
| K5-05 | OPEN (was OUTSTANDING) | `publishToNpm:true` with no provenance pin observed; built-dist vs source integrity still unchecked | Next fork lane |

Rebase-survival (MB `785266d` → upstream `045a9de`, 111 commits): every
hardening LOW conflict risk — context-hygiene/catalog-pins/approval/
truncation/remote-image/brain-extension/update files untouched upstream
since MB; `hooks.ts` touched twice upstream (message_sending +
allSettled only, prompt-build seam intact, grep-verified);
`cli-runner/prepare.ts` touched (hook call site intact). Full table in
the lane transcript; headline: **the next rebase is mechanical, not
semantic**.

### F. Regulatory gaps (L5-*; matrix §6)

| ID | Sev | Finding | Fix owner |
|---|---|---|---|
| L5-01 | LOW-MED | CO row misses **HB26-1263 Chatbot Safety Act** (signed 07-01-2026, eff 01-01-2027: age estimation, AI-not-human disclosure, teen safeguards) + stay-note imprecise (Apr-27-2026 stay attached to repealed SB24-205; extension to SB26-189 rests on the order's parenthetical) | map lane |
| L5-02 | LOW-MED | IL row misses **SB315 AI Safety Measures Act** (signed 07-06-2026, eff 01-01-2027; frontier third-party audits Jan-2028, $1M/$3M) | map lane |
| L5-03 | LOW | Takedown bucket misses the federal 48h clock (**TAKE IT DOWN Act Pub.L.119-12**, eff 05-19-2026, FTC) — platform-deployers follow the wrong SLA | map lane |
| L5-04 | LOW | CT sign-date: map says Jun 2; primary governor notification says **signed May 27, 2026** (notified May 29). PA26-100 cited in COMPLIANCE.md:601 unverified | map lane |
| L5-05 | LOW | FL row misses 2025 48h platform-removal duty (CS/SB1400 amending §836.13) | map lane |
| L5-06 | LOW | WA "study only, no filing" stale — final report **07-01-2026** exists (11 recommendations, 4 enacted incl. companion-chatbot Jan-2027) | map lane |
| L5-07 | LOW-MED | Single-forget default retains proposal copies with no Art 17(3) basis recorded — DSAR purge unaffected (clears proposal refs); needs a docs directive ("Art 17 requests: use `/dsar purge` or `?scrub_proposals=1`; single-delete preserves the decision record per Art 17(3) legal-claims/audit") | forget/DSAR lane |

L4-01 re-verified CLOSED in code + docs. US map core dates/duties HELD
on every re-verified row (TX TRAIGA, CA SB53/AB2013/SB942+AB853/CCPA-ADMT,
CO SB26-189, UT SB149, IL HB3773, NYC LL144, CT phasing, FL HB919, WA
SB5838). EU Art 50 split posture (bridge, not satisfaction), GDPR clocks,
CRA runbook clocks, Data Act/PLD/EAA/NIS2 scoping all HELD. Standards:
ISO 42001:2023, NIST RMF 1.0 + GenAI Profile Jul-2024 (revision pending —
INFO stamp owed), OWASP LLM Top 10 2026 (08-2026, exact), ASVS 5.0.0
(05-2025). Canada/Australia/Singapore/India/OECD/CoE-in-force/CWE/SLSA/
CycloneDX-spec-numbers remain OUTSTANDING (not findings).

---

## §4 — Parity matrix (server ↔ plugin ↔ client ↔ fork) — REBUILT

| Surface | Server | Plugin | Client | Fork host | Pin | GAP |
|---|---|---|---|---|---|---|
| Invisible set | `strip_invisible.rs:22` canonical | `format.ts:192` | vendored `main.rs:99` | `unicode-visibility.ts` (SUPERSET) | 4-lane fixture (`invisible-classes.json` v1; server exhaustive, plugin >300, client 240, fork 189 — all re-verified present) | **S5-01** (superset wording only; behavior in-sync) |
| Read-seam fixed-point strips | gate.rs + fence.rs fixpoints | sanitizeForBlock ordering | renders server-shaped text; `dangerous_inner_html` banned by grep-pin | merge-seam sanitizer | per-tree pins; weld attacks re-attempted (nested/mixed-case/entities/`\uXXXX`) | none — HELD |
| Plugin source 0.6.7 | `plugin/src` | — (source) | — | `extensions/brain-server/src` | `diff -rq` exit 0 (orchestrator, today) | none |
| untrusted:true | recall/suggest live (4th) + code | fence emission | server-shaped render | quoted-replay labeling | X-R1 pin | none found |
| Digest approvals + replay | 409 live (4th) | approval args | console decide (same CAS) | broker + gateway paths | gate/channel pins | none found |
| Token handling | Twokeys + revoke live | multi-line refusal both rungs | — | env/file refusal + tests | fork tests | none found |
| Truncation head+tail | n/a (server-side unaffected) | envelope | — | unconditional + exact-count | X-L2 pins | CLI-runner tool-result path has display-only truncation (matches 4th-pass OUTSTANDING; fork-owned) |
| MCP scope | `mcp.rs` fail-closed + `x-brain-scope` | tool usage | — | catalog pins + `pendingAck` | dispatch-gate tests | none (signed-ack wording = S5-04 INFO) |
| SSE/WS | 403-before-stream (`alert.rs:185-191`) | — | poll-fallback + 512-cap dedup | — | pre-open pin | mid-stream revocation not re-checked per event (read-only residual, unclaimed — no ID) |

---

## §5 — Mode B verdicts (held claims — attacks that failed)

Weld-heal (element + markdown-ref, incl. entities/`\uXXXX`/ref-style/
autolink) HELD (`fence.rs:58-152,361-373`; fixpoint deletes-only).
Element-set gaps (`animate`/`details`/`math` + `on*`/`javascript:`)
HELD as disclosed ceiling (ponytail at `gate.rs:369-392` + KB
`default-src 'none'` + client inner-HTML ban). Token smuggling
(query/header fallback, multi-line, `\r`/tab) HELD. Digest replay across
transports HELD. Revoked reach (`/auth/refresh`, SSE pre-open,
`valet/due` opt-in+domain, console `actor_revoked`) HELD. Chain-key vs
`.bak` HELD as disclosed ceiling (demonstration succeeds by design).
Provenance strip/substitute on all 4 classes HELD (14-test family
re-run: green). Traverse exact-kind HELD. Drain/ack tenant derivation
(never body-derived) HELD. Dormancy pin HELD (recursive + concat
needle). Plaintext temps 0600/0700 HELD. Model-manifest symlink refusal
HELD (check-then-read TOCTOU = disclosed host ceiling). Bridge
redirect-never-follow + media-URL gate HELD. NAT64 row HELD. Source-64 +
jti/iss caps HELD. OTLP disclosed ceiling HELD. Badges selfcheck HELD
(exit 0, today). Docs-truth guard HELD within scope. Parity-zero HELD
as "no known gaps". reg_watch dual clocks HELD. AgBOM endpoint HELD
(authz + live regen + disclosed MCP/caller-side scope). OWASP-2026 +
NIST/compliance maps HELD as control-coverage, not elimination.
WEAKENED→findings: `no_sql` wording (R5-02), `/app` CSP seat (R5-01),
forget exactness disclosure (A5-04). New-pin vacuity checks: all four
closure pins FAIL on revert — genuinely behavioral.

---

## §6 — Regulatory applicability matrix (summary; full rows in lane transcript)

Verified 2026-09-12 against primary sources (capitol.texas.gov,
leginfo, coag.gov/ai, CourtListener 18-1, ILGA, portal.ct.gov,
flsenate, atg.wa.gov, EUR-Lex, cppa.ca.gov, MSIT, gov-online.go.jp).
US: TX/CA×4/CO/UT/IL/NYC/CT/FL/WA core duties+dates HELD; gaps L5-01–L5-06
(additive rows/precision). EU: Art 50 split, GDPR clocks, CRA clocks,
Data Act/PLD/EAA/NIS2 scoping HELD; gap L5-07 (directive). World:
UK/China/Korea/Japan/Brazil/CoE spot-held or correctly-unclaimed;
Canada/Australia/Singapore/India/OECD OUTSTANDING. Component-vs-deployer
split preserved every row; no watch upgraded to duty.

---

## §7 — Coverage grid (layer × dimension) — HONEST

Legend: ✅ evidence-backed this pass · O = OUTSTANDING (named next step).

| Layer | D1 design | D2 correct | D3 conc | D4 res | D5 bounds | D6 authz | D7 crypto | D8 integrity | D9 inject | D10 egress | D16 DoS | D18 config |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| handlers/service seam | ✅ A5-02/03/04 | ✅ A5-04/09 | O (spot only) | O | ✅ A5-05/R5-01 | ✅ A5-01/T5-01 | — | ✅ A5-02 | ✅ SQL-bind audit | — | O | — |
| auth/policy | ✅ A5-01 | ✅ A5-05 | ✅ poison-posture | — | ✅ jti/iss | ✅ live drill | ✅ provenance-14 | — | — | — | — | ✅ A5-08 |
| workflow/channels | O | ✅ A5-06/11 | ✅ drain-CAS read | O | ✅ caps present | ✅ HMAC read | — | O | — | — | O | — |
| storage | — | ✅ live erasure | — | O | O | — | — | ✅ tombstone | — | — | O | — |
| gate/read seam | ✅ | ✅ weld-attacks | — | — | ✅ source-64 | — | — | — | ✅ live+pins | — | — | — |
| screen/scorer | ✅ budget read | O | ✅ mutex read | ✅ caps read | O | — | — | — | ✅ b64 read | — | O (unmeasured) | — |
| egress | — | ✅ redirect/media read | — | — | — | — | — | — | — | ✅ table+pins | — | ✅ fail-closed |
| provenance | — | ✅ 14 green | — | — | — | — | ✅ | ✅ cert live | — | — | — | — |
| plugin/transport | ✅ envelope | ✅ redirect/origin | — | — | ✅ 8k blocks | ✅ scope gate | — | — | ✅ fence order | ✅ no-follow | — | ✅ scheme gate |
| client | ✅ Dioxus escape | ✅ inner-HTML ban | — | — | ✅ badge counts | — | — | — | ✅ grep-pin | — | ✅ SSE caps | — |
| fork host | ✅ S5-02 | ✅ hardenings wired | — | — | ✅ truncation | ✅ catalog | ✅ TOFU 0600 | ✅ loud rebuild | ✅ hygiene | ✅ images gate | — | ✅ token rungs |
| CI/release | ✅ GHA SHAs | T5-01 RED main | — | — | — | — | — | ✅ SBOM+AgBOM | — | — | — | ✅ tiers |

O-cells: deep concurrency (lock-ordering proof), resource-exhaustion
budgets under adversarial load, scorer-mutex DoS measurement, full
storage-migration discipline — next instrumented pass.

---

## §8 — Remediation plan (release-sequenced, naming discipline)

- **v1.28.83 "Recall"** (kill-switch availability, release-blocking):
  A5-01 warn-not-refuse (`revoked:true` + `"known":false` + hint;
  `revoke_via_route` unchanged) with red-first pin
  (revoke-never-seen-JWT-sub → 200 → 401 on route); T5-01 gate fix
  (closure/release checklist runs EVERY test binary incl.
  `authz_matrix`; selfcheck refuses "green" with unrun binaries);
  A5-02 erasure audit row; A5-03 feedback cascade. CRATE_TEST_FLOOR +4.
- **v1.28.84 "Quarterly"**: L5-01–L5-07 map rows + erasure directive;
  S5-03 dir modes; R5-01 CSP predicate; R5-02 needle-or-claim;
  A5-05/A5-06/A5-07/A5-08/A5-10/A5-11; S5-01 contract rename;
  S5-02 seam proof-or-route.
- **Fork lane**: K5-05 npm provenance decision; K5-04 bind note in
  operator docs; rebase at operator cadence (mechanical per survival
  table); S5-02 with upstream if the seam is host-owned.
- **Re-audit triggers**: v1.28.83 ships → re-run `authz_matrix` +
  drill §2 verbatim; quarterly reg drift per the map's cadence.

---

## §9 — Gates

| Gate | Result |
|---|---|
| `cargo test --test authz_matrix` (full) | ❌ **15 passed / 7 failed** (lane) — A5-01 |
| `cargo test --test authz_matrix revoked` (subset) | ❌ 1 passed / 5 failed (orchestrator, today) |
| `cargo fmt --check` (server) | ✅ |
| `cargo fmt --manifest-path client/Cargo.toml --check` | ✅ |
| `./scripts/badges.sh --selfcheck` | ✅ exit 0 |
| Live drill §2 (fresh DB, test port) | ✅ closures held; A5-01 live-proven |
| `diff -rq plugin/src` ↔ fork extension | ✅ exit 0 |
| clippy/otel/engine-crates/steward/client-gate | NOT RUN (owed before any v1.28.83 push per AGENTS.md dry-run list) |

## §10 — Ceilings (what this audit could NOT verify)

1. Deep concurrency proofs (lock ordering under contention, pool
   exhaustion under adversarial load) — read-only spot checks.
2. Scorer-mutex DoS measurement (budget code-read; no adversarial timing).
3. Fork npm-bundle dist integrity (K5-05), `__openclaw_vitest__` full
   content, compose/fly runtime exposure beyond config read.
4. Canada/Australia/Singapore/India/OECD/CoE-in-force/spec-number cells.
5. Client-console rendering leg (no headless browser) — code-shape only.
6. Subagent lane transcripts are summarized here; full text lives in
   this session's execution log, not in the repo.

— Orchestrator, fifth pass, 2026-09-12 evening. Main is red: fix A5-01 first.
