# CLI Reference

The `brain` binary is the operator command-line surface. This page is the command reference.
The CLI covers retrieval, ingest (directories), self-correction, domain/retention/backup/key
management, UMP, and health — the commands it ships in `src/bin/brain.rs` (hand-rolled argument
parsing, no clap). For actions the CLI does **not** expose (erasure, proposal approval, DSAR,
reading the audit log), use the HTTP API or the client console.

## Health & operations

| Command | Purpose |
|---|---|
| `brain doctor` [`--backup <path> [--passphrase-file PATH]`] | Health + readiness; optionally verify a backup file |
| `brain status` | Counts, model, version |
| `brain check-consistency` | Report duplicates, conflicts, stale sources, near-duplicates |
| `brain snapshot-status` | Show the point-in-time snapshot state |
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
| `brain source-delete <id>` | Retire a source |

## Domains & retention

| Command | Purpose |
|---|---|
| `brain domain-move <id> [<id> ...] --to <domain> [--confirm global]` | Move chunks to another domain |
| `brain domains-recompute` | Recompute domain membership / stats |
| `brain retention get` \| `set <kind> <days>` | Per-kind retention expiry policy |

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
| `brain key generate` [`--kid ID`] [`--alg RS256`] [`--dir PATH`] | Generate an RSA-2048 JWT signing keypair (JWT mode) |
| `brain key list` [`--dir PATH`] | Show loaded keys |
| `brain key prune` [`--dir PATH`] [`--keep N`] | Drop expired keys from JWKS |

## UMP (Universal Memory Protocol)

| Command | Purpose |
|---|---|
| `brain ump export [--format md\|ump] [--out FILE]` | Export the memory corpus |
| `brain ump import <file>` | Import a UMP export |
| `brain ump keygen [--dir PATH]` | Generate the UMP operator (Ed25519) signing key |

## Backup & restore

| Command | Purpose |
|---|---|
| `brain backup <out-path>` [`--passphrase-file PATH`] | Encrypted AES-256-GCM backup (checksummed, excludes secrets). DB path is taken from `BRAIN_DB_PATH`/default, not a positional. A passphrase is required. |
| `brain restore <in-path>` [`--passphrase-file PATH`] | Restore from an encrypted backup |

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
