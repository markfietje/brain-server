# Agent Execution Log — brain-server

> Current release (unreleased): **R70 "Seams"** — the **third** of the five
> `docs/audit8/08-design-grid-remediation-gates.md` §9.3 releases. Theme:
> **the cheap enforcement wins — six seams where the machine was right for the
> wrong reason, or right by luck.** Findings **F8-03** (HIGH), **F8-04**,
> **F8-06** (MEDIUM), **F8-07**, **F8-09**, **F8-10** (LOW) from
> [`docs/audit8/`](docs/audit8/README.md). **No authz change**:
> `git diff src/authz/` is **empty**, no new route, no wire field, no new
> dependency edge, no schema change (**1.32.26** unchanged).
>
> **(1) Every §1 premise was re-measured, and TWO of the round's own claims were
> wrong — which is the round's first finding.** All six §1 figures matched (raw
> needle **2 957**, stripped **2 941**, lib **2 319**, 52 handler files, schema
> **1.32.26**), and F8-03's `VACUUM` half is confirmed **already closed by R68**
> (`domains.rs:266` is `if let Err(e) = …`, not `let _ =`) — so it was **not
> re-fixed**, exactly as the note warned. But (a) the prompt's suggested reuse of
> `spire_inventory::strip_rust_comments` is **IMPOSSIBLE and was not attempted**:
> it is `pub fn`, but `spire_inventory` is `#[cfg(test)] pub mod`
> (`src/lib.rs:346`), so it does not exist in the lib an integration test links
> against — the `cfg` is the blocker, not visibility. The F8-04 pin therefore
> lives in `tests/main_suite.rs` and reuses the two existing **test-side** house
> lexers (`strip_line_comments` / `strip_cfg_test_regions`); **no second `src/`
> stripper was written and `dup_guard` is untouched**. And (b) **F8-09's
> reachability claim was wrong in the direction that matters**: the note predicted
> the row-mapping arm unreachable because `TEXT` affinity coerces every storage
> class, and that is true for `INTEGER` and `REAL` but **FALSE for `BLOB`**
> (verified against SQLite directly). So the honest **behavioural** pin was
> available after all. Had the premise been carried — or the note's suggested
> `42`/`1.5` fixtures used, which coerce to `'42'` — the pin would have been
> **green before the fix** while proving the arm that was not changed. It now
> asserts `typeof(roster_json) == 'blob'` as a **precondition**.
>
> **(2) F8-04: the log seam is unskippable, and the guard found THREE offenders
> the audit did not name.** `sanitize_log_value` had **one** production call site
> and fourteen tests, **none** asserting any call site USES it — a seam nothing
> forces through is a convention. The guard found **eight** request/config-derived
> sites before any was fixed, across five files: `recall.rs` `{domain}`,
> `domains.rs` `{name}`, `webhooks.rs` ×2, `mod.rs` `error = %message`,
> `observe.rs` `{url}`, `ump_ops.rs` `owner`/`declared`. The fix is a `LogValue`
> newtype beside the seam whose **only** constructor is `sanitize_log_value`: no
> `From<&str>`, no `From<String>`, no `Deref`, no `Default`, private field — each
> pinned, because any one re-opens the hole. The scan reads **both** value-carrying
> syntaxes (`{ident}` placeholders **and** `%ident`/`?ident` structured fields),
> because the `webhooks.rs` offender is the field form and a placeholder-only scan
> would have passed it; multi-line invocations are scanned whole. The remaining 31
> sites are exempt **by category**, justified in code, with the integer-id set a
> **closed list rather than a shape** — a shape rule would have exempted exactly
> the request-derived names (`name`, `domain`, `path`, `url`, `owner`).
>
> **(3) F8-06: the prefix rule is gone, and the pin caught a fail-open
> regression this round nearly shipped.** `path.starts_with("/webhooks/")`
> exempted whatever landed under `/webhooks/`, including any future route — not a
> live hole (all six verify and fail closed) and precisely an **unenforced
> convention**. Replaced with `WEBHOOK_PATHS` naming all six. **The three
> `is_public_path` call sites DISAGREE:** `auth.rs:129` passes axum's
> `MatchedPath` (the TEMPLATE, `/webhooks/{kind}`) while `:277`/`:549` pass
> `req.uri().path()` (the CONCRETE path, `/webhooks/github`). A `contains()` on
> the template list would have exempted the template and **refused every real
> request — silently disabling all six webhooks.** `is_webhook_path` matches
> segment-wise instead. The pin then caught **two fail-open bugs in the first
> draft**: `split('/')` on `{kind}` never equals the literal `"{kind}"`, and a
> stale list entry would keep exempting a path nothing serves — so both
> directions are checked against the **router** (the authority), never the list
> against itself.
>
> **(4) F8-03 (the surviving half): the deadline moves INSIDE the closure.**
> `TimeoutLayer` drops the handler future at 30 s, but a `spawn_blocking`
> closure is **not cancellable** — it runs to completion and **COMMITS**, so the
> client sees a 408 while the row lands anyway and a retry double-commits. A
> check *outside* the closure is **decorative**: the work is already queued and
> nothing can call it back. `src/service/write_deadline.rs` reads the clock **at
> the moment work starts** and refuses before any statement runs, on
> `DELETE /domains/{name}` — the gate is the closure's **first** statement,
> before `pool.get()`, so a refusal provably took no connection and opened no
> transaction. The 30 s is now `config::REQUEST_TIMEOUT_SECS` with
> `WRITE_DEADLINE_MARGIN_SECS` held back, so handler and middleware cannot drift.
>
> **(5) F8-10 and F8-07, both measured rather than assumed.** `BIND_PORT` was
> `.parse().unwrap_or(8765)`: a typo bound **the production port** with no
> diagnostic. It now reuses the `WRITE_POSTURE` shape (absent = default; only
> present-and-invalid refuses; empty = unset), so **no deployment changes
> behaviour**. A throwaway probe (since deleted) established what `u16::from_str`
> actually does: `abc`/`65536`/`-1`/`""` all fail to parse, and **`0` parses
> successfully** — so a parse-only fix would **not** have closed the finding,
> since port 0 binds a kernel-chosen ephemeral port that changes every restart.
> `876` is deliberately **not** a refusal (a valid `u16`; refusing every
> "surprising" number would invent policy the audit did not ask for). F8-07's
> harder half: `to_ipv4_mapped()` unwraps **only** `::ffff:0:0/96` (confirmed
> against the std source — bytes 10..12 == `0xff,0xff`), **not** the
> IPv4-**compatible** `::/96`, so `::169.254.169.254` was **ADMITTED** while the
> v4 table sat fully present and never consulted. **Normalised, not "add a
> row"** — a row refuses the `::/96` block; normalisation subjects the embedded
> v4 to the **whole** v4 table and names the real reason. `::` and `::1` are
> deliberately not embeddings.
>
> **Spire at ship:** lib **2 325** passed / 0 failed / 2 ignored (baseline 2 319,
> **+6**); full suite **green**; `crates/` green; harness green; `cargo fmt`
> clean; clippy clean on **bench, default, otel, `crates/`** and **all six**
> feature lanes; `lipstyk-gate` **0 findings**; `badges.sh --selfcheck` clean;
> `env-truth.sh` clean; `docs-truth.sh` **LOW=17 (pre-existing, unmoved)**;
> `check-doc-links.py` clean (404 links); **`cargo audit` clean** (514 deps);
> shell gate **82 passed / 18 files** incl. the drift gate, `tsc` clean. `main.rs`
> 124≤300, router routes 258≥255, coverage 217≥214, authz rows 203≥200. **The
> floor was NOT raised: `CRATE_TEST_FLOOR` is unchanged at `2 758`** (measured
> **2 954** stripped — headroom 183 → **196**; raw **2 970**). Raw and stripped
> moved by the **same +13**, so this round added **no** fixture-string inflation
> (the raw−stripped gap is unchanged at 16 and belongs to the baseline). **Zero
> new dependency edges: all `Cargo.lock` files byte-identical**; `src/authz/`
> **0 diff**; `openapi.yaml` and `shell/src/lib/api/schema.d.ts` **0 diff** this
> round so no regeneration was owed; **no new `OPENAPI_ROUTES`/`PUBLIC_PATHS`
> row**. R69's `no_sql_in_handlers_enforced` green.
>
> **Red-first: all eight pins were proven ABLE TO FAIL**, and **two caught real
> defects in this round's own first draft** — which is what writing them was for.
> The load-bearing one is **#8**: keeping the F8-03 gate but moving it one line
> down, after `pool.get()`, is exactly the "machine checks under-delivered" shape
> and **a presence-only guard would have passed it**. #4/#5 caught the `::/96`
> normalisation and the two v4 rows (reverting each admits the address). #7 caught
> `.flatten()` returning `Ok(SweepReport { crew_rows: 0, .. })` where a refusal was
> required. One house gate fired on this round's own code
> (`comments_never_reference_versions_plans_audit_ids` rejected the finding labels
> in fifteen source comments) and was fixed at the root — labels dropped,
> reasoning kept.
>
> **What did NOT ship, stated plainly.** **Not** the write **idempotency/
> receipt registry** or the per-route openapi ceiling annotations — a wire
> contract and a new table, the next decision. **Named residual:** a write that
> *starts* within budget and is then killed **mid-commit** (crash, not timeout)
> is still uncovered. **Not** F8-03's `VACUUM` half (**R68 closed it**). **Not**
> every DB-touching handler — the deadline lands on the named route plus the
> shared helper, and the ~50 unreached `spawn_blocking` write handlers in
> `src/handlers/**` are **named as a residual, not silently skipped**. **Not**
> F8-01 (R69). **Not** K8-* (**R71**, a different repository; K8-04 needs a
> *decision*). Not R8-01/02/03, S8-11, L8-01/05/06/07, P8-01, K8-15 (**R72**).
> **No runtime authorization change.** **No migration is added, so this round has
> no irreversible risk.**
>
> Predecessor: **R69 "Erasure"** — the second of the five §9.3 releases. Theme:
> **a compliance certificate can certify an erasure that did not happen** — the
> DSAR erasure deleted the memories it could attribute to a subject and left the
> approved proposals behind them, because its only reach into `proposals` was a
> literal `content LIKE '%subject%'` and a candidate body almost never contains
> its owner's identity. Finding **F8-08** (HIGH, drill-proven) from
> [`docs/audit8/`](docs/audit8/README.md). **No authz change**:
> `git diff src/authz/` is **empty**, no new route, no wire field, no new
> dependency edge.
>
> **(1) The §9.3 plan's prescribed fix was IMPOSSIBLE as written, and the tree
> won.** The plan said *"carry the approved chunk ids the erasure just deleted
> and delete their proposals by `id IN (…)` — the proposal that produced a
> memory is reachable from the memory."* Reproduced by hand at `1c00c83a`,
> **there is no id to carry**: `knowledge` carries no proposal ref (base
> `CREATE TABLE` + every `ALTER TABLE knowledge ADD COLUMN`); neither
> `promote_chunk_insert` (`service/gate.rs:186-208`) nor `kcs_draft_insert`
> (`:87-116`) binds one; `cas_proposal_approved` (`service/review.rs`) records
> none; no linking table exists; and the two tables share no hash column
> (`proposals` has **no** `content_hash`). Option (a), the audit chain, was
> **measured** closed first: `audit_events` has no later `ALTER`, and both
> `target_hash` and `detail_hash` are SHA-256 digests — **a hash is not
> reversible**, so the raw proposal id cannot be recovered from the chain.
>
> **(2) So the fix is a schema migration, and it contradicts the audit's "no
> migration is needed" note — which was right about that reasoning and wrong
> about the schema.** `proposals.promoted_chunk_id INTEGER`, schema **1.32.25 →
> 1.32.26**: additive, NULLable, `pragma_table_info`-guarded. **The proposal is
> the row extended; `knowledge` gains nothing**, so the FK-children maps in
> `service/dsar.rs:25-44` and `service/purge.rs:10`/`:29` stay accurate — and the
> migration comment says so explicitly, because a reader will look for a map
> entry that legitimately does not exist.
>
> **(3) The edge is a SEPARATE write beside the CAS, not a CAS parameter.**
> `cas_proposal_approved` has **seven** call sites and four promote nothing, so a
> NULL `promoted_chunk_id` is the correct recorded state there. `record_promoted_chunk`
> rides the caller's approve tx. Wired into the two arms that create a memory:
> the generic promote, and the KCS draft — which deliberately records **no** edge
> for `KIND_LINK_ONLY`, because that arm reuses an existing article and a
> fabricated edge is exactly the kind of lie this round exists to remove.
>
> **(4) The erasure arm erases those proposals by id, in the caller's tx, after
> the knowledge purge** so it walks the chunks genuinely deleted. The
> `content LIKE` arm is **KEPT**: removing it would *reduce* erasure coverage for
> subjects whose text genuinely appears in a proposal. The IN-list is chunked at
> **900**, under a **measured** ceiling — this crate's bundled SQLite prepares
> 32,766 bound params and refuses 32,767 ("too many SQL variables"), measured
> with a throwaway probe that was then deleted. The count goes to `tracing`,
> **not** the certificate JSON: §2 keeps the round off the wire, and a
> certificate field nothing consumes is a field no one verifies.
>
> **(5) Red-first, and red TWICE.** The §3 pin failed before the fix
> (`left: 1, right: 0`, the proposal still present, with both anti-vacuity
> assertions above it passing so the fixture was known-real), and the red-proof
> was **re-run on the finished fixture** by disabling the arm — which failed
> identically. Six further tests from §4.4, and **four planted mutants were all
> killed** (wrong column, chunking removed, swallowed delete error, arm
> disabled). **Two tests could not fail** under the first mutant run and were
> **rewritten rather than kept**: their fixtures did not collide proposal ids
> with purged chunk ids, so an arm keyed on the wrong column passed them. The
> chunking test needed the same treatment — at 925 ids it passed with the
> chunking removed, so its fixture now spans the measured 32,766 ceiling.
>
> **Spire at ship:** lib **2 319** passed / 0 failed / 2 ignored (baseline 2 310,
> **+7**); `main_suite` **329**; `crates/` **308**; harness **44**; default
> all-targets **3 013**; clippy clean on bench, default, otel, `crates/` and
> **all six** feature lanes; `cargo fmt --check` clean; lipstyk **0 findings**;
> `badges.sh --selfcheck` clean; `env-truth.sh` clean; `docs-truth.sh` LOW=**17**
> (pre-existing); `check-doc-links.py` clean (404 links); **`cargo audit` clean**
> (514 deps); `main.rs` 124≤300, router routes 258≥255, coverage 217≥214, authz
> rows 203≥200. **The floor was NOT raised: `CRATE_TEST_FLOOR` is unchanged at
> `2 758`** (measured **2 941** stripped, headroom 137 → **183**; raw needle
> 2 957, and the raw−stripped delta is this round's fixture strings, disclosed
> here rather than absorbed by re-baselining). **Zero new dependency edges: all
> `Cargo.lock` files byte-identical**; `openapi.yaml`, `route_guards.rs` and
> `src/authz/` all **0 diff**. R68's `no_sql_in_handlers_enforced` still green.
>
> **Known pre-existing, NOT fixed here.** `tests/no_engagement_name.rs` fails —
> **re-verified at the baseline this round** by stashing the entire diff and
> re-running it there, where it fails identically; it is the only red in the
> suite and it is not R69's. One **intermittent** flake appeared during
> validation and is **not** R69's: `handlers::webhooks::inbound_signal_becomes_
> screened_steering` sets a process-global env var without taking an env lock
> (the class `config.rs` guards with `TOKEN_ENV_LOCK`/`IP_ENV_LOCK`), raced once
> across three runs, and passed on three subsequent full-suite runs; R69's diff
> does not touch that file. `shell/src/lib/api/schema.d.ts` is stale against
> `openapi.yaml` (§6.1 of the prompt) — which is precisely why R69 adds no wire
> field. `AGENTS.md`'s own header figures disagree with measurement (that is
> `R8-01`, assigned to **R72**, and one theme per release means it is **not**
> re-baselined here).
>
> **§7 note — both "known pre-existing" items above were closed AFTER R69's own
> scope, by a separate follow-up. Stated here so the header does not outlive its
> debunking.** (1) `tests/no_engagement_name.rs` was **not** a content leak: the
> pin scans git-TRACKED files and flagged **itself**, on the two `NAMES`
> literals it must hold to police the vocabulary. That is a structural
> false-positive, and it made the suite's only red permanently unreadable as
> "pre-existing noise". Fixed by exempting the pin's own file — **named** in
> `ALLOWED_FILES`, never a blanket skip — plus an anti-vacuity assertion that
> fails if the vocabulary ever leaves the file, so the exemption cannot rot
> into a silent pass. **Proven non-vacuous:** planting the name in
> `src/storage_layout.rs` makes the pin fail. (2) The webhooks flake is closed
> by a `tokio::sync::Mutex` fence around `BRAIN_SIGNAL_WEBHOOK_SECRET_FILE`,
> with **two** deterministic red-proof tests — the natural race fired ~1 run in
> several, which is not evidence a fence works. Measured: **20/20 green** with
> the fence, **20/20 red** with it bypassed, and the mutant still passes the
> original signal test. `tokio::sync::Mutex` rather than `std::sync::Mutex`
> because the guard is held across `.await` (clippy's `await_holding_lock`
> correctly refuses the `std` form), and **non-reentrant/FIFO** — an early
> version acquired it twice in one test and deadlocked.
>
> **§0 note — the prompt's baseline was stale and was re-verified rather than
> carried.** The prompt pins schema **1.32.24**; measured at `1c00c83a` it is
> **1.32.25**, and the prompt's `is_newer_than_known(Some("1.32.26"))` probe was
> *already in the tree* — i.e. the prompt was written against the 1.32.24
> ceiling and the tree had moved twice since. Every other §1 figure was
> re-measured and matched: floors `2 758` / 255 / 214 / 200, 52 handler files,
> raw needle **2 954**, stripped needle **2 934**, headroom **137**.
>
> **The largest residual, named.** Historical approved proposals keep a NULL
> edge and are **not** retro-linked: a proposal approved before this release
> promoted a memory that may be purged tomorrow, and the correspondence was
> never stored, so the erasure reaches it only if the subject's string happens to
> appear in its body. Backfilling by inferring the edge from `content` would be
> the substring match this round exists to stop trusting.
>
> **What did NOT ship, stated plainly.** **Not** the §9.3 fix as specified (it is
> impossible). **Not** a certificate wire field (blocked by the shell drift).
> **Not** an owner column on `proposals` — declined by the audit, and the edge
> belongs on the proposal→chunk correspondence, not on a subject attribution
> proposals do not carry. **Not** F8-03/04/06/07/09/10 (**R70**), K8-* (**R71**,
> the openclaw fork), R8-01/02/03, S8-11, L8-*, P8-01, K8-15 (**R72**).
> **No runtime authorization change.**
>
> Predecessor: **R68 "Silence"** — the first of the five §9.3 releases. Theme:
> **the machine checks under-delivered** — three guards/pins passed while their
> subject was violated, or asserted a property they could not fail. Findings
> **F8-01** (HIGH), **F8-02** (HIGH), **F8-05** (MEDIUM) from
> [`docs/audit8/`](docs/audit8/README.md). **No runtime authorization behaviour
> changes**: `git diff src/authz/policy.rs` is **empty**, and the authz half is
> prose plus pins.
>
> **(1) The Architecture Law's SQL guard was blind, and ten production
> violations were live under it.** `count_sql_statements` recognised exactly
> four keyword openers (`select` / `insert` / `update` / `delete … from`) and
> was structurally blind to `PRAGMA`, `VACUUM`, `BEGIN`/`COMMIT`/`ROLLBACK`,
> `REPLACE INTO`, and the whole rusqlite method surface. The guard reported
> `ok` the entire time. **RED-FIRST, recorded:** extending the guard *before*
> migrating anything made it fail naming exactly **10** violations across four
> files — `domains.rs` ×2, `govern.rs` ×4, `shifts.rs` ×3, `ump_ops.rs` ×1 —
> matching the audit's census (which named 2; a full scan found 10). The audit's
> suggested fix — deny any direct rusqlite surface — is **refused as stated**:
> a keyword extension over comment-stripped handler source yields **25**
> production hits of which only **10** are real and **15** are ordinary Rust
> method calls (`h.update(`, `policy.insert(`, `headers_mut().insert(`). A
> guard that false-fires 15 times per run gets deleted. **So the counter is
> STRUCTURAL — it matches CALL SHAPES including the `(`.** That is what makes
> it both sensitive and silent-on-clean-Rust, and it needs **no comment
> stripper** to be correct (comment-stripping changes the DIRECT hit count in
> `src/handlers` by exactly **0**: raw 19 == stripped 19), so the deliberate
> "comment residue counts too" self-pin on the keyword counter is untouched and
> `count_sql_statements` is **byte-identical**. All ten sites now live in cores:
> `domains_admin::file_domain_counts_at`, the post-delete `VACUUM`,
> `ump_ops::record_forbidden_scope_at_db`, and the new
> `service/snapshot_probe`. The `#[cfg(test)]` exemption is **structural-counter
> only**, and test regions remain held to the keyword counter.
>
> **(2) The highest-value fix is one the guard found by accident: `shifts`'**
> **hand-rolled transaction.** `BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK` became
> the RAII `WorkflowTx`, closing **three** defects in one move — the discarded
> ROLLBACK error (`let _ =`, forbidden by the fail-closed law), a **panic
> unwinding past the rollback and returning an OPEN TRANSACTION to the pool**,
> and the lost `note_busy_error` contention telemetry the raw `execute_batch`
> bypassed. The `VACUUM` migration closed a fourth instance of the same law:
> `let _ = conn.execute_batch("VACUUM;")` is now a `tracing::warn!`.
>
> **(3) A security improvement, disclosed.** The snapshot probe's own doc
> comment called it "Read-only — it never creates or mutates a snapshot", but
> `Connection::open` does **not** set `SQLITE_OPEN_READ_ONLY`, so it opened
> every `.bak` read-**WRITE**. The new core opens `SQLITE_OPEN_READ_ONLY |
> SQLITE_OPEN_URI`, so a probe can no longer alter the evidence it reports on.
> The round's §9 named this as the only irreversible risk ("if `integrity_check`
> refuses a read-only handle, revert that one line") — **measured, it does
> not**: the new pins prove a real fixture passes `integrity_check` on the
> read-only handle, so no revert was needed.
>
> **(4) `CRATE_TEST_FLOOR` is no longer gameable, and was NOT re-baselined.**
> Ten `#[test]` written inside a doc comment satisfied the floor. The needle now
> strips comments first, via a **string-aware** stripper (`strip_rust_comments`):
> `//`, **nested** `/* */`, `"…"` with escapes, raw strings at ANY `#` count
> (`r"…"`, `r#"…"#`, `br##"…"##`) with or without a `b`/`c` prefix, and `'"'` as
> the char literal it is. **Every defect in the naive form pushed the count
> DOWNWARD** — a broken stripper looks *safe* — so the pin asserts BOTH
> directions (prose removed AND real code surviving), and the anti-vacuity case
> is a fixture that must count **1**. **RED-PROOF, recorded:** a planted
> 10-attribute doc comment moved the raw needle by **+11** and the stripped
> needle by **+0**; reverted. **The floor stays `2 758`** — headroom was 137,
> so raising it would have spent the guard's budget on a measurement.
>
> **(5) The second-order hazard was worse than the round claimed.** The prompt
> said `tests/rbac_evaluation_pins.rs:802-810` re-implemented the *unstripped*
> needle and reported 2 908 against the spire pin's 2 895 — a 24-unit
> disagreement. True, and it is worse: that file **contains the literal
> `"#[test]"` it counts**, so the counter inflated the number it validated.
> The needle is now built from parts. **The prompt's preferred fix (option a,
> "make the spire helper `pub`") is IMPOSSIBLE and was not attempted.**
> `spire_inventory` is `#[cfg(test)] pub mod`, so it does not exist in the lib an
> integration test links against — visibility is not the blocker, the `cfg` is.
> A second `src/` definition would fire `dup_guard`. So the copy lives in the
> test crate and **the two are pinned to agree** (`r68_both_counters_measure_the_
> same_tree`), which is what makes the duplication defensible.
>
> **(6) F8-02: the authz prose is now true, and the pin is no longer
> self-asserting.** The middleware claimed three enforced properties; **two were
> unreachable in production** — the agent-class arm (deliberately removed; two
> matrix rows measured the agent posture and broke, see `policy.rs:205-220`) and
> the deny-only capability arm (dead because the sole production constructor
> hardcodes `required_capability: ""`, `gates.rs:167`). The
> **opposite-direction** overclaim is corrected too: the authz matrix pins
> each row to the handler's `authorize()` literal, which substantiates
> **handler-side** agreement and does **not** make the *oracle* enforce the
> action. `r47_gate_rows_read_their_declared_action` used `gate_for` as its own
> oracle — delete every enforcement use of `required_action` and it stayed
> green. It now reads its expectation from the **`AUTHZ_GATES` table literal**,
> and a new behavioural pin drives the production-shaped `Gate` through
> `decide_gate_verdict` with a real `Principal::agent_loopback()` and asserts
> the verdict is **not** a `Deny` — green today, **RED the moment someone adds
> the agent arm without updating the doc**, which is the drift F8-02 names.
>
> **Spire at ship:** lib **2 310** passed / 0 failed / 2 ignored;
> `main_suite` **329**; `crates/` green; harness green; `badges.sh --selfcheck`
> clean; `env-truth.sh` / `docs-truth.sh` (LOW=17, pre-existing) /
> `check-doc-links.py` clean; **`cargo audit` clean** (514 deps); clippy clean
> on bench, default, otel, crates and **all six feature lanes**; `lipstyk-gate`
> **0 findings**; `main.rs` 124≤300, router routes 258≥255, coverage 217≥214,
> authz rows 203≥203. **The floor was NOT raised: `CRATE_TEST_FLOOR` is
> unchanged at `2 758`** (measured **2 934** stripped — headroom 137 → **176**,
> of which +39 is this round's own fixture strings, disclosed as a ceiling in
> the CHANGELOG rather than absorbed by re-baselining; **10** real `#[test]`
> attributes added). **Zero new dependency edges: all three `Cargo.lock` files
> byte-identical**; `src/migration.rs`, `openapi.yaml`, `route_guards.rs` and
> `src/authz/policy.rs` all **0 diff**.
>
> **Known pre-existing, NOT fixed here (§6, each measured).**
> `tests/no_engagement_name.rs` fails at the baseline commit — **proven** by
> running it in a pristine worktree of `d11326c5`, where it fails identically;
> this is the only red in the suite and it is not R68's. `shell/src/lib/api/
> schema.d.ts` is stale against `openapi.yaml` (drift gate already red).
> `AGENTS.md`'s own header figures disagree with measurement (claims needle
> 3 069, measured 2 908) — that is `R8-01`, assigned to **R72**, and one theme
> per release means it is **not** re-baselined here.
>
> **§0 note — the prompt's baseline was stale and was re-verified rather than
> carried.** The prompt pins `7dc6cb47` with `M AUDIT.md` + `?? docs/audit8/`;
> actual HEAD was **`d11326c5`**, which had committed the audit8 report as its
> own docs-only commit (what §6.2 anticipated). Every §1 figure was
> **re-measured** and all matched: floors `2 758` / 255 / 214 / 200, 52 handler
> files, 8 router files, raw needle **2 908**, DIRECT hits **19** (10
> production), stripped needle **2 895**, headroom **137**.
>
> **What did NOT ship, stated plainly.** Not the `/ops/authz/explain`
> disclosure field (blocked by the shell drift above; named the next round's
> first wire item). Not end-to-end pins for the two unreachable deny reasons —
> they would assert a fiction; they are machine-pinned as ceilings instead. Not
> `ROUTER_SITES_FLOOR` hardening (measured 258 raw == 258 stripped; gameable in
> principle, no honest lever this round). Not the 16 `cfg(test)` handler sites
> (no shared test-DB helper exists; a design decision, not a move). **No
> runtime authorization change.** Not F8-08 (erasure) — that is **R69**, and it
> is the highest-severity item still open. Not F8-03/04/06/07/09/10 (**R70**),
> K8-* (**R71**, the openclaw fork), R8-01/02/03, S8-11, L8-*, P8-01, K8-15
> (**R72**).
>
> Predecessor: **R2 "Consumer"** — the replay gate gets the
> production consumer it was shipped without, and the one verification gap in
> this programme's history closes. Theme: **a gate that could never fire, wired
> to a path that can — and the fixture the measurement corrected on the way.**
> The §0 sweep (`scripts/verification-sweep.sh`, 15 lanes) **ran to completion for
> the first time ever**: **15/15 PASS, `SWEEP_EXIT=0`**, all-targets **2 989
> passed / 0 failed** across 36 binaries, `crates/` 306, harness 44. Every named
> gate had passed before at `dd5ecd95`; **no full run had ever finished**, so
> nothing carried a release sign-off. It does now. (1) **The replay-determinism
> gate is wired to the LIVE release promotion** — `releases::promote_release`,
> after `chain_defect` and before `brain_delivery_core::promote`, refusing with
> an audit `Denied` and **no state change**, and never repairing. It refuses
> because it detects, not because it is broken: an identical trace still reaches
> `allowed`. **`DenyReason` was NOT extended** (frozen crate enum) — the reason
> travels as a server-namespaced `replay_divergent` /
> `replay_insufficient_evidence` string, derived from a new `refusal_code()`
> kept deliberately separate from `refusal_reason()`. **Both red-proofs fired,
> and the second is the load-bearing one:** removing the gate promoted the
> divergent trace (`left: "allowed"`), and a planted **always-refuse** gate left
> that first proof **GREEN** — caught only by the anti-vacuity pin. Without that
> pin the round could have shipped a gate that refuses everything and proves
> nothing. (2) **Two premises the brief states measured FALSE, and both are
> refusals rather than implementations.** `classify_replay` is **not** on the
> classifier axis — it compares re-derived vs recorded **stage digests** in
> `delivery_traces` and **never calls the classifier**; `classifier_gold_set` /
> `predicted_category` appear **zero times** in `src/ crates/ tools/ client/`.
> And the promotion the brief reaches, `create::promote`, is **inert by
> construction**: `PROMOTION_ENABLED` is a compile-time `false` checked FIRST,
> and its route never even calls `promote()`. Wiring a gate there would have
> produced a green suite over a function no caller can reach. (3) **Two things
> the round found rather than assumed.** The first fixture rewrote a `seq` and
> expected one mismatch; it produced **three**, because the sort order *and* the
> digest (`canonical_bytes` includes `seq`) both moved — **the exact defect the
> gate's own fixture doc warns about**. The pin now asserts the **verdict, not
> a count**. And `InsufficientEvidence` is **unreachable at this seam**:
> emptying the traces makes `live_subject_digest` answer `no_artifact` first, so
> the artifact law precedes the gate — recorded as a **ceiling**, and
> `r2_c` asserts that fact rather than a discrimination the seam lacks.
>
> **Spire at ship:** needle-measured `src`+`tests` **3 069** `#[test]` (the
> canonical `find src tests -name "*.rs"` invocation), `crates/` **310**,
> `tools/` **100**. `CRATE_TEST_FLOOR` **2 758 in code — DELIBERATELY NOT
> RAISED**: the needle reads 3 069, so the floor was already 311 under the
> truth and raising it to the needle would have spent the guard's headroom on a
> measurement rather than on a round. The three other floors stay
> **255 / 214 / 200**. Lib **2 242** passed / 0 failed / 2 ignored
> (`--features bench`); all-targets **2 716** passed / 0 failed. Schema
> **1.32.23** unchanged (probe 1.32.24). `verify_claims.py` **53/53** — it was
> **50/53 and exiting non-zero**, from three pre-existing drifted figures in
> files R2 never touched; re-baselined to measured truth with the movement
> **attributed, not absorbed** (see below). **Zero new dependency edges: all
> three `Cargo.lock` files byte-identical**, `src/migration.rs` and
> `openapi.yaml` and `route_guards.rs` all **0 diff**. `cargo audit` clean over
> the root lockfile. Diff: `releases.rs` +336, `replay_gate.rs` +33/−6.
>
> **The debt batch (R8), landed with this line.** `chacha20` **0.10.1 → 0.10.2**
> in `tools/steward-harness/Cargo.lock` — the harness had drifted a patch behind
> the root lockfile, which already carried 0.10.2; one package, checksum-only
> delta, harness **44 passed / 0 failed**, and `cargo audit --file
> tools/steward-harness/Cargo.lock` clean over 162 dependencies. **This is the
> one `Cargo.lock` that MOVED this round**, and it moved because the brief asked
> for it, not because a new edge appeared. **And this file's own figures were
> stale:** the header said `CRATE_TEST_FLOOR` **2 618** and lib **2 818**; the
> code said **2 758** and the needle said **3 069**. The 140-line gap the brief
> flagged was real, and it was **document drift, not a typo** — every figure in
> the R59 header predated R60–R65.
>
> **What did NOT ship, stated plainly.** **Not a classifier-fidelity gate** —
> nothing in R2 reads the gold pack; that consumption is a named separate
> increment, because attaching it here would mean the round's red-proof proved a
> property of a function that does not exist. **Not a measured gate-quality
> figure** — "divergence is detectable" moves from non-claim to machine-checked
> **and no further**: no out-of-sample rate, no false-promotion rate, no owner.
> **Not R53.** `gates_vacuous` is still advisory-only and R55/R58a/R58b/R61
> remain blocked: **R2 does not unblock the critical path, and a reader who sees
> R2 land must not infer that it did.** Not an autonomy change —
> offer-never-assign stands, `refuse_agent` is untouched, and
> `PROMOTION_ENABLED` is still `false`. **No SME sign-off was performed** and no
> simulated approvals were used; **D-6 stays OPEN.** Preregistration committed
> before any code at `3f80d0c`, evidence after, code at `9fcb7adc`.
>
> Predecessor: **R59 "Gate"** — the admission-gate suite, and
> two DoD items refused. Theme: **a filter becomes a control loop — two things
> the tree already *stated* and did not *do*, and two DoD items refused rather
> than worked around.** (1) The **admission-gate suite** in `crates/gold-sets`
> (209 lines before the round; the round adds **2 428** lines across 7 files,
> **zero deletions, zero existing bytes modified**): 12 checks in the plan's
> numbering, **faithfulness executed first** (its 42.5% failure rate makes it
> the dominant property), each resolving to exactly one of mechanical /
> adversarial / **inapplicable-with-a-reason** / **negative-routes-to-a-stage**.
>
> **The real deliverable is the routing, not the vertical** — "more than the
> vertical", as the prompt put it. A negative check **names the stage that owns
> the defect** (Create/Solve/Evolve/Deflect/Operate) and the case goes THERE.
> `patch_at_gate` returns `Result<Infallible, PatchRefusal>`, so "patched at
> the gate" is **not a state the type can represent** and the SWE-Proof
> revision-routing rule is mechanical rather than advisory. That guarantee is
> **compile-time**: widening the `Ok` type breaks the build inside the pin's own
> `fn`-pointer binding, so it never produces a `test result: FAILED` line.
>
> **Inapplicable is a THIRD state, not a pass.** 3 of 12 checks
> (ContentDisposition, ReadSeam, AuditCalibration) are unreachable from this
> workspace node, each with a measured reason **and a `REOPEN` condition**. A
> case with an UNDECLARED inapplicability is refused; an admitted case **names
> its gaps in its receipt**, so **admitted never looks green**.
>
> The suite caught a defect in **its own attacker panel**, and **the pin caught
> it before a human did**: `Mutant::DropSurface` popped an element from
> `required_surface` and declared Faithfulness as its watcher, but popping a
> requirement can only make coverage look BETTER, so a coverage check is
> structurally incapable of noticing. It is now `UncoveredSurface`, which
> **widens the CLAIM**. A **declared survivor** mutant
> (`HumanVerdictContradiction`) is declared, not assumed: no mechanical check
> reads the corpus's `human_pass` verdict, so it survives — and the panel now
> *verifies* the declaration, reporting a stale one as its own negative.
>
> **What did NOT ship, stated plainly.** **`I59.4` STOPPED and `D59.5` recorded
> UNSATISFIABLE** — not "hard", unsatisfiable. All four routes to a production
> consumer for `brain-care-core` were measured closed **between the prompt's own
> §7 and §10**: the root manifest edge moves `Cargo.lock` (§7 forbids it); the
> harness edge is not production (`publish = false`, dev-dependency only, and
> §0's own table classifies harness-only as NOT consumed); the internal
> unconsumed-island edge is forbidden by name (§10); and R57's
> `the_census_reaches_the_corpus_without_a_new_dependency_edge` forbids the
> manifest edge outright. `cargo tree --invert brain-care-core` still returns a
> **bare node** — the dead vertical points at itself via `brain-interview-core`.
>
> **`D59.6` reports "insufficient corpus"; `D59.7` reports the panel
> UNCALIBRATED.** Both escapes were preregistered *for use* and used. **No check
> was shrunk to fit the corpus**, and **no threshold was invented for a
> calibration the assessment never supplied** — its only figure is the phrase "a
> third to two thirds", and **a band that is a sentence is not a measurement**.
> The claim R59 was bought for — §7's "One vertical is real", falsified by "a
> number without provenance" — **is NOT MADE**: the suite is real, the vertical
> is unwired, and **by the execution order's own falsification criterion the
> round did not buy the claim.**
>
> **The critical path did not move.** R59 does not unblock R55/R58a/R58b/R61;
> they wait on the R53 decision. `R53's halt is unchanged:` `gates_vacuous` is
> still advisory-only, and **a reader who sees R59 land must not infer that it
> did.** `R59.4` is **not** `R53`'s fork — R59.4 is about a gold case's
> assertion, R53 is about the harness's `gates_vacuous`; neither resolved,
> evaded, nor pre-empted the other.
>
> **The fifth wrong number — and this time in the handoff instrument itself.**
> The prompt's §9 baseline said lib **2 158** passed; measured **2 818**: the
> same digits transposed, **a number that was never a measurement**. That is
> the **fifth** inherited figure wrong in consecutive rounds (after `crates/`
> 281 vs 275), and sharper than the last — **the corruption is now in the
> instrument that hands the work over, not only in predecessor ship notes.** Two
> more of the round's own brief's premises measured false at the first
> measurement: the pre-existing pin count is **11, not 7** (the 7 is the frozen
> *case* count), and `lib.rs` had **5** pre-existing tests, not 4.
>
> Spire at ship: `crates/` **275 → 306**; lib **2 818** passed / 0 failed / 3
> ignored (31 binaries); `main_suite` unchanged; harness **29**, `--test gold`
> **6** unchanged; `gold-sets` **5 → 36**. `CRATE_TEST_FLOOR` **2 618 —
> DELIBERATELY NOT RAISED**: the needle `find src tests -name "*.rs"` does not
> walk `crates/`, so hand-counting this round's 31 additions would have set it
> wrong by 31. The three floors stay **255 / 214 / 200**. Schema **1.32.21**
> unchanged (probe 1.32.22). `verify_claims.py` **53/53**. Full gate **22/22** —
> including two steps that **FAILED first on the round's own new code** and were
> fixed at the root (`expect_used` denied in library code, restructured into a
> `let-else` returning the same Negative, which also removed a duplicate bounds
> check; and 42 unformatted hunks → `cargo fmt`). **Zero new dependency edges:
> all three `Cargo.lock` files byte-identical.** `cargo audit` clean over all
> three audited workspaces. All six frozen gold fixtures hash-verified
> **IDENTICAL** to `6dcfff13`. Preregistered before any code at `6182f47`,
> evidence after at `b7b829c`, shipped at `7001e478`.
>
> **Honest note on the "80-line vertical".** `brain-care-core` is 80 lines, but
> its only production decision is a **two-element array membership test**
> (`CARE_KINDS.contains`), its one production import (`DraftStore`) is held in a
> field **no production path reads or writes**, and every ambiguity/draft/repair
> call lives in `#[cfg(test)]`. With `brain-interview-core` the island is 552
> lines, but 472 of those are interview machinery this crate does not exercise
> in production either. **The behavioural figure, not the line figure, is the
> load-bearing one** — which is how the round's preregistration was amended.
>
> Predecessor: **R57b "Cite"** — decision lineage + the trace
> citation. Theme: **two things the tree already *stated* and did not *do*, and
> three plan premises that measured FALSE before a line was written.** (1) The
> **three-site lineage append**: `write_handoff_transition`,
> `write_back_referral_return` and `advance_pipeline` already emitted a
> session-store row and an audit row, and **no lineage event** — while the
> operator's timeline (`GET /workflow/runs/{id}/events`) is served by the
> `outbox`. A decision was on two of three chains and invisible on the one an
> operator reads. One helper over the EXISTING `append_lineage`, topic
> `case/decision`, refs-only payload, idempotency key `(run, kind, subject,
> outcome)`, each append inside its caller's EXISTING transaction. (2) The
> **trace citation**: three additive nullable columns on `decision_run_traces`
> using the same names `decision_evaluation_runs` already uses, written at the
> host seam from **what `resolve_for_execution` returned** — never re-derived,
> never the requested key. Schema **1.32.20 → 1.32.21**, the coupled migration
> shipped as one unit and the refuse-newer probe moved to **1.32.22**.
>
> **Three premises measured false, and the tree won each time.** (a) The plan's
> **second write seam does not exist**: `grep -c "decision_run_traces"
> src/workflow/delivery.rs` → **0**; the delivery loop writes `delivery_traces`,
> which **already** carries `model_ref` + `config_digest`. The plan conflated
> "two callers of `resolve_for_execution`" (true) with "two writers of the
> trace" (false) — **one** seam, and a pin now holds it at one. (b) `P57b.2`'s
> re-anchor claim is **partly false in its load-bearing half**:
> `anchor.rs:238` deliberately does NOT compare `schema_version`, so a migration
> cannot read as the behind-the-chain tamper class — `--verify` printed OK across
> the bump. (c) `P57b.5` predicted `main_suite`'s global `COUNT(*) FROM outbox`
> assertion survives "because the steering control is about a refused write" —
> the outcome held, the **reason did not**: those tests are per-test isolated
> (`drawbridge_state(&tmp)`), and the plan named **one** such assertion where
> the tree has **two**.
>
> **The round's own most instructive failure was authored by the round.**
> `I57b.3` first shipped `citation: Option<&ModelCitation>`, which leaves "a
> trace row cannot be written without its citation" true only of a convention;
> it is now `&ModelCitation` and `NULL` is unreachable on a new row. The pin
> proving that ALSO string-scanned the handler for `registry_row.…` — the exact
> R57 anti-pattern — failed on its first run because `cargo fmt` reflowed the
> line, and was **deleted** in favour of driving the real seam. No new route (so
> no `openapi.yaml`/authz/guard-table row is owed; the three floors stay
> **255 / 214 / 200**), **no new table**, **zero new dependency edges**,
> `Cargo.lock` byte-identical. Spire at ship: crate floor **2 613 → 2 618**
> (walk-measured), lib **2 158** passed / 0 failed, `main_suite` **329**,
> `crates/` **275** measured, `verify_claims.py` **53/53**, `cargo audit` clean
> over all three workspaces, full gate **18/18**. A sixth gap, found at the
> floor and not in the code: the predecessor's record claimed `crates/` **281**,
> and `crates/` is **byte-identical to R57's own commit** (no commit touching
> it since `0a0315f1`, no working-tree change), so the true count under the
> canonical invocation is **275** — R57's number was overstated by 6 and is a
> *record* defect, not a regression. The source-needle reads 279, so 281
> matches neither the run nor the needle. **A number shipped without anyone
> diffing it against a measurement** — the exact failure this programme's
> preregistration discipline exists to prevent, found in the record the
> discipline produced. `R53's halt is unchanged:**
> `gates_vacuous` is still advisory-only and R55/R58a/R59 are still blocked —
> **this round did not advance the critical path, and a reader who sees R57b land
> must not infer that it did.**
>
> Predecessor: **R57 "Census"** — the scoreboard, scoped down to
> what the tree can actually prove. Theme: **four of the plan's eight items
> shipped; the other four are named findings, because their premises measured
> FALSE.** (1) The **drift census**: the frozen gold corpus re-scored every run
> and diffed cell-by-cell against a committed baseline under **one** global
> tolerance (500 units, preregistered before any measurement). A cell with no
> baseline is **refused, never scored**; a breach past tolerance lands as a
> hash-chained `findings` row (`source=drift_census`, the **second** writer under
> the run-less `run_id=0` sentinel, named rather than incidental) and the verb
> exits **non-zero**. A clean pass writes **nothing** — green rows are noise and
> a breach buried in them is unread. Cadence is **externally cron-driven on
> purpose**: a shipper inside the server it measures is a correlated failure.
> (2) The **ranked gap queue** with a preregistered exploration quota, a spend
> ceiling over **cost** (never over the forecast — a budget that rewards a flood
> for promising more inverts), and a kill condition measured on the census's own
> tolerance; one item traced end to end to `promotion_disabled`, and **the refusal
> is the traceable end state** — the R50 non-claim was not weakened to make a
> trace complete. (3) `ttr` ships as an explicit **NON-CLAIM, pinned**: no
> resolution event exists anywhere, and a backfilled number is a fabrication
> wearing a measurement's name.
>
> **Four plan premises measured false, and the tree won each time:** `I57.4`
> schedules falsification for a disproof representation **that does not exist**;
> `I57.5` attributes to model versions **recorded on no row** (independently
> confirming R53a §9.4's ceiling, now load-bearing); `I57.7`/`I57.8` fall back
> to an R55 registry that **has not shipped** (blocked on R53's halt);
> `I57.1`'s "lineage keyed to the case" is **not expressible** because `claims`
> has no `run_id`. A fifth, found this session: **the gold corpus is not
> reachable from the server crate** without a new workspace path edge — so the
> census compiles the packs in by `include_str!` and **`Cargo.lock` is
> byte-identical, zero new dependency edges**. A bespoke per-cell tolerance is
> **unrepresentable**: planting one first failed to COMPILE, because `Cell` has
> nowhere to put it.
>
> Schema `1.32.20` **unchanged**; **no new route** (a CLI verb, so no openapi
> row and no guard-table row are owed), **no new table** (the existing `findings`
> table is reused, not duplicated), **zero new dependency edges**. `cargo audit`
> clean over all three audited workspaces. A prior round's pin proved itself: a
> tolerance-inheritance pin **passed with the pathology planted**, and was
> rewritten to read the type's field list instead of a hand-chosen string list.
> **No existing test was weakened and no baseline row was edited to make a gate
> pass** — four gates fired and each was fixed at the root. Spire at ship: crate
> floor **2 544 → 2 613** (walk-measured; the needle does not walk `tools/`), lib
> **2 158** passed / 0 failed, `crates/` **281** (the `gold-sets` calibration lane
> is no longer dead), `verify_claims.py` **53/53**. R53's halt is **unchanged**:
> `gates_vacuous` is still advisory-only and R55/R58a are still blocked — this
> round did not make that decision, and a reader who sees R57 land must not infer
> the critical path advanced. It did not.

> Predecessor: **R50 "Create"** — the first loop that *authors*
> knowledge, shipped **INERT**. Five phase cores, a typed claim record, and a
> four-trigger database fence. **No claim reaches durable state**: the promotion
> route returns `promotion_disabled` in every configuration for every actor,
> behind a compile-time constant with no env var and no flag. Four non-claims
> are stated in `docs/create-loop.md` in those words — the out-of-sample
> false-promotion rate is **not yet measured**; the promote route is
> **disabled**; gap generation has **no reliable published detection method**;
> the set-level control has **no published prior art** and is a declared
> approximation over declared predicate interactions. The fence keys on
> application-set strings, so it defends a compromised **model path**, not host
> compromise — the boundary the audit chain already draws. Schema `1.32.19`,
> additive only, four tables, no rebuild; the gated read is a query, never a
> view. One new **workspace path edge** (the evidence crate the gate calls and
> does not reimplement); **zero new registry edges**, so `cargo audit` over the
> root lockfile is unaffected. The R46 pin that forbade the kernel edge is
> **amended, not deleted**, and still refuses a registry edge. If a later round
> quietly gives promotion a production caller, that is a broken invariant — say
> so in the evidence file, do not quietly enable it.

> Predecessor: **v1.28.92 "Ledger"** (2026-09-22) — the governed loop's
> record layers. Theme: the 1.32.x line stamped through 1.32.7 "Diagnostic
> Closure", with two preregistered record layers riding the loop's own rows
> additively (no new table, no migration) and the System-1 decide modules
> landing pure. Fifty-four commits, six prereg-first rounds (R16–R21), every
> round hash-pinned before its first edit with evidence after; zero new
> runtime dependency edges. (1) **1.32.7 "Diagnostic Closure"**: R17
> TreeHandoff (session tree + compaction admission as harness substrate) +
> R18 triage duty (ESI/MTS acuity, monitor-only beside P-class), red-flag
> forcing function + monotonic lock + per-domain must-miss catalog (with a
> shipped 6-entry `health` table), NAM-step-6 closure gate at the single
> resolution seam, back-referral contract + overdue HITL sweep that never
> auto-resolves, I-PASS pre-fill sender-owned only. (2) **R20 disagreement
> corpus (Reflect/learn)**: closing-tx reflection derived ONLY from audited
> gate rows (free text can never mint a tuple), proven retrospective-only
> (same case twice byte-identical), DPO dual-gate export with seam
> de-identification + frozen train/holdout partitions. (3) **R21 StewardOS
> accounts (deliberately-not-a-CRM)**: `kind=account` workflow rows,
> identifiers only, `decision_ref`-gated pipeline the machine never advances,
> history as pure decision join; six account routes + corpus export = seven
> new record-layer routes. (4) **R19 LAYA Phase 0**: decide pure modules
> (lang/router/sequence/calibration/presets + splitter), 134 tests, zero
> Cargo change, zero behavior change — ungated with no callers. (5) **Exec
> OS boundary**: typed sandbox seam (deny-default sandbox-exec / Landlock,
> fail-closed on unavailable backend); the X-W7 unwired pin retired by
> design with the Loop landing. Security posture: machine-refusal law at
> surface AND core (`decision_ref` required, agent class refused pre-write),
> DPO dual gates + per-call audit on both bulk reads, probe-blind 404s on
> every new id-scoped route, CI on the `cargo-audit` binary over all
> lockfiles + the conformance two-door rule. DELIBERATELY ABSENT: the 1.32.8
> classifier consume (opener-gated on the operator labeling round — the lane
> stamps when that lane ships, not before). Spire at ship: routes 216,
> tests 2007, coverage 180, authz 164; crate floor 1,568.
> Predecessor: **v1.28.91 "Notary"** (2026-09-15) — the operator-held
> evidence pair. Theme: two disclosed ceilings narrowed by two CLI verbs —
> the off-host witness and the physical shred. Zero wire; zero routes; no
> schema. (1) `brain anchor` / `--verify`: a deterministic state
> fingerprint (audit chain head + knowledge content census + row counts)
> the operator records OFF-HOST; `--verify` recomputes and diffs — any
> state change trips it, the audit chain explains legitimate ones, and a
> moved knowledge census on a clean chain is exactly the R7-08
> behind-the-chain tamper class no in-tree verifier caught. Read-only by
> design (an anchor's own audit row would move the head it fingerprinted;
> the off-host copy IS the evidence). Pins: tamper detection (the attack
> reproduced), chain truncation, reopen determinism, VACUUM stability,
> line round-trip/refusal. (2) `brain shred`: the physical residue drop
> after a logical purge — secure_delete=ON (readback asserted) →
> wal_checkpoint(TRUNCATE) → VACUUM → second TRUNCATE → integrity_check →
> one hash-chained `forget` row; freelist reads back 0; the marker pin
> proves the deleted bytes greppable pre-shred and absent from main AND
> wal post-shred. Ceilings printed per run: filesystem copies, `.bak`,
> standby chunks, SSD wear-leveling stay operator-level. (3) Register:
> the aarch64 CI-execution gap CLOSED not-applicable (no Jetson/fleet
> deployment; reopen at first aarch64 fleet deploy); K7-01/02/04 FINAL
> (no upstream PRs — procedural compensating controls in THREAT_MODEL
> §5b); CodeQL #74 rode ahead (`b695c77`). Floor walk: 1,455 → 1,462
> (seven new pins, all in the two new lib modules, CLI-only consumers —
> the standby precedent). **Log-gap note:** v1.28.89 "Bounded" and
> v1.28.90 "Refresh" shipped without rows in this file — per-release
> detail lives in `CHANGELOG.md` §[1.28.89]/§[1.28.90].
> Predecessor: **v1.28.88 "Clocktruth"** (2026-09-14) — seventh-pass
> closures, release 3 of 4. Theme: clocks, labels, and guards at law — the
> one legally-wrong clock in the repo, the guard that couldn't see two
> subdirectories, and the docs rows that outlived their debunkings. Zero
> wire; zero routes; no schema; mostly docs + two small code fixes.
> (1) L7-01: the CRA runbook's final-report clock was legally wrong for the
> vulnerability trigger (one month for BOTH; law: ≤14 days after the
> corrective/mitigating measure is available, Art 14(2)(c); one month binds
> severe incidents only, 14(4)(c)); runbook split by trigger, CSIRT framing
> corrected to single-platform → coordinator CSIRT (main establishment) +
> ENISA, reg_watch citations re-numbered to final-OJ (14(1)-(2)/(3)-(4)/(5),
> Art 71(2)) and the AI Act horizon re-cited to Regulation (EU) 2026/1744
> (OJ CONFIRMED — L7-04's fallback not needed); Annex III 2027-12-02 /
> Annex I 2028-08-02 deployer horizons stamped in COMPLIANCE.md; drill
> script template + timing report carry both clocks. New pin
> `reg_watch_runbook_clock_anchor` anchors the 14-day wording (RED→GREEN).
> Citations re-verified 2026-09-14. (2) R7-09: the transport-free guard's
> collector extracted + made recursive (the no-SQL walker idiom) — the four
> `dsar/`+`lifecycle/` files were invisible to the top-level walk;
> `transport_free_guard_walks_recursively` floors subdir files at the
> measured 4 (plan's draft ≥5 declined — walk-measured truth); red-proof:
> planted `use axum::` in `lifecycle/` failed the new guard, never landed.
> (3) The docs-truth batch: T7-02 tamper-evidence scope sentence; T7-03
> staleness rows re-stamped (THREAT_MODEL ×3 + R-14 + R-06's same dead
> cell; residual = registry-unavailability-fails-closed); T7-04 the
> crypto-inventory PRIMITIVE CENSUS (closed 8-row crate→inventory mapping +
> crypto-family heuristic over `[dependencies]`; planted `p256` fails it,
> never landed); T7-05 stamps moved + the standing same-commit stamp
> policy; T7-06 verify-JSON scoped as the consumer's out-of-band act;
> R7-10 docstring math fixed ("systme"→"sysetm" + boundary negative pin, no
> verdict change); R7-11 cross-chunk weld scope disclosed (chunker arm +
> THREAT_MODEL ceiling); L7-02 TIDA dates un-inverted; L7-03 CA 2026-09-10
> package (SB 1119) + multi-state chatbot family (GA SB 540, OR SB 1546);
> L7-06 AI RMF mid-revision footnote. (4) L7-05 HONEST CEILING: SBOM spec
> 1.3 → **1.5** — cargo-cyclonedx 0.5.9 (latest) emits 1.3/1.4/1.5 only and
> reads NO config file (the plan's `.cargo/cyclonedx.toml` route does not
> exist); `--spec-version 1.5` pinned in sbom.sh, one-flag bump when
> upstream ships 1.6/1.7. Floor walk: 1,455 (1,452 → 1,455; all three new
> pins ride plain `#[test]`). Predecessor: **v1.28.87 "Ownerstamp"**
> (2026-09-14) — seventh-pass
> closures, release 2 of 4. Theme: the seams' last mile — the DSAR root
> semantics question, the one roster that attested a seam it lacked, the
> admin-evidence surfaces the unconditional read-seam law hadn't reached,
> and the site-table guard hardened to read code, not prose. (1) F7-02: the
> STAMP decision (a) — every content write carries an owner stamp
> (`content_owner_stamp` + the fixed `loopback` label for the opaque
> superuser; JWT subs unchanged) at five write edges; the DSAR locate
> (`owner = subject`) now covers operator-authored ingests. Write-side only,
> no migration: historical NULL-owner rows stay stamp-blind by declaration
> (dated); NO OR-arm sweep (a legacy arm would mis-attribute every
> NULL-owner row in multi-principal trees); `suggest_feedback` keeps
> sub-or-NULL (disclosed ceiling); procedure rows carry no owner column at
> all (schema-level, beyond the no-schema scope). Live drill: ingest →
> `/dsar` export for `loopback` → `roots:1` (was 0); sqlite readback
> `owner=loopback`. (2) F7-05: both crew views ride the seam at the emission
> map — the roster core's invisible pass covered `principal`/`current_case_ref`
> only; `roles`/`skills`/site now strip too; the skills-view comment is true
> now; site-table rows for both. HONEST RED-FIRST NOTE: the first pin
> attempt planted only the two core-stripped fields and PASSED pre-fix —
> reshaped to plant hostile roles/skills/site before the fix landed.
> (3) F7-06: `sanitize_value_strings` (deep string-leaf seam composition) at
> nine emission sites — breach list/detail, TIA/DPA, roles/profiles list+get,
> `/audit` actor; no digest impact (none bind `review_digest`); static TIA
> prompt text verified seam-clean. (4) F7-07: `handler_body` comment-strips
> sources (string-aware: line/block/doc comments, strings, the `'"'` char
> literal, `r#"…"#` raw strings) before the substring assert — owned-body
> signature change inherited by every consuming guard (authz coverage,
> screen routing, read-seam table, audit-order); red-proof pin covers the
> false-pass + honest call site + lexing hazards; the same-commit site-table
> row is now a release-checklist standing rule. Pins: the three surface pins
> RED→GREEN + `handler_body_ignores_comments_naming_the_symbol` +
> `content_owner_stamp_always_attributes`; site table +12 rows. Floor walk:
> 1,452 needle-visible (1,450 → 1,452; the three surface pins ride
> `#[tokio::test]`, which the spire needle doesn't count — floor set to the
> walk-measured truth). No schema; no routes; openapi.yaml untouched;
> x-api-version unchanged (no wire move — content-level hardening).
> Predecessor: **v1.28.86 "Attrbane"** (2026-09-13) — seventh-pass
> closures, release 1 of 4. Theme: close every open seam-door and wire every
> dormant defense. (1) F7-01: the read seam gains the ATTRIBUTE tier inside
> the hostile-element fixpoint — `on*` handlers and
> `javascript:`/`vbscript:`/`data:` schemes (one bounded entity-decode pass,
> whitespace/control compaction) drop from SURVIVING elements; scheme-hostile
> not attribute-hostile (benign http(s) hrefs byte-identical; the .76 weld
> family unchanged; `sanitize_read_cow` fast path untouched). (2) F7-03: the
> graph family rides the seam — four handlers + the traverse mapper via
> `sanitize_read_cow` (site-table rows added); the markdown write edge is
> DECLINE-AND-COUNT (`normalize_name`/`normalize_rel_type` or skip, response
> `edges_skipped` + in-tx audit note; hostile heading 200s, never 400s, never
> becomes graph structure); structured `entity_type` gains the closed
> charset (400s; lowercased first). (3) F7-04: `AuditKind::Procedure` +
> in-tx row in `store_procedure`; knowledge-row audit beside the edge audits
> in `store_record`; `/add` + `/ingest/markdown` audits moved INSIDE their
> txs (the post-commit crash window closed; rollback twin = trigger poison).
> (4) S7-01/02/03: plugin 0.6.9 — `sanitizeForBlock` INVOKES the
> hostile-element mirror (server-canonical position); proposal details
> become a sanitized projection (sourcePrompt dropped), traverse paths +
> decision rule text + label fields ride the boundary; `provenance`/`evidence`
> get a deep string-leaf sanitize. Fork synced (vitest 71/71, tsc clean,
> byte-parity). (5) S7-04: sync-plugin's post-sync check is the
> declared-exception form AND the format.test.ts delta is eliminated
> canonical-side. **DIGEST DISCLOSURE:** the tier widens `sanitize_read` →
> rows with strip targets move `review_digest` → outstanding approvals 409
> at approve (observed LIVE: pre-M1 digest → 409, re-review → 200) —
> re-review required, expected, not avoided. Live drill: canary rows raw on
> .85 / attribute-free on .86; hostile-heading ingest 200 `edges_skipped:2`;
> procedure evidence row on the chain (verify 6/6 signed); live DB untouched
> (hash-verified). Floor walk: 1,450 crate `#[test]` (the plan's +6 are real;
> four ride `#[tokio::test]` which the spire needle doesn't count — floor set
> to the walk-measured truth). Plugin attribute tier stays server-side by
> design (the mirror is the element backstop). x-api-version moves with the
> crate version (informational stamp; the wire contract delta this release:
> additive `edges_skipped` only). No schema; no routes. Proof commits:
> `713748a`, `0d797ba`, `48fef68`, `15a7c99`.
> Predecessor: **v1.28.85 "SixthPass"** (2026-09-13) — the sixth-pass
> audit's closures. Theme: evidence labeled what it is, mirrors matching.
> (1) Forget erasure audit rows carry the Forget kind (G6-02): the erasure
> and per-proposal scrub rows wrote kind `ingest`; both now write kind
> `forget` (red-first pins failed pre-fix, green post-fix). Historical rows
> keep their meaning. (2) Fork extension carries the hostile-element
> mirror (G6-01): synced to plugin 0.6.7, byte-parity verified, 70
> extension tests green, typecheck clean. Prev: v1.28.84 "Quarterly"
> (SSE revocation kill + required webhook signing + 26-element strip +
> docs-truth pass; see `CHANGELOG.md` §[1.28.84]).
> (4) Newer-schema databases REFUSE to open (was: undefined behavior on
> unknown columns) with migrate-rehearse parity (55 tables); the static
> embedder gains a std-only saturation gauge (`SatGauge`/`SatGuard`) so
> the .76 serialized-inference cost class is measured, not discovered
> under load. (5) Docs-truth: README UMP badge derives from the CI
> conformance gate (loud degrade to "self-attested" when absent);
> `scripts/env-truth.sh` is the docs-vs-code env gate; the release
> checklist discloses SBOM scope per CISA-2026 (runtime closure, 375 vs
> 520 lockfile packages — NOT the dev+build tree), the 8-route
> intentional OpenAPI exclusions, and the 7-route well-known wiring. (6)
> A 25-row error taxonomy with operator-safe `Display` impls is pinned
> (`tests/error_taxonomy.rs`); `tests/singularity_pins.rs` adds 7 pins
> (incl. revocation-cache ZERO staleness — the "60s" claim debunked).
> (7) CodeQL #73 cleared (generated test key, `7d63f32`). WIRE: the
> `/ready` probe returns JSON (was text/plain) — the release's only
> wire-visible delta; consumers scraping the body must read `status`.
> No schema; no routes; x-api-version unchanged. CRATE_TEST_FLOOR
> 1,418 → 1,448. Ceilings (honest): SSE re-auth is polling (a revocation
> lands within the interval, not instantly); signing covers the two env
> sinks (hostcall HTTP keeps its .69 loopback allowlist); the gauge
> observes the static embedder only. Proof commits: `7d63f32`,
> `2567d84`, `60c344c`, `935d215`, `89a6233`.
> **Log-gap note:** releases v1.28.77–v1.28.83 ("Recall" is .83, the
> fifth-pass fix release; "Vigil" .82, "AgBOM" .81) shipped without
> notes in this file — per-release detail lives in `CHANGELOG.md`
> §[version] and the SECURITY.md history table for that span.
> Predecessor (last noted here): **v1.28.76 "Selfheal"** (2026-09-09) — the second-pass
> audit's fix release (docs/SECOND_PASS_AUDIT_20260909.md; 30 findings
> across both trees). Theme: nothing stripped may reassemble, and no gate
> has a side door. (1) BOTH read-seam strips are bounded FIXED-POINT now —
> `<scr<script>ipt>` welded into a live `<script>` and
> `[![a](i) c](o)` into a live auto-fetch image under the one-pass
> strips (demonstrated, then fixed; overflow fails closed by dropping the
> trigger bytes). (2) The ONNX scorer is budgeted (64 sentences / 16k
> chars — every inference serializes on one mutex; a 1 MiB ingest pinned
> all screened writes); embedder input budgeted at 8k chars (stored text
> verbatim; one helper, all backends). (3) X-W4 completion: valet `what`
> vets at the CAS seam too (`valet_what_refused` + Denied audit). (4)
> Kill-switch reach: `/auth/refresh` (public route — 401
> `identity_revoked`) and console actors (`actor_revoked` before
> capability). (5) Live `valet/due` SSE rides the opt-in + domain gate
> (replay already did; private labels streamed unfiltered). (6) Egress
> table: mapped-v6 normalizes into the v4 table + NAT64/6to4/Teredo/
> discard rows. (7) MCP read-scope denies `ump.feedback`. (8) Purge
> deletes suggest_feedback rows for purged chunks (+ sweep tenant arm).
> **.75 correction of record:** the `exec_spawn_carries_kill_on_drop` pin
> was VACUOUS (its target string occurred only in the assertion) and the
> exec spawn is std::process (no kill_on_drop) — the honest mechanism is
> the deadline block, re-pinned behaviorally (`exec_deadline_kills_child`,
> injectable `exec_effect_for`). Docs truth: THREAT_MODEL §5b (the
> .63–.75 controls; was frozen at .68), SECURITY history (was stopped at
> .17), OWASP matrix re-stamp, AI_LITERACY/openclaw-integration/plugin-
> CHANGELOG current, repo-brief.sh un-crashed (route sites live in the
> router). FORK: Truthglass + Pin merged to fork main 09-09
> (cherry-picks `1e5d31fb85e`/`fde920cb23a`/`6d54452a029` + the
> localeCompare→code-unit pin fix; fork CHANGELOG/lockfile byte-untouched;
> branches deleted, tips recorded in the audit doc). Digest-invalidation
> disclosure: the strips move `review_digest` for weld-bearing rows →
> fail-closed 409 at approve, re-review required. CRATE_TEST_FLOOR
> 1,363 → 1,372 (+10 pins). No schema; no routes; x-api-version unchanged.
> Planned: v1.28.77 "Erasure", v1.28.78 "Unconditional", v1.28.79
> "Ceremony" (audit doc §4).
> Predecessor: **v1.28.75 "Preflight"** (2026-09-08) — the PROGRAM
> EXIT GATE; the 1.32.x Loop line may open. (1) X-W7: the dormant exec
> mediation HARDENED (argv0 canonicalize-and-refuse-divergence closes
> the symlink-masquerade door; the danger screen is the documented
> TRIPWIRE and gains the pipe-to-shell family; kill_on_drop pinned at
> the spawn seam) + `hostcalls_mediation_stays_unwired_until_loop_line`
> — dormancy is a declared, machine-checked state (the Loop wiring
> commit DELETES the pin by design). (2) X-A4b: the installer writes
> `BRAIN_WRITE_POSTURE=review` for plists with NO explicit posture only
> — operator-set values are never stomped (the old unconditional
> remove+insert did, every re-run); the completion message names the
> posture + the opt-out; compiled default stays `open`. (3) X-C5/C6:
> the two ceilings are DOCS TRUTH in THREAT_MODEL + SECURITY reporter
> scope (chain key + pin share the host — detects SQL-level tampering,
> not host compromise; live DB + `.bak` plaintext on the primary,
> follower-only encryption law). (4) X-C8: `badges.sh --selfcheck`
> REFUSES without the committed `sbom/brain-server-<version>.cdx.json`
> — the human generate+commit step is unforgoable. (5) M5: the program
> close-out in `docs/AUDIT.md` — 55 findings × disposition (the plan's
> "41" undercounted), the four-leg exit-gate drill (channel/out forge,
> steering launder, revoked principal, poisoned-memory canary — ALL
> fail closed), per-release deltas, surviving ceilings. CRATE_TEST_FLOOR
> 1,358 → 1,363. No schema; no routes; x-api-version unchanged.
> RELEASED as tag `v1.28.75` (2026-09-08) at the follow-up fix commit
> `4890d89`: Ubuntu CI (merged-usr, /bin -> /usr/bin) caught what macOS
> could not — the admission canonicalized only argv0, turning honest
> textual allowlist entries (/bin/ls) into refusals. The fix
> canonicalizes the ALLOWLIST ENTRY too (honest aliases admit; planted
> symlinks and un-named siblings still refuse, pinned); release.sh
> refused the tag on the red matrix exactly as designed, and the
> release shipped only after green. docs/AUDIT.md carries the close-out.
> Predecessor: **v1.28.74 "Origin"** (2026-09-08) — taint labels
> survive the whole trip; THREE TREES (X-S2 proportionate, X-F3). (1)
> brain: `/ingest` + `/ingest/proposal` accept `origin_context:
> "owner"|"channel"` (absent = owner; unknown = 400); channel captures
> store origin `channel-capture`; the review-queue proposal SOURCE
> carries the badge and approval promotes it onto the row. (2) plugin
> 0.6.0: hit lines prefix `[memory | channel-capture]` inside the fence
> (owner untagged); `untrustedOrigins: "label"|"exclude"` (default
> label) drops captured hits from AUTO-INJECT under exclude; the
> memory_recall TOOL path always labels; autoCapture sends `channel`
> from the gating chat-type. (3) openclaw fork: the inbound boundary
> marks quoted/replayed `[memory | …]` prefixes as `[quoted memory ·
> origin: … — untrusted replay, not fresh prose]` (no brain-store
> coupling — one textual convention). (4) X-F3: span attribute values
> pass ANSI/C1 strip + unconditional PII redaction (`domain` at recall/
> gate spans); `query_hash` untouched; the OTLP export path logs
> "telemetry attributes are sanitized; treat any collector as
> untrusted infrastructure" at startup. No lattice, no policy engine —
> ONE boolean-grade label. CRATE_TEST_FLOOR 1,356 → 1,358. openapi
> additive; x-api-version unchanged; no schema (origin value extension).
> Released as tag `v1.28.74` (2026-09-08; the fork half in the openclaw
> repo at d724cb8).
> Predecessor: **v1.28.73 "Keyring"** (2026-09-08) — key + evidence
> lifecycle (X-C4, X-C3, X-W8). (1) X-C4: the operator signing key is
> DETERMINISTIC — fixed filename `operator.ed25519` (one-time
> transparent rename from the legacy first-file scan, logged);
> wrong-size/leaked seed → LOUD refusal (the silent L2 degrade dies);
> `brain key rotate` (inside the existing `brain key` dispatcher) moves
> current → `.prev` (verify-only, ONE key deep, second rotate refuses),
> writes a new 0600 seed, bumps `operator_key_generation` in schema_meta,
> and writes the hash-chained audit row (the register IS the audit
> chain — the plan's lineage event maps there honestly); cards gain
> `signing_epoch` (additive column, schema 1.28.73, contract test
> extended) and `verify_card` picks current/prev by epoch (legacy NULL
> = try both). (2) X-C3: a chain-less backup image REFUSES restore
> (`chainless_image_refused`) unless `--allow-chainless`; legacy-epoch
> chains restore marked `legacy_unkeyed_chain: forgeable: true` +
> disclosure evidence row naming `--re-audit`; head-pin rollback stays
> disclosed-not-refused. (3) X-W8: the replay cache evicts the OLDEST
> QUARTER at cap (was: clear-all); the revocation drain pages 10×200 and
> writes a `drain_incomplete` audit row naming the remainder.
> CRATE_TEST_FLOOR 1,345 → 1,356. No routes; wire additive only;
> x-api-version unchanged. Drills + migration notes in CHANGELOG
> §[1.28.73]. Released as tag `v1.28.73` (2026-09-08).
> Predecessor: **v1.28.72 "Scrim"** (2026-09-08) — every emitted
> surface is shaped (X-R3, X-W6, X-L4, X-E5). (1) X-R3: the read seam
> (`sanitize_read`) gains `strip_hostile_elements` AFTER the markdown-ref
> strip — a CLOSED case-insensitive attribute-greedy element-name set
> (script/img/iframe/svg/object/embed/link/meta/form/input/video/audio/
> source/track/base); prose angle-brackets survive (pinned); storage
> stays verbatim so `review_digest` does NOT move; the
> `sanitize_read_cow` fast path now requires a `<`-free row. (2) X-W6:
> the `GET suggestions` KCS evidence side-effect requires Write + the
> `workflow` role; Read-only principals get the body unchanged +
> `evidence_recorded: false` (additive, openapi'd). (3) X-L4: a denied
> `/events` subscriber gets HTTP 403 BEFORE the stream opens (was
> 200+error-event); mid-stream failures keep the error-EVENT path; the
> client driver already handled non-200; the authz matrix moved
> `/events` out of `SSE_SOFT`. (4) X-E5: the KB generator escapes
> operator-configured base_url/locales everywhere (hreflang, sitemap
> loc/alternates, canonical already escaped at render) and the library
> enforces the CLI locale contract (alnum+hyphen ≤12;
> `locale_matches_cli_contract`). Bytes ledger: stored markup vanishes
> from read seams; digests unchanged (corpus green). CRATE_TEST_FLOOR
> 1,336 → 1,345. openapi additive; x-api-version unchanged. No schema.
> CHANGELOG §[1.28.72]. Released as tag `v1.28.72` (2026-09-08).
> Predecessor: **v1.28.71 "Pores"** (2026-09-08) — the screen sees
> what the model sees (X-R4, X-R6, X-R7; plan + execution prompt in the
> repo root). (1) X-R4a: layer-1 `contains_suspicious_pattern` runs on
> `strip_invisible(trimmed)` at the `Screen::screen` seam — the
> raw-vs-stripped disagreement (bidi-split `sys\u{202E}tem:` dodging the
> line-anchor matcher) is CLOSED; verdicts can only move
> Clean→Quarantine/Reject. (2) X-R4b: the blocklist stops being 13
> English phrases — translation families (es/de/fr/nl/fil, same six
> intents, data-driven `FAMILIES` table pinned by
> `blocklist_families_cover_five_languages`), the typoglycemia anagram
> tier (first+last + sorted-middle equal, length ≥ 4; EXACT keywords
> never trip — bare "system" stays prose), and the bounded encoding tier
> (base64/hex runs ≥ 24 chars, first 8 runs, ≤ 4 KiB decode, ONE decode
> level — `encoding_scan_bounded` pins run #9 stays undecoded). (3)
> X-R4c: the layer-2 classifier AUTO-LOADS when the artifact resolves
> (`~/.config/brain-server/models/injection-classifier/{model.onnx,
> tokenizer.json}`); `BRAIN_INJECTION_CLASSIFIER=off` opts out; a
> non-existent explicit path REFUSES the boot; `/health/db` echoes
> `injection_classifier: on|off|absent`; poison posture unchanged
> (fail-open 0.0). (4) X-R7: `sanitize_log_value` routes through the
> shared `strip_control_chars` — ANSI/C1 escapes leave log VALUES (\r
> now removed, \n→space + tab-survival pinned). (5) X-R6: the bridge
> edge strips the canonical invisible class + dereferences markdown
> refs before the 4000 clamp (kernel screen stays authoritative).
> **Verdict-shift disclosure: quarantine-ward at the margin only; the
> clean-corpus pins hold; the .65 Meridian canary keeps its Clean
> screen verdict (pinned — it is a read-seam catch by design).**
> CRATE_TEST_FLOOR 1,318 → 1,336. Tests: full bench 1,426 passed /
> 7 ignored; bridge crate 39 green (its clippy is deny-lint strict —
> the scanner port is checked-arith/.get by construction). No schema;
> no routes; openapi untouched; x-api-version unchanged. Drill + the
> standing "tripwire, not boundary" note in CHANGELOG §[1.28.71].
> Released as tag `v1.28.71` (2026-09-08).
> Predecessor: **v1.28.70 "Twokeys"** (2026-09-08) — the opaque-mode
> operator/agent split; the REGISTER LINE opens. Token-file line 2
> becomes `PrincipalKind::AgentLoopback` (scoped `write:*/global`, ship
> `agent` preset, Blackout kill-switch + `agent_forbidden` audits);
> `/health/db` full body Admin-on-global (Read gets the reduced probe);
> `/metrics` per-domain labels render only in-scope (`domain="other"`
> summed collapse). Single-token deployments keep the legacy superuser
> posture byte-identically (honest F-W1 disclosure — the boot warn is
> the nudge). Full note: docs/AGENTS_HISTORY.md + CHANGELOG §[1.28.70].
> Predecessor: **v1.28.69 "Deadbolt"** (2026-09-08) — the egress and
> process boundary. THE SEAM LINE CLOSES (X-E3, X-M4, X-M5, X-M6; plans +
> execution prompt in the repo root). (1) X-E3: the shared egress client
> stops reaching private networks — resolve → validate → PIN per the OWASP
> bypass-proof form (`validate_public_addrs` in `webhook.rs`: every resolved
> address must clear the IANA IPv4/IPv6 special-purpose table — real
> `IpAddr` math, the plan's exact table incl. 100.64/10 CGNAT; metadata
> hostnames refused pre-resolution; `IpAddr` parsing + the url crate's
> literal canonicalization kill the hex/octal/dword class). The two env
> sinks (`BRAIN_ALERT_WEBHOOK_URL`, `BRAIN_DSAR_WEBHOOK_URL`) resolve +
> validate + pin AT BOOT (post-tracing-init — the drill caught the
> pre-init placement swallowing the pin logs); private sink without
> `BRAIN_EGRESS_ALLOW_PRIVATE=1` → REFUSE BOOT (WRITE_POSTURE pattern;
> the env parses fail-closed in `config.rs`); opt-out = loud warn + still
> pinned; boot DNS failure → warn + lazy fail-closed on first send
> (`egress_unresolved`); pins are insert-only (first resolution wins for
> the process lifetime — `pinned_client_survives_dns_rebind` pins an
> RFC 6761 `.invalid` name and proves the shadow listener sees zero
> connections). reqwest 0.13.4 `resolve_to_addrs` is the seam (SNI
> preserved — verified against the vendored source). The hostcall HTTP
> path KEEPS its allowlist (loopback mediation is a pinned feature — the
> public-only table does NOT apply there) and gains validate-on-first-use
> + an insert-only per-host client cache bounded STRUCTURALLY by the
> allowlist. (2) X-M4: `resolve_harness_bin`'s PATH scan DELETED —
> absolute `BRAIN_STEWARD_BIN` only (relative → named refusal, no silent
> exe-dir fallback) or beside-the-kernel; the CLI's own PATH resolution
> stays (operator context — documented ceiling). (3) X-M5: crank spawn
> gains `kill_on_drop(true)` (`run_harness_crank_bounded` carries the
> scaled test seam); the hostcall exec path's `try_wait`-error early
> return now kills+reaps too; the DRILL found the router's 30 s
> TimeoutLayer drops the handler future first — and the child dies on
> THAT drop as well (previously it survived every abandonment path).
> (4) X-M6: console `pending` requires the mapped actor's `read`
> capability via the existing `resolve_console_actor` machinery (empty
> grants nothing); decide/due/crank unchanged. Migration notes (both in
> the CHANGELOG): private-sink operators need the opt-out env; PATH
> users need absolute `BRAIN_STEWARD_BIN`. Tests: 6 egress pins + 1
> hostcall cache pin + 3 harness pins + 2 crank pins + 3 console role
> pins; the Art-19 loopback drill test now runs under the opt-out env
> (the fail-closed default REFUSES loopback sinks — the suite wedged on
> the old expectation before the fix). CRATE_TEST_FLOOR 1,303 → 1,313
> (needle re-measured at the release commit). No schema; no routes;
> openapi untouched; x-api-version unchanged. Drill transcript (4 legs)
> in CHANGELOG §[1.28.69].
> Predecessor: **v1.28.68 "Shutter"** (2026-09-07) — the image + beacon
> egress seats close upstream (docs half).
> DOCS-ONLY in this tree: THREAT_MODEL §5 "Exfiltration surfaces" (the two
> carried EchoLeak-class seats F-E1 doc-mode remote images + F-E2 favicon
> beacon, closed default-OFF + host-allowlisted in the openclaw fork —
> commits ef6a84a/f1a8b0f/c9afcb6 there; X-E4 data-URI 64 KiB decoded
> budget) + the standing ceilings (bare URLs linkified-but-inert, the
> gate.rs ceiling; allowlists are trust, not safety); SECURITY.md names the
> image/beacon class in reporter guidance; CHANGELOG §[1.28.68]. Built in
> PARALLEL from a v1.28.63 cut in the `brain-server-68` worktree, rebased
> onto the post-.67 main (ship order .64 → .65 → .66 → .67 → .68 held).
> New `gateway.controlUi.remoteImageHosts` config surface lives in the
> FORK (exact hosts, empty = fail-safe; one setting, two consumers — UI
> images + server favicon route, server re-verifies). CRATE_TEST_FLOOR
> UNCHANGED at 1,303 (docs-only); measured 1,384 bench / 1,401 default /
> 1,403 otel at the release commit. No code, no openapi, no schema.
> Predecessor: **v1.28.67 "Pin"** (2026-09-07) — identity is pinned.
> Built in PARALLEL from a v1.28.63 cut in the `brain-server-67` /
> `openclaw-67` worktrees, then REBASED onto the post-.66 main once
> Truthglass shipped (floor/version/changelog reconciled in the rebase;
> ship order .64 → .65 → .66 → .67 held). Closes the 2026-09-06 audit's
> X-M1, X-M3 (HIGH), X-C1 (HIGH), X-C2. **Honest disclosure: attribution
> was self-asserted until .67** — marks verified that SOMETHING signed the
> bytes, never WHO. (1) M1 (openclaw fork, `pin-1.28.67`): MCP catalog
> pins — per-tool `sha256(name+\0+description+\0+stableStringify(schema))`
> + per-server digest, reconciled EVERY run (openclaw materializes per
> RUN — per-run re-hash IS the per-execution cadence; OWASP §2/§7),
> `mcp-catalog-pins.json` beside the agent bundle (agentDir discipline,
> 0644, not a secret); drift notifies (name+description+fp prefix) and
> drifted tools carry `pendingAck` meta; ack is an explicit operator
> touch; corruption reads as empty → LOUD rebuild (it can never silence
> drift); colliding display renames pin the ORIGINAL server-side name;
> rug-pull demo GREEN (`scripts/rug-pull-demo.mts`). (2) M2 (X-M1):
> `BRAIN_MCP_SCOPE` ∈ read|full — fail-closed parse (unknown refuses
> boot), dispatch-time gate BEFORE the network seam on `brain_ingest`/
> `ump.remember`/`ump.revise`/`ump.forget`, additive `tools/list`
> `"x-brain-scope": "read-denied"` annotation, default `full`
> byte-identical, startup logs the scope. (3) M3 (X-C1, THE ONE BREAKING
> WIRE CHANGE): parcels `expected_signer` REQUIRED (400
> `signer_required` — migration: name your counterparty) + `signer_alias`
> 409 when the local operator did is aliased for a foreign-produced
> parcel (self-parcels pass and verify on their own signature);
> provenance verify gains the operator pin —
> `verify_artifact_detailed(value, pinned)` returns
> `Ok|ForeignSigner{signed_by}|Unsigned|Tampered|Malformed` (the pin
> checks LAST, so tampering reports Tampered under a pin),
> `verify_artifact_json` always surfaces `signed_by` (self-assertion
> visible in L2), and the four emission-adjacent verify sites (remedy,
> ADR, campaign, KB manifest) wire the pinned variant. (4) M4 (X-C2):
> `/ump/audit/verify` carries the additive `integrity` census
> `{verified, signed, hash_only}` under the serve posture + the
> transitional `note: hash_only_records_present` (key exists AND
> hash-only seen — dead code today by design); serve behavior
> UNCHANGED. CRATE_TEST_FLOOR 1,290 → 1,303 (re-measured on the rebased
> tree). Gates: full suite green; clippy bench/default/otel clean; fmt +
> lipstyk clean; engine-crates + steward-harness green; badges
> selfcheck clean; wire additive except the parcel signer
> (x-api-version unchanged, schema untouched, no new deps). Ceilings:
> drift is SURFACED not gated (no ack UX; .73 owns rotation); the MCP
> scope is process-lifetime (correct for stdio's single parent); the
> alias gate needs the local key (keyless → signer_mismatch instead);
> the fork's committed pnpm lockfile disagrees with its own typebox
> catalog (upstream drift predating this line). See CHANGELOG
> §[1.28.67].
> Predecessor: **v1.28.66 "Truthglass"** (2026-09-07) — the approver
> sees the truth. BUILT IN PARALLEL from a v1.28.63 cut, then REBASED onto
> the v1.28.65 main once Blackout+Meridian shipped (floor/version/changelog
> reconciled in the rebase; ship order .64 → .65 → .66 held). All four
> Lies-in-the-Loop findings closed (audit §4.6): (1) X-L1 (HIGH, fork):
> plugin approvals carry `args` — the EFFECTIVE tool-call arguments (base ⊕
> approval overrides) as display JSON, persistence-redacted, capped 2000
> chars with a VISIBLE exact-count marker — on BOTH transports (embedded
> broker + gateway), computed once (`buildApprovalArgs`) so both surfaces
> see identical truth; gateway sanitizes + re-caps at its boundary (the
> `detail` discipline); TypeBox closed-object schema registers the field,
> Swift model regenerated in-commit (Kotlin doesn't model these params);
> the plugin's title/description STAY (prose claim AND raw act; OWASP MCP
> Cheat Sheet §4 now true at this surface). (2) X-L2 (fork, shipped AFTER
> Meridian's host half landed — the dependency rule, dependency HELD):
> truncation keeps head AND tail UNCONDITIONALLY (the keyword-gated
> "important tail" heuristic is GONE), last 400 chars always ride, middle
> marker carries the EXACT count (`[... N chars elided between head and
> tail ...]`); the tail reservation is bounded to half the budget minus
> the marker's widest form so tight budgets shrink the tail, never fall
> back to head-only; aggregate elision marker is COUNT-FIRST (crushed
> budgets cost the rerun guidance before the count); a notice larger than
> the result it replaces is skipped (net increase — the honest refusal);
> 16k cap + budget discipline untouched. The audit's shaping scenario is
> THE fixture: 100k, injection at 500, disclaimer at 99k — disclaimer
> survives, injection visible. (3) X-L3 (CLI): `brain client dsar`
> requires `--action` (closed vocab purge|export|both from the server's
> OBSERVED truth, not the plan's `rectify`); purge/both prompt with the
> subject digest (`sha256:<12-hex>`) + resolved domain (fail-loud) +
> irreversibility line; `--yes` is the automation seam; CLI BREAK:
> scripted purge adds `--action purge --yes`. (4) X-L5 (CLI): restore
> ALWAYS prompts unless `--yes`; `--force` skips the liveness PROBE only;
> prompt shows resolved ABSOLUTE target + size + audit chain head;
> passphrase files must be 0600 (`check_secret_file_mode` extracted +
> shared with the rotator); CLI BREAK: scripted restore adds `--yes`.
> Tests: 9 CLI pins (red-first) + 5 fork approval tests + 4 truncation
> pins (incl. exact-count arithmetic + the shaping fixture + source drift
> locks for the already-counted compact/default notices); the FULL
> embedded-agent lane re-run green after M2 (1,771 tests / 85 files);
> CRATE_TEST_FLOOR 1,281 → 1,290 at the rebase (Blackout's 1,281 + 9).
> main.rs untouched; no routes/schema/wire change (openapi + x-api-version
> unchanged). Fork-side: the typebox 1.3.15-lock/1.3.18-manifest drift is
> repaired IN COMMIT (main's working tree still carries it uncommitted —
> land or absorb). Manual ceilings: the approval-surface screenshot is
> pending a human run; `args` renders as a plain field (monospace block =
> cosmetic follow-up). See CHANGELOG §[1.28.66].
> Predecessor: **v1.28.65 "Meridian"** (2026-09-07) — content hygiene
> across the model seam, THREE TREES the same day. Closes the 2026-09-06
> audit's content-door findings: X-R1, X-R5, X-S1 (HIGH), X-M2 (HIGH).
> Theme: nothing enters model context unstripped and unlabeled, regardless
> of which door it used — four doors, four small structural layers.
> (1) X-R1 (brain): `/suggest` hits carry `untrusted: true` (recall/search
> parity — the one content-returning surface breaking the consumer
> contract); openapi additive; pins `suggest_hits_carry_untrusted_true` +
> the three-surface parity source pin; CRATE_TEST_FLOOR 1,267 → 1,269
> (Blackout's .64 raise to 1,281 lands with ITS release commit — floor
> counts are per-commit monotone, both truths hold at HEAD). (2) X-R5
> (plugin 0.5.1): `sanitizeForBlock`'s invisible set is now EXACTLY the
> Rust canonical set, exported as `INVISIBLE_CLASSES`; the parity fixture
> (`plugin_invisible_set_matches_rust_canonical`, one probe per Rust-set
> class) is THE drift pin — either tree changing alone fails CI; the
> plan's five-member add list was itself short of parity (missed
> U+FE00–FE0F, U+2060–2063, U+00AD, U+034F) — the fixture forced the full
> set. (3) X-S1 (openclaw fork): the ONE merge seam strips + neutralizes —
> every plugin context segment passes `sanitizePluginContextSegment` (new
> `src/plugins/context-hygiene.ts`) inside `mergeBeforePromptBuild` (the
> convergence point BOTH runners ride): canonical invisible strip +
> ZWSP-split of forged `⟦openclaw:ctx⟧` markers and the built-in
> `<active_memory_plugin>` fence tags (strip runs FIRST → idempotent; the
> JOINED accumulator is sanitized → no cross-segment synthesis; the
> built-in's own emitted tags split too — uniform, no per-plugin logic);
> the brain plugin's `UNTRUSTED_BEGIN/END` fence survives BYTE-IDENTICAL
> (pinned). The fork's `stripInvisibleUnicode` widened to the canonical
> set (bidi isolates U+2066–2069 + ALM were missing — the same drift
> class, closed host-side). (4) X-M2 (openclaw fork): MCP tool results
> ride the external-content idiom — text blocks invisible-stripped, the
> joined result enveloped ONCE (never per block) via the extracted
> `createExternalContentEnvelopeSegments` (`wrapExternalContent` composed
> on it, byte-identical); `Source: MCP Tool Result` label; the
> `untrustedMcpOutput` flag finally renders as prompt framing; the
> host-authored empty placeholder stays unwrapped; forged wrapper
> boundaries neutralized by the existing sanitizer. THE LINE'S FIRST LIVE
> END-TO-END PROOF (GREEN): `docs/MERIDIAN_PROOF_20260907.md` — a memory
> carrying the U+E0000 tag block + forged host markers, recalled through
> the real plugin fence + host merge + CLI composition on a TEST server
> (fresh DB, test port, copies-only): all forgeries absent from the
> composed prompt. Ledger: Blackout (1.28.64) SHIPPED FIRST off the same
> main push that carried the Meridian fix commits (0b66d3b, 03819bf);
> keep-a-changelog order .65-above-.64 is the honest history. Gates: full
> suite 1,360 passed / 7 ignored; clippy bench/default/otel clean; fmt
> clean; engine-crates + steward-harness green; lipstyk green (one
> comment-only condensation 4e6c477 rode the pre-push gate — density
> finding on plugin/src/format.ts, zero behavior change); badges
> selfcheck clean; the known connector-stub race fired once under the
> parallel sessions' load (passed on rerun, pre-existing test-infra).
> Ceilings: X-R2/X-R3 stand (consumers fence; HTML strip is .72); no
> taint lattice (X-S2 → .74 Origin); truncation-after-wrap can elide the
> envelope end marker until Truthglass (.66); MCP schema pinning is .67;
> no server-side fence envelope on HTTP JSON; the fork's `pnpm-lock.yaml`
> typebox bump predates this line. openapi additive only; x-api-version
> UNCHANGED; schema untouched; no new deps in any tree.
> See CHANGELOG §[1.28.65].

> Predecessor: **v1.28.63 "Wardline"** (2026-09-06) — the SEAM LINE opens:
> reserved vocabulary at the workflow input seam (`RESERVED_OUTBOX_TOPICS`
> at `enqueue_child`, closed run statuses, the valet fence function-held,
> alert-bus kind auth), closing the only code-false security law in the
> repo's history (X-W1..X-W5). Full note retired to
> `docs/AGENTS_HISTORY.md`; see CHANGELOG §[1.28.63].
> Predecessor: **v1.28.62 "Attestation"** (2026-09-06) — the Enterprise
> Line CLOSES. Four milestones, all additive (no breaking wire change, no new
> crypto primitive, no C2PA claim anywhere). (1) PROVENANCE MARKS
> (`src/provenance.rs`, the Art 50(2) posture): engine-generated TEXT
> artifacts carry `{"provenance": {mark: AIGEN|HUMAN, generator,
> generated_at, signed_by, sig}}` — Ed25519 via `sign_manifest_bytes`, but
> the SIGNED MESSAGE is the claim-bound wrapper `{artifact, claim}` (the
> body-only design let a flipped mark verify; the pin caught it in
> development). The four classes ride their REAL emission shapes: remedy
> drafts (`handlers::workflow::remedy_response`), ADR packets +
> outreach exports (`seal_adr_packet` / `seal_campaign_packet` — read-seam
> shaping THEN the seal, the mark signs boundary bytes), KB manifests
> (`kb::sealed_manifest_json` — the pure digest rule byte-unchanged). No
> operator key ⇒ mark present but visibly unsigned (verify refuses). Pins
> `provenance_marks_present_on_all_four_classes` + `tampered_provenance_
> fails_verify`; reg_watch ai_act WATCH → DELIVERABLE (horizon 2026-12-02
> stays stamped). openapi additive; x-api-version UNCHANGED. (2) THE
> PRINCIPAL KILL-SWITCH (ASI03/07): `revoked_principals` (schema → 1.28.62);
> `verify_card` checks revocation BEFORE signature work AND before the row
> lookup (probe-blind); dispatch/result re-check at decision time; re-
> provisioning does NOT resurrect. The drain: revoke upsert + hash-chained
> auth audit row + in-flight-run cancel via the EXISTING cas_update path
> (`status 'cancelled'`) + `delegation/revoked` lineage events, all in the
> caller's tx. Routes `POST /ops/agents/revoke` (Admin on global —
> identity-wide) + `GET /ops/agents/revocations`; openapi + both guard
> tables + api.md in-commit. Pins `revoked_principal_cards_fail_closed` +
> `revoked_owner_no_new_dispatch`; the authz matrix carries the route's body
> template. NOT JWT revocation (that is auth/revocation.rs — separate
> layer). (3) APPROVAL-FATIGUE TELEMETRY (ASI09): the client detector's
> arithmetic server-side — `scoreboard::approval_uniformity`, verdict
> expression the client's f64 form VERBATIM (0.9 boundary identical), ratio
> = integer ten-thousandths; the data fn mirrors the client's fetch (7d on
> created_at, latest 200/status, decided-only); scoreboard JSON gains
> `review_independence_risk` + `approval_uniformity_ratio` +
> `review_decisions_window`; role gate UNCHANGED (DPO/admin). Dictionary
> twins (metrics.md ASI09 section + metrics.json) same-commit; parity pin
> `scoreboard_uniformity_matches_client_math`. (4) CRYPTO INVENTORY
> (`docs/crypto-inventory.md`, SP 1800-38B shape): every shipped algorithm ×
> HNDL verdict × swap path; the JWT ML-DSA landing procedure against the
> real `ALLOWED_ALGS` seam; the UMP did:key multicodec version-prefix rule.
> NO PQC deployed — classical signatures are the printed ceiling (JWT waits
> on the IdP). reg_watch pqc WATCH → DELIVERABLE (2030-12-31 stamped); the
> watch clock keeps its own self-test. SOC 2 proof-map refreshed; brain-fuzz
> corpus rides a nightly CI schedule (corpus replay + harness-kernel +
> libfuzzer compile check; caught a latent libfuzzer-feature warning).
> CRATE_TEST_FLOOR 1,244 → 1,256. DRILL 2026-09-06 vs a COPY of the live
> 50.6 MB db: kill-switch end-to-end (cards 403, dispatch 403, owner
> revoked → `runs_drained:1`, run cancelled CAS 0→1, lineage event,
> `/audit/verify` ok); ADR packet + kb manifest carried signed AIGEN marks
> (digests unmoved); 21 digest-bound approvals flipped the scoreboard to
> risk 1 / ratio 10000. Runbook section + dated record + reg_watch
> `revocation_drill_recorded`. main.rs untouched (net delta 0); wire/schema
> additive only. See CHANGELOG §[1.28.62].
> Predecessor: **v1.28.61 "Standby"** (2026-09-06) —
> WARM standby from shipped mechanisms — encrypted base + WAL chunks + a
> rehearsed promote; NO hot failover claims anywhere. Library
> `src/standby.rs`, consumed ONLY by the `brain standby` CLI subcommands
> (operator-run process; a shipper inside the server it protects is a
> correlated failure). ship_cycle order is LOAD-BEARING (commented): PASSIVE
> checkpoint → base.v3 via the SHIPPED backup v3 writer → wal/NNNN
> frame-chunk copied AFTER the base (the writer TRUNCATEs the wal — an
> earlier copy would replay pre-base frames and roll the restore back) →
> manifest signed via the shared `ump_integrity::sign_manifest_bytes`
> (Ed25519 over the hex-SHA-256 STRING — the parcels convention; parcels
> refactored onto it byte-identically, `parcel_signature_bytes_unchanged`)
> and written LAST. Chunks ride `backup::encrypt_v3_blob` — NO unencrypted
> byte at rest on the follower. `verify_follower` = signature over exact
> bytes + recomputed artifact hashes, fail closed; `promote_check` reuses
> the shipped restore path, registers sqlite-vec first (vec0 tables),
> integrity_check, timed RTO, RPO = interval + measured checkpoint lag
> (`promote_check_rpo_math`; the follower-side twin of the .58
> brain_wal_pages_pending gauge). Pins incl. `manifest_sig_verified_after_
> copy`, `follower_tamper_detected` (one flipped chunk byte → status exit 1),
> the roundtrip proptest, `cli_reference_covers_subcommands` (the
> cli-reference law — closed four pre-existing doc gaps: parcel,
> wfm-import, valet, ropa), reg_watch `standby_drill_recorded` (dated
> record with measured timings, CRA-drill precedent). CRATE_TEST_FLOOR
> 1,228 → 1,244. DRILL 2026-09-06 vs a COPY of the live 48.8 MB db: RTO
> 0.55s, RPO 10.4s, 9,091-row fidelity, tamper fail-closed. DISCLOSED: the
> .bak safety mechanism proved itself live — a development-phase
> `restore --force` mis-aimed at the live db (target is BRAIN_DB_PATH/
> default, not the positional); the automatic snapshot carried the memory
> back; lesson encoded in the runbook promote procedure. The CodeQL
> security closures (7 alerts: path-injection ×3 storage_layout traversal
> refusals, log-injection ×1 sanitize_log_value seam, cleartext-logging ×3
> DSAR cert out of assert messages) landed in the same line — full record
> in CHANGELOG §[1.28.61]. main.rs untouched (net delta 0); wire/schema
> unchanged. See CHANGELOG §[1.28.61].
> Predecessor: **v1.28.60 "Loom"** (2026-09-06) —
> CPU parallelism as an OPT-IN tier: feature `loom = ["dep:rayon"]`
> (rayon 1.12.0; the ONLY new dep this line allows) × capacity target !=
> jetson × `BRAIN_LOOM=1` — fail-closed parse; pool capped `min(cores-1,4)`;
> `/health/db` echoes all four states. EXACTLY TWO fan-out sites
> (batch-ingest embed pre-pass; consolidate near-dup pure-CPU preprocessing
> — the KNN loop STAYS SERIAL on the shared `&Connection`). THE INVARIANT:
> no cross-chunk reduction — adding one breaks `loom_preserves_fused_ranks`.
> Seven pins (CRATE_TEST_FLOOR 1,221 → 1,228). Live proof
> docs/LOOM_PROOF_20260906.md + BENCHMARKS §v1.28.60: byte-identical vec
> index (sha256) across loom/serial postures; eval floors identical to 3
> decimals; HONEST: the static potion tier is too cheap for the fan-out to
> pay — the value case is the CPU-bound neural profile, unmeasured.
> See CHANGELOG §[1.28.60].
> Predecessor: v1.28.59 "Headroom" (2026-09-05) —
> A documentation-first release wearing a test harness; nothing behavioral
> changes on any default target (the envelope-defaults pin enforces). (1)
> DURABILITY POLICY: `CapacityEnvelope` gains `synchronous_mode` +
> `wal_autocheckpoint_pages` with defaults == the MEASURED pre-change
> behavior (empirical readback: a fresh pooled connection ran
> synchronous=FULL / autocheckpoint=1000 — the compile defaults; the
> migration connection's NORMAL never covered the pool). Applied beside
> `busy_timeout` at every pooled connection's init via the new named fn
> `main_pool_connection_init` (the inline closure is gone; boot file
> shrinks); env overrides `BRAIN_SYNCHRONOUS` | `BRAIN_WAL_AUTOCHECKPOINT`
> fail closed (unknown value refuses boot, the WRITE_POSTURE pattern;
> autocheckpoint 0 = off is refused); `/health/db` gains the static
> boot-time `durability` echo (additive). Pins:
> `envelope_defaults_equal_current_behavior` + `pool_init_pragmas_read_back`
> + the resolver bounds pins. (2) LOCK-WAIT TELEMETRY: every production
> `Mutex`/`RwLock` site (19 fields — 2 found beyond the plan's list) carries
> a Lock-bounds comment; 15 request-path holders record CONTENDED-only
> acquire waits (try_lock fast path = zero clock reads) into the new fixed-
> edge `LockWaitHistogram`; `/metrics` gains `brain_lock_wait_micros_p50|
p95` (bucket-quantile edges, no histograms crate); poison postures
> preserved per-site via the five measured-lock helpers. Named pins:
> limiter purity, token-swap single-assignment (>1k reads, >50 real
> rotations, zero torn), helper contention/poison contracts. (3) THE
> WRITE-DISCIPLINE RATCHET (`tests/write_discipline.rs`): the plan's
> "allowlist empty on arrival" was EMPIRICALLY FALSE (handlers construct
> transactions and delegate statements to service cores — the no-SQL gate
> counts statements, not BEGINs; plus sanctioned seams) — shipped instead
> as the Plumb debt-lock: DEFERRED inventory frozen at 38 sites / 21 files
> (down-only, unlisted hits fail with file:line), IMMEDIATE floored at 20
> (up-only), RED-PROOFED against a planted violation. (4) LIVE PROOF:
> docs/HEADROOM_PROOF_20260905.md + the BENCHMARKS.md §v1.28.59 table —
> WAL trajectory flat 0 in both cells (the 6000-doc burst showed the one
> mechanistic delta: 34-page transient under full/1000 vs flat 0 under
> 256); p95 24.52 → 24.19 ms (noise); lock-wait gauges' first live reading
> ≤10 µs; durability echo verified in both postures. CRATE_TEST_FLOOR
> 1,207 → 1,221. Pre-existing rerank-tier build break fixed in passing
> (`search::` → `crate::search::` in bootstrap). Ceilings (honest): the
> ratchet is NOT the plan's zero-allowlist; the split idiom's mid-file
> blind spot is shared with every house gate; quantiles are bucket edges;
> Jetson durability unmeasured; lock-wait coverage excludes the mcp
> binary, the connector token cache, and scrape-path locks (reasons
> inline). See CHANGELOG.md §[1.28.59].
> Predecessor: v1.28.58 "Throughput" — the Enterprise Line opens: concurrent truth
> (BENCH_CLIENTS fan-out, same-seed determinism), visible contention (pool-timeout,
> busy-error, WAL-pending gauges + the /health/db concurrency echo), the calendar
> as code (CRA 2026-09-11, AI Act, PQC seams) + the CRA reporting runbook + drill.
> Full note: CHANGELOG.md §[1.28.58]; retired detail: docs/AGENTS_HISTORY.md.
> **Version note:** **v1.28.54 "Scaffold" — THE SPIRE LINE OPENS.** The
> monolith dismantling starts with the risk-zero third: measure-and-freeze.
> Scope 1: `src/spire_inventory.rs` (cfg(test), sibling of the retired
> `sql_inventory_baseline` idiom) — the frozen ledger over main.rs:
> ceilings `MAIN_RS_LINES`, `TEST_REGION_LINES`, `ROUTE_CALL_SITES` (down
> only, in the commit that earns the shrink); floors `MAIN_RS_TEST`,
> crate-wide `#[test]` total, guard-table rows 151/141 (up only). Shipped
> red-then-green: commit 1 asserts deliberately tight wrong ceilings and
> fails loudly on all three; commit 2 sets the measured session-start
> truth (19,906 lines / region from L6,565 / 234 route sites / 139 pins —
> the roadmap's v1.28.35-era numbers were stale; re-measured per the
> executor stop-rule). Scope 2: the route-coverage table (151 paths) +
> route-authz table (141 gates) are data in `src/route_guards.rs` —
> declared from main.rs, NOT handlers/mod.rs, because docs_truth skips
> files included by `#[cfg(test)] mod X;` in their declaring parent and a
> handlers-side decl would have exempted the tables from the comment
> guard. Tests consume the consts; verdicts identical; row counts floored
> in the ledger. Scope 3: ten pure-unit pins relocated verbatim to their
> subjects — 6 into `handlers/mod.rs` (authorize ×3, audit_scope ×2,
> typed-edge), then auth_tokens→config, temporal→temporal,
> trace_caps→trace, eval_metrics→eval. Every router/DB/main.rs-subject
> suite stays (screen family waits for its subject's M2 promotion; the
> `test_db()` mass moves at the family/lib-flip milestones). The floor
> discipline bit once in development: relocating a family without the
> same-commit ledger edit failed exactly as designed. Landed truth:
> 19,906 → 19,282 lines; region 13,342 → 12,712; route ceiling frozen at
> 234 (routes move in Vaulting, not Scaffold). Full suite 1,314 passed /
> 7 ignored per commit; clippy -D warnings clean; lipstyk diff-strict
> green; wire artifacts diff-empty (openapi.yaml, route-coverage,
> route-authz, x-api-version); /health smoke green. Ceilings (honest): no
> chunker/capacity pure pins existed in main.rs to relocate; no behavior
> change of any kind — the whole release is proving that with gates.
> See CHANGELOG.md §[1.28.54]. Predecessor: v1.28.53 "Triage" — the
> review queue is domain-scoped for real (full note in CHANGELOG
> §[1.28.53]; no AGENTS.md row was cut for it).

> **Version note (retained predecessor):** **v1.28.52 "Cornerstone" — THE
> FIN.** The Foundation Line is complete and machine-enforced. AMENDMENT
> first: v1.28.51
> shipped with gate.rs (78) still holding SQL, so the milestone opened
> with the AGENTS.md-prescribed Masonry-class extraction of the final
> vein — a new `service::gate` core, six surfaces, six commits, full
> gate + baseline-row-lowered per commit (78 → 68 → 66 → 59 → 57 → 21 →
> 0: the review-queue read, the creation insert + conflict pre-check,
> the expire/reject family, the edit path, the approve family with the
> decision CAS one-defined across six branches and the article state
> CAS typed so `public_slug_taken` keeps its 409, and the export
> bundle). Then scope 1: the enforcing flip — `SQL_BASELINE`, the floor
> pin, and the allowlist machinery DELETED; `no_sql_in_handlers_enforced`
> walks `src/handlers/` recursively and fails on ANY statement match
> (production, test, or comment residue), with a ≥30-file sanity against
> the vacuous pass and a counter self-pin (`sql_statement_counter_still_fires`).
> Scope 2: the transport-free grep takes its line-plan name
> `service_layer_free_of_http_types` (born a hard error at Plumb — there
> was never a warning phase); both guards ride CI via the test jobs.
> Scope 3: `docs/architecture.md` gains the two layer rules, the
> request-flow diagram, and the seam table; this file's Architecture Law
> points at it. Scope 4: the Foundation Line report appended to
> `docs/AUDIT.md` (pin counts, eval floor, smoke matrix, wire + schema
> identity). Quirk preserved verbatim + filed: the translation CAS's
> `decided_at = datetime('now')` (SQL-side clock) needs a pin or fix.
> Full suite 1306 → 1308 passed / 7 ignored; clippy green (bench); the
> compliance-pack test run from the Confluence note still owed before
> push. See CHANGELOG.md §[1.28.52]. Predecessor: v1.28.51

> **Version note:** **v1.28.51 "Confluence" shipped 2026-09-02** — the
> long tail: every handler file EXCEPT gate.rs drains to ZERO embedded
> SQL; the inventory floor moves 241 → 78 with the single remaining row
> `("gate.rs", 78)`. Sixteen files across 15 extraction commits, one
> commit per file in roadmap order, full gate per commit, baseline row
> lowered in the same commit as each move. New/extended cores:
> service::procedure (store tx: root → per-chunk quarantine flags →
> steps → next_step edges skipped on a quarantined root; step-chain/
> meta/decision reads; best-effort vec-shadow writes), service::ump_ops
> (urn lookup, supersession read, raw relations, soft-forget block
> flag+hash-only-tombstone+in-tx-audit, consent-denial audit helper
> moved WITH its pin), service::forget (single-chunk erasure; tombstone
> only when a row actually deleted), service::suggest (last-wins
> feedback upsert, fail-open existence fence, retyped off HandlerError;
> grouped outcome counts), service::compliance (best-effort oversight
> write, evidence counts unwrap_or(-1), legacy-JSON RoPA read, RoPA
> upsert with in-tx audit), service::art30 (register reads: fail-the-
> request categories, best-effort connector/DSAR, fail-open lifecycle),
> service::webhook_ingest (kb-feedback flood/finding/hot-count, Signal
> flood bound, draft-approve read + digest-gated UPDATE), workflow side:
> state run-row reads + open_run, outbox steering inbox + lineage
> reads, scoreboard.rs NEW (runs page, fail-closed hash linkage,
> aftersales cohort, score_units_now + its test module), kcs article
> lifecycle, valet brief projections, relay handover reads, crew
> touch + skills proposal, channels user-map proposal + shared
> seen-window flood count; plus role::defined_count,
> capacity::knowledge_docs (fail-open),
> legal_hold::first_missing_id, service::recall::chunk_for_verify.
> The twelve borrowed fence pins in clients.rs moved onto
> service::register's test module (fixtures already there from
> Terrace); the valet brief tests moved onto workflow::valet's test
> module; the scoreboard tests moved with their fns. CAS discipline,
> verify-before-serve UMP orchestration, digest gates, probe-blind 404
> families: byte-identical, pinned through every move. Body-scan +
> authz + read-seam guards passed with no additions. Well_known.rs
> verified 0-SQL (the drained-file template). Dup-guard + transport-
> free greps green. Full suite 1306 → 1316 passed / 7 ignored; clippy
> green on bench/default/otel/compliance-pack (clippy only); CI
> dry-run green (crates, steward-harness, default-features); lipstyk
> diff-strict clean (one string-params finding fixed);
> openapi.yaml diff-empty; schema untouched at 1.28.45.
> CEILINGS (honest): **the allowlist did not reach empty — gate.rs
> (78: 50 prod + 28 test, the HITL proposal engine) was the one
> straggler and blocked the v1.28.52 enforcing flip; resolved by the
> Cornerstone amendment (the final-vein extraction ran inside
> v1.28.52, before the flip)**.
> The compliance-pack TEST RUN is deferred (clippy green; three
> interrupted attempts — one-time full rebuild); run before push. The
> compliance-pack's own pins were made compilable (full 9-column
> evidence fixture; the decision test seam is
> cfg(any(test, feature = "compliance-pack"))). Known-flaky backup
> tests raced twice (console-seam test sets BRAIN_CONNECTOR_CONFIG_DIR
> without the env-lock; pre-existing test-infra, untouched). Filed
> follow-ups: forget writes no audit_events row (tombstone-only
> evidence); the Signal digest-mismatch Denied audit still rolls back
> with its tx; put_run_state's 200-body revision quirk (run-id-as-
> revision) preserved verbatim, needs a pin or fix. See
> CHANGELOG.md §[1.28.51].

> **Version note:** **v1.28.46 "Plumb" shipped 2026-08-28** — the Foundation
> Line opens: the release that ships the LINE'S MACHINERY and the first
> vein, zero features/endpoints/schema/wire changes by design. The debt lock
> (`sql_inventory_baseline_freezes_the_debt` in `src/service/mod.rs`)
> freezes the per-file SQL-statement inventory of `src/handlers/*.rs` at the
> measured numbers (445 across 29 files; case-insensitive non-overlapping
> occurrences of `SELECT `/`INSERT `/`UPDATE `/`DELETE FROM`): any file
> above its frozen count, or SQL in an unlisted file, fails CI; below-
> baseline progress prints deltas — the lock stops regrowth, it does NOT
> force pace (the enforcing flip is the line's LAST milestone). The service
> layer (`src/service/`) opens as the convergence target: the layer contract
> as docs + greps-as-tests (services take connections, never pools/state/
> transport types; own SQL + bounds + FK-children + audit-per-write inside
> the caller's tx; typed errors, handler-side HTTP mapping). First
> extraction exemplar: the govern retention family → `src/service/
> retention.rs` (override upsert + report queries + evidence audit inside
> ONE WorkflowTx — closing the unevidenced-write window where the audit
> rode a second pooled connection after the commit; the upsert set is now
> atomic). `govern.rs` 18 → 6 embedded statements (−12 incl. moved tests);
> pins 993 → 1000 (+7: the lock, its floor pin, the transport-free grep,
> the byte-for-byte legacy fixture captured pre-move, the in-tx audit law
> + rollback twin, the fence pin, and the report pin moved verbatim).
> Wire artifacts byte-identical; schema untouched at 1.28.45. Ceilings:
> report rows stay legacy JSON maps (byte-for-byte pin outranks the
> domain-type aspiration); the baseline counts comments + test seeds
> (substring lock, not a precision instrument); kind charset validation
> stays handler-side (handler-typed) with the core fence covering
> bounds + emptiness only. See CHANGELOG.md §[1.28.46].
> Predecessor: v1.28.45 "Herald" — Slack + Teams as the console's annexes

> **Version note:** **v1.28.45 "Herald" shipped 2026-08-27** — Slack +
> Teams as the console's OPERATOR ANNEXES: the bridge (one binary, two new
> adapters, config-off default) dials Slack Socket Mode with NO inbound
> listener (source-grep pinned) and serves Teams via Bot Framework +
> Adaptive Cards with BF-JWT verify before parse; mapped-channel messages
> become screened threaded notes; pending proposals render as Blocks/Cards
> with the DIGEST in the block and approve/reject actions that MUST carry
> it (bridge-side refuse-and-log, then the kernel re-verifies inside the
> byte-identical approve verb — two independent enforcement points); ONE
> new kernel seam `POST /webhooks/channel/{kind}/console` (pending/decide/
> due/crank, HMAC, closed vocabulary) relays the console in; the Slack user
> map is a proposal-maintained table (`channel_user_map`, schema 1.28.45;
> `/workflow/channel/user-map` files the proposal, approval is the ONLY
> writer; opaque platform ids, roles resolved at file+apply, audited per
> change); Relay handover offers enqueue `channel/ping` rows the drain
> delivers in-channel with the I-PASS completeness state (refs only,
> unmapped = audited loud + consumed); mapped operator activity feeds Crew
> presence as the new `channel` activity KIND only, DPO-switch governed at
> the write. Server-diff verification: the plan expected zero server diff;
> the no-brain-token law + bearer-mode 401s made an additive seam NECESSARY
> (ledger row in CHANGELOG §[1.28.45]); approve/reject handler machinery is
> REUSED unchanged. Routes additive: `/webhooks/channel/{kind}/console`,
> `/workflow/channel/user-map`. Ceilings: replayed decides return the
> console's 404 (the `{moved:false}` receipt stays channel/template-
> specific); slash approve acts only on proposals the bridge rendered this
> session; `/brain due` lists, never fires; Teams drain delivery uses the
> standard regional BF host (no per-activity serviceUrl echo on the drain);
> no channel-side handover accept/decline. See CHANGELOG.md §[1.28.45].
> Predecessor: v1.28.44 "Caravel" — WhatsApp for Business as a governed edge

> **Version note:** **v1.28.44 "Caravel" shipped 2026-08-27** — WhatsApp for
> Business as a GOVERNED EDGE: `tools/channel-bridge` (standalone Rust crate,
> config-off default) owns the public webhook surface — answers the
> `hub.challenge` handshake ITSELF (never the kernel), verifies every POST's
> `X-Hub-Signature-256` raw-body constant-time + length-checked BEFORE any
> parse, then forwards verified envelopes over the Switchboard HMAC seam.
> The 24-hour window binds `reply_window_allows` exactly: outside it ONLY
> approved `channel/template` acts WITH standing consent pass (free-form
> refused even when approved+consented); template sends are digest-bound
> HITL proposals dispatched in the approval tx (replay-safe `{moved:false}`);
> business-initiated = template + consent + approved proposal ALL THREE,
> cold contacts auto-open their care case with the window CLOSED; status
> receipts land as `case/channel_status` lineage events (refs never bodies);
> quality tiers throttle deterministically with FRESH=most-restrictive and
> metadata-only downgrade alerts; attachment SHA-256s ride ON the note while
> bytes stay quarantined edge-side. Schema UNCHANGED at 1.28.44; no routes.
> Ceilings: TLS terminates at the operator proxy; tier taxonomy pinned per
> graph_api_version at deploy; parameterless templates only; kernel does not
> pace by tier. See CHANGELOG.md §[1.28.44].
> Predecessor: v1.28.43 "Switchboard" — the channel seams, Signal first-class

> **Version note:** **v1.28.43 "Switchboard" shipped 2026-08-27** — the
> channel bridge framework: `/webhooks/channel/{kind}` + `/drain`
> (Standard-Webhooks HMAC, replay-capped on bridge+external_id),
> `channel_threads` tenant-scoped thread map (schema 1.28.43), `channel/out`
> outbox topic gated by `enqueue_out` type-fence + reply-window + consent,
> tokenless mount registration with server-recomputed config digests, and
> `tools/signal-gateway` promoted to the first-class Signal edge.
> See CHANGELOG.md §[1.28.43].
> Predecessor: v1.28.42 "Valet" — the personal AI assistant, dogfooded
> (full note retired to `docs/AGENTS_HISTORY.md`)

> **Version note:** **v1.28.41 "Terrain" shipped 2026-08-26** — G8 + the
> series-exit gate closed, ending the Conformance Line. Tiers are tested
> config: checked-in profiles (`deploy/tiers/t{1..4}.env`), the expanded
> T1–T4 guide in `docs/deployment.md` (per-tier env matrix, sizing, cron
> cadences, upgrade path), a CI tier-smoke matrix job that boots each
> profile end-to-end, and two meta-tests (`guide_and_profiles_never_drift`,
> `tier_profiles_boot_and_pass_smoke`). The CONTACT_CENTER_STANDARDS matrix
> is re-audited green-or-ceiling-marked, pinned by
> `series_exit_gate_checklist_green_or_ceiling_marked`; AUDIT.md carries the
> close-out. Schema UNCHANGED at 1.28.41; no route changes; CI-only matrix
> is independently disableable. Ceilings: no installer wizard; tier smoke
> proves config boots, not sizing; G10 watch item stands.
> See CHANGELOG.md §[1.28.41].
> Predecessor: v1.28.40 "Handshake" — G5+G7, the WFM seam + workload views
> **Version note:** **v1.28.40 "Handshake" shipped 2026-08-26** — G5+G7 of
> the Conformance Line closed: the WFM seam is first-party and versioned
> (`wfm/1`, additive-only, two-way pinned against `docs/wfm-seam.md` via
> `wfm_schema_is_versioned_and_additive_only`; generic `brain wfm-import`
> CSV/JSON adapters; skills import lands as HITL proposals), and workload
> visibility completes the people picture (`GET /ops/workload` lineage-only
> per-principal burden + fatigue signals that alert and never reassign;
> `GET /ops/coverage` joins skills to worktype demand). Schema UNCHANGED at
> 1.28.40; routes additive: `/ops/workload`, `/ops/coverage` (both Read).
> Ceilings: gate-backlog attribution is domain-lineage-only (proposals have
> no domain column); fatigue alerting is view-only (no push channel); no
> forecasting/adherence/reassignment; vendor-specific WFM connectors later.
> See CHANGELOG.md §[1.28.40].
> Predecessor: v1.28.39 "Access" — G3+G4, WCAG 2.2 AA as hard gates
> **Version note:** **v1.28.39 "Access" shipped 2026-08-26** — G3+G4 of the
> Conformance Line closed: the six WCAG 2.2 AA criteria new in 2.2 are
> release-blocking automated gates over the console (`focus_never_obscured_
> by_docks`, `drag_alternatives_exist_for_every_drag`, `target_size_floor_
> 24px_enforced_by_classes`, `help_entry_consistent_across_panels`,
> `no_redundant_entry_in_approval_flow`, plus ACR-honesty and RTL/
> pseudolocale pins), the shell ships ONE consistent help entry (3.2.6),
> the layout uses logical CSS properties so `ar` mirrors fully under
> `dir="rtl"`, and `en-XA` elongation is budgeted at test time via the
> `fluent-pseudo` dev-dependency. Checklist/ACR rows now cite their pinning
> tests. Schema UNCHANGED at 1.28.39; no route changes.
> Ceilings: axe-browser CI gate stays operator-run; exact-match locale
> negotiation (no BCP-47 subtags); manual SR matrix rows still unchecked.
> See CHANGELOG.md §[1.28.39].
> Predecessor: v1.28.38 "Lexicon" — G2, the normative metric dictionary

> **v1.28.37 "Advocate" (2026-08-26)** — G1 closed: the whole ISO 10002
> complaint lifecycle on the shipped machinery (the register IS the audit
> chain — no parallel complaint database). The published complaints policy
> (`knowledge.source='complaint_policy'`) renders as the public
> `how-to-complain.html`, linked from EVERY status-page footer;
> `kb build --with-case-status` refuses loudly without a published policy.
> Acknowledgment is its own audited step (`/workflow/runs/{id}/complaint/ack`)
> with an idempotent overdue sweep on the alert bus
> (`/workflow/complaints/ack-sweep`). The closure confirm-gate is wired into
> the lifecycle itself; safety-relevant complaints hit the GPSR path before
> the complaint keyword; the monthly register extract (dispositions, ack-SLA
> attainment, ADR referrals) rides the SAME audited `calibration/sign` row.
> Schema UNCHANGED at 1.28.37 (additive code only).
> Ceilings: no certification claim; ack sweep is on-demand/cron (no internal
> scheduler); register covers the trailing window at sign time only.
> See CHANGELOG.md §[1.28.37].
> Predecessor: v1.28.36 "Keystone" — the last three Order-of-Care gaps

## Architecture Law (the steering)

- **Two layers, one pattern.** Handlers are protocol adapters ONLY: parse →
  OptPrincipal → `authorize`/`authorize_role` → one `spawn_blocking` →
  service-core call → read-seam shaping → response. ALL SQL, bounds/caps,
  FK-children ordering, and invariants live in domain modules
  (`src/workflow/*`; `src/service/*` from v1.28.46) taking `&Connection` /
  `WorkflowTx` — never Pool, AppState, or axum types.
- **Audit-per-write.** Every mutation emits its hash-chained audit row INSIDE
  the caller's transaction (`record_tenant`, SAVEPOINT-nested): a transition
  and its evidence commit or roll back together.
- **Fail-closed everywhere.** Authz gates, poisoned locks, unreadable config,
  quarantine, legal holds, DPO switches: error paths deny loudly; silence is
  never certified (`let _ =` on writes is forbidden).
- **Read seam unconditional.** Every emitted text field passes
  `sanitize_read`; untrusted content is screened + stripped at WRITE time
  (`screen_content` pattern); LLM-facing payloads are fenced.
- **Bounds law.** Every list surface capped, every input bounded, every cap
  pinned by a test; SQL never decides a row's fate (Rust-side pure arbiter).
- **Wire-contract discipline.** Any route change ships openapi.yaml +
  route-coverage + route-authz guard-table entries in the same commit; the
  `x-api-version` stamp moves only when the wire contract moves.
- **Convergence.** New code is ALWAYS a service core; legacy extraction is
  trigger-based (piggyback / pre-storage-adapter deadline); handler-side SQL
  is ENFORCED ZERO by the `no_sql_in_handlers_enforced` guard (Cornerstone —
  any match in `src/handlers/**` fails CI; there is no allowlist). The law's
  public statement lives in `docs/architecture.md` (the two layer rules, the
  request-flow diagram, the seam table); this file is the operational
  summary.
- **The thin binary (Capstone).** main.rs is WIRING ONLY (bootstrap →
  compose → serve), pinned ≤ 300 lines with no cfg(test) region (the mass
  lives in `tests/`); route registrations live ONLY under
  `src/server/router/**`; `server::bootstrap` stays protocol-free (no axum
  types). Each clause is machine-checked by the spire gates in
  `src/spire_inventory.rs` (`route_registrations_live_only_under_router`,
  `bootstrap_stays_protocol_free`,
  `spire_inventory_freezes_the_thin_binary`); the one fenced exception is
  `src/bin/mcp.rs` — a separate binary's single-endpoint /mcp protocol
  edge, pinned at exactly one site. The line's measured record:
  `docs/AUDIT.md` (the Spire Line close-out).

## Service-layer convergence (Foundation Line, v1.28.46+)

- New code ALWAYS builds as a service core (`src/service/<domain>.rs` owning
  SQL + invariants + audit_write inside the caller's tx); handlers are
  protocol adapters only. Templates: `src/workflow/channel.rs` +
  `src/handlers/channel.rs`, and since Plumb `src/service/retention.rs` +
  `src/handlers/govern.rs` (the extraction exemplar).
- Legacy extraction is TRIGGER-BASED, never cosmetic: (1) piggyback — when a
  feature substantively touches `recall/gate/observe/domains/clients`, extract
  that surface's core in the same release (pins first, move second);
  (2) deadline — immediately before any v2.x storage-adapter work.
  (The historical freeze-and-burn machinery — the `sql_inventory_baseline`
  per-file table shipped in Plumb at 445 across 29 files, lowered line-wide
  by Quarry/Masonry/Terrace/Aqueduct/Confluence to 78 — is RETIRED: the
  Cornerstone extraction took gate.rs 78 → 0 and the enforcing flip deleted
  the table. `no_sql_in_handlers_enforced` in `src/service/mod.rs` now fails
  on ANY statement under `src/handlers/**`, recursively. Line plan:
  `IMPLEMENTATION_ROADMAP_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`;
  executor: `EXECUTION_PROMPT_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`.)

## Operational Context (read first)

**Paths**
| What | Where |
|---|---|
| Repo | `/Users/mark/Sites/brain-server` |
| **Live DB** | `~/.openclaw/workspace/brain.db` (override: `BRAIN_DB_PATH` env) |
| Binaries | `~/.local/bin/{brain-server, brain, mcp, bench, brain-connector-stub, brain-migrate-rehearse}` (+ `brain-connector-gh`/`brain-connector-crm` when their features are built) |
| Auth token | `~/.config/brain-server/auth-token` (0600; written by `install-service.sh`) |
| Launchd plist | `~/Library/LaunchAgents/com.brain.server.plist` |
| Logs | `~/Library/Logs/brain-server.{log,err.log}` |
| Config module | `src/config.rs` (all constants + env var resolution) |

**Commands**
```sh
# Build all 4 binaries
 cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench

# Tests + quality gates (always run with --features bench — the bench binary is feature-gated)
cargo test --features bench                                  # current count: scripts/badges.sh (2,818 passed at HEAD 7001e478, 2026-09-30)
cargo clippy --all-targets --features bench -- -D warnings   # zero warnings enforced
cargo fmt --check

# CI DRY-RUN — REQUIRED before every push/release (v1.28.29 lesson):
# the local gate alone does not cover the default-feature build or the
# side workspaces CI runs on Ubuntu. Run these before pushing:
export RUSTFLAGS="-D warnings"
cargo clippy --all-targets -- -D warnings                    # job lint-test (default features)
cargo test --all-targets                                     # job lint-test (default features)
cargo test  --manifest-path crates/Cargo.toml --all-targets          # engine-crates
cargo clippy --manifest-path crates/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/steward-harness/Cargo.toml         # steward-harness-gate
cargo clippy --all-targets --features otel -- -D warnings    # otel-gate
cargo test  --all-targets --features otel
# client-gate only when client/ touched (slow: wasm + desktop headers).

# FEATURE LANES — every `[features]` entry has a job in .github/workflows/ci.yml.
# Six did not, and two of those did not compile (`injection-classifier`,
# 9 errors; `neural-embed`, 2 errors), so the breakage accumulated unnoticed.
# A lane that does not exist looks exactly like a lane that passes.
# `tests/feature_lane_pins.rs` derives the feature list from Cargo.toml and
# fails on any feature no lane builds — do NOT delete that pin; it is the only
# thing making the six lanes below durable.
for f in compliance-pack multivec injection-classifier neural-embed loom rerank-tier; do
  cargo clippy --all-targets --features "$f" -- -D warnings
done
# A lane proves a feature COMPILES. It does not prove the feature WORKS — the
# ONNX/model artifacts are not fetched in every lane, and no lane exercises
# layer-2 scoring or a live rerank.

# Repo briefing — ONE-SHOT agent overview before any work (runs <1s):
scripts/repo-brief.sh
# (versions, HEAD, dirty paths, main.rs structure, env/route/CLI counts,
#  which guards exist vs ABSENT, stale-marker probe of living docs)

# v1.28.31 lesson — these two CI jobs are NOT covered by the pre-push hook
# (it fmts only the server tree) and burned the first Charter push. Run them
# locally as part of the gate, ALWAYS before push:
scripts/lipstyk-gate.sh
 cargo fmt --manifest-path client/Cargo.toml -- --check
# lipstyk-gate: the CI watchdog's trap-proof wrapper — the raw
# `lipstyk --diff "$(git rev-parse origin/main)"` form goes VACUOUS once you
# push (origin/main == HEAD → empty diff → exit 0 having scanned nothing) and
# it never sees UNTRACKED new modules (both teeth bit on the Terrace push;
# CI caught what the local run waved through). The wrapper pins the base
# (pre-push merge-base, else HEAD~1, else pass the old tip / release tag),
# marks new files intent-to-add, and REFUSES to pass vacuously. lipstyk:
# changed lines must add no diagnostics (Box<dyn Error> returns in
# tests included — helpers inside #[cfg(test)] mods are still scanned; use
# .expect() like sibling tests instead of Result-returning test bodies).

# Tag pushes do NOT re-run CI: the branches-only push filter already excludes
# tags (`tags-ignore: ['v*']` pins that intent explicitly). The tag triggers
# release.yml only. Run the dry-run gate ONCE per main push, not per tag.
# release.sh BLOCKS on green CI for the tagged SHA (gh run watch, fail-closed):
# the tag re-runs no tests, so the main-push CI run is the only automated
# bridge between "pushed" and "shipped" — red or unfinished = no tag. The
# release builds are 4 parallel per-target jobs (linux x86_64/aarch64,
# macOS arm64/x86_64); the verify-required-assets gate refuses to publish if
# any primary brain-server binary is missing.

# README badges — NEVER hand-type them; regenerate from the real build:
scripts/badges.sh                         # prints version/test/UMP/SBOM badge block
scripts/badges.sh --selfcheck             # drift + release-checklist completeness guard
# (derives version from Cargo.toml, test count from cargo test --features bench,migrate)

# Install + restart launchd service (also installs CLI binaries, strips macOS provenance xattr)
scripts/install-service.sh

# Health + stats against running server
brain doctor
brain status
```

**Runtime**
- Server: launchd-managed, `127.0.0.1:8765`, `KeepAlive=true`, `RunAtLoad=true`.
- Auth: bearer token via `AUTH_TOKEN_FILE` → `AUTH_TOKEN` (server) / `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → default file (CLI). Off by default if no token resolves.
- macOS Sonoma+: newly-copied executables get `com.apple.provenance` xattr → Gatekeeper SIGKILLs on first exec (exit 137). `install-service.sh` strips it; manual `cp` does not.

**OpenClaw integration**
- brain-server is the **memory backend** for openclaw; the plugin (`brain-server/plugin/`, TypeScript) calls `/recall` each turn via openclaw's `before_prompt_build` hook.
- openclaw config: `~/.openclaw/openclaw.json` (plugin block at key `brain-server`).
- The `brain` CLI is built from this repo's `Cargo.toml` (`[[bin]] name = "brain"`), **not** from openclaw.

**Customer-domain assets live in a private repo (2026-08-19)**
- The case-classification work is **not** part of brain-server: plan, customer
  taxonomy, routing matrix, playbooks, spine builder + outputs moved to a
  **private** repo (not named here — this file is published to the public
  portfolio, and naming the customer is a confidentiality problem
  independent of the assets themselves). Never commit customer-domain content
  here — `.gitignore` covers `spine/` + the two spine scripts defensively.
- This repo stays the vendor-agnostic product: graph-default-on recall, the
  feature-gated `src/classify` engine (when built), proposals/suggest/audit.
  The taxonomy is *data the engine consumes*, owned by the private repo.

**Steward engine IP lives in a private repo (2026-08-21)**
- The advanced architecture docs (Steward architecture, harness audit, state
  mapping, port specs, rubric pin, diagnostics loop, compliance mapping) are
  **MemorySteward LLC IP** and live in the **private** repo `brain-steward-ip`
  (local checkout `~/Sites/brain-steward-ip`). Never commit them here —
  `.gitignore` defends the names defensively (same pattern as the
  customer-domain repo).
  The public repo ships the substrate only; the engine crates (`brain-engine-sdk`
  5,887 top-level + `host/`/`pure/` lines, 8,352 total / 127 tests at v1.28.23,
  plus the five `*-core` engines) are PUBLIC, REAL code — measured 2026-09-15.
  (The old "scaffolds stay public while empty" claim was stale — do not repeat
  it; the SDK is built, kernel-consumed, and dependency-free by design.)
- **The Steward-line implementation plans (1.27.32 → 1.27.42) live in
  `brain-steward-ip/plans/`** with the master `ROADMAP_1.27.x.md`. This repo
  keeps **no** kernel-root plan files: every `IMPLEMENTATION_PLAN_*`,
  `IMPLEMENTATION_ROADMAP_*`, `ROADMAP.md`, `DESIGN_*`, `CLIENT_ROADMAP.md`,
  `REALITY_CHECK.md`, `MARKETING_PLAN.md`, `TODO.md` and `USE_CASES.md` that
  used to sit here was **moved to `brain-steward-ip/plans/archive/` on
  2026-10-04** — see `plans/archive/KERNEL_ROOT_PLANS.md` and
  `KERNEL_ROOT_NONPLAN.md` there for provenance and the list. None was ever
  tracked (`.gitignore` 55–70), so the public tree carried 222 root `.md` files
  while git held 16; the root is now 19, all of them real project docs.
  **Do not re-add one "for context"** — the private archive is the home, and
  `tests/external_claim_pins.rs` discovers the claim-sensitive ones by CONTENT
  rather than by path, so a copy here is not what any gate reads.

---

## Known issues (open)

- ~~**CI gap — tests run on x86_64 runners only.**~~ **CLOSED 2026-09-15
  as NOT-APPLICABLE:** no Jetson/fleet deployment exists (brain-server is
  not installed on any aarch64 host), so the advisory's precondition —
  "before fleet deploys" — is absent. The release matrix still
  cross-builds aarch64 and CI still executes tests on x86_64 only;
  REOPEN at the first aarch64 fleet deployment, and then as an ARM-hosted
  CI test lane, not a manual smoke.
- ~~**Historical token leak (openclaw-side).**~~ **CLOSED 2026-09-09:** the
  final `transcript_events` copy of the agent token was redacted + VACUUMed
  (backup: `~/Downloads/audit/openclaw-agent-pre-purge-*.sqlite`, 0600), the
  agent token was ROTATED (old value dead → 401; watcher reload audited), and
  the discovery of the night: the openclaw gateway's env carried the OPERATOR
  token — the plugin had been authenticating as full superuser. It now runs
  on the AGENT token (`service-env/ai.openclaw.gateway.env`), so the Twokeys
  agent boundary is ENFORCED live (purge → 403 proven). The operator token is
  unchanged. The `${BRAIN_SERVER_AUTH_TOKEN}` placeholder in
  `openclaw.json` resolves from the same env file. Hygiene:
  `~/.config/brain-server/auth-agent-token` (the installer's designated
  `AGENT_TOKEN_FILE` target, unreferenced by the running service) was
  re-synced to the live agent token so a future installer re-run or
  `AGENT_TOKEN_FILE` adoption cannot resurrect the dead value.

History: per-release detail lives in `CHANGELOG.md` §[version] and
`docs/AGENTS_HISTORY.md` (load on demand). Nothing older than the note above
is kept here.
