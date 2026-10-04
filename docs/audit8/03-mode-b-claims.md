# §4 — MODE B: CLAIMS → FALSIFICATION

Every documented control claim attacked. **Verdicts: HELD (with the attack that failed) /
WEAKENED / FALSE / UNVERIFIABLE.** Nothing is left as "probably fine".

---

## 4.1 The anti-vacuity sweep — the highest-value task, and the result is a *pass*

I set out to find vacuous pins, hunting the `exec_spawn_carries_kill_on_drop` shape (a pin whose
target string occurred only inside the assertion). Method: extract every `.contains("…")` literal
used in tests, cross-reference against all other `.rs` files to find self-referential needles,
then ask of each: *if I deleted the production code it guards, would this still pass?*

**Zero genuinely vacuous pins found.** The suspicious shapes each turned out to be defended:

| Pin | Vacuous *shape* | Why it survives |
|---|---|---|
| `soft_handoff_threshold_is_not_decorative` (`main_suite.rs:19705`) | asserts a symbol is **absent** — the exact precedent shape | Defended three ways: comment-stripped absence, raw-text absence, **and** a positive assert that the replacement `fn soft_handoff_latch_fires(` exists. Its own doc records that an absence pin here has failed twice. |
| `r46_the_create_loop_boundary_is_now_owned_by_its_own_suite` (`evidence_byte_range_pins.rs:891`) | final `assert!` is trivially true given the preceding `find_map` | **WEAKENED, not FALSE.** The last assertion is redundant, but the load-bearing work is the `find_map` + `panic!`, which *would* fail if the owner pin were deleted. |
| `the_rounds_own_pins_live_under_the_floor_walk` (`drift_census_pins.rs:1070`) | asserts its own test names live in exactly one file | Self-referential by construction, but it guards a **real** property — that the `CRATE_TEST_FLOOR` walk can see the round's pins — and fails if the pins move to `tools/`. |
| `sql_statement_counter_still_fires` (`service/mod.rs:221`) | a counter self-pin | **Exemplary** — ten cases including the negative (`"selected rows"` → 0) and the `(`-is-not-a-boundary case. |
| `handler_body_ignores_comments_naming_the_symbol` (`main_suite.rs:8359`) | | A true red-proof with planted comment/string/raw-string/char-literal hazards. |
| `hostile_elements_fixture_pins_server_set` (`gate.rs:1840`) | | Two-way fixture↔code pin plus 3 probes × 30 MathML names through the real `sanitize_read`. |
| Read-seam site table (`main_suite.rs:8003`) | a *regression lock*, not a detector | Honest in its own doc: *"a REGRESSION LOCK for known sites, NOT a detector for new ones."* It `panic!`s if a named function disappears, so rows cannot silently rot. |

**One vacuity-adjacent weakness found:** `client/src/main.rs:2738` `xss_escape_hatch_is_unused`
walks `walk_dir("src")` with `if let Ok(…)` — a `read_dir` failure yields an **empty list and a
green test.** No anti-vacuity floor (contrast `write_discipline.rs:85,90`, which carries
`MIN_SCANNED_LINES = 20_000` / `MIN_SCANNED_FILES = 100`). It runs in CI so it is currently
non-vacuous in practice, but it **fails open** if the path ever resolves wrong.

**This is the audit's most important positive result and I want it stated plainly: the
anti-vacuity discipline in this repo works.** The lessons were learned and institutionalised.

---

## 4.2 Falsification table

