# Second-Pass Security Audit — brain-server v1.28.75 × openclaw fork — 2026-09-09

**Auditor:** second-pass full-stack review (code-level, read-only, fix pass shipped as v1.28.76)
**Targets:**
- `brain-server` v1.28.75 @ `1a39bc4` (server), plugin 0.6.0 — `/Users/mark/Sites/brain-server`
- `openclaw` fork @ `82ad327057f` (pre-merge main) — `/Users/mark/Sites/openclaw`

**Method.** Six parallel deep-audit lanes over the surfaces the 2026-09-06 audit
covered, PLUS two lanes it under-covered (storage/SQL construction, and
resource-exhaustion bounds) and a docs-truth sweep. Every finding below was
verified against current source (file:line), and the exploited behaviors were
demonstrated by compiling and executing exact ports of the strip functions
before they were fixed. Prior namespaces stand: `F-` (08-16), `X-` (09-06).
**Fresh namespace: `SP-` (second pass, 09-09).**

**Frameworks:** unchanged from the 09-06 audit — OWASP Agentic Top 10,
Microsoft AI Red Team taxonomy v2, LITL, IFC (FIDES/CaMeL), Rule of Two, MCP
rug pulls, EchoLeak/Unicode exfil — applied with second-pass questions: *are
the closures themselves sound? do the strips survive re-assembly? do the
gates reach every door? do the docs match the code?*

---

## 0. Executive summary

**Posture: STRONG and improving — the 09-06 closures held (36 of 41 re-verified
sound), but the second pass found what first passes find: the closures had
seams of their own.** The headline classes:

1. **The read-seam strips heal under re-assembly (2 HIGH).** A single-pass
   strip is not a closure: `<scr<script>ipt>` welds back into a live
   `<script>` after `strip_hostile_elements`, and `[![a](inner) c](outer)`
   welds back into a live auto-fetch `![…](outer)` image after
   `strip_markdown_refs` — the EchoLeak class the strip exists to kill.
   Fixed by bounded fixed-point iteration in v1.28.76.
2. **The fork's .66/.67 halves were never shipped (HIGH, openclaw).**
   `truthglass-approval` and `pin-1.28.67` were local branches, never merged:
   fork main had NO approval-args, NO head+tail truncation, NO MCP catalog
   pins. Merged 2026-09-09 (four commits on fork main).
3. **Compute bounds were missing on the model seam (HIGH+MED).** The ONNX
   injection scorer had no sentence/size budget (every inference serializes
   on one process-wide mutex — a 1 MiB ingest pins every screened write),
   and the embedder encoded full-size content. Budgeted in v1.28.76.
4. **Some gates didn't reach every door.** The kill-switch missed
   `/auth/refresh` (public route) and the console actors; the MCP read-scope
   gate missed `ump.feedback` (a durable write); the live SSE stream missed
   `valet/due` (private reminder labels streamed unfiltered); the X-W4 valet
   fence missed the CAS state-advance path. All closed in v1.28.76.
5. **The docs had drifted from the code.** THREAT_MODEL frozen at v1.28.68;
   SECURITY.md history stopped at v1.28.17; the plugin changelog missing
   0.6.0; `repo-brief.sh` (the agent briefing tool) crashed at HEAD. All
   corrected in v1.28.76.

Fresh finding count: **30** (5 HIGH-class, 12 MEDIUM, 9 LOW, 4 INFO) across
both trees. v1.28.76 closes the 5 HIGH and 7 of the MEDIUM surgically; the
rest are planned (§4) with named releases.

---

## 1. Closure verification of the 09-06 ledger (X-)

Re-verified SOUND at code level (each by direct read this session): X-W1
(reserved vocabulary at the shared funnel `enqueue_child`; every caller
checked), X-W2, X-W3, X-W5, X-M6, X-A1 (kill-switch in bearer AuthN,
probe-blind), X-A2 (denylist TTL = token exp, clamped 24 h), X-A3a (per-kid
alg pin), X-A5 (+ no metrics label-injection: domain charset), X-A6 (ONE
`PUBLIC_PATHS`), X-A7/A8 (two-direction guard coverage), X-R1, X-R4a
(strip-before-screen), X-R4b/c, X-R5 (invisible-set parity, exact), X-R6,
X-R7, X-R3 fast-path SOUNDNESS (byte `<`/`[` checks match the strips' own
byte definitions — a row with `<` always takes the slow path), X-S1 (merge
seam), X-M1 (scope gate: fail-closed parse, dispatch-time), X-M2, X-M4, X-M5
(crank), X-E3 CORE (resolve→validate-every→pin insert-only incl. the
check-then-pin race; lazy path re-validates; redirects refused), X-C1 (parcel
signer required + alias gate + strict verify), X-C2 (integrity census),
X-C3 (other halves: head-pin disclosed, legacy-epoch marked, `--allow-chainless`
loud), X-C4 (rotate mechanics one-deep), X-W7 dormancy (zero production call
sources) — **except as re-opened below.**

**Re-opened / weakened closures:**

| ID | Was | Second-pass verdict |
|---|---|---|
| SP-R1 | X-R3 (hostile elements) | **Closure unsound — HIGH.** `strip_hostile_elements` is single-pass; the `<`+prose of a non-set tag re-emitted before a stripped set-tag welds into a live element (`<scr<script>ipt>` → `<script>alert(1)`; demonstrated). Also the in-code digest claim ("review_digest binds the stored form") is FALSE — the digest binds the read-canonical form (fixed doc; see SP-D1). |
| SP-R2 | EchoLeak strip (`strip_markdown_refs`) | **Closure unsound — HIGH.** The outer label is re-emitted unscanned: `[![a](junk) c](url)` → live `![a c](url)` (demonstrated). Not idempotent-first-pass; the plugin's JS regex does NOT heal (drift). |
| SP-W4 | X-W4 (valet `what` fence) | **Closure incomplete — HIGH.** The fence held at `post_run` only; `PUT /workflow/runs/{id}/state` (CAS advance) rewrote `what` unscreened, and the label rides the bus to Signal at fire time. |
| SP-W7 | v1.28.75 "kill_on_drop pinned at the spawn seam" | **Claim code-false — HIGH (dormant module).** The pin `exec_spawn_carries_kill_on_drop` asserted a source string whose ONLY occurrence was the assertion itself (vacuous, can never fail), and the exec spawn is `std::process::Command` — no kill_on_drop API exists there. The real mechanism is the 30 s deadline kill+wait+join block. Re-pinned behaviorally (injectable deadline; a 30 s sleep budgeted at 250 ms must return the refusal in <5 s or the pin fails). |
| SP-F4b | v1.28.74 Origin (fork half) | **Closure not deployed — MED.** The fork's runtime plugin was 0.5.1 (not 0.6.0): no `origin_context` sent, so channel captures stored as `owner` — the labeling law defeated end-to-end in the live deployment. (Sync is the .76 fork follow-up; server behavior correct.) |
| SP-F1 | v1.28.66/.67 (fork halves) | **Never shipped — HIGH.** Truthglass (X-L1, X-L2) and Pin (X-M3) lived on unmerged local branches; fork main had none of it. Merged 09-09 (see §3). |

---

## 2. Fresh findings (SP-)

### 2.1 brain-server

**SP-R1 (HIGH — fixed .76)** heal-welding `strip_hostile_elements` — §1.
Fixed by `strip_to_fixpoint` (bounded 64 passes; overflow sweep drops `<`);
pins `hostile_element_strip_does_not_heal_nested_tag` incl. the 65-level
overflow construction.

**SP-R2 (HIGH — fixed .76)** heal-welding `strip_markdown_refs` — §1.
Fixed the same way (overflow sweep drops `[]!`); pin
`strip_markdown_refs_does_not_heal_nested_construct`.

**SP-S1 (HIGH — fixed .76)** classifier scoring had no sentence or size
budget: `score_field` scored EVERY sentence through ONNX behind one
process-wide mutex; two amplifiers stack (1 MiB ingest → ~10⁶ inferences;
the review queue re-screens every row per listing). Budgeted:
first 64 sentences of the first 16 k chars. Pin `score_field_is_budgeted`.

**SP-W4 (HIGH — fixed .76)** valet `what` rewrite via the CAS path — §1.
Closed at the kind: valet-kind runs run `vet_open_state` in `put_run_state`;
named 400 `valet_what_refused` + Denied audit row. Pin
`put_state_refuses_unscreened_valet_what` (integration).

