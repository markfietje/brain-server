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
| `brain doctor` [`--backup <path> [--passphrase-file PATH]`] | Health + readiness; optionally verify a backup file |
| `brain kb build --domain <d> --out <dir> [--db <path>] [--base-url <url>] [--with-case-status] [--locales en,de,fr,es,nl]` | Build the public KB as a static artifact from published articles (deterministic bytes + SHA-256 manifest carrying the Art 50(2) provenance seal; sign before hosting). `--with-case-status` also emits the live `status/{ref}.json|.html` case-status pages; `--locales` emits per-locale pages with hreflang alternates |
| `brain status` | Counts, model, version |
| `brain check-consistency` | Report duplicates, conflicts, stale sources, near-duplicates |
| `brain snapshot-status` | Show the point-in-time snapshot state |
| `brain setup [domain] [--profile NAME] [--yes]` | Interactive first-run: pick a profile preset, preview its knobs, bind it to a domain (`--yes` scripts it) |
| `brain bench` | Benchmark harness (feature-gated `bench`) |

## Retrieval

| Command | Purpose |
|---|---|
| `brain query "q"` [`--phrase …`] [`--exclude …`] [`--code …`] [`--source …`] [`--since DATE`] [`--k N`] [`--intent …`] [`--profile …`] [`--graph`] [`--explain`] | Structured recall |
| `brain get <id>` | Fetch a chunk |
| `brain explain "q"` | Provenance + telemetry |
| `brain suggest "<context>"` `[--exclude id[,id...]]` `[--k N]` `[--session S]` `[--domain D]` | Opt-in anticipation pull |
| `brain suggest-feedback <id> accept\|dismiss` `[--reason "..."]` `[--session S]` | Record a suggestion outcome |
| `brain suggest-metrics` `[--session S]` `[--since DATE]` | False-positive rate over the feedback ledger |

## Ingest & sources

| Command | Purpose |
|---|---|
| `brain ingest-dir <path>` [`--dry-run`] [`--replace`] [`--source S`] [`--domain D`] | Ingest a vault directory |
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
| `brain client add <name> --domain D --jurisdiction J [--profile P] [--yes]` | Register an operating client (one isolation domain per client) |
| `brain client dpa get <name>` | Show a client's DPA terms |
| `brain client dpa set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S` | Set a client's DPA terms |
| `brain client dsar <name> <subject> [--action purge\|export\|both] [--dry-run]` | Run a per-client jurisdiction-aware DSAR |
| `brain client hold add <name> <id> [<id> ...] --reason R` \| `list <name>` | Legal-hold / release a client's domain; list holds |
| `brain client qa list <name>` \| `coach <name> <id> --note N [--flag]` | Supervisor QA queue + coaching note (v1.27.8, Admin) |
| `brain client end <name> [--purge\|--return] [--dataset D] [--yes]` | Terminate a client: purge-or-return + archive + certificate |

## Self-correction & maintenance

| Command | Purpose |
|---|---|
| `brain resolve <new_id> <old_id>` | Mark new chunk as superseding old; expires old from current recall |
| `brain undo-resolve <old_id> [<old_id> ...]` | Reverse a prior supersession; restores chunk to current recall |
| `brain procedure <title>` [`--step "title: content"` …] [`--domain D`] | Ingest a root + ordered steps in one transaction |
| `brain classify "<text>"` | Deterministic keyword categorization |
| `brain evaluate <decision_id>` `--var name=value` … | Evaluate a stored decision rule |
| `brain eval` [`--floor r5=0.85 r10=0.9`] | Run the frozen recall-eval harness (feature-gated `bench`) |

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

## Token management

| Command | Purpose |
|---|---|
| `brain token rotate` | Atomically rotate the bearer token (v1.27.12): a fresh 32-byte hex token is written to a 0600 temp file (`create_new`, never umask-dependent), fsync'd, and renamed over the configured token file. Refuses to overwrite a group/world-readable target. Restart the server to pick it up. |

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
| `brain parcel import --file <file> --domain <d> [--expected-signer <did>]` | Verify + import a parcel; rows land as pending proposals, never direct writes |
| `brain parcel ledger [--domain <d>]` | Show the parcel crossing ledger |

## Personal assistant & compliance register (v1.28.42+)

| Command | Purpose |
|---|---|
| `brain valet add "what" --at <iso\|HH:MM\|unix>` `[--repeat none\|daily\|weekly]` `[--domain D]` | Add a valet reminder |
| `brain valet due [--now <unix>]` \| `brain valet brief` \| `brain valet consent grant\|revoke` | Due items, the brief, and consent state |
| `brain ropa list` \| `brain ropa add --activity A --controller C --processor P --lawful-basis B` `[--categories S] [--recipients S] [--retention-days N] [--security-measures S] [--transfers S]` | Records-of-processing register (read + propose an activity row) |

## Backup & restore

| Command | Purpose |
|---|---|
| `brain backup <out-path>` [`--passphrase-file PATH`] [`--format v1\|v2\|v3`] | Encrypted AES-256-GCM backup (checksummed, excludes secrets; v3 is the current format — header bytes are GCM AAD). DB path is taken from `BRAIN_DB_PATH`/default, not a positional. A passphrase is required. |
| `brain restore <in-path>` [`--passphrase-file PATH`] | Restore from an encrypted backup |

## Warm standby (v1.28.61)

Warm, never hot: the shipper is an operator-run process (launchd/systemd —
see deployment.md), NOT a server thread, and promote is a rehearsed manual
step. There is NO hot failover and NO RPO=0 claim anywhere.

| Command | Purpose |
|---|---|
| `brain standby start --to <dir>` `[--interval-secs 30]` `[--passphrase-file PATH]` | Long-running shipper: per cycle a PASSIVE wal_checkpoint, then the encrypted base via the backup v3 writer, the WAL chunk (same v3 encryption — no plaintext at rest), and the signed manifest (written last). An interrupted cycle self-heals on the next one. |
| `brain standby status [--to <dir>]` | Integrity self-check of the follower: verifies the manifest's Ed25519 signature and recomputes artifact hashes — any tamper or torn cycle FAILS (exit 1). Prints cycle, age, cycles behind, and `rpo_max = interval + checkpoint lag`. |
| `brain standby promote-check --from <dir>` `[--passphrase-file PATH]` | THE DRILL: restores the follower into a temp dir (the shipped restore path), replays the WAL chunk, runs `PRAGMA integrity_check`, and prints measured RTO plus computed RPO. Exit code gates. |

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

# Back up the database (passphrase via file; DB path from BRAIN_DB_PATH)
brain backup ~/backups/brain-$(date +%F).enc --passphrase-file ~/.config/brain-server/backup.pass
```

## Next steps

- **[API Reference](./api.md)** — the same surface over HTTP.
- **[Client GUI](./client-gui.md)** — the same surface as a visual app.
- **[Quickstart](./quickstart.md)** — a working end-to-end example.
