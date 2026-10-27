# The support vertical — the mounted runbook (the C1 rehearsal)

**The boundary, first: this page is ARCHITECTURE and stays vendor-neutral —
the support vertical's CONTENT (procedure texts, rule bodies, ticket-shaped
fixtures) lives in the fixture home on the private side, never in this
tree's tracked docs, and never on the public mirror** (the 2026-08-19
customer-domain law). A replay is `the runbook's commands + the fixture
home = the mounted state`; the kernel itself gains nothing per vertical.

**This was a REHEARSAL round (plan 28 §6 C1): zero kernel code, zero new
doors.** Every step below walks a door that already existed at
`67b65cd7`–`daebe7fe`; a missing door was MEASURED, NAMED, and recorded —
never built. The deliverable is this page, the mounted state on a fixture
DB, and the C2 gap list at the bottom.

## The fixture home (private)

`plans/brain-shell/c1-support-fixture/` in the plans repo holds:
`corpus/support/` (three synthetic ticket-resolution notes, one
escalation-shaped), `corpus/finance/` (one synthetic row in a FOREIGN
domain — so the cross-domain refusal is measured against a domain that
EXISTS with a matching row), `packs/support-escalation-pack.json` (the
escalation pack authored per the REGISTERED wizard-pack shape —
`support-ticket.json` in `crates/brain-fuzz/corpus/accounts/packs/` is
the tree's ratified reference), `sql/rules-support.sql` (the rules rows),
and `mount.sh` — the replay driver that executes this page end to end on
a fresh fixture DB and prints the measured transcript.

## The door inventory (all pre-existing; measured at the round's HEAD)

| Door | Where | Used for |
|---|---|---|
| `POST /ingest` (+ `brain ingest-dir`) | the memory surface | the `support` domain (implicit creation) + corpora |
| `POST /domains/{d}/profile` (`brain setup`) | the profiles surface | binding the `call-center` profile |
| `brain principal grant` | the local roster door | three principals on EXISTING presets |
| `POST /procedure` | the workflow surface | the escalation runbook, authored by the supervisor principal |
| `POST /ingest` as the agent | the review posture | the gateway-captured row → 202 pending |
| `GET /proposals` + `POST /proposals/{id}/approve?digest=` | the queue | the operator approves with the displayed digest |
| direct SQL (`sqlite3`) | the operator's own hand | the rules rows — the insert door is MISSING (a C2 gap, named below) |
| `GET /audit?principal=` | the audit filter | the footprint, principal-attributable |
| `POST /recall` | recall | the positive controls + the existence-blind refusals |
| `brain verify-governance` | the CLI | the chain verdict on the fixture |

## The mount sequence (replayable as written)

`sh $FIXTURE_HOME/mount.sh 8181` executes everything below; the commands
are shown with `$URL` = the fixture server and `$DB` = the fixture DB.
Content arguments come from the fixture home (the privacy law above).

1. **Fresh fixture.** `BRAIN_DB_PATH=$DB BIND_PORT=8181
   AUTH_TOKEN_FILE=$FIXTURE/tokens/operator.token BRAIN_REQUIRE_AUTH=1
   brain-server` — the only mount point is `BRAIN_DB_PATH`; auth required.
2. **The domain** (§1.1). `brain ingest-dir $FIXTURE/corpus/support
   --domain support` — ingest that names the domain creates it
   implicitly (the measured path; recorded per the prompt's measure).
   Same for `corpus/finance` (the foreign-domain control). Then
   `brain setup support --profile call-center --yes` binds the vertical's
   seeded profile (episodic retention 90d — the same clock the retention
   rule row names).
3. **The principals** (§1.2). Three `brain principal grant` calls —
   `support-supervisor` (role `supervisor`, tools
   `ump.recall,ump.get,ump.revise,ump.feedback,ump.remember`),
   `support-qa` (role `qa-specialist`,
   `ump.recall,ump.get,ump.feedback`), `support-agent` (role `agent`,
   `ump.recall,ump.get,ump.feedback`) — each `--domain support
   --ttl-days 90 --db $DB --out $FIXTURE/tokens/<name>.token`. Each
   grant writes the 0600 token file, the roster row (hash only), and a
   hash-chained audit row attributed to the operator (the operator
   minted it). The `plugin` preset is for EXTERNAL machine clients
   (K5), never for the vertical's own staff postures.
4. **The escalation procedure** (§1.3). `POST /procedure` with the
   pack-derived title + three ordered steps + `domain: support`,
   authorized as the **supervisor principal** — the vertical's author
   posture through the route's registered schema (`openapi.yaml`
   `/procedure`: title/content/steps/domain, Write gate). The registered
   wizard-pack catalog is compile-time-embedded; the pack JSON in the
   fixture home rides its SHAPE as content and lands as procedure DATA.
   **Negative arm:** the same POST as `support-agent` 403s — the agent
   class is refused outright (procedures are operator-authored and are
   later EXECUTED by decision evaluation; "propose instead" is not
   available on this route). Audit trail: one `procedure:ok` row for the
   supervisor; the refusal is audited under the masked subject, not the
   roster name (the machine's existing denial attribution — measured).
5. **The gateway write** (§1.5a). `POST /ingest` as `support-agent` →
   **HTTP 202, `proposal_pending`** — the agent-class write posture:
   the captured-chat row waits in the queue, tainted `channel-capture`
   by the server (the class decides the label, not the caller).
6. **The queue** (§1.5b/c). The supervisor principal CANNOT reach
   `GET /proposals` — **the queue gate is the global leg, and a
   domain-scoped principal holds no global grant** (measured 403, the
   C2 gap named below). The operator — the queue's standing keyholder —
   lists the queue, and approves with the digest it was displayed:
   `POST /proposals/1/approve?digest=<content_digest>` → 200, the row
   publishes (chunk id in the response). A digest mismatch is a 409;
   an absent digest is a 400 `digest_required`.
7. **The rules rows** (§1.4). MEASURED: **no rules-insert door exists**
   — no `brain rules` verb, no route writer; the tables'
   only references are the migration and the migrate-rehearse list
   (the separate curated legal DB at `BRAIN_LEGAL_DB_PATH` is a
   different mechanism: DPO jurisdiction law per `docs/legal-db-import.md`,
   never vertical rules). The honest rehearsal lands the rows by the
   operator's direct SQL — `sqlite3 $DB` over
   `$FIXTURE_HOME/sql/rules-support.sql` (ticket retention 90d
   post-resolution, breach posture, DSAR clock; idempotent by
   `rule_key`). When C2 ships the door, those rows become its first
   regression fixture.
8. **The proofs, three ways** (§1.5):
   - **Footprint:** `GET /audit?principal=support-supervisor`
     (admin bearer) returns exactly that principal's acts and nothing
     else — measured per principal (supervisor: its procedure + recall;
     qa: its recall; agent: its ingest + recall), and a
     `?principal=no-such-actor` returns `{"events":[]}`.
   - **Isolation:** as `support-agent`, recall against `finance` (a
     domain that EXISTS, holding a matching row) → 403; against
     `no-such-domain-x` → 403 with a body byte-identical once the
     caller's echoed input is normalized (existence-blind); the
     domain-less sweep → 403. This is the machine M3-g pinned —
     STRICTER than this round's prompt-draft "empty, not an error";
     the pin holds the machine. Positive control: the agent's own
     `domain: support` recall answers 200 with its domain's rows.
   - **Health:** `brain verify-governance --db $DB` → every hashed
     row's governance tuple matches its stored hash (`tampered: 0`).

## The measured postures the presets were designed for (and one gap)

- `support-agent` (shared-pool posture) reads its whole domain and
  writes through the review queue — the working day-one shape.
- `support-supervisor` / `support-qa` (`owner_filter: "reports"`)
  resolve their reports from the JWT `manages` claim. The roster lane
  carries no such claim, so a reports-role ROSTER principal sees its
  own acts and nothing else — deny-by-default, working as designed,
  and the reason the supervisor's queue reach and reports visibility
  are the same open item (the gap below).

## The C2 gap list (verbatim, the round's ledger)

1. **`the_rules_insert_door_is_missing`** — the `rules`/`rule_rates`
   tables have a schema and no door (no CLI verb, no route writer); the
   rehearsal's rows landed by operator SQL and wait to become the
   door's first regression fixture.
2. **`the_approve_action_is_global_gated`** — `/proposals` and
   `/proposals/{id}/approve` authorize on the `global` leg, so the
   vertical's own supervisor preset (which has `approve` in `can`)
   cannot reach the queue; the operator approves in its stead. A
   domain-scoped queue decision is C2's to size.
3. **`the_roster_lane_carries_no_manages`** — the supervisor/qa
   `reports` visibility rides the JWT `manages` claim; the roster has
   no such column, so a roster-borne supervisor sees no reports' rows.
   Either the roster grows the column or the staff posture rides an
   IdP — a recorded decision, not a convenience.
4. **The pack format + `brain plugin install` + the Tier A manifest**
   (plan 28 §4, unchanged) — the escalation pack exists as CONTENT in
   the registered wizard-pack shape; embedding a vertical pack in the
   catalog, signing it, and installing it are C2's build.
5. **Denial attribution nuance** (measured, not a build item):
   handler-scope 403s are audited under the masked subject, so
   `?principal=<name>` shows a principal's OK-acts but not its refused
   ones. Attributing refusals to roster names is a wire decision for
   whichever round next touches the audit filter.

## Where the teeth live

- The presets and their day-zero law: `src/role.rs`.
- The roster, the lane, the CLI cores: `src/auth/principals.rs`.
- The cross-domain existence-blind refusals and the tool-reach pin:
  `tests/authz_matrix.rs`.
- The queue's digest law: `src/handlers/gate.rs` (`approve_proposal`).
- The rules schema (tables only, no door): `src/migration.rs`.
- The fixture home + replay driver: the private plans repo,
  `plans/brain-shell/c1-support-fixture/` (`mount.sh`).
- The register row with the round's full ledger: `docs/gate-law-register.md` (C1).