**SP-W7 (HIGH, dormant — fixed .76)** vacuous kill_on_drop pin — §1.
Behavioral re-pin `exec_deadline_kills_child`; release-record correction in
CHANGELOG §[1.28.76].

**SP-A1 (MED — fixed .76)** `/auth/refresh` never consulted the principal
kill-switch (public route ⇒ middleware check bypassed): a revoked identity's
refresh chain rotated forever behind the revocation. Closed with
`refresh_principal_alive` → 401 `identity_revoked` (the middleware's own
code). Pin `refresh_refuses_revoked_identity`.

**SP-A4 (MED — fixed .76)** console actors bypassed the kill-switch:
`resolve_console_actor` consulted map+roles only — a revoked principal could
list/decide on bridge HMAC alone. Closed: `actor_revoked` before capability
work. Pin `console_actor_revoked_refused` (fixtures ride the production
cores: role::upsert, apply_user_map_change, revoke_principal).

**SP-A7 (MED — fixed .76)** live `valet/due` SSE events skipped the
opt-in + per-domain gate (the replay path already had it) — every private
reminder label streamed to any unfiltered Read-on-global subscriber across
all domains. Closed: `live_event_admissible` gates BOTH kinds. Pin
`valet_due_requires_optin_and_domain_authz`.

**SP-E1 (MED — fixed .76)** `validate_public_addrs` missed IPv4-mapped IPv6
(`::ffff:169.254.169.254` passed the v6 table; the kernel routes to the
embedded private v4), NAT64 `64:ff9b::/96`, 6to4 `2002::/16`, Teredo
`2001::/32`, discard `100::/64`. Closed: mapped-v6 normalizes into the v4
table + five new rows + edge-literal pins (mapped PUBLIC v4 still admitted —
pinned complement).

**SP-M1 (LOW — fixed .76)** `BRAIN_MCP_SCOPE=read` omitted `ump.feedback`
(a durable last-wins upsert steering ranking/KCS evidence). Now gated +
annotated; pins updated (5 annotated verbs).

**SP-C1 (MED — planned .77)** restore's chainless/chain-verify refusals fire
AFTER `write_atomic` replaced the live DB (both checks could run on the
in-memory image pre-overwrite); the refused path leaves the unattested image
in place and the CLI never names the `.bak` on failure.

**SP-C3 (MED — planned .78)** standby `verify_follower` is self-asserted:
the manifest signature is verified against the did named INSIDE the
attacker-replaceable sig file — precisely the class parcels closed with
`expected_signer`. Needs the operator-did pin (foreign-signer refusal).

**SP-S2 (MED — planned .78)** legacy vector search (`perform_search_legacy`,
reached when vec0 is absent — e.g. restored-legacy images) has NO `flagged`
predicate and hardcodes `flagged:false` in its hits: quarantined plants are
clean-looking hits. The vec0 path filters correctly.

**SP-S3 (MED — planned .78)** quarantined rows' vectors occupy the
sqlite-vec k-budget (inserted before the quarantine gate; the `flagged = 0`
predicate applies AFTER the ANN top-k, overfetch k·max(RRF)=20): ~20
vector-near plants shadow a target memory out of recall (denial, not
poisoning — the plants themselves stay quarantined).

**SP-S5 (MED, compliance — planned .77)** `suggest_feedback` was missing
from every erasure path (subject's tenant/session/chunk-refs survive a
certified purge) — a mantra-1 ("…and erase") gap. **The chunk arm shipped in
.76** (purge deletes feedback rows for purged chunks; sweep adds the
tenant arm, counted under dependent rows; pin
`purge_removes_suggest_feedback_for_purged_chunk`). The session-join
question (session ids are not principal ids) stays open for .77's erasure
release.

**SP-S2e (MED — planned .78)** embedder input unbounded — **fixed in .76**
(`MAX_EMBED_CHARS` 8 000 at every backend boundary; stored text stays
verbatim; pin `embed_input_is_budgeted`). Listed here for the ledger trail.

**SP-S6 (LOW-MED — planned .78)** global content-hash dedup is cross-domain:
a second tenant ingesting byte-identical boilerplate gets the FIRST tenant's
row id + `duplicate` — a cross-tenant content-existence oracle. Fix: domain
in the dedup predicate.

**SP-S3b (LOW — planned .77)** by-id reads return quarantined rows with no
`flagged` marker (recall emits it; get/multi-get don't).

**SP-W8 (LOW — planned .77)** DSAR subject patterns flow into
`LIKE %subject%` unescaped (`%`/`_` widen over-match → over-deletion); the
correct fence exists in `workflow/kcs.rs` — reuse it.

**SP-S9 (LOW — planned .77)** GDPR `export_bundle` materializes the whole
DB unbounded (Admin-gated; page/stream cap wanted).

**SP-W12 (LOW — planned .77)** valet brief `what` bypasses the read seam
(the one unsanitized text field in that handler).

**SP-W1 (MED — planned .77)** `PUT /workflow/runs/{id}/state` aside (fixed),
the valet crank livelocks at ≥100 due envelopes: `due()` truncates at 100,
then the handler REFUSES at ≥100 — a full backlog can never drain
(operator-reachable wedge). Fix: fire the capped batch, report the remainder.

**SP-W9 (MED — planned .78)** `drain_ping_batch` has no kind/tenant scoping:
any bridge consumes (and kills) every bridge's handover pings — cross-bridge
theft via the shared outbox.

**SP-C2 (MED — planned .78)** standby manifest verification self-asserted —
see SP-C3 (same release).

**SP-C4 (LOW-MED — planned .79)** `brain key rotate` writes the new seed
non-atomically at umask mode (the token rotator one family over does it
right: 0600 temp + rename), and `.prev` inherits the OLD file's mode; a
post-rename crash desyncs `operator_key_generation` from the epoch picker.

**SP-C6 (LOW — planned .79)** legacy operator-key migration adopts the
FIRST 32-byte 0600 file in readdir order — an unrelated secret becomes the
signing key silently. Restrict to the known legacy name (`operator.key`).

**SP-A2 (LOW — planned .79)** legacy multi-operator token files silently
demote token #2 to the scoped agent principal at upgrade (info log only).
Fix: explicit ack env when the operator source parses to ≥2 tokens.

**SP-A5 (LOW — planned .79)** one refresh chain per (iss, sub): two live
sessions refreshing out of order burn each other's families (self-DoS).
Fix: per-session chain id.

**SP-A6 (LOW — planned .79)** failed-auth requests execute the one-time key
migration on the auth path (`capability_pass_through` calls
`operator_signing_key()` before the path check). Reorder; migration at boot.

**SP-W10 (LOW — planned .78)** `post_event` accepts a foreign-run
`parent_event_id` (lineage-integrity latent false-red).

**SP-W11 (LOW — planned .78)** channel-out drain marks rows delivered
before the bridge sends (at-most-once; docs claim at-least-once).

**SP-I1 (INFO — planned .79)** mediated hostcall HTTP buffers
undeclared-length bodies before truncation (bounded by allowlisted-host
goodwill); stream-read with a cap.

**SP-I2 (INFO — planned .79)** `/events` authorizes at connect only — a
principal revoked mid-stream keeps the feed (document-or-recheck).

**SP-I3 (INFO — planned .79)** capability tokens verify against the CURRENT
operator key only (cards get one generation of overlap) — undocumented
asymmetry.

**SP-D1 (LOW, code-doc — fixed .76)** the gate.rs claim "review_digest
binds the stored form" was false (it binds the read-canonical form;
Scrim's strip addition moved markup rows' digests — fail-closed 409 at
approve, impact = re-review friction). Corrected in-code + release
discipline note. Related (fixed .76): the hostile-element doc honestly
scopes the set (fetch/embed class — `on*=` handlers and script-scheme
hrefs on OTHER elements are the documented ceiling; KB ships
`default-src 'none'`).

**SP-D2 (LOW — planned .79)** reg_watch pins `attach_aigen` EXISTENCE at
the four known sites — a fifth emission site added since would pass unsealed.
Add a floored site-count.

### 2.2 openclaw fork

**SP-F1 (HIGH — fixed 09-09)** stranded Truthglass/Pin branches — §1.
**Merged** to fork main 2026-09-09, four commits, fork `CHANGELOG.md` +
`pnpm-lock.yaml` byte-untouched (user policy: no upstream-conflict surface):
`1e5d31fb85e` (approval args X-L1), `fde920cb23a` (truncation X-L2),
`6d54452a029` (MCP catalog pins X-M3), `1f3e8a36cff` (fix: pins'
`stableStringify` key ordering is code-unit compare, never `localeCompare` —
locale collation would fingerprint the same catalog differently across ICU
builds = permanent false drift; pinned `stable_stringify_is_icu_independent`).
211 affected-lane tests green; branches + their worktrees deleted (tips
`85a4ab438ce`, `ff6f3249e6b` recorded here).

**SP-F2 (HIGH — planned, fork)** the Meridian sanitizer covers only
`before_prompt_build`; `agent_turn_prepare`, `heartbeat_prompt_contribution`,
and queued next-turn injections ride raw (`mergeAgentTurnPrepare`,
`buildPluginAgentTurnPrepareContext`, the harness twin) — the host's own
taxonomy names all three PROMPT_INJECTION_HOOK_NAMES. A hostile plugin can
forge `⟦openclaw:ctx⟧` provenance markers or close the memory fence through
any of them. Fix: `sanitizePluginContextSegment` at the aggregate joins.

**SP-F3 (MED — planned, fork)** multi-block MCP tool results skip marker
sanitization (only invisible-strip inside; the single-text path sanitizes),
and the `structuredContent` JSON mirror does not escape the envelope markers.

**SP-F4 (MED — planned, fork)** the replay marking never fires on the
ACTIVE turn (only when the text becomes history), and the runtime plugin
must be synced to 0.6.0 (SP-F4b — see §1).

**SP-F6 (LOW — planned, fork)** `fetchClawHubSkillCard` fetches a
skill-author-controlled URL with raw `fetch` (CLI/operator context; route
through the guarded fetch or pin to the ClawHub origin).

### 2.3 Docs truth (SP-D — all fixed in .76 unless noted)

| Item | Was | Now |
|---|---|---|
| SP-D3 | `scripts/repo-brief.sh` crashed at HEAD (`grep -c` exit-1 on 0 route sites in main.rs under `set -e`; routes moved to `src/server/router/**` in the Capstone) — the ONE-SHOT briefing tool every agent session starts with was broken | Fixed: counts router sites; runs clean |
| SP-D4 | THREAT_MODEL.md content frozen at v1.28.68 (Shutter): no egress pinning, agent identity, screening layers, key lifecycle, origin labels, or exec-mediation dormancy | New §5b (the .63–.75 line: 13 control rows + kept ceilings); residual-risk item 3 updated; coverage stamp .76 |
| SP-D5 | SECURITY.md version history stopped at v1.28.17 (08-23) | 13 rows added (Wardline → Preflight) |
| SP-D6 | `docs/OWASP_AGENTIC_2026.md` last reviewed 08-15 at v1.27.12; ASI05 row said "no eval path" while a (dormant) exec seam now exists | Review stamp 09-09; ASI05 row states the dormant, pinned seam; dated addendum lists the line's deltas |
| SP-D7 | `docs/AI_LITERACY.md` "Applies to: 1.28.34"; audit chain called plain "SHA-256" | Stamp 1.28.76; "keyed HMAC-SHA256 chain, per-DB epoch + pinned head" |
| SP-D8 | `docs/openclaw-integration.md` documented plugin 0.5.0; zero coverage of `untrustedOrigins`/`channel-capture` | 0.6.0 + the origin-label contract documented |
| SP-D9 | `plugin/CHANGELOG.md` had no 0.6.0 row (features shipped in code, changelog silent) | [0.6.0] row written from the shipped behavior |

---

## 3. The fork merge (user-authorized, 2026-09-09)

The branches carried 39/36 branch-only commits — almost entirely the plugin
tree's own history duplicated in parallel (same content, different SHAs), so
a literal merge would have conflicted across the whole `extensions/brain-server`
tree for zero content gain. The unique payload (three commits) landed via
`git cherry-pick -x` with metadata files excluded:

| Commit | Content | Upstream-conflict surface |
|---|---|---|
| `1e5d31fb85e` | Approval args (X-L1): payload field, caps, `buildApprovalArgs`, gateway sanitize+re-cap, Swift model | src/agents, src/gateway, packages/gateway-protocol, apps (protocol regen) — functional only |
| `fde920cb23a` | Truncation head+tail + exact counts (X-L2); `hasImportantTail` deleted | 1 file + 2 test files |
| `6d54452a029` | MCP catalog pins (X-M3): per-tool+per-server sha256, per-run reconcile, `pendingAck`, rug-pull demo | new files + 3 small edits |
| `1f3e8a36cff` | Pin-quality fix: code-unit key ordering (SP-F5) | 1 file + pin |

Verified: content equivalence vs both branch tips (Pin files byte-identical;
Truthglass superseded only by main having evolved past the stale bases),
203+ affected-lane tests green, `oxfmt --check` clean, `CHANGELOG.md`/
`pnpm-lock.yaml` diff-empty vs pre-merge parent. Branches `truthglass-approval`
and `pin-1.28.67` deleted (local-only; tips recorded above). The branch's own
parallel Meridian variant (`bcb2d3062d0`) dies with the branch — main's
shipped Meridian (`2b163501e63`) is the tested one.

**Still open on the fork (planned, not forgotten):** SP-F2 (sanitizer reach),
SP-F3 (multi-block markers), SP-F4 (active-turn marking + plugin 0.6.0 sync
into `extensions/brain-server`), SP-F6 (ClawHub fetch). These are fork
commits riding the next fork-sync, NOT brain-server releases.

---

## 4. Remediation plan (multi-release, ponytail-scoped)

**v1.28.76 "Selfheal" (shipped with this audit)** — theme: *nothing stripped
may reassemble, and no gate has a side door.* Closes SP-R1, SP-R2, SP-S1,
SP-W4, SP-W7, SP-A1, SP-A4, SP-A7, SP-E1, SP-M1, the purge half of SP-S5,
SP-D1–D9, and the embed budget.

**v1.28.77 "Erasure"** — the mantra-1 release: SP-S5 session arm, SP-W8
(DSAR LIKE escaping), SP-S3b (by-id flagged marker), SP-S9 (export cap),
SP-C2/SP-C1 (restore verifies BEFORE the live DB is overwritten; failure
names the `.bak`), SP-W1 (valet crank backpressure), SP-W12 (valet brief
read seam).

**v1.28.78 "Unconditional"** — quarantine is unconditional on every leg:
SP-S2 (legacy vector path), SP-S3 (KNN overfetch/metadata filter), SP-S6
(dedup domain predicate), SP-C3+SP-C2 (standby operator pin), SP-W9 (ping
scoping), SP-W10/W11 (lineage + at-least-once truth), SP-F2/F3/F4 fork
surfaces ride the next fork-sync.

**v1.28.79 "Ceremony"** — operator-ceremony hardening: SP-C4 (rotate atomic
0600 + epoch desync), SP-C6 (migration naming), SP-A2 (token-demotion ack),
SP-A5 (session chain ids), SP-A6 (auth-path migration ordering), SP-D2
(reg_watch exhaustiveness floor), SP-I1/I2/I3 (hostcall streaming, SSE
re-check, capability-token asymmetry doc).

---

## 5. What the second pass did NOT do (`ponytail:`)

No dynamic testing or exploit development against the live deployment (the
strip heals were demonstrated against compiled PORTS, then fixed in tree);
no fuzzing beyond the nightly corpus; no dependency CVE trawl beyond config
review; no `dist/`-vs-src integrity diff on the fork; no `acpx` review; no
host-level re-audit; the 1.32.x Loop-line plans were read only for
precondition checks. The fork's SP-F2/F3/F4/F6 are AUDITED but not merged —
they are fork commits, deliberately not brain-server releases. `repo-brief.sh`'s
stale-marker list remains the quick probe it was; a deeper docs-crawler is
YAGNI until the next drift incident.

## 6. What is genuinely strong (verified this pass, keep)

The no-SQL-in-handlers guard caught THREE violations from this very fix pass
within one test run — the house gates police their own authors. Fail-closed
direction discipline held everywhere: verdicts only tighten, refusals only
deny, digests fail closed (409) rather than bless modified content. The
parity-fixture pattern (brain↔plugin invisible sets) is exactly right and
caught nothing this pass because it was exactly built. The openclaw SSRF
library, the audit chain's verify-before-prune, the probe-blind revocation
ordering, and the channel digest double-gate all survived adversarial
re-verification untouched.
