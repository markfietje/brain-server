# CLI Reference

The `brain` binary is the operator command-line surface. This page is the command reference.
The CLI covers retrieval, ingest (directories), self-correction, domain/retention/backup/key
management, UMP, clients, and health — the commands it ships in `src/bin/brain.rs` (hand-rolled argument
parsing, no clap). Global flags on every invocation: `--json` (machine-readable envelope for the
commands that support it) and `-V/--version`. Per-client DSAR and legal hold are exposed here via `brain client`; the actions
the CLI does **not** expose (erasure of a bare chunk, proposal approval, the global audit log) live
on the HTTP API or the client console.

## Health & operations

| Command | Purpose |
|---|---|
| `brain doctor` [`--backup <path> [--passphrase-file PATH]`] | Health + readiness, local DB-volume encryption status (where supported); optionally verify a backup file |
| `brain kb build --domain <d> --out <dir> [--db <path>] [--base-url <url>] [--with-case-status] [--locales en,de,fr,es,nl]` | Build the public KB as a static artifact from published articles (deterministic bytes + SHA-256 manifest carrying the Art 50(2) provenance seal; sign before hosting). `--with-case-status` also emits the live `status/{ref}.json|.html` case-status pages; `--locales` emits per-locale pages with hreflang alternates |
| `brain status` | Counts, model, version |
| `brain check-consistency` | Report duplicates, conflicts, stale sources, near-duplicates |
| `brain verify-governance` [`--db PATH`] | Row-level governance-integrity scan: recount every knowledge row's SHA-256 over its governance tuple (owner, access_scope, domain); reports ok / tampered (ids) / legacy_unhashed, exit ≠ 0 on tampering |
| `brain snapshot-status` | Show the point-in-time snapshot state |
| `brain setup [domain] [--profile NAME] [--yes]` | Interactive first-run: pick a profile preset, preview its knobs, bind it to a domain (`--yes` scripts it) |
| `brain bench` | Benchmark harness (always compiled into `brain`; the `bench` Cargo feature gates the separate `bench` BINARY) |

## Retrieval

| Command | Purpose |
|---|---|
| `brain query "q"` [`--phrase …`] [`--exclude …`] [`--code …`] [`--source …`] [`--since DATE`] [`--k N`] [`--intent …`] [`--profile …`] [`--graph`] [`--explain`] | Structured recall |
| `brain get <id>` | Fetch a chunk |
| `brain explain "q"` [`--source S` …] [`--since ISO`] | Provenance + telemetry |
| `brain suggest "<context>"` `[--exclude id[,id...]]` `[--k N]` `[--session S]` `[--domain D]` | Opt-in anticipation pull |
| `brain suggest-feedback <id> accept\|dismiss` `[--reason "..."]` `[--session S]` | Record a suggestion outcome |
| `brain suggest-metrics` `[--session S]` `[--since DATE]` | False-positive rate over the feedback ledger |

## Ingest & sources

| Command | Purpose |
|---|---|
| `brain ingest-dir <path>` [`--dry-run`] [`--replace` \| `-r`] [`--source S`] [`--domain D`] | Ingest a vault directory (`-r` is the short alias of `--replace`) |
| `brain reconcile <path>` [`--dry-run`] [`--kind vault`] | Sweep deleted sources |
| `brain source-delete <id> [--yes]` | Retire a source (`--yes` skips the confirmation prompt) |

## Domains & retention

| Command | Purpose |
|---|---|
| `brain domain-move <id> [<id> ...] --to <domain> [--confirm global]` | Move chunks to another domain |
| `brain domains-recompute` | Recompute domain membership / stats |
| `brain retention get` \| `set <kind> <days>` | Per-kind retention expiry policy |

## Clients (BPO register, v1.27)

| Command | Purpose |
|---|---|
| `brain client add <name> --jurisdiction J [--domain D] [--profile P] [--yes]` | Register an operating client (one isolation domain per client). `--jurisdiction` is required; `--domain` defaults to the client name |
| `brain client dpa get <name>` | Show a client's DPA terms |
| `brain client dpa set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S` | Set a client's DPA terms |
| `brain client dsar <name> <subject> --action purge\|export\|both [--dry-run] [--yes]` | Run a per-client jurisdiction-aware DSAR. `--action` is REQUIRED (400-free refusal without it — the old silent purge default is gone); purge/both prompt with the subject digest unless `--yes` |
| `brain client hold add <name> <id> [<id> ...] --reason R` \| `list <name>` | Add a per-client legal hold; list holds. (Release lives on the HTTP API — `POST /legal-hold/{id}/release` — there is no CLI release verb) |
| `brain client qa list <name>` \| `coach <name> <id> [--note N] [--flag]` | Supervisor QA queue + coaching note (v1.27.8, Admin). Note and flagged are both optional |
| `brain client end <name> [--purge\|--return] [--dataset D] [--yes]` | Terminate a client: purge-or-return + archive + certificate |

## Self-correction & maintenance

| Command | Purpose |
|---|---|
| `brain resolve <new_id> <old_id>` | Mark new chunk as superseding old; expires old from current recall |
| `brain undo-resolve <old_id> [<old_id> ...]` | Reverse a prior supersession; restores chunk to current recall |
| `brain procedure <title>` [`--step "title: content"` …] [`--domain D`] | Ingest a root + ordered steps in one transaction |
| `brain classify "<text>"` | Deterministic keyword categorization |
| `brain evaluate <decision_id>` `--var name=value` … | Evaluate a stored decision rule |
| `brain eval` [`--floor r5=0.85 r10=0.9`] [`--safety-violations N`] | Run the frozen recall-eval harness (always compiled into `brain`). `--safety-violations` declares the caller-counted safety term of the joint admission (the harness never invents it) |

## Connectors

| Command | Purpose |
|---|---|
| `brain connect github` [`--kind github`] `--app-id N --install-id N --key-file PATH` [`--webhook-secret-file PATH`] `--repo O/R [--repo O/R] …` | Configure the GitHub connector |
| `brain sync [github]` `[--config PATH \| --instance NAME]` | Run a connector sync |
| `brain connector-status` | List registered connectors |

## JWT key management

| Command | Purpose |
|---|---|
| `brain key generate` [`--kid ID`] [`--dir PATH`] | Generate an RSA-2048 (RS256) JWT signing keypair (JWT mode). Algorithm is fixed at RSA-2048/RS256. |
| `brain key list` [`--dir PATH`] | Show loaded keys |
| `brain key prune` [`--dir PATH`] [`--keep N`] | Drop expired keys from JWKS |
| `brain key rotate` [`--db PATH`] | Rotate the UMP operator signing key (`operator.ed25519`): current → `.prev` (verify-only, ONE key deep), new seed 0600, generation bump + hash-chained audit row. Operator verb — no scheduling, no background anything. |

## Token management

| Command | Purpose |
|---|---|
| `brain token rotate` | Atomically rotate the bearer token (v1.27.12): a fresh 32-byte hex token is written to a 0600 temp file (`create_new`, never umask-dependent), fsync'd, and renamed over the configured token file. Refuses to overwrite a group/world-readable target. No restart needed — the running server and file-reading consumers hot-reload it within ~5s (the rotation watcher). |

## Governed workflow runs (v1.28)

| Command | Purpose |
|---|---|
| `brain workflow open [DOMAIN]` | Open a governed `troubleshoot` run in the domain (default `global`) — `POST /workflow/runs`. |
| `brain workflow status <run>` | Fetch a run's state, revision, and pending question. |
| `brain workflow answer <run> <text>` | Answer the run's pending AskHuman question (digest-bound to the live question). |
| `brain workflow approve <run> <step>` | Approve a step gated on human approval. |
| `brain workflow crank <run> [steps]` | Advance the engine loop up to `[steps]` transitions. |
| `brain workflow handoff <run>` | Emit the I-PASS handoff packet for a run (read-seam sanitized). Supports `--json`. |
| `brain workflow note <run> <text> [--reask]` | Post a screened case note on the run (`@skill:`/`@principal` mentions resolve into swarm invites); `--reask` additionally marks the operator re-ask (the `case/reask` effort-proxy source). |
| `brain wfm-import <file.csv\|file.json>` `[--domain D]` `[--dry-run]` | Import WFM shifts (POST `/ops/shifts`) and skills (they land as HITL `crew_skills_update` proposals — never direct writes) |

## UMP (Universal Memory Protocol)

| Command | Purpose |
|---|---|
| `brain ump export [--format md\|ump] [--out FILE]` | Export the memory corpus |
| `brain ump import <file>` | Import a UMP export |
| `brain ump keygen [--dir PATH]` | Generate the UMP operator (Ed25519) signing key |
| `brain parcel export --domain <d> [--since <ts>] --out <file>` | Export approved knowledge rows as a signed parcel (quarantined rows never leave) |
| `brain parcel import --file <file> --domain <d> --expected-signer <did>` | Verify + import a parcel; rows land as pending proposals, never direct writes. `--expected-signer` is shown unbracketed because the SERVER refuses without it (`400 signer_required`) — an optional-looking flag would document a call that cannot succeed |
| `brain parcel ledger [--domain <d>]` | Show the parcel crossing ledger |

## Personal assistant & compliance register (v1.28.42+)

| Command | Purpose |
|---|---|
| `brain valet add "what" --at <iso\|HH:MM\|unix>` `[--repeat none\|daily\|weekly]` `[--domain D]` | Add a valet reminder |
| `brain valet due [--now <unix>]` \| `brain valet brief` \| `brain valet consent grant\|revoke` | Due items, the brief, and consent state |
| `brain ropa list` \| `brain ropa add --activity A --controller C --processor P --lawful-basis B` `[--categories S] [--recipients S] [--retention-days N] [--security-measures S] [--transfers S]` | Records-of-processing register (read + propose an activity row) |
| `brain principal grant --name N --role plugin --domain D` `[--tools t1,t2] [--ttl-days N] [--out PATH] [--db PATH]` | Mint one **plugin principal** (M3-g, plan 27): a 0600 token file (the bearer NEVER touches stdout or the DB — the roster stores its SHA-256), a `principals` roster row, and a hash-chained audit row. The granted role must EXIST in the roles store; `plugin` ships able to do nothing beyond scoped read/write of its own rows in its own domain, and `--tools` is the ONLY widening (each name an explicit operator act, enforced by the `authorize_tool` roster arm). Re-granting a name rotates it; a REVOKED name stays dead — grant a new name instead. NO HTTP mint route exists: minting is an operator act on the machine that owns the token files |
| `brain principal revoke --name N` `[--reason R] [--db PATH]` | The kill-switch: a `revoked_principals` row (same upsert the mesh writes — latest wins) + an audit row. The minted token 401s `identity_revoked` on the next call — the live proof shape of the 2026-09-09 agent-token enforcement |
| `brain principal list` `[--db PATH]` | The roster: name, role, domain, tools, created, expiry, and each row's kill-switch state (`--json` for the machine envelope) |

| `brain rules insert --jurisdiction J --subject S --rule-key K --body "…" --source-ref R --revision N` [`--effective-at T`] [`--reviewed-at T`] [`--expires-at T`] [`--db PATH`] | Insert or revise ONE governance rule row (C2 G-3, plan 28 — closing C1's `the_rules_insert_door_is_missing`). A LOCAL operator act on the named DB — NO HTTP route exists (a wire writer would let any JWT-bearing principal mint jurisdiction law). One row per natural key `(jurisdiction, subject, rule_key)`: a HIGHER revision replaces the row in place; the same or lower revision is REFUSED naming both numbers, so a replay lands clean and a stale write cannot clobber fresh law. One audit row per landed act (`AuditKind::Rules`), the body riding the chain as a hash only |
| `brain rules list` [`--db PATH`] | The rules door's rows, natural-key ordered (`--json` for the machine envelope) |
| `brain plugin install <pack.tar.gz>` [`--dry-run`] [`--yes`] [`--sig PATH`] [`--db PATH`] | Verify → dry-run → import one signed DATA pack (C2 G-4, plan 28 §4): the detached minisign signature is verified against the PINNED pack key (`BRAIN_PACK_PUBKEY`, default `pack.pub` beside the data root — a SECOND key from the updater's; a pack signed by the updater key is REFUSED naming both keynums), every declaration validates against the machinery that consumes it, and DRY-RUN IS THE DEFAULT SURFACE — nothing writes without `--yes`, and the rows the dry-run prints are the rows the import lands, count for count. The import runs through the EXISTING doors in one transaction (domain bind, role upsert, the procedure door's internals, the rules door), one audit row per act, and registers the pack (id + version + blake3 hashes). Re-installing the same id+version is REFUSED; a higher version is the upgrade path. Packs are DATA — executable content is refused naming the file, and there is no code to load |
| `brain plugin list` [`--db PATH`] | The installed packs: id, version, kinds, content hashes, the import act's audit ref; a removed pack reads `removed` — history, not a live registration (`--json` for the machine envelope, `state` + `removed_at` included) |
| `brain plugin uninstall <pack-id>` [`--dry-run`] [`--yes`] [`--db PATH`] | The pack lifecycle's other half — a REGISTRATION act, never an erasure act: the register row is MARKED (`removed_at`), ONE `Plugin` audit row lands, and the imported DATA stays exactly where it is, still governed (erasing it is the doors' work: `brain source-delete`, `brain shred`). REFUSE-WHILE-LIVE, fail-closed: the act is refused while a roster principal holds a pack-upserted role (clear it: `brain principal revoke`) or the pack's profile binding stands (clear it: the profile unbind) — the refusal names what is live and the doors that clear it. An already-removed registration refuses (history, not a live registration); the re-install law resets — the SAME version installs again as a fresh decision. DRY-RUN IS THE DEFAULT SURFACE: nothing writes without `--yes`. No HTTP route — the mint refusal's posture, third time |

## Backup & restore

| Command | Purpose |
|---|---|
| `brain backup <out-path>` [`--passphrase-file PATH`] [`--format v1\|v2\|v3`] [`--allow-weak-legacy`] | Encrypted AES-256-GCM backup (checksummed, excludes secrets; v3 is the current format — header bytes are GCM AAD). DB path is taken from `BRAIN_DB_PATH`/default, not a positional. A passphrase is required. Writing `--format v1` additionally requires `--allow-weak-legacy` (v1's unsalted fast KDF + deterministic nonce are offline-brute-forceable); v1/v2 files remain readable with no flag. |
| `brain restore <in-path>` [`--passphrase-file PATH`] [`--force`] [`--yes`] [`--allow-chainless`] | Restore from an encrypted backup. Always prompts unless `--yes` (`--force` skips only the liveness probe, never the human gate). A chain-less image (no `audit_events` table) REFUSES without `--allow-chainless`; the flag restores with a loud disclosure. Legacy-epoch (unkeyed) chains are marked `forgeable: true` until the operator re-anchors the chain (`brain-server --re-audit` — the SERVER binary's offline mode, not a `brain` flag). |

## Warm standby (v1.28.61)

Warm, never hot: the shipper is an operator-run process (launchd/systemd —
see deployment.md), NOT a server thread, and promote is a rehearsed manual
step. There is NO hot failover and NO RPO=0 claim anywhere.

| Command | Purpose |
|---|---|
| `brain standby start --to <dir>` `[--interval-secs 30]` `[--passphrase-file PATH]` | Long-running shipper: per cycle a PASSIVE wal_checkpoint, then the encrypted base via the backup v3 writer, the WAL chunk (same v3 encryption — no plaintext at rest), and the signed manifest (written last). An interrupted cycle self-heals on the next one. |
| `brain standby ship --to <dir>` `[--passphrase-file PATH]` `[--db PATH]` | ONE ship cycle then exit — the timer/CronJob form (an operator scheduler owns the cadence; the binary never loops). Same per-cycle order as `start`, cycle numbering resumes an interrupted sequence. |
| `brain standby status [--to <dir>]` | Integrity self-check of the follower: verifies the manifest's Ed25519 signature and recomputes artifact hashes — any tamper or torn cycle FAILS (exit 1). Prints cycle, age, cycles behind, and `rpo_max = interval + checkpoint lag`. |
| `brain standby promote-check --from <dir>` `[--passphrase-file PATH]` `[--expected-signer DID]` | THE DRILL: restores the follower into a temp dir (the shipped restore path), replays the WAL chunk, runs `PRAGMA integrity_check`, and prints measured RTO plus computed RPO. Exit code gates. |

| `brain disproof` [`--claim-id ID`] [`--db PATH`] | Evaluates every claim's stored **disproof condition** against the claim's own subject and reports the **three** states: `satisfied` (the named disproof was NOT observed — these stand), `REFUTED` (it WAS observed — these do **NOT** stand, and are listed by id), and `no verdict` (a prose condition, or a claim predating the field — **never green**). The three are counted separately on purpose: a single "ok" number would make a prose condition read as a pass. |
| `brain disproof` (posture) | **Writes nothing.** No `status` is set, nothing is demoted, nothing is ratified — a sweep that wrote its own verdicts back would be a promotion path, and promotion is disabled. Run it **on a cadence from cron**: a shipper inside the server it falsifies is a correlated failure. A sweep over zero claims says so explicitly, because an empty denominator is not a clean bill of health. |

## Routing (operator-run, writes nothing)

`brain route` is the operator-run entry point to the routing seam. It is a verb and not
a route because routing has no cadence: nothing polls for a routing decision, and a
shipper inside the server it measures is a correlated failure. It runs in the operator's
own process on the operator's own filesystem access, writes nothing, and needs no
credential — the trust boundary is the operator's.

| Command | Purpose |
|---|---|
| `brain route --domain D --class LABEL` [`--queue Q`] [`--confidence N`] [`--db PATH`] | Routes ONE case and prints the receipt: the queue it went to, whether it escalated, and the declared vocabulary it was routed against. The queue routes **only if the taxonomy has declared it** — an undeclared queue escalates to a human, and so does every case in a domain that has declared nothing. That is the anti-invention law made visible: the seam holds **no class→queue table of its own**, so a queue becomes routable when something declares it. |
| `brain route --class human_unmeasured` (and any unrecognised label) | **REFUSED, exit 2.** `--class` must be a label the classifier emits. An unrecognised label is *no class* — never a default. Routing a case under a class the classifier did not emit is routing on nothing, and silently defaulting would make that invisible. |
| `brain route --confidence N` | Accepted and **DISCARDED**, and the receipt says so. Confidence is a quality signal; whether a destination exists is a fact about the declared vocabulary. No number, however high, can make an undeclared destination declared. |

## Evidence & physical erasure (v1.28.91)

| Command | Purpose |
|---|---|
| `brain anchor` [`--db PATH`] | Prints the deterministic state fingerprint (audit chain head + knowledge content census + row counts) — record the line OFF-HOST (paper, password manager, second machine). Read-only, audited by nothing on purpose: the anchor's own audit row would move the chain head it just fingerprinted; the off-host copy IS the evidence. Run per domain DB. |
| `brain anchor --verify "<recorded line>"` [`--db PATH`] | Recomputes and diffs against a recorded line. ANY state change since the record trips it — legitimate writes too (the audit chain explains those); what it uniquely catches is a moved knowledge census on a chain that still verifies: business-row tamper behind the chain, the class no in-tree verifier detected (seventh pass, R7-08). |
| `brain census` [`--db PATH`] | The **drift census**: re-scores the FROZEN gold corpus and diffs every cell against the committed baseline (`evals/R57_DRIFT_BASELINE.json`) under ONE global tolerance (500 units of 10000). A breach writes a hash-chained `findings` row (`source=drift_census`) and **exits non-zero**; a clean pass writes nothing at all and exits 0. Externally cron-driven on purpose — there is NO in-process scheduler, because a shipper running inside the server it measures is a correlated failure. A cell with no baseline is **reported loudly** (the unbaselined/orphaned counts print with their names) — honest ceiling: only a tolerance breach changes the exit code today; the library's own `is_clean` law (`breaches == 0 && unbaselined == 0 && orphaned == 0`) is stricter than the CLI gate, so read the printed counts, not just the exit code. |
| `brain census --print-baseline` | Emits the measured vector in the committed baseline's exact shape. Re-anchoring is a **deliberate, diffable act**: commit the result with a message saying WHY the scores moved. A baseline that drifts without a reason in the log is a census that has stopped measuring. Needs no database. |
| `brain shred [--db PATH] --yes` | The operator-invoked physical residue drop after a logical purge. `--yes` is REQUIRED (there is no interactive prompt — the refusal without it is deliberate); `--db` is optional and defaults to `BRAIN_DB_PATH`/the default DB. Steps: `secure_delete=ON` (readback asserted) → `wal_checkpoint(TRUNCATE)` → `VACUUM` (rebuild from live pages only) → `wal_checkpoint(TRUNCATE)` → `integrity_check`, evidenced by one hash-chained `forget` audit row. Freelist reads back 0. Does NOT touch filesystem copies, `<db>.bak` snapshots, standby follower chunks, or SSD wear-leveling — printed on every run. Run per domain DB, ideally in a quiet moment (VACUUM holds the writer). |

## Examples

```bash
# Health + stats
brain status

# Structured recall with lexical control
brain query "blueberry alternative" --phrase "antioxidant" --exclude "smoothie" --k 5

# Explain why results were chosen
brain explain "blueberry alternative"

# Ingest a whole vault directory (dry-run first, then for real)
brain ingest-dir ~/notes/health --dry-run
brain ingest-dir ~/notes/health

# Check the memory for duplicates and conflicts
brain check-consistency

# Verify row-level governance integrity (owner/scope/domain tamper-evidence)
brain verify-governance

# Back up the database (passphrase via file; DB path from BRAIN_DB_PATH)
brain backup ~/backups/brain-$(date +%F).enc --passphrase-file ~/.config/brain-server/backup.pass
```

## Next steps

- **[API Reference](./api.md)** — the same surface over HTTP.
- **[Client GUI](./client-gui.md)** — the same surface as a visual app.
- **[Quickstart](./quickstart.md)** — a working end-to-end example.