| # | Claim (file:line) | Enforcing check | My attack | Verdict |
|---|---|---|---|---|
| 1 | Badge test count "3120 passed" | `docs_truth.rs:112` checks prefix only; `badges.sh:42-51` declines to verify | I ran the full suite: **3122 passed** | **WEAKENED** — number off by 2; no guard would catch it |
| 2 | "badges derive values, never hand-typed" (`badges.sh:10-12`) | `--selfcheck` | selfcheck verifies version + disclaimer + SBOM; for the count it greps only for the string `"not selfcheck-verified"`. Change the number → green. | **WEAKENED** (disclosed at `README.md:62`; the header comment overclaims) |
| 3 | `AGENTS.md:1418` "2,818 passed at HEAD 7001e478" | none | Measured **3122** at `e9c71919`. Figure is **304 stale** and names a commit many releases back. | **FALSE** |
| 4 | `CRATE_TEST_FLOOR=2758` | `spire_inventory_freezes_the_thin_binary` | Ran: `crate tests 2904≥2758 · coverage rows 217≥214 · authz rows 203≥200 · router routes 258≥255 · main 124≤300` | **HELD** (but see F8-05: gameable) |
| 5 | Read seam strips 26 hostile elements + attribute tier (`THREAT_MODEL.md:368`) | `hostile_elements_fixture_pins_server_set` (`gate.rs:1840`), `recall_hits_carry_no_event_handlers_or_dangerous_schemes` (`:1489`) | Compiled the verbatim strip into a probe; ran 54 payloads incl. welds at n=300 | **HELD on the element tier** |
| 6 | Attribute tier is "scheme-hostile, not attribute-hostile" (`gate.rs:388-391`) | `attr_is_hostile` (`gate.rs:688`) allowlists 7 URL attrs | `<a on:click=…>`, `<a @click=…>`, `<a v-on:click=…>`, `<a (click)=…>` all survive **byte-identical** | **WEAKENED** — the `on[a-z]+` name rule (`:696`) cannot match `on:`, `@`, `v-on:`, `(click)` |
| 7 | `<details>/<animate>/<math>/<svg>` covered | `HOSTILE_ELEMENTS` (`gate.rs:430`) | All four die; `math`/`style` opaque-swallow inner content | **HELD** — the gap the brief suspected does **not** exist |
| 8 | Provenance marks on 4 artifact classes | `provenance_marks_present_on_all_four_classes` (`provenance.rs:415`) | Read the pin: real `OperatorKey`, real DB fixtures, `verify_artifact_detailed(…, Some(&operator_did))` per class; 4 real `attach_aigen` sites (`workflow.rs:1761,2066,2119`, `kb.rs:837`) | **HELD** — behavioural, not string-matching |
| 9 | Digest-bound approval, 409 on drift (`THREAT_MODEL.md:81`) | `gate.rs:675-686` | Missing → 400 `digest_required`; mismatch → 409, **before** any decision CAS. **Drill-confirmed live** (§7) | **HELD** |
| 10 | Approvals replay-safe across transports | `gate.rs:1166-1178` `moved:false` | `cas_proposal_approved` in-tx; `moved==0` → rollback + receipt. **Drill-confirmed**: replay returned 404, never double-applied | **HELD** |
| 11 | Revocation reaches every entry point | `auth.rs:301,317,452`, `sse_reauth.rs:56-69` | `jti+iss` check in the shared auth path (covers `/auth/refresh`), principal check, and **mid-stream** SSE re-check | **HELD** |
| 12 | Token never in URL/logs (`THREAT_MODEL.md:95`) | none | `grep -rn "token="` → only 2 hits, both **negative egress fixtures** | **HELD** |
| 13 | `alg:none`/HS*/PS* rejected | `none_algorithm_rejected`, `hs256_rejected_even_with_matching_key` | `ALLOWED_ALGS` checked **before** key lookup | **HELD** |
| 14 | Audit chain detects tampering, not host compromise | — | Attempted escalation: none available. The ceiling is stated precisely, including `.bak` plaintext and shared-host key | **HELD (honest ceiling)** |
| 15 | CRA Art 14 clocks correct | `reg_watch_runbook_clock_anchor` (`reg_watch.rs:180`) | Ran the pin: `ok`. Both clocks present with trigger labels | **HELD as a pin** — but see `L8-04`: the *runbook's channel* is unfillable |
| 16 | `check-doc-links.py` catches broken links | the script | Scans **only** `docs/**.md`; 19 root `.md` files with 46 links are out of scope | **WEAKENED** — scope never disclosed |
| 17 | `docs_truth.rs` catches doc drift | `readme_badges_and_openapi_version_are_derived_not_hand_typed` | No `EXEMPT`/`SKIP` constants exist; it `include_str!`s specific files and derives from `Cargo.toml` | **HELD — not vacuous** |
| 18 | SQLCipher/KMS at rest | none | Zero `sqlcipher` in `Cargo.toml`; `rusqlite` features unencrypted | **HELD** — honestly marked 🚧 |
| 19 | OWASP Agentic matrix (`docs/OWASP_AGENTIC_2026.md`) | **NONE** | `grep -rn "OWASP_AGENTIC\|owasp_agentic" src/ tests/ scripts/ .github/` → **0 hits** | **UNVERIFIABLE** — 20 rows, zero machine checks |
| 20 | AUDIT.md G3/G6/G7 dispositions (`docs/AUDIT.md:22,25`) | none | Both cite `IMPLEMENTATION_PLAN_v1.11.0_HippoRAG.md` — **file does not exist** (moved to the private repo per AGENTS.md). I confirmed with `ls` | **FALSE** — dead reference, invisible to the link checker |
| 21 | US_STATE_MAP "Component live" column | none | Scoped to evidence *primitives*; operator duties in a separate column; every date carries a source | **HELD** in structure — but see `L8-01` |
| 22 | OpenAPI exclusions are "intentional" (`release-checklist.md:84`) | none | 8 routes absent from `openapi.yaml`; `docs-truth.sh` reports them **LOW** and exits 0 | **WEAKENED** — documented, never enforced |
| 23 | `risk-register.md` update rule ("close a row only when pinned") | none | All 18 rows read honest; R-11/R-02 residuals match THREAT_MODEL | **HELD** |
| 24 | Four mantras (§8 of the brief) | mixed | Read seam ✅ drill-verified · approve ✅ drill-verified · erase ⚠️ **partially falsified** (F8-08) · "no autonomous anything" ✅ (`PROMOTION_ENABLED` is a compile-time `false` checked first) | **3.5 / 4 HELD** |

---

## 4.3 The three FALSE claims, prominently

**R8-01 — `AGENTS.md:1418` "2,818 passed at HEAD 7001e478".** Measured **3122** at `e9c71919`.
304 stale. This is the sixth instance of the pattern AGENTS.md's own header documents as having
occurred five times — *"a number shipped without anyone diffing it against a measurement."*

**R8-02 — `docs/AUDIT.md:22,25`.** Findings G3 and G6/G7 are dispositioned *"Carried to v2.0
(see sweep table in `IMPLEMENTATION_PLAN_v1.11.0_HippoRAG.md`)"*. That file does not exist —
moved to the private `brain-steward-ip` repo. **An auditor following the register finds nothing.**
`check-doc-links.py` cannot catch it: the reference is bare text in a table cell, not a markdown
link, and the checker only walks `docs/`.

**R8-03 — `scripts/badges.sh:10-12`** — *"It never fabricates a number it did not measure."*
True of the **generation** path. The `--selfcheck` path it advertises as verification (`:6-8`,
"exits nonzero on any drift") **cannot detect test-count drift at all** — it greps only for the
presence of the string `"not selfcheck-verified"` (`:48`). Change 3122 → 9999 and selfcheck stays green.

---