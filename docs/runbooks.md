# Procedures & Runbooks

> Procedures are how a team stops improvising the same thing over and over.
> Brain Server stores the **current, correct way to do something** as a
> retrievable, ordered sequence of steps — so recall returns the same runbook to
> everyone, instead of each person's half-remembered version.

This page is the practical guide to authoring, finding, and maintaining
**procedures** (runbooks) in Brain Server.

## What a procedure is

A procedure is a `procedure`-kind **root** chunk, plus a series of `step`-kind
chunks linked to it with `next_step` edges. The root names the outcome; the
steps give the ordered actions.

```
        ┌────────────────────────────┐
        │  procedure "Onboard a new   │   root chunk (memory_kind=procedure)
        │  engineer"                  │
        └──────────────┬─────────────┘
                       │ next_step
              ┌────────▼────────┐
              │ step 1: "Create │   step chunk (memory_kind=step)
              │  a laptop image" │
              └────────┬────────┘
                       │ next_step
              ┌────────▼────────┐
              │ step 2: "Grant  │   ...
              │  repo access"   │
              └────────┬────────┘
                       ▼
```

Because steps are separate retrievable chunks, a recall can surface the exact
step a person needs, not just the whole runbook.

## Authoring a procedure

### From the CLI (fastest for a quick runbook)

```bash
brain procedure "Onboard a new engineer" \
  --step "Create a laptop image: build from the base image, tag with the date" \
  --step "Grant repo access: add to github team on-call, set membership to maintainer"
```

Rules for `--step`:
- Each step must be `title: content` (colon-separated, both non-empty).
- The root's default content is the title itself if you give no steps.
- Add `--domain <name>` to file the runbook under a team domain.

### Via the API

```bash
curl -X POST http://localhost:8765/procedure \
  -H 'content-type: application/json' \
  -d '{"title":"Onboard a new engineer","content":"Onboard a new engineer","steps":[
        {"title":"Create a laptop image","content":"build from base image, tag with date"},
        {"title":"Grant repo access","content":"add to github team, set maintainer"}
      ]}'
```

The response returns the procedure `id` and the `step_ids`.

## Finding a procedure

- **By recall** — scope to procedures so you don't get ordinary facts back:
  `POST /recall` with `{"query":"onboard new engineer","memory_kind":"procedure"}`,
  or `GET /search?memory_kind=procedure&q=…`. The plugin's `memory_recall` does
  this with `memoryKind: "procedure"`.
- **Read the ordered steps** — `GET /procedure/{id}/steps`.
- **Fetch a single step** — `GET /get/{id}` (the step's chunk id) or `brain get <id>`.
- **Walk a chained workflow** — `GET /graph/traverse` with
  `start: "<procedure title>", kind:"next_step"` walks from one runbook to the
  ones that follow it, so multi-stage processes are discoverable end to end.

## Changing a procedure

Procedures are versioned like any fact: when the steps change, **supersede**
rather than leave two competing runbooks. A new procedure supersedes the old
one (via the same supersession link the review queue uses), so recall returns
the current steps while the old sequence stays recallable `?at=<past>` for
history and audit.

Keep the *same* title when you supersede a procedure, so the "find by outcome"
query still resolves — the current version wins, and older versions are
preserved, not duplicated.

## Authoring habits that make procedures consistent

- **One procedure = one outcome.** A runbook titled "Onboard a new engineer"
  should not also contain "decommission a laptop." Split outcomes so recall
  returns the right one.
- **Title with the outcome, not the owner.** "How to grant emergency DB access"
  outlives "Mark's script." Owner names in titles are how islands start.
- **Steps are imperative and self-contained.** Each step should be actionable
  without the reader having to guess context, since it may be recalled alone.
- **Put the trigger in the root.** The root content should say *when* to run the
  procedure (e.g. "Run when a new engineer starts"), which makes `memory_kind`
  recall match the situation people describe.
- **Reference the source.** Add a `source` label so the team can trace where a
  runbook came from and when it was last reviewed.

## Procedures vs. proposals vs. plain facts

| Content | Where | Gated? |
|---|---|---|
| An ordered, repeatable runbook | `POST /procedure` / `brain procedure` | Direct (no proposal) |
| A durable fact or decision that needs human sign-off | `POST /ingest/proposal` (plugin `memory_store` default) | Yes — Review queue |
| A fact, policy, or note | `POST /ingest` / `POST /ingest/markdown` | Direct (screened) |

Use a **procedure** when there is an order and a repeatable outcome. Use a
**proposal** when a new durable fact should not enter shared recall until a
human approves it. Both are retrievable by `memory_kind`; they answer different
questions.

## Warm standby (v1.28.61)

Single-node SQLite is the doctrine; losing the box loses the memory. The
honest enterprise answer at this scale is a **warm** standby built from
shipped mechanisms — the encrypted backup v3 writer, a shipped WAL-chunk
copy, and a REHEARSED promote. There is no hot failover, no consensus, no
replication protocol, and no RPO=0 claim anywhere in this product; the
shipper is an operator-run process (launchd/systemd — snippets in
[deployment.md](./deployment.md)), never a server thread, because a
shipper inside the server it protects is a correlated failure.

### Setup

1. The follower dir must live on a **different disk or different box** than
   the primary (`--to <dir>`; default `~/.local/share/brain-server/standby`,
   override `BRAIN_STANDBY_DIR`).
2. A UMP operator signing key must resolve (`~/.config/brain-server/ump/`,
   0600 seed file) — manifests are Ed25519-signed and an unsigned follower
   refuses to ship.
3. A backup passphrase file (the same one `brain backup` uses — there is no
   unencrypted follower option; the base AND every WAL chunk are AES-GCM
   sealed at rest).
4. Start the shipper: `brain standby start --to <dir> [--interval-secs 30]`.
   Each cycle: PASSIVE checkpoint → encrypted base via the backup v3 writer
   → the WAL chunk (copied AFTER the base — the writer truncates the WAL) →
   the signed manifest, written last. An interrupted cycle self-heals on the
   next one; `status` fails closed until then.

### Monitoring

`brain standby status [--to <dir>]` prints cycle, last-cycle age, cycles
behind, `rpo_max = interval + checkpoint lag`, and the integrity self-check
(signature + recomputed artifact hashes). Alarm on **age**: from cron, flag
when `last cycle` exceeds `2 × interval` — that means the shipper is dead
(the exact scenario the standby exists for). Any integrity line other than
OK is a page, not a warning: a tampered or torn follower must not be
trusted until a fresh cycle verifies.

### Promote procedure (warm — manual, rehearsed)

1. **Stop the primary** (or confirm it is dead). Restoring over a running
   server is the split-brain scenario `brain restore`'s port guard exists to
   refuse — never `--force` past it against the live DB.
2. `brain standby promote-check --from <dir> --passphrase-file PATH` — the
   rehearsal: restores into a temp dir, replays the chunk, runs
   `PRAGMA integrity_check`, prints RTO/RPO. It never touches the live DB.
3. Promote for real: `BRAIN_DB_PATH=<target> brain restore <dir>/base.v3
   --passphrase-file PATH`. Note `restore`'s target is the DB path from
   `BRAIN_DB_PATH`/default — the positional is the backup source. The
   pre-restore state is saved to `<target>.bak` automatically (that
   snapshot has already saved the memory once — see the incident note
   below).
4. Restart the server against the promoted DB; clients reconnect manually.
5. Re-point the shipper at the new primary and start a fresh follower.

### Ceilings (honest)

- **RPO is bounded, not zero**: at most `interval + checkpoint lag` of
  commits after the last chunk can be lost (plus a sub-second race: a write
  that lands, gets fully checkpointed, and has its WAL reset inside the
  cycle's millisecond copy window self-heals in the NEXT cycle's base but
  is lost if the primary dies inside that window and you promote the stale
  cycle).
- **Warm, not hot**: promote is a manual, rehearsed procedure; measured RTO
  on this box is sub-second (drill record below), but nothing fails over by
  itself.
- **Single-region**: the follower is a file copy; there is no cross-region
  story beyond pointing `--to` at a mounted remote volume.
- **Client reconnect is manual** — no session draining, no read-proxy.
- Chunk history (`wal/NNNN.frame-chunk`) accumulates; each is the full
  current WAL encrypted, so disk grows by roughly `wal_size × cycles`.
- `status` verifies the LATEST cycle only; a torn interrupted cycle fails
  closed until the next cycle lands (by design).

### Drill record — 2026-09-06

Executed against a **copy** of the live DB (48.8 MB, 8,790 knowledge rows,
online-backup API; the live server kept serving), release build, real UMP
operator key, `--interval-secs 10`:

```text
shipper : 3 cycles @10s — lag 425/406/414 ms (two Argon2id + 48 MB VACUUM
          INTO per cycle); rpo_max 10.4s per cycle
burst   : 301 rows mid-drill — carried visibly (base 48,824,639 →
          48,910,655 B at cycle 0003)
status  : cycle 0003, 0 cycles behind, integrity OK (sig + hashes), exit 0
promote : RTO 0.55s (restore 0.37s / open+integrity 0.18s) — PASS, exit 0
          RPO 10.4s (interval 10 + lag 0.414)
fidelity: promoted db = 9,091 rows (8,790 original + 301 burst);
          the row committed AFTER the last cycle is absent — inside the
          RPO window, exactly as the ceilings say
tamper  : one flipped byte in wal/0003.frame-chunk → status exit 1
          (fails closed); byte restored → status exit 0
```

### Incident note — 2026-09-06 (the .bak mechanism, live)

During development rehearsal, a `brain restore --force` was mis-aimed at
the LIVE DB (its target is `BRAIN_DB_PATH`/default, not the positional).
The port guard was bypassed with `--force`, but restore's automatic safety
snapshot did exactly what it is designed to do: the pre-restore memory
(48 MB, 8,790 rows) survived in `<db>.bak`, the server was stopped, the
`.bak` swapped back, and the service re-verified healthy (integrity ok,
full row counts). Lessons encoded above: the promote procedure names the
target explicitly via `BRAIN_DB_PATH`, and `--force` against a live server
is the one step that must never be routine.

## Principal kill-switch (v1.28.62)

An agent (or operator principal) that is compromised, offboarded, or
misbehaving has ONE switch: `POST /ops/agents/revoke {principal, reason}`
(Admin on `global`). Revocation is identity-wide, and the machinery is
already shipped — the procedure below is the whole story, no new tooling.

### What revocation does, in one transaction

1. The `revoked_principals` row upserts (latest revocation wins) and a
   hash-chained audit row lands (`kind=auth`, target
   `principal:<name>`, detail `revoke:<reason>`).
2. Every card use, delegation dispatch, and result submission re-checks
   the table BEFORE signature verification and refuses `403
   principal_revoked` — including cards already provisioned
   (re-provisioning does NOT resurrect the identity).
3. Every ACTIVE run where the principal OWNS in-flight (`requested`)
   delegation work drains through the EXISTING cancel path (the run CAS →
   status `cancelled`), each with a `delegation/revoked` lineage event and
   a run-scoped audit row. The response reports `runs_drained: <n>`.

### Procedure

1. Revoke: `curl -X POST -H 'authorization: Bearer …' -d '{"principal":
   "agent:atlas", "reason": "<why>"}' …/ops/agents/revoke` — record
   `runs_drained`.
2. Verify fail-closed: `GET /ops/agents/cards?domain=…` (any domain the
   agent has a card in) must answer `403 principal_revoked`; a dispatch
   naming the principal must refuse the same way.
3. Verify the drain: the drained runs read `status = cancelled`
   (`GET /workflow/runs/{id}`), and their event log carries the
   `delegation/revoked` lineage event.
4. Verify the story: `GET /ops/agents/revocations` shows the register;
   `GET /audit/verify` stays `{"ok":true}` — the revoke and every drain
   are hash-chained rows in the same transaction that did the work.

### Ceilings (honest)

- Revocation gates the MESH decision paths (cards, dispatch, results) —
  it is NOT a JWT revocation (that is `auth/revocation.rs`, the token
  layer, separate machinery with its own runbook).
- A revoked AGENT's already-`requested` delegations stay in that state
  (evidence), they just can never complete; the owning run's remaining
  work is the operator's to re-dispatch to a healthy agent.
- The drain covers runs where the principal owns in-flight work; a run
  they merely participated in historically is untouched.

### Drill record — 2026-09-06 (Attestation milestone)

Executed against a COPY of the live DB (50.6 MB, 8,790 knowledge rows),
server on a spare port, drill token only. Binary built from the
attestation line (version stamp bumps with the release commit).

```
revoke agent : drill-agent (card holder) → {"revoked":true,"runs_drained":0}
cards list   : 403 principal_revoked (fails CLOSED on the revoked card)
dispatch     : 403 principal_revoked (no new work to a revoked agent)
revoke owner : loopback (holds in-flight work) → {"revoked":true,
               "runs_drained":1}
drain        : run 1 status = cancelled, state_revision 0 → 1 (the CAS
               advanced exactly once; state_json untouched)
events       : delegation/revoked {"action":"revocation_drain",
               "principal":"loopback"} present in the run's lineage
no new disp. : 403 principal_revoked BEFORE any row was written
register     : 2 rows (loopback, drill-agent), newest first, reasons kept
audit chain  : /audit/verify {"ok":true}; two kind=auth rows (revoke) +
               one kind=workflow row (drain), all hash-chained
same-tx law  : revocation + audit + drain committed atomically (the pin
               revoked_owner_no_new_dispatch asserts the rollback twin)
```

## Next steps

- **[One Brain for the Whole Team](./team-workflow.md)** — where procedures fit in the shared-store workflow.
- **[Knowledge graph](./knowledge-graph.md)** — `next_step` edges and typed traversal.
- **[Memory lifecycle](./memory-lifecycle.md)** — how a chunk is stored, versioned, and recalled.