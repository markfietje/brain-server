# Input Hygiene and Transport Limits

> Era pin: **v1.29.2**, verified **2026-10-06** against `src/http_limit.rs`,
> `src/hygiene.rs`, `src/pii_mask.rs`, `src/strip_invisible.rs`.
> Complements [security.md](./security.md) (posture summary) and
> [17-injection-screen.md](./research/17-injection-screen.md) (the two-layer
> screen in depth) — it does not re-argue either. The screen, gate, and fence
> appear here only where the four modules below plug into them.

## 1. Pipeline order (what runs where on ingress)

1. **Body cap** — `RequestBodyLimitLayer::new(config::MAX_REQUEST_SIZE)`
   (`1 MiB`, `src/config.rs`) applied in `src/server/router/mod.rs::app`,
   *before* the `import_router()` merge. The import dial raises only its own
   sub-router to `1 GiB` (`src/server/router/memory.rs::import_router`); an
   outer limit can never be raised by an inner one (tower-http
   eager-application pitfall — stated in both files' comments).
2. **Rate limit** — `rate_limit_middleware` (`src/server/router/mod.rs`) calls
   `http_limit::RateLimiter::is_allowed(&ip)` *outside* both auth layers, so
   `429` fires before any token work. Denial body:
   `{"error":"rate_limited","code":"rate_limited"}`.
3. **Capacity + content caps** — `measure_capacity` / `guard_capacity` may
   refuse with `507`; per-field ceilings `MAX_CONTENT = 1_000_000`,
   `MAX_SOURCE_PROMPT = 2048`, `MAX_SOURCE = 64`
   (`src/handlers/mod.rs`) are enforced at the write seams.
4. **Hygiene door** (`src/hygiene.rs`) — `/add` strips reasoning blocks;
   `/ingest/memory` runs the combined `clean` per entry. Curated
   `/ingest` and `/ingest/markdown` are deliberately **not** filtered here
   (operator-authored; false-positive risk — module doc law).
5. **Injection screen** (`src/screen.rs`) — the single `screen()` seam decides
   `Clean` / `Quarantine` / `Reject`. See
   [17-injection-screen.md](./research/17-injection-screen.md) for the full
   treatment; §6 below covers only the handoff points.
6. **Read seam** (`src/gate.rs::sanitize_read`, `src/fence.rs`) — storage stays
   verbatim; every emitted text field is transformed at read. Hygiene never
   rewrites history; cleaning stored rows is a separate sweep (module doc law).

## 2. `src/http_limit.rs` — load control at the edge

**Job.** Three mechanisms, no transport types: the per-IP `RateLimiter`, the
`ConnectionTracker` with its RAII `TrackerEntry` slot guard, and the two
watchdogs. Wiring lives in `src/server/bootstrap.rs`; the middleware lives in
`src/server/router/mod.rs`.

**Documented law.**

- Budget: `max_requests = 10_000`, `window = 60 s` (`RateLimiter::new`).
- Bounded memory: at most `config::RATE_LIMIT_MAX_KEYS = 10_000` IP buckets
  (`src/config.rs`); on the cap-hit path the oldest 25% of buckets (by newest
  timestamp) are evicted, then the new IP is tracked. The limiter keeps
  working instead of OOMing under spoofed-`X-Forwarded-For` cycling.
- Identity: the socket peer address by default; `X-Forwarded-For` is honored
  **only** under `BRAIN_TRUST_PROXY=1`, and then the **rightmost** entry (the
  one the trusted proxy appended) is used (`rate_limit_middleware`).
- Poison posture, stated in the lock-bound comments: limiter poison is
  **fail-closed** (deny); tracker poison is **fail-open** (skip the
  insert/remove, scan reads empty). The limiter decision under the lock is
  pure arithmetic — the clock is read before acquisition.
- `TrackerEntry` releases on **every** exit path (`Drop`: early `return`, `?`,
  panic unwind, ingest-timeout task drop) — the F-53 pin.
- Watchdogs: connection watchdog ticks every
  `CONNECTION_WATCHDOG_INTERVAL_SECS = 30`, flagging slots held longer than
  `CONNECTION_WATCHDOG_THRESHOLD_SECS = 300` (`src/config.rs`) via stderr.
  RSS watchdog uses the same cadence; two consecutive samples over the active
  envelope's `max_rss_mib` log `error!` (target `brain::rss`) and exit only
  with `BRAIN_RSS_RESTART=1` — default is log-only.
- `process_rss_mib` measures **this process's** RSS (per-process ceiling),
  returning `0` fail-open on lookup failure.

**Observe / verify.**

- Over-budget callers get HTTP `429` with the `rate_limited` code; distinct
  IPs are isolated (one user's exhaustion never denies another — pinned).
- `GET /metrics` carries the `brain_rss_mib` gauge
  (`src/server/router/core.rs`); `GET /health` reports the `capacity` object
  (`docs`, `max_docs`, `db_mib`, `max_db_mib`, `rss_mib`, `max_rss_mib`,
  `status`). Capacity and RSS behavior is also described in
  [configuration.md](./configuration.md) and [metrics.md](./metrics.md).
- Unit pins (`cargo test http_limit`): `test_rate_limiter` (10 000-allow /
  then-deny per IP), `rate_limiter_caps_tracked_ips_and_evicts_oldest`,
  `rate_limiter_evicts_oldest_quarter_and_stays_bounded`,
  `rate_limiter_decision_is_pure_under_lock`,
  `tracker_entry_releases_on_drop_and_panic`,
  `ingest_timeout_releases_tracker_slot`,
  `process_rss_mib_reports_plausible_process_footprint`; plus the
  `WINDOW_BUDGET_PROBE` router-level pin in `src/server/router/auth.rs`.

## 3. `src/hygiene.rs` — ingest-door capture hygiene

**Job.** Two pure transforms at the raw-text ingest doors, stopping the server
from silently storing model reasoning traces and foreign synthesis prompts:

- `strip_reasoning_blocks` — removes paired reasoning-tag blocks, including an
  unclosed trailing block (dropped to end-of-string, the conservative privacy
  choice).
- `should_skip` / `skip_patterns` / `clean` — drops an entry whose text starts
  with a configured `BRAIN_INGEST_SKIP_PATTERNS` prefix (the dream-prompt
  mechanism); otherwise returns the stripped text.

**Documented law.**

- Allow-list, not a detector: `REASONING_TAGS = ["thinking", "think",
  "reasoning", "reflection", "analysis"]` — the tags the audit proved leak.
  Extend the list as new delimiters appear; do not build a content
  classifier (module doc law).
- Matching: case-insensitive; open tags may carry attributes (`<tag …>`);
  `<tagx>` (longer identifier) never matches; non-matching text passes through
  verbatim, UTF-8-safe.
- Skip: case-**sensitive** prefix match on `trim_start`ed text; patterns split
  on commas/newlines, blanks ignored; unset/empty env means **no** patterns
  (opt-in, default unchanged).
- Placement: `/add` applies `strip_reasoning_blocks` only (single explicit
  text — no skip-pattern drop); `/ingest/memory` applies `clean` per entry
  (`src/server/router/memory.rs`).

**Observe / verify** (`cargo test hygiene`): `strips_paired_thinking_block_with_content`,
`strips_is_think_tag`, `strips_case_insensitive_and_attributes`,
`unclosed_block_drops_to_end`, `no_tags_passthrough_unchanged`,
`does_not_match_tag_prefix_of_longer_word`, `multiple_blocks_all_stripped`,
`should_skip_matches_configured_prefix`,
`should_skip_ignores_leading_whitespace_and_empty_patterns`,
`clean_drops_skip_matches_and_strips_others`. Behaviorally: ingest
`<thinking>trace</thinking>` prose via `/add` and read back the stripped
form; set `BRAIN_INGEST_SKIP_PATTERNS` and confirm matching `/ingest/memory`
entries vanish while siblings persist.

## 4. `src/pii_mask.rs` — deterministic masking primitives

**Job.** The canonical email / phone / card maskers and their unconditional
composition: `mask_email` → `[redacted:email]`, `mask_phone` (runs of 10–15
digits, separators ` -().+` allowed) → `[redacted:phone]`, `mask_card`
(Luhn-valid 13–19 digit runs, contiguous digits only) → `[redacted:card]`,
plus `luhn_ok` (ISO/IEC 7812, double-every-second-from-right) and
`redact_unconditional` (all three passes, no principal argument — the
public-artifact posture). Order is load-bearing: email first, then phone, then
card (the 10–15 range never overlaps a real 16–19 card, so the passes are
independent).

**Consumers (verified call sites).** `kb::sanitize_public` (the strict public
render seam) and the single-line OTel span scrub (`src/otel.rs`) call
`pii_mask::redact_unconditional`. The read gate (`gate::redact_content`,
principal-gated on `pii:read`) and the write screen
(`gate::screen_source_prompt`, unconditional, for persisted `source_prompt`
provenance) implement the **same vocabulary with local copies** in
`src/gate.rs`.

**Documented law — and the open divergence.** The module header claims one
definition for every path; the code today has two: `src/dup_guard.rs`
carries explicit `TODO(unify)` rows for `mask_email`, `mask_phone`,
`mask_card`, `luhn_ok`, and `count_digits` (`pii_mask.rs` canonical vs
`gate.rs` local copies). Treat the `[redacted:*]` placeholder vocabulary as
the contract and the duplication as tracked debt, not as a guarantee.

**Observe / verify** (`cargo test pii_mask`):
`redact_unconditional_masks_all_classes`, `luhn_rejects_bad_checksum`; on the
gate side, the `redact_content` / `screen_source_prompt` pins in
`src/gate.rs` (masked-vs-admin-vs-plain arms, multibyte masking arm). There is
no `pii_map` vault — removed in v1.20.19 per [security.md](./security.md).

## 5. `src/strip_invisible.rs` — the one invisible-Unicode boundary

**Job.** The single shared strip definition for the bidi / zero-width /
tag-block smuggling class, living in the lib so four surfaces close it
identically: the server screen, the MCP binary, the `brain` CLI, and the
client (module doc law; `screen.rs` re-exports the pair so existing paths are
unchanged).

**Documented law.**

- `strip_invisible` removes the canonical set: tag block `U+E0000–E007F`,
  variation selectors `U+FE00–FE0F` + supplemental `U+E0100–E01EF`, bidi
  controls (`U+200E/200F`, `U+202A–202E`, `U+2066–2069`, `U+061C` ALM — the
  Trojan Source / W3C TR#20 class), zero-width `U+200B/200C/200D/2060`,
  legacy `U+FEFF/2061–2063/00AD/034F`, plus `U+180E/115F/1160` and
  `U+FFF9–FFFB`. Idempotent + pure.
- `strip_control_chars` is deliberately narrower: C0 (except tab/newline),
  DEL, C1 — for terminal-facing output (CLI prints, MCP payloads) where an
  ANSI escape could script the operator's shell. NBSP is preserved.
- Render/output only — storage stays verbatim; legitimate invisible Unicode
  is preserved at rest (module doc law).

**Observe / verify** (`cargo test strip_invisible`):
`arabic_letter_mark_stripped`, `supplementary_variation_selectors_stripped`,
`existing_invisible_classes_still_stripped`, `control_chars_stripped_preserves_tab_newline`,
`control_strip_preserves_visible_unicode`, `strip_fns_idempotent`, and the
exhaustive `invisible_set_fixture_is_exhaustive_truth`, which asserts
`is_invisible` equal to `plugin/fixtures/invisible-classes.json` over every
scalar value — a class added or removed on either side fails the build.

## 6. Where `screen.rs`, `gate.rs`, and `fence.rs` meet these modules

Short handoffs only — the deep accounts stay in
[17-injection-screen.md](./research/17-injection-screen.md) and
[security.md](./security.md):

- `screen.rs` runs layer 1 on the **stripped** form
  (`strip_invisible(content.trim())`), so a bidi-wrapped phrase cannot dodge
  the blocklist while the classifier sees it clean; verdicts move only
  `Clean → Quarantine/Reject` after the strip. Posture is inspectable on
  `GET /health` (`injection_classifier` tri-state `on`/`off`/`absent`,
  `injection_classifier_loaded`, `injection_policy`, `allow_policy_bypasses`
  — `src/server/router/core.rs`).
- Quarantine stores flagged and excluded from retrieval until a human reviews
  (`GET /quarantine`, release/delete endpoints — `src/server/router/memory.rs`).
- `gate::sanitize_read` order is fixed and pinned:
  `redact_content → strip_invisible → strip_markdown_refs →
  strip_control_chars → strip_hostile_elements → strip_sentinels` (invisible
  first, sentinels last — both orders are PoC-pinned against heal/forge
  regressions). `review_digest` binds this read-canonical form, so any
  widening of the pipeline moves digests and fails outstanding approvals
  closed (`409`).
- `fence::wrap_fenced` enforces the same Fencepost invariant
  (`strip_invisible → strip_markdown_refs → strip_control_chars →
  strip_sentinels → wrap`) with no transform after the final sentinel strip.

## 7. Honest limits — what hygiene does NOT catch

- **Hygiene is an allow-list of five tag names**, not an AI-text detector.
  Novel reasoning delimiters, paraphrased traces without tags, and
  double-encoded payloads pass untouched. The screen's own ceilings apply
  behind it: one decode level, a finite five-language phrase table, and
  classifier budgets past which input is unscored — see
  [17-injection-screen.md](./research/17-injection-screen.md) §"Measured
  ceiling".
- **Two ingest doors are unfiltered by design** (`/ingest`,
  `/ingest/markdown`), and **history is never rewritten** — rows stored before
  a widening keep their bytes. The read seam is the backstop for old rows,
  not a rewrite.
- **Skip patterns are opt-in and brittle**: unset means nothing is dropped;
  matching is case-sensitive prefix-only, so rephrasing or leading-payload
  tricks bypass it. It stops known dream-prompt shapes, not synthesis.
- **Invisible-strip is a closed set**: widening it shrinks but never closes
  the smuggling gap (the screen stays a tripwire — `screen.rs` module law).
  NBSP is intentionally preserved; bare prose URLs are intentionally kept
  (only markdown link/image *constructs* are de-linked), so a
  "visit attacker.example" exfil vector in plain prose survives — that is
  model-discipline / host-contract territory.
- **PII masking is shape-heuristic**: non-conforming PII (short numbers,
  names, addresses, non-email identifiers) is not masked, and the
  `gate.rs` / `pii_mask.rs` duplication (§4) can drift until the
  `TODO(unify)` rows are closed.
- **Transport limits bound cost, not malice**: 10 000 req/min per IP is
  generous — it is a load control, not a scraping control. The 10 000-bucket
  cap evicts the oldest 25%, so sustained IP rotation churns buckets by
  design (bounded memory wins over perfect attribution). `BRAIN_TRUST_PROXY=1`
  shifts trust to the proxy-appended XFF entry — a misconfigured proxy
  re-opens spoofing.
- **Fail-open spots are chosen, not accidental**: tracker-poison fail-open,
  RSS-lookup fail-open (`0`), classifier-unavailable fallback to the
  mechanical layer. Each is named at its site; the `/health` posture echo (§6)
  is the mitigation that makes them legible. Only the rate limiter fails
  closed.
