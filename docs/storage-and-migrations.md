# Storage and migrations

*Where the bytes live, how the schema advances, and how to rehearse an upgrade before it touches the live DB. Every claim here is read from `src/storage_layout.rs`, `src/migration.rs`, `src/bin/brain_migrate_rehearse.rs`, `src/capacity.rs`, `src/backup.rs`, `src/server/bootstrap.rs`, and `src/bin/brain.rs`.*

**Verified against:** package `v1.29.2` (`Cargo.toml:3`) at `bea659a0` (2026-10-06). The migration in that tree stamps `schema_version = '1.32.26'` (`src/migration.rs:3188`) and `LATEST_KNOWN_SCHEMA` is `1.32.26` (`src/storage_layout.rs:323`). Schema constants therefore run ahead of the package version — read the stamp, not the tag. If this document and the code disagree, the code is right.

**What this page is not:** [memory-lifecycle](./memory-lifecycle.md) owns the write path (capture → gate → admission) and its §5 table summary; [deployment-filesystem](./deployment-filesystem.md) owns the mount, WAL, pragma-tuning, and ranked backup-mechanism reference (§1–§4). This page owns the file layout, the version-advance discipline, the rehearsal tool, the backup/migration interplay, and the upgrade runbook. It links to those pages where they are authoritative rather than repeating them.

---

## 1. Storage layout: one root, derived paths

All on-disk paths derive from one root (`src/storage_layout.rs:449-580`). Resolution order in `StorageLayout::detect()` (`src/storage_layout.rs:462-467`):

1. `BRAIN_DATA_ROOT` — the relocation knob. Must be absolute; any value containing `..` is refused (`InvalidRoot`).
2. The parent of `BRAIN_DB_PATH` — preserves the install layout.
3. `~/.openclaw/workspace` — the historical default.

| Path | Derived as | Status |
|---|---|---|
| Legacy live DB | `legacy_db()`: `BRAIN_DB_PATH` verbatim, else `<root>/brain.db` (`src/storage_layout.rs:519-537`) | **What the runtime reads today.** |
| Candidate global DB | `global_domain_db()`: `<root>/global.db` (`src/storage_layout.rs:542-544`) | Rehearsal `dest` default. The multi-db cutover target; the live runtime still reads `legacy_db()`. |
| Per-domain file | `domain_db(name)`: `<root>/brain-<domain>.db` (`src/storage_layout.rs:549-554`) | Validated by `is_valid_domain` (`^[a-z0-9][a-z0-9_-]{0,62}$`, `src/storage_layout.rs:400-410`). `../evil`, `a/b`, uppercase, spaces all refuse with `InvalidDomain`. |
| Backups | `backups_dir()`: `<root>/backups` (`src/storage_layout.rs:558-560`) | Replaces the old CWD-relative default in `backup.rs`. |
| Registry | `registry_db()`: `<root>/registry.db` (`src/storage_layout.rs:563-566`) | Created lazily; does not exist unless `BRAIN_MULTI_DB=true`. |
| Connector configs | `~/.config/brain-server/connectors` (`src/storage_layout.rs:570-573`) | Lifted from `backup::default_connector_config_dir`; one source of truth. |

Residency stamp: `BRAIN_REGION` → `knowledge.region` via `storage_layout::region()` (`src/storage_layout.rs:372-393`). Shape is lowercase alnum + hyphen, 1–63 chars, alnum first; anything else yields `None` (no stamp, pre-v1.22 behavior). The trigger backfills only `NULL` rows — a region change never rewrites where old rows lived (`src/migration.rs:1385-1429`).

Connection posture (why two pragma stories exist, both true): the one-shot migration connection sets `PRAGMA synchronous=NORMAL` (`src/migration.rs:56-64`); the pooled live connections default to `FULL` and only the migration connection ever sets `NORMAL` (`src/capacity.rs:57-58`, `SynchronousMode::#[default]`). `BRAIN_SYNCHRONOUS=normal` opts into the faster posture; `BRAIN_WAL_AUTOCHECKPOINT` bounds the checkpoint pages (`src/config.rs:659-704`). Full tuning table lives in [deployment-filesystem §3](./deployment-filesystem.md#3-the-tunings-the-server-actually-applies).

## 2. Migration discipline: how versions advance

`run_migration` is **idempotent, additive-only, and runs unchanged on every per-domain file** (`src/migration.rs:1-10`). The pattern throughout is `CREATE TABLE/INDEX IF NOT EXISTS` plus guarded `ALTER TABLE … ADD COLUMN` probed via `pragma_table_info` — re-running is a no-op, never a rebuild (a rebuild is the one operation that can lose rows under a crash; stated at `src/migration.rs:2800-2803`).

### 2.1 The gates that run before any DDL

- **WAL readback.** `PRAGMA journal_mode=WAL` succeeds even when it cannot apply, so the migration reads the mode back and refuses anything filesystem-backed that is not `wal` (`memory` is allowed: a deliberate in-memory test store, `src/migration.rs:67-108`). The refusal names the cause and the remedy (local block filesystem). Detail and mount guidance: [deployment-filesystem §1](./deployment-filesystem.md#1-the-recommended-filesystem).
- **Embedding-dimension stamp.** `schema_meta.embedding_dim` is checked *before* the vec0 DDL because the DDL interpolates the dim (`src/migration.rs:389-431`). Fresh DB stamps the active embedder's `store_dim`; same dim is a no-op; different dim returns `Err` naming both dims and directing the operator to `brain-server --re-embed <profile>` (`src/migration.rs:410-414`). The default `run_migration` path builds at 512-d; the live boot path passes the active profile's `store_dim` (512 edge / 768 desktop / 1024 enterprise, `src/migration.rs:30-36`). A cross-dim comparison would be garbage recall, so it fails closed rather than auto-migrating.
- **Newer-schema refusal.** `refuse_newer_schema` compares numerically (`schema_cmp`, `src/storage_layout.rs:329-333` — lexicographic would misorder `1.28.9` vs `1.28.77`) and refuses a DB stamped newer than `LATEST_KNOWN_SCHEMA` (`src/storage_layout.rs:348-356`). `None` (pre-`schema_meta` legacy) is never newer — it is always an upgrade. The lockstep test `latest_stamp_matches_migration` fails the build if the stamp and the const drift (`src/storage_layout.rs:793-830`).

`schema_meta` keys the migration reads/writes: `embedding_dim`, `vec_metric` (`cosine`, `src/migration.rs:465-495`), `schema_version` (`1.32.26`, `src/migration.rs:3187-3191`), `audit_chain_head` (v1.27.31 pin, `src/migration.rs:3193-3233`; epoch key `audit_chain_epoch` is runtime-written, absent = legacy).

### 2.2 The 1.32.x schema story (what each stamp added)

Constants live in `src/storage_layout.rs:192-301`; DDL lives in `src/migration.rs` at the cited sites. All are additive; v1.28.18 onward the down-migration is a documented no-op (keep the column/table, drop the code).

| Stamp | What it added (real table / column names) |
|---|---|
| `1.32.0` | `agent_session_events` — append-only session event log, `UNIQUE(run_id, seq)` + `UNIQUE(run_id, idempotency_key)` (`src/migration.rs:2374-2388`). |
| `1.32.11` | `decision_run_traces` — digests and refs per decision run, the `recall_traces` precedent (`src/migration.rs:2399-2411`). |
| `1.32.12` | `proposals.decision_run_ref` — nullable provenance ref; `NULL` for every ordinary human/loop proposal (`src/migration.rs:2464-2474`). |
| `1.32.13` | `decision_model_registry` — one digest-pinned row per `(id, version)` model identity (`src/migration.rs:2480-2500`). |
| `1.32.14` | `decision_evaluation_runs` — bounded evaluation records, `acceptance_state = 'operator_accepted_non_authoritative'` (`src/migration.rs:2505-2533`). |
| `1.32.15` | `delivery_traces` + `delivery_budgets` — per-run trace index (refs/digests, `blast_radius` admitted by CHECK but unenforced) and per-run budget head, stored-unenforced (`src/migration.rs:2544-2584`). |
| `1.32.16` | `delivery_attestations` (twelve evidence columns, never a disposition) + `delivery_traces.seq` with `UNIQUE(run_id, seq)`; pre-1.32.16 rows are backfilled `1..n` per run in `(created_at, rowid)` order before the index is created (`src/migration.rs:2599-2665`). |
| `1.32.17` | `delivery_bindings` — standing per-tenant authority, `UNIQUE(domain, target_kind, target_ref)`; `secret_file_name` added by guarded `ALTER` for DBs that ran the first 1.32.17 batch (`src/migration.rs:2690-2735`). |
| `1.32.18` | `delivery_releases` — governed release row with nine-value `ReleaseStatus` CHECK and approval-as-columns (`src/migration.rs:2764-2798`). |
| `1.32.19` | `claim_schemas`, `claim_batches`, `claims`, `claim_evidence` plus four write-fence triggers (`claims_fence_recall_visibility`, `claims_fence_cid_rewrite`, `claims_fence_self_ratification`, `claims_fence_batch_flip`) (`src/migration.rs:2804-2984`). |
| `1.32.20` | `workflow_runs.knowledge_version` — nullable integer basis marker; `NULL` = predates tracking (`src/migration.rs:2355-2365`). |
| `1.32.21` | `decision_run_traces.model_registry_id/_version/_digest` — nullable citation triple, same names as the evaluation table so the two join with no translation (`src/migration.rs:2434-2455`). |
| `1.32.22` / `1.32.23` | `claims.disproof_form/_body/_op/_citation/_coverage/_audit_ref` (six) + `claims.disproof_scope` (seventh); `NULL` = predates tracking, stamp-blind by declaration (`src/migration.rs:3009-3065`). |
| `1.32.24` | `knowledge_domain_versions` — one `(domain, version, bumped_at, bumped_by, bumped_article)` row per domain (`src/migration.rs:3078-3087`). |
| `1.32.25` | `delivery_traces.model_registry_id/_version` — the resolver-returned registry key, deliberately outside the row content address (`src/migration.rs:3121-3141`). |
| `1.32.26` | `proposals.promoted_chunk_id` — nullable integer written at approve time beside the decision CAS; closes the approved-proposal plaintext surviving a DSAR certificate (`src/migration.rs:3169-3185`). |

Older tables the rehearsal verifies (full list is `PARITY_TABLES`, `src/bin/brain_migrate_rehearse.rs:58-162`): `knowledge`, `embeddings`, `vec_knowledge`, `knowledge_fts` (explicit check, not in the list), `entities`, `relationships`, `tombstones`, `sources`, `source_revisions`, `connectors`, `connector_checkpoints`, `audit_events`, `webhook_queue`, `webhook_seen`, `evidence_links`, `revoked_tokens`, `refresh_chains`, `retention_policy`, `profiles`, `domain_profiles`, `legal_holds`, `roles`, `breach_events`, `breaches`, `transfers`, `clients`, `proposals`, `recall_traces`, `dsar_requests`, `suggest_feedback`, `shifts`, `presence`, `principal_skills`, `crew_config`, `handover_offers`, `case_notes`, `case_status_refs`, `kcs_translations`, `agent_cards`, `delegations`, `parcel_ledger`, `consent_registry`, `channel_threads`, `channel_user_map`, `valet_consents`, `workflow_runs`, `workflow_steps`, `outbox`, `findings`, `contradictions`, `case_articles`, `crm_cases`, `revoked_principals`, `rules`, `rule_rates`, `domain_centroids`, plus every 1.32.x table above.

Reversibility: the only down-migration is `migrate_down_0_9_0` (drops vec0 + FTS5 + vocab + triggers, keeps `knowledge` + JSON `embeddings`, `src/migration.rs:3357-3380`). Everything from v1.28.18 onward is one-way by declaration.

## 3. Rehearsal procedure (`brain-migrate-rehearse`)

A standalone binary, not a `brain` subcommand, because it must run against a **stopped** server — a hot copy would miss WAL pages (`src/bin/brain_migrate_rehearse.rs:10-12`). Feature-gated so the default build is unchanged (`Cargo.toml:19-21`, `Cargo.toml:323-329`):

```sh
cargo run --release --features migrate --bin brain-migrate-rehearse -- \
  <backup|copy|verify|report|rollback|rehearse> \
  [--source PATH] [--dest PATH] [--strict] [--force] [--keep-snapshot]
```

Defaults: `--source` = `legacy_db()`, `--dest` = `global_domain_db()` (`src/bin/brain_migrate_rehearse.rs:277-286`).

| Phase | What it does (never touches the live runtime beyond reading the source) |
|---|---|
| `backup` | Encrypted pre-rehearsal snapshot via `backup::backup` + `backup::verify`; passphrase from `BRAIN_BACKUP_PASSPHRASE_FILE` → `BRAIN_BACKUP_PASSPHRASE` (`src/bin/brain_migrate_rehearse.rs:291-318`, `896-918`). A failed verify is a hard stop. |
| `copy` | Refuses newer-schema *before* touching dest, then `VACUUM INTO '<dest>'` from the same version-checked session (no TOCTOU), then `run_migration` on dest — the exact cutover code path (`src/bin/brain_migrate_rehearse.rs:322-393`). Writes `<dest>.copy-meta.json` atomically (tmp + rename, `src/bin/brain_migrate_rehearse.rs:415-430`). |
| `verify` | Refuse-newer again (the source may have been upgraded since `copy`), then: per-table row counts, `knowledge_fts` count, `content_hash` multiset, source/revision linkage count, `schema_version` dest ≥ source, and a 50-row random vec0 byte spot-check. Prints a markdown table, writes `<dest>.verify-report.md`, exits non-zero on any `FAIL` (`src/bin/brain_migrate_rehearse.rs:476-607`, `714-752`). |
| `report` | Pure-read human summary (sizes, versions, per-table counts). Opens no write tx (`src/bin/brain_migrate_rehearse.rs:756-797`). |
| `rollback` | Removes dest + sidecars (`.copy-meta.json`, `.verify-report.md`, `.rehearsal-source.sha256`). Never touches source (`src/bin/brain_migrate_rehearse.rs:801-827`). |
| `rehearse` | `backup → copy → verify → report`; on success rolls back unless `--keep-snapshot`; on failure leaves dest in place for inspection (`src/bin/brain_migrate_rehearse.rs:831-854`). |

Hot-server guard: a source `-wal` larger than 1024 bytes refuses unless `--force` (`WAL_ACTIVE_HEURISTIC_BYTES`, `src/bin/brain_migrate_rehearse.rs:170-173`, `868-891`). The comment is explicit that the precise check (`wal_checkpoint`) would mutate the file, so the heuristic stands. `--strict` currently escalates nothing (all checks emit OK/FAIL; reserved for future WARN-class checks).

## 4. Backup/restore interplay

This section is the migration operator's view. The mechanism reference — ranked options, `brain.db` + `brain.db-wal` travel together, never hand-delete `-wal`, `VACUUM INTO` target must not exist, 2× headroom, the `close()` hazard, `integrity_check` + `foreign_key_check` — lives in [deployment-filesystem §4](./deployment-filesystem.md#4-backup-and-restore) and is not repeated here.

What migration adds to that picture:

- **Rehearse from a copy, restore from a backup.** The rehearsal's `copy` phase *is* a `VACUUM INTO` (defragmented, WAL-flattened) followed by `run_migration` — the same primitive the runbook uses for a pre-migration snapshot. The rehearsal's `backup` phase uses the product backup writer (`backup::backup`/`verify`), not a bare `cp`.
- **Real `brain` verbs** (full reference: [cli-reference](./cli-reference.md)): `brain backup <out-path>`, `brain restore <in-path> [--yes] [--allow-chainless]`; `brain standby ship --to <dir>` (one cycle, timer-owned) / `start` (loop) / `status` / `promote-check --from <dir>` (the drill: restore follower to temp, replay WAL, `integrity_check`, print measured RTO + computed RPO); `brain shred [--db PATH] --yes` (post-purge freelist drop; per-domain DB, quiet moment, `VACUUM` holds the writer). `brain doctor` can verify a backup file.
- **Chain-aware restore.** A chain-less image refuses without `--allow-chainless`; a legacy-epoch (unkeyed) chain restores with `forgeable: true` until the operator re-anchors with the server binary's offline `brain-server --re-audit` (`src/backup.rs:1065-1100`, `src/server/bootstrap.rs:245-314`). `--re-audit` completion itself instructs: run `brain backup` now — the post-anchor snapshot is the new baseline.
- **Dimension changes are not migrations.** An `embedding_dim` mismatch fails closed at boot; the sanctioned bypass is offline `brain-server --re-embed <profile>`, which repoints the stamp, drops/recreates `vec_knowledge` at the new dim, clears legacy `embeddings`, and leaves the store **empty** for the caller to re-embed every chunk (`src/migration.rs:3320-3354`, `src/server/bootstrap.rs:201-243`). Treat it like a re-index window, not a rolling upgrade.

## 5. Operator runbook for upgrades

1. **Stop the server.** The rehearsal refuses a hot source (`-wal` > 1 KiB without `--force`). Do not `--force` past this on a live deployment — stop first, then the heuristic passes silently.
2. **Snapshot.** `brain backup <timestamped-path> --passphrase-file <file>` (passphrase ladder: `BRAIN_BACKUP_PASSPHRASE_FILE` → `BRAIN_BACKUP_PASSPHRASE`). Keep the verified `.bbk`; it is the rollback anchor.
3. **Rehearse the cutover on a copy.**
   ```sh
   cargo run --release --features migrate --bin brain-migrate-rehearse -- \
     rehearse --keep-snapshot
   ```
   Read `<dest>.verify-report.md`. Any `FAIL` (row count, `content_hash` multiset, FTS, vec0 bytes, `schema_version` downgrade) is a stop: inspect dest, do not proceed.
4. **Upgrade the binary, then boot.** Boot runs `run_migration` on every domain file. Expected: idempotent no-op if the rehearsal already brought the copy up, additive backfills otherwise.
5. **Watch for the two loud refusals**, both fail-closed by design:
   - `journal mode is '…' , not 'wal'` → move the data directory to local block storage ([deployment-filesystem §1](./deployment-filesystem.md#1-the-recommended-filesystem)). No override exists.
   - `embedding dimension mismatch … run brain-server --re-embed <profile>` → switch to a compatible profile, or schedule the offline re-embed window (store goes empty mid-procedure).
   - `DB schema … is newer than this binary knows` → a newer release owns this file; upgrade `brain-server` before touching it (`StorageLayoutError::SchemaTooNew`, `src/storage_layout.rs:424-435`).
6. **Verify the live DB.** `brain doctor`, plus both `PRAGMA integrity_check` and `PRAGMA foreign_key_check` on a *copy* (never probe the live file with a second opener — the `close()` hazard in [deployment-filesystem §4](./deployment-filesystem.md#4-backup-and-restore)). For standby deployments, `brain standby promote-check --from <dir>` gates promotion on measured RTO/RPO.
7. **Roll back by restoring, never by downgrading.** There is no supported down-migration past v0.9.0. A bad upgrade is `brain restore <verified-bbk> --yes` (chain flags above apply), not an old binary against new tables.

## 6. Honest limits and ceilings

- **No down-migration past v0.9.0.** Only `migrate_down_0_9_0` exists (vec0 + FTS5 removal); every v1.28.18+ step is a documented one-way no-op. Rolling back means restoring a backup.
- **The rehearsal proves parity, not recall quality.** The formal guarantee is row counts + `content_hash` multiset; the 50-row vec0 spot-check is a heuristic for the silent-corruption class (`VEC_SPOT_CHECK_SIZE`, `src/bin/brain_migrate_rehearse.rs:164-168`). A green report does not certify ranking.
- **Excluded from parity by declaration, not oversight** (`src/bin/brain_migrate_rehearse.rs:46-56`): `schema_meta` counts (version bump + audit-head pin move legitimately), FTS5 shadows, `sqlite_sequence`, and `oversight_evidence` + `ropa_registry` under default builds (feature `compliance-pack` only — `0 = 0` there would be theater).
- **Version skew is real in this tree.** Package `1.29.2` ships schema `1.32.26`. Operators must compare `schema_meta.schema_version` via `report`, never assume package == schema.
- **`knowledge_version` records; it does not yet prevent.** The per-case basis (`1.32.20`) is written as a constant and the per-domain counter (`1.32.24`) has no Evolve bump site yet — mixed-basis prevention lives in an undefined delta offer (`src/migration.rs:2351-2354`, `3067-3077`).
- **`blast_radius` is stored, unenforced** (`src/migration.rs:2541-2543`); **attestations are evidence, never dispositions** (no status/decision column by design, `src/migration.rs:2595-2598`); **the delivery trace id does not commit to the model citation** (folding it in would re-derive every historical id, `src/migration.rs:3114-3120`).
- **`brain shred` is per-file and partial by print.** It drops freelist residue in one domain DB; filesystem copies, `<db>.bak`, standby chunks, and SSD wear-leveling are excepted — printed on every run (`src/bin/brain.rs:3138-3162`).
- **Pre-1.32.26 rows are stamp-blind, not evidence.** `NULL` disproof columns, `NULL` citations, `NULL` `promoted_chunk_id`, and `'global'`-backfilled `proposals.domain` mean "predates tracking" — never read them as proof of soundness, attribution, or residency.
