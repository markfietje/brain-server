# §2 — MODE A FINDINGS (server Rust tree)

Every finding below was re-verified **by me at final HEAD `e9c71919`**, not taken from a leg.
Where a leg's claim was decisive I re-ran the attack and quote the executed output.

---

## F8-01 — HIGH — The Architecture Law's SQL guard is blind, and two violations already shipped

**Law claimed:** *"handler-side SQL is ENFORCED ZERO by the `no_sql_in_handlers_enforced`
guard — any match in `src/handlers/**` fails CI; there is no allowlist."* (AGENTS.md)

**The counter** (`src/service/mod.rs:104-151`) recognises exactly four openers: `select`,
`insert`, `update` (with an ASCII-whitespace right boundary) and `delete`…`from`. It is
structurally blind to `PRAGMA`, `VACUUM`, `REPLACE INTO`, `ATTACH`, `CREATE`, `ALTER`, `DROP`,
`REINDEX`, `ANALYZE`, `BEGIN`/`COMMIT`, and `WITH … INSERT`.

**Two live violations are in the tree right now:**

- `src/handlers/govern.rs:417-419` — `rusqlite::Connection::open(p)?` then
  `conn.query_row("PRAGMA integrity_check", …)`, inside `check_snapshot`. (Unix twin at
  `:425-427`.) Note `Connection::open`, not `open_with_flags(READ_ONLY)` — a surface whose own
  doc comment at `:354-355` calls itself "Read-only".
- `src/handlers/domains.rs:261` — `let _ = conn.execute_batch("VACUUM;");` inside
  `delete_domain`'s `spawn_blocking`.

**Decisive proof — I ran the guard with both present:**

```
$ cargo test --features bench,migrate --lib no_sql
running 1 test
test service::pins::no_sql_in_handlers_enforced ... ok
test result: ok. 1 passed; 0 failed
```

**Green.** The law's only mechanical enforcement passes with two violations live.

**Exploit path:** a committer adds a handler-side *write* — `REPLACE INTO knowledge …`,
`PRAGMA journal_mode=…`, `VACUUM INTO '…'` — and CI stays green. The keyword list reads as
exhaustive to a reviewer, which is exactly why this survived.

**Fix:** invert the guard rather than extend the needle list. Deny *any* direct rusqlite
surface in `src/handlers/**` (`Connection::open`, `Query`/`Statement` methods, `params!`) and
require handlers to name only `crate::service::*` / `crate::workflow::*` connection-taking
functions. Keyword lists cannot enumerate a language; a type boundary can.

**Law clause:** Convergence — "handler-side SQL is ENFORCED ZERO."

---

## F8-02 — HIGH — The RBAC middleware enforces coverage only; two of three claimed properties are unreachable

**Claim** (`src/server/router/auth.rs:49-54`): the middleware enforces *"a matched, non-public
route with no row in the shared gate table is refused as `route_ungated` … the agent principal
class is refused on Admin rows by class; and a capability in the frozen deny-only class is a
permanent refusal."*

**Oracle** (`src/authz/policy.rs:198-229`, read verbatim) — `decide_gate_verdict` reads only:

- `gate.public` → `Defer(PublicPath)`
- `gate.permits_method(method)` → `Deny(MethodNotPermitted)`
- `gate.required_capability` → `Deny(CapabilityDenyOnly)`

**It never reads `gate.required_action`** — the field carrying all 200 rows' Read/Write/Admin
declarations. I verified this by grep: `required_action` appears in `src/authz/` only at
`gates.rs:152,164,208,212` (the table constructor + its test) and `policy.rs:68,83,256`
(struct/constructors). The oracle reads none of it.

**The other two arms are unreachable in production**, because the only production constructor
hardcodes them away (`src/authz/gates.rs:163,167`):

- `method_policy: MethodPolicy::Any` ⇒ `MethodNotPermitted` can never fire.
- `required_capability: ""` ⇒ `CapabilityDenyOnly` can never fire.

**Net runtime effect:** the only denial this layer can ever produce is `route_ungated`. The
action column — 200 rows — is documentation.

**Partial honesty, and why it still fails:** `policy.rs:205-220` openly documents that the
agent-class arm was *deliberately removed* after two matrix rows measured it. That reasoning is
sound — but it directly contradicts `auth.rs:53-54`, which still claims the middleware refuses
the agent class. **The prose survived the code change that invalidated it.**

**Anti-vacuity:** `src/authz/gates.rs:206-214` (`r47_gate_rows_read_their_declared_action`)
reads `required_action` back out of `gate_for` and asserts its value. Delete every
enforcement use of `required_action` and **the pin still passes** — it asserts its own premise.

**Fix:** either make the oracle compare `required_action` against the principal's scopes, or
delete `required_action` and the two dead `DenyReason` variants and correct `auth.rs:49-54` to
say "coverage only." Do not leave a doc claiming three properties over one.

**Law clause:** Fail-closed everywhere — "error paths deny loudly; silence is never certified."

---

## F8-03 — HIGH — A 408 is returned while the abandoned write still commits

`src/server/router/mod.rs:217-220` wraps every route in
`TimeoutLayer::with_status_code(REQUEST_TIMEOUT, 30s)`. Every DB-touching handler awaits
`tokio::task::spawn_blocking` (e.g. `src/handlers/domains.rs:249-265`). **Tokio's blocking pool
is not cancellable** — dropping the `JoinHandle` does not stop the closure; it runs to
completion and commits.

**Failure path:** `DELETE /domains/{name}` cascades FTS + vec + tombstone + queue
(`src/service/domains_admin.rs:350-407`) over a large domain, exceeding 30 s. The client gets
**408**. The operator retries. The first transaction commits regardless. There is no receipt
keyed by request id and no idempotency key — and the post-commit `VACUUM` is swallowed with
`let _ =` (`domains.rs:259-261`), so nothing correlates the 408 with the committed delete.

I grepped `TimeoutLayer|408|REQUEST_TIMEOUT` across `src/`: the only hits are the layer
construction itself and an unrelated note in `src/agentloop/provider.rs`. **No mitigation, no
documented ceiling.**

**Fix:** carry the deadline *inside* the `spawn_blocking` closure so the work refuses to begin
rather than being abandoned mid-commit — the pattern `src/workflow/hostcalls.rs:535-575`
already uses for exec. Add a receipt keyed by request id. At minimum, state the ceiling on
every write route's openapi entry.

**Law clause:** Audit-per-write — the evidence commits; the client's receipt does not.

---

## F8-04 — MEDIUM — The log-injection seam has one production call site and no coverage guard

`sanitize_log_value` is defined at `src/server/router/memory.rs:270`. I grepped it across
`src/**/*.rs`: **one** production call (`:1956`), the definition, and the rest inside
`mod log_injection_seam_tests` (`:3510-3553`). Fourteen tests exercise the function; **none
asserts that call sites use it.**

**Unsanitised bypasses carrying request- or operator-derived values into log lines:**

- `src/handlers/recall.rs:571-573` — `tracing::warn!("per-domain recall failed ({domain}): …")`,
  `domain` straight from the request.
- `src/handlers/webhooks.rs:71-73` and `:570-572` — `path = %secret_path`.

**Anti-vacuity:** all fourteen tests would pass with every production call site deleted.

**Fix:** make the seam unskippable — a `LogValue` newtype constructible only via
`sanitize_log_value` — plus a pin failing any `tracing::(warn|error|info|debug)!` in
`src/handlers/**` whose format string interpolates a request-derived identifier.

---

## F8-05 — MEDIUM — `CRATE_TEST_FLOOR` is a gameable substring count

`src/spire_inventory.rs:293-296` counts `#[test]` by raw substring with **no comment
stripping**. The module's own doc admits it (`:62-63`): *"The needle counts doc-comment
literals too — a deliberate substring lock."*

Floor is `2_758` (`:177`). Measured at this HEAD the needle reads **2,904** (I ran the
inventory test) — roughly 146 units of slack. **Gaming path:** delete real tests, replace with
`#[test]` mentions in doc comments; CI stays green across ~146 deletions.

**Positive result, and it matters:** the *other four* spire guards are **not** gameable, and I
verified each carries a genuine self-pin — `route_registrations_live_only_under_router`
(`:417-481`, plants a violation string at `:465-475` and proves the scanner fires),
`spire_inventory_freezes_the_thin_binary` (`:261-273`, `:288-292`), `bootstrap_stays_protocol_free`
(`:504-525`, self-pins each needle class), and `sql_statement_counter_still_fires`
(`src/service/mod.rs:220-254`, ten cases including the negative). **This one floor is the sole
exception** — which makes it the most likely target, being the only soft one.

**Fix:** count with the existing comment-stripping scanner.

---

## F8-06 — MEDIUM — `/webhooks/` is exempt from authN *and* authZ by prefix, with no HMAC-enforcement pin

`src/server/router/route_guards.rs:59-67` — `path.starts_with("/webhooks/")` short-circuits
both auth middlewares (`src/server/router/auth.rs:100-102`). Six routes are registered under it
(`src/server/router/workflow.rs:132-154`).

**I verified all six do verify in-handler and fail closed:** `receive`
(`src/handlers/webhooks.rs:82`, `:137-158`), `receive_delivery` (`:259-279`), and every secret
loader refuses on absent/insecure config (`:66-77`, `:570-574`, `:767-770`). **So this is not
a live hole — it is an unenforced convention.**

**The gap is directional:** the test that would catch a missing HMAC exempts these routes *for
the same reason* (`tests/main_suite.rs:6884-6885` — *"/webhooks/* verifies its own HMAC inside
the handler … no authorize() by design"*). A future `/webhooks/<anything>` route is silently
exempt from both guard tables and every authz check, and **nothing fails if its author forgets
the HMAC.**

**Fix:** replace the prefix rule with an explicit `WEBHOOK_PATHS` const (the pattern this file
already uses for `PUBLIC_PATHS`) and pin "every registered webhook route's body calls a verifier."

---

## F8-07 — LOW — IPv4 multicast missing from the egress deny table

`src/webhook.rs:370-385` (`IPV4_DENY`) has no `224.0.0.0/4` row, while `IPV6_DENY` **does**
carry multicast `ff00::/8` at `:397`. `validate_public_addrs` → `ipv4_denied` (`:408-421`)
therefore admits `224.0.0.0/4`. Also absent: `192.88.99.0/24` (deprecated 6to4 relay anycast).
IPv4-compatible `::a.b.c.d` is not normalised by `to_ipv4_mapped()` (`:452-458`).

LOW because reach is limited to operator-configured sinks, not request-controlled input.

**Fix:** add `(0xE000_0000, 4, "multicast 224/4")` and `(0xC058_6300, 24, "6to4-relay-anycast")`;
unwrap `::/96` before the v4 arm.

**Rest of the egress guard verified sound:** redirect refusal (`:307`), insert-only DNS pins
(`:291-301`, `:532-538`), metadata hostnames refused pre-resolution (`:471-479`), hex/octal/dword
forms killed by `IpAddr` parsing + url-crate canonicalisation (`:319-326`), per-client 5 s
connect / 15 s total bounds (`:305-313`), NAT64/6to4/Teredo/discard denied wholesale.

---

## F8-08 — HIGH (drill-proven) — A DSAR certificate can certify an incomplete erasure

This is the finding that most directly touches the product's core promise.

**The code names the hazard itself, then does not close it.** `src/service/dsar.rs:790-797`:

> *"proposals hold raw candidate content with no owner column, so a DSAR could never locate
> them and their plaintext (possibly PII about the subject) survived a **'complete' erasure**.
> Sweep them by the subject verbatim … the review-queue provenance for the subject is
> intentionally erased with the memory per Art 17."*

The implementation (`src/service/dsar.rs:803-814`) is
`DELETE FROM proposals WHERE content LIKE '%subject%' ESCAPE '\'` — a **literal substring match
on the subject string inside the proposal's own text.**

**A proposal's text almost never contains its owner's identity.** Mine did not: the canary was
approved into `knowledge` with `owner='loopback'`, and the proposal body contained no
occurrence of `loopback`.

**Drill evidence** (fresh DB, port 18765, §7):

```
DSAR purge subject="loopback"  ->  {"status":"completed", "found_count":1, "action":"both"}
knowledge rows remaining: 0
FTS rows remaining:        0
proposal still present:    1     (decided_at set — it WAS approved)
content contains subject string "loopback"? -> False
```

**The erasure certifies `completed`, the memory and its FTS index are genuinely gone, and the
approved proposal's full text — including every hostile canary string — survives in
`proposals.content`.** `strings brain.db` still finds it (2 hits). There is **no pin** covering
this: `grep` for a proposals+DSAR pin across `tests/*.rs` returns nothing.

**Severity: HIGH, not MEDIUM**, because the artifact produced is a *compliance certificate*.
An operator holding it would reasonably tell a data subject their data was erased. It is also
exactly the class the comment says it wanted to prevent, so the fix is one the authors already
described — the join they rejected was rejected for a good reason (no owner column), which is
precisely why the *content* sweep cannot work.

**Fix (one option, minimal):** the erasure should carry the **approved chunk ids** it just
deleted and delete proposals by `id IN (…)` — the proposal that produced a memory is reachable
from the memory, which *is* owner-attributed. That closes the gap without inventing a join.
Add a red-first pin: approve a proposal → purge its owner → assert zero surviving rows whose
`content` equals the approved body.

**Law clause:** Fail-closed everywhere; and the erasure half of the first mantra.

---

## F8-09 — LOW — DSAR shift-roster sweep drops row-mapping errors

`src/service/dsar/sweep.rs:250-256` — `stmt.query_map(…).flatten()` discards rows whose
`r.get()` fails, under-counting `report.crew_rows`, so the certificate can be issued without
covering that row. The adjacent branch (`:259-265`) correctly fails closed on a corrupt cell —
inconsistent posture **within the same function**.

LOW because unreachable today (`shifts.roster_json` is `TEXT NOT NULL DEFAULT '[]'`,
`src/migration.rs:1874-1875`).

**Fix:** collect `Result`s and map to `DsarError::Database`, matching the corrupt-cell arm.

---