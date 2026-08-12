# CLI Reference

The `brain` binary is the operator command-line surface. It gives you everything the HTTP API does, from a terminal. This page is the complete command reference.

## Health & operations

| Command | Purpose |
|---|---|
| `brain doctor` | Health + readiness |
| `brain status` | Counts, model, version |
| `brain audit [--kind K] [--limit N]` | Read the audit log |
| `brain check-consistency` | Report duplicates, conflicts, stale sources, near-duplicates |

## Retrieval

| Command | Purpose |
|---|---|
| `brain query "q"` [`--phrase` …] [`--exclude` …] [`--code` …] [`--source` …] [`--since` …] [`--k N`] [`--explain`] | Structured recall |
| `brain get <id>` | Fetch a chunk |
| `brain explain "q"` | Provenance + telemetry |
| `brain suggest "<context>"` `[--exclude id[,id...]]` `[--k N]` `[--session S]` | Opt-in anticipation pull |
| `brain suggest-feedback <id> accept\|dismiss` `[--reason "..."]` `[--session S]` | Record a suggestion outcome |
| `brain suggest-metrics` `[--session S]` `[--since DATE]` | False-positive rate over the feedback ledger |

## Ingest & sources

| Command | Purpose |
|---|---|
| `brain ingest-dir <path>` [`--dry-run`] | Ingest a vault directory |
| `brain reconcile <path>` [`--dry-run`] | Sweep deleted sources |
| `brain source-delete <id>` | Retire a source |

## Self-correction & maintenance

| Command | Purpose |
|---|---|
| `brain resolve <new_id> <old_id>` | Mark new chunk as superseding old; expires old from current recall |
| `brain undo-resolve <old_id> [<old_id> ...]` | Reverse a prior supersession; restores chunk to current recall |
| `brain procedure <title>` [`--step "title: content"` …] [`--domain D`] | Ingest a root + ordered steps in one transaction |
| `brain classify "<text>"` | Deterministic keyword categorization |
| `brain evaluate <decision_id>` `--var name=value` … | Evaluate a stored decision rule |

## Connectors

| Command | Purpose |
|---|---|
| `brain connect github --app-id N --install-id N --key-file PATH --repo O/R` | Configure the GitHub connector |
| `brain sync [github]` `[--config PATH \| --instance NAME]` | Run a connector sync |
| `brain connector-status` | List registered connectors |

## JWT key management

| Command | Purpose |
|---|---|
| `brain key generate` [`--kind rsa`] | Generate a JWT signing keypair (JWT mode) |
| `brain key list` | Show loaded keys |
| `brain key prune` | Drop expired keys from JWKS |

## Backup & restore

| Command | Purpose |
|---|---|
| `brain backup <db> <out>` | Encrypted AES-256-GCM backup (checksummed, excludes secrets) |
| `brain restore <backup> <db>` | Restore from an encrypted backup |

## Examples

```bash
# Health + stats
brain status

# Structured recall with lexical control
brain query "blueberry alternative" --phrase "antioxidant" --exclude "smoothie" --k 5

# Explain why results were chosen
brain explain "blueberry alternative"

# Ingest a whole vault directory
brain ingest-dir ~/notes/health

# Approve a human-gated write-back candidate
brain proposals                     # list the queue
brain proposal-approve <id>          # promote it to memory

# Check the memory for duplicates and conflicts
brain check-consistency

# Back up the database
brain backup ~/.openclaw/workspace/brain.db ~/backups/brain-$(date +%F).enc
```

## Next steps

- **[API Reference](./api-reference.md)** — the same surface over HTTP.
- **[Client GUI](./client-gui.md)** — the same surface as a visual app.
- **[Quickstart](./quickstart.md)** — a working end-to-end example.
