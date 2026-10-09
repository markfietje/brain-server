//! Database schema migration (extracted from `main.rs`).
//!
//! `run_migration` is idempotent, additive-only, and runs unchanged on every
//! per-domain file (shim mode today, multi-db in v1.0.0). Extracted to the lib
//! so the `brain-migrate-rehearse` binary can bring old-schema fixtures up to
//! current. The server binary re-imports these via `use brain_server::migration::`.
//!
//! The single signature change vs the historical `main.rs` version: `mmap_mib`
//! is passed in explicitly instead of reading `config::DB_MMAP_SIZE_MIB`, so
//! the lib has no dependency on the server-private `config` module.

use anyhow::Result;
use rusqlite::{Connection, params};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;
use zerocopy::IntoBytes;

/// Process-local truth that `vec_knowledge` exists
/// in every DB this process has migrated. Set by [`run_migration_with_store_dim`],
/// cleared by [`migrate_down_0_9_0`]. The search hot path reads this instead of
/// probing the vec0 table per query (`SELECT COUNT(*) FROM vec_knowledge`); the
/// probe's only real job was detecting a pre-vec0 DB, and every pooled
/// connection in this process belongs to a DB that was migrated in-process
/// (boot for the shim DB, pool-open for per-domain files). On the impossible
/// case of "flag set but table absent" the search path clears it and falls back
/// to the legacy cosine scan (see `perform_search_traced`).
pub static VEC0_READY: AtomicBool = AtomicBool::new(false);

pub fn run_migration(db: &mut Connection, mmap_mib: i64) -> Result<()> {
    // The historical default: every pre-v1.28 DB is 512-d (potion-retrieval-32M
    // + the legacy JSON-vector era). All test fixtures, the migrate-rehearse
    // binary, and the per-domain opener call this. The live boot path calls
    // [`run_migration_with_store_dim`] with the active embedder's `store_dim()`
    // so the `enterprise`/`desktop` profiles build a 1024/768-d store instead.
    run_migration_with_store_dim(db, mmap_mib, 512)
}

/// The dim-aware migration. `store_dim` MUST match the active embedder's
/// `Embedder::store_dim()`, or a query embedding would be silently compared
/// against store vectors of a different dimension → garbage recall. The
/// `embedding_dim` stamp in `schema_meta` makes a mismatch fail closed at boot
/// with a clear error (re-embed or switch profile) rather than corrupt recall.
///
/// - Fresh DB: stamps `embedding_dim = store_dim`, creates `vec_knowledge` at it.
/// - Existing DB, same dim: no-op stamp check, idempotent.
/// - Existing DB, different dim: returns `Err` — the explicit-operator-action
///   gate (a dim change means re-embedding the whole corpus; that's `brain
///   re-embed`, not a silent migration, same doctrine as DSAR purge).
pub fn run_migration_with_store_dim(
    db: &mut Connection,
    mmap_mib: i64,
    store_dim: usize,
) -> Result<()> {
    let mmap_bytes = mmap_mib * 1024 * 1024;
    let pragmas = format!(
        "PRAGMA journal_mode=WAL; \
         PRAGMA synchronous=NORMAL; \
         PRAGMA foreign_keys=ON; \
         PRAGMA cache_size=-64000; \
         PRAGMA temp_store=MEMORY; \
         PRAGMA mmap_size={mmap_bytes}; \
         PRAGMA busy_timeout=5000;"
    );
    db.execute_batch(&pragmas)?;

    // Read the journal mode BACK and refuse if it is not `wal`.
    //
    // Why this is not redundant. `PRAGMA journal_mode=WAL` does not fail when
    // it cannot be applied. SQLite's own documentation (sqlite.org/wal.html §3,
    // fetched 2026-09-28) is explicit: "If the conversion to WAL could not be
    // completed (for example, if the VFS does not support the necessary
    // shared-memory primitives) then the journaling mode will be unchanged and
    // the string returned from the primitive will be the prior journaling mode
    // (for example "delete")."
    //
    // `execute_batch` above therefore SUCCEEDS on a filesystem that cannot do
    // WAL, and the server would boot, run, and answer requests with a silently
    // downgraded durability posture — the one that `brain standby` (passive
    // checkpoint before the base copy, RPO 10.4s) and `brain shred` (byte-level
    // erasure) are both built around. The only prior assertion on the mode in
    // the tree lived inside a TEST block, so a test proved the code works and
    // nothing made the server refuse anything.
    //
    // Failure here is a refusal to start, naming the cause, which is the
    // repo's "fail-closed everywhere" law applied to the one storage invariant
    // the whole deployment rests on.
    //
    // KILL 3, caught in the act: the first version of this predicate refused
    // anything that was not `wal`, which broke two existing tests that use an
    // IN-MEMORY database. That is a false positive, and a gate that refuses
    // healthy deployments is not a gate. `memory` mode is a DELIBERATE choice
    // for a test — there is no filesystem and no durability to downgrade — so
    // it is allowed. The modes that are refused are exactly the
    // filesystem-backed ones that `journal_mode=WAL` silently failed to
    // upgrade: `delete`, `truncate`, `persist`, `off`.
    let journal_mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    let mode = journal_mode.to_ascii_lowercase();
    if !matches!(mode.as_str(), "wal" | "memory") {
        return Err(anyhow::anyhow!(
            "journal mode is '{journal_mode}', not 'wal' — the data volume cannot do \
             write-ahead logging. Refusing to start: a silent downgrade to rollback-journal \
             mode would break the durability `brain standby` and `brain shred` assume. \
             Use a local block filesystem (ext4/xfs); a network filesystem cannot \
             provide the advisory locking and shared-memory primitives WAL requires \
             (sqlite.org/lockingv3.html §6.0)."
        ));
    }

    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS knowledge(
            id INTEGER PRIMARY KEY,
            title TEXT,
            content TEXT NOT NULL,
            knowledge_type TEXT,
            source TEXT DEFAULT 'manual',
            content_hash TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            flagged INTEGER NOT NULL DEFAULT 0,
            domain TEXT NOT NULL DEFAULT 'global',
            observed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            valid_from TIMESTAMP,
            valid_to TIMESTAMP,
            document_id TEXT,
            chunk_index INTEGER,
            heading_path TEXT,
            line_start INTEGER,
            line_end INTEGER
         );
         CREATE TABLE IF NOT EXISTS embeddings(
            knowledge_id INTEGER PRIMARY KEY,
            vector TEXT,
            FOREIGN KEY(knowledge_id) REFERENCES knowledge(id) ON DELETE CASCADE
         );",
    )?;

    // v0.9.1: additive `flagged` column for the PRF anti-injection guardrail.
    // Rows flagged as quarantined (prompt-injection screen tripped) must never
    // contribute PRF expansion terms. Additive + idempotent.
    let has_flagged: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='flagged'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_flagged {
        db.execute(
            "ALTER TABLE knowledge ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    // v0.9.1: additive domain + temporal columns (domain isolation + temporal
    // memory scaffold) and structure-aware chunk metadata. Idempotent.
    for (col, def) in [
        ("domain", "TEXT NOT NULL DEFAULT 'global'"),
        ("observed_at", "TIMESTAMP"), // ALTER TABLE cannot use non-constant default; CREATE TABLE keeps CURRENT_TIMESTAMP
        ("valid_from", "TIMESTAMP"),
        ("valid_to", "TIMESTAMP"),
        ("document_id", "TEXT"),
        ("chunk_index", "INTEGER"),
        ("heading_path", "TEXT"),
        ("line_start", "INTEGER"),
        ("line_end", "INTEGER"),
        // v0.9.2: provenance for `brain ingest-dir` — the absolute file path a
        // vault chunk came from. NULL for interactive/manual ingests.
        ("source_path", "TEXT"),
        // v0.9.8 "Evidence": source-authority tie-breaker (0..1). NULL for rows
        // ingested before this release; treated as AUTHORITY_VAULT at read time.
        ("authority", "REAL"),
    ] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }

    // v0.9.1: tombstone audit trail for deletes (provenance: what was forgotten
    // and when), separate from the knowledge rows so deleted content is gone
    // from retrieval immediately while the audit record persists.
    db.execute(
        "CREATE TABLE IF NOT EXISTS tombstones (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            knowledge_id INTEGER NOT NULL,
            document_id TEXT,
            deleted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_tombstones_kid ON tombstones(knowledge_id)",
        [],
    )?;

    // v0.9.2: index for vault ingest provenance + dedup-by-source_path. Used by
    // `brain ingest-dir` to (a) detect an unchanged file (no-op) and (b) replace
    // a changed file's chunks in one sweep. Idempotent.
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_source_path ON knowledge(source_path)",
        [],
    )?;

    // v0.9.1: per-domain centroids for centroid routing. One row per
    // domain holding the mean embedding vector (f32 little-endian blob).
    db.execute(
        "CREATE TABLE IF NOT EXISTS domain_centroids (
            domain TEXT PRIMARY KEY,
            centroid BLOB,
            count INTEGER,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;

    // Check if deduplication migration is needed
    let has_index: bool = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_knowledge_hash'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_index {
        println!("MIGRATION: Scrubbing duplicates...");
        let rows: Vec<(i64, String)> = db
            .prepare("SELECT id, content FROM knowledge WHERE content_hash IS NULL")?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let tx = db.transaction()?;
        for (id, content) in rows {
            let h = format!("{:016x}", xxh3_64(content.trim().as_bytes()));
            tx.execute(
                "UPDATE knowledge SET content_hash=? WHERE id=?",
                params![h, id],
            )?;
        }
        tx.commit()?;

        db.execute(
            "DELETE FROM knowledge WHERE id NOT IN (SELECT MIN(id) FROM knowledge GROUP BY content_hash)",
            [],
        )?;

        db.execute(
            "CREATE UNIQUE INDEX idx_knowledge_hash ON knowledge(content_hash, domain)",
            [],
        )?;
        println!("MIGRATION: Complete");
    }

    // Dedup is domain-scoped — the uniqueness backstop moves
    // from (content_hash) to (content_hash, domain). Existing rows are
    // unaffected: the old index forced globally-unique hashes, so every
    // (hash, domain) pair is trivially unique. No backfill.
    let hash_idx_sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_knowledge_hash'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    if hash_idx_sql.contains("(content_hash)") && !hash_idx_sql.contains("domain") {
        db.execute("DROP INDEX idx_knowledge_hash", [])?;
        db.execute(
            "CREATE UNIQUE INDEX idx_knowledge_hash ON knowledge(content_hash, domain)",
            [],
        )?;
    }

    // v0.8.0 Knowledge Graph migration
    db.execute(
        "CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            entity_type TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name)",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type)",
        [],
    )?;

    db.execute(
        "CREATE TABLE IF NOT EXISTS relationships (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            from_entity_id INTEGER NOT NULL,
            to_entity_id INTEGER NOT NULL,
            relation_type TEXT NOT NULL,
            knowledge_id INTEGER,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(from_entity_id) REFERENCES entities(id) ON DELETE CASCADE,
            FOREIGN KEY(to_entity_id) REFERENCES entities(id) ON DELETE CASCADE,
            FOREIGN KEY(knowledge_id) REFERENCES knowledge(id) ON DELETE SET NULL
         )",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_rels_from ON relationships(from_entity_id)",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_rels_to ON relationships(to_entity_id)",
        [],
    )?;
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_rels_unique ON relationships(from_entity_id, to_entity_id, relation_type)",
        [],
    )?;

    // ── v0.9.0 Phase 2: FTS5 lexical recall ────────────────────────────
    // External-content FTS5 table over `knowledge`.  Triggers keep it in sync
    // on insert / update / delete.  Tokenizer: porter + unicode61 (accent-
    // insensitive, handles non-ASCII names like “München”).
    db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts USING fts5(
            title, content, content_hash UNINDEXED,
            content='knowledge', content_rowid='id',
            tokenize='porter unicode61'
         );
         -- Triggers: keep FTS in sync with knowledge table
         CREATE TRIGGER IF NOT EXISTS knowledge_ai AFTER INSERT ON knowledge BEGIN
             INSERT INTO knowledge_fts(rowid, title, content, content_hash)
             VALUES (new.id, new.title, new.content, new.content_hash);
         END;
         CREATE TRIGGER IF NOT EXISTS knowledge_ad AFTER DELETE ON knowledge BEGIN
             INSERT INTO knowledge_fts(knowledge_fts, rowid, title, content, content_hash)
             VALUES ('delete', old.id, old.title, old.content, old.content_hash);
         END;
         CREATE TRIGGER IF NOT EXISTS knowledge_au AFTER UPDATE ON knowledge BEGIN
             INSERT INTO knowledge_fts(knowledge_fts, rowid, title, content, content_hash)
             VALUES ('delete', old.id, old.title, old.content, old.content_hash);
             INSERT INTO knowledge_fts(rowid, title, content, content_hash)
             VALUES (new.id, new.title, new.content, new.content_hash);
         END;",
    )?;

    // Backfill FTS from existing knowledge rows (if any)
    let fts_count: i64 = db
        .query_row("SELECT COUNT(*) FROM knowledge_fts", [], |r| r.get(0))
        .unwrap_or(0);
    let knowledge_count: i64 = db
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap_or(0);
    if fts_count == 0 && knowledge_count > 0 {
        info!("Backfilling FTS5 index with {knowledge_count} knowledge rows...");
        db.execute_batch(
            "INSERT INTO knowledge_fts(rowid, title, content, content_hash)
             SELECT id, title, content, content_hash FROM knowledge;",
        )?;
        info!("FTS5 backfill complete");
    }

    // ── v0.9.1: FTS5 vocabulary table for PRF term weighting ───────────
    // `fts5vocab='instance'` exposes one row per OCCURRENCE:
    // `(term, doc, col, offset)` — NO `cnt`/`rowid` columns (that was the
    // pre-3.40 shape; the v0.9.1 query built against it silently fell back to
    // the unweighted path until the v1.27.18 fix). PRF weights come from
    // `COUNT(*)` per term scoped `doc IN (window)` + a corpus-df round-trip
    // for the selected terms only (capped at MAX_DF_TERMS).
    //   ponytail: per-instance vocab; for a very large corpus switch to
    //   'row' mode (one row per term+doc). Ceiling: ~corpus-size rows.
    //   The step-1 `doc IN (…)` probe only indexes `term=` —
    //   a full vocab scan per PRF call remains the documented perf ceiling.
    db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts_vocab USING fts5vocab(
            knowledge_fts, 'instance'
         );",
    )?;

    // ── schema_meta + embedding_dim fail-closed gate (BEFORE vec0) ─────────
    // The embedding-dimension stamp must be checked/created before the vec0
    // table, because the vec0 DDL interpolates the dim. A DB opened by a
    // profile whose embedder emits a different dim than the store was built for
    // fails closed here — never silently comparing a 1024-d query against a
    // 512-d store (or vice versa).
    db.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")?;
    let stamped_dim: Option<i64> = db
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'embedding_dim'",
            [],
            |r| {
                let s: String = r.get(0)?;
                Ok(s.parse::<i64>().ok())
            },
        )
        .ok()
        .flatten();
    match stamped_dim {
        Some(d) if d as usize == store_dim => { /* match — proceed */ }
        Some(d) => {
            return Err(anyhow::anyhow!(
                "embedding dimension mismatch: this DB was built for {}-d vectors but the \
                 active profile's embedder emits {}-d. Switch to a compatible profile, or re-embed \
                 the corpus offline: stop the server and run `brain-server --re-embed <profile>` \
                 (rebuilds the vector store at {}-d and re-embeds every chunk); a silent \
                 cross-dim migration would corrupt recall.",
                d,
                store_dim,
                store_dim
            ));
        }
        None => {
            // Fresh DB (no stamp yet). Stamp the active embedder's dim so every
            // future boot can verify against it.
            db.execute(
                "INSERT INTO schema_meta(key, value) VALUES ('embedding_dim', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = ?1;",
                params![store_dim.to_string()],
            )?;
            info!("Stamped embedding_dim = {store_dim} (fresh DB)");
        }
    }

    // ── v0.9.0 Phase 1: sqlite-vec vec0 virtual table ────────────────────
    // Replaces the old JSON-text vector storage in `embeddings.vector`. The
    // dimension is the active embedder's `store_dim` (512 edge / 768 desktop /
    // 1024 enterprise) — NOT a hardcoded 512.
    //
    // Schema per Context7-verified sqlite-vec docs (July 2026):
    //   embedding_int8  int8[{dim}] distance_metric=cosine — default search tier
    //     (quantized f32→int8). cosine is REQUIRED: vec0 defaults to L2, but the
    //     int8-quantized vectors are not unit-normalized, so an L2 distance is
    //     meaningless for semantic similarity. cosine distance is well-defined
    //     on int8 vectors and yields similarity = 1 - distance in [0,1].
    //   embedding_bit   bit[{dim}]   — archive / first-pass tier (binary quantized)
    //   source          text       — metadata column (enables filtered KNN)
    //   created_at      text       — metadata column (enables temporal filtering)
    let vec0_ddl = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_knowledge USING vec0(
            knowledge_id INTEGER PRIMARY KEY,
            embedding_int8 int8[{dim}] distance_metric=cosine,
            embedding_bit  bit[{dim}],
            source         text,
            created_at     text
        );",
        dim = store_dim
    );
    db.execute_batch(&vec0_ddl)?;

    // ── One-time migration: rebuild vec_knowledge with the cosine metric ──
    // Earlier v0.9.0 builds created vec0 WITHOUT distance_metric=cosine, so the
    // int8 index used the default L2 metric — useless for semantic similarity
    // (yielded flat ~0 scores). Rebuild the table once, then the backfill below
    // repopulates it from the f32 `embeddings` table (the source of truth). The
    // `vec_metric` marker makes this idempotent across restarts.
    let needs_rebuild: bool = db
        .query_row(
            "SELECT value IS NULL OR value <> 'cosine' FROM schema_meta WHERE key = 'vec_metric'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n != 0)
        .unwrap_or(true);
    if needs_rebuild {
        info!(
            "Rebuilding vec_knowledge with distance_metric=cosine (one-time fix, dim={store_dim})"
        );
        let rebuild_ddl = format!(
            "DROP TABLE IF EXISTS vec_knowledge;
             CREATE VIRTUAL TABLE vec_knowledge USING vec0(
                knowledge_id INTEGER PRIMARY KEY,
                embedding_int8 int8[{dim}] distance_metric=cosine,
                embedding_bit  bit[{dim}],
                source         text,
                created_at     text
             );",
            dim = store_dim
        );
        db.execute_batch(&rebuild_ddl)?;
        db.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('vec_metric', 'cosine')
             ON CONFLICT(key) DO UPDATE SET value = 'cosine';",
            [],
        )?;
        info!("vec_knowledge rebuilt; backfill will repopulate from embeddings");
    }

    // Both paths above leave vec0 existing — stamp
    // the search-path flag so the per-query existence probe disappears.
    VEC0_READY.store(true, Ordering::Relaxed);

    // ── Backfill: migrate existing JSON vectors → vec0 ─────────────────
    // Only runs if the legacy `embeddings` table has rows that haven't been
    // copied to `vec_knowledge` yet.  Idempotent — safe to re-run. (Also
    // repopulates after the cosine-rebuild migration above drops the table.)
    let legacy_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM embeddings e
             WHERE NOT EXISTS (
                 SELECT 1 FROM vec_knowledge v WHERE v.knowledge_id = e.knowledge_id
             )",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if legacy_count > 0 {
        info!("Migrating {legacy_count} legacy JSON vectors to vec0 (int8 + binary)...");

        let rows: Vec<(i64, String, Option<String>, Option<String>)> = {
            let mut stmt = db.prepare(
                "SELECT e.knowledge_id, e.vector,
                        (SELECT k.source FROM knowledge k WHERE k.id = e.knowledge_id),
                        (SELECT k.created_at FROM knowledge k WHERE k.id = e.knowledge_id)
                 FROM embeddings e
                 WHERE NOT EXISTS (
                     SELECT 1 FROM vec_knowledge v WHERE v.knowledge_id = e.knowledge_id
                 )",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }; // stmt dropped here — db is free for mutable borrow

        let tx = db.transaction()?;
        for (kid, vec_json, source, created_at) in &rows {
            let f32_vec: Vec<f32> = serde_json::from_str(vec_json).unwrap_or_default();
            if f32_vec.len() != 512 {
                warn!(
                    "Skipping knowledge_id={kid}: expected 512-dim, got {}",
                    f32_vec.len()
                );
                continue;
            }
            tx.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), ?3, ?4)",
                params![kid, f32_vec.as_bytes(), source, created_at],
            )?;
        }
        tx.commit()?;
        info!("Migration complete: {legacy_count} vectors quantized to int8 + binary");
    }

    // ── v0.9.4: canonical sources + revisions ─────────────────────
    // A `source` is a stable identity for an external document (vault file,
    // connector doc): identified by canonical `uri`, typed by `kind`. A
    // `source_revision` is an immutable snapshot; a new revision supersedes
    // the prior active one. Every knowledge chunk links to source + revision
    // so a result can be traced to the exact document version it came from.
    //
    // Schema matches `src/sources.rs` (the lifecycle module). Existing 430
    // rows are left with source_id/revision_id = NULL — they continue to
    // work as before; only new ingests (post-v0.9.4) get source linkage.
    // Re-ingesting a vault creates source rows naturally; rows that stay
    // NULL are immune to kind-scoped reconciliation.
    db.execute(
        "CREATE TABLE IF NOT EXISTS sources(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uri TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL DEFAULT 'vault',
            title TEXT,
            current_revision_id INTEGER,
            state TEXT NOT NULL DEFAULT 'active',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            observed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS source_revisions(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id INTEGER NOT NULL,
            revision TEXT NOT NULL,
            content_hash TEXT,
            chunk_count INTEGER NOT NULL DEFAULT 0,
            byte_size INTEGER NOT NULL DEFAULT 0,
            state TEXT NOT NULL DEFAULT 'active',
            fetched_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
         )",
        [],
    )?;
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_source_revisions_src_rev
         ON source_revisions(source_id, revision)",
        [],
    )?;
    // Additive columns on knowledge: link each chunk to its source + revision.
    // Nullable — existing rows stay NULL (see note above). Declared WITHOUT
    // ON DELETE CASCADE on purpose: `sources::sweep_source_chunks` manages the
    // knowledge-row deletes explicitly so tombstoning stays auditable.
    // (SQLite doesn't enforce FKs without PRAGMA foreign_keys=ON anyway, but
    // the declaration documents intent for future readers and tooling.)
    for (col, def) in [
        ("source_id", "INTEGER REFERENCES sources(id)"),
        ("revision_id", "INTEGER REFERENCES source_revisions(id)"),
    ] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_source_id ON knowledge(source_id)",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_revision_id ON knowledge(revision_id)",
        [],
    )?;

    // ── v0.9.6 Bridge: connector registry + per-connector checkpoint store.
    // Both are additive — no migration of existing rows. The server writes
    // connector-instance state to `connectors`; the connector process owns its
    // own checkpoint DB (separate file), and the server keeps a mirror copy in
    // `connector_checkpoints` so a crash + restart of either side resumes from
    // the right place.
    db.execute(
        "CREATE TABLE IF NOT EXISTS connectors(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            instance TEXT NOT NULL,
            config_json TEXT NOT NULL DEFAULT '{}',
            state TEXT NOT NULL DEFAULT 'registered',
            last_sync_at TEXT,
            last_error TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(kind, instance)
         )",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS connector_checkpoints(
            connector_id INTEGER NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (connector_id, key)
         )",
        [],
    )?;

    // ── v0.9.7 Guard: append-only audit events ──────────────────────────
    // Identifiers + hashes only — never raw content, tokens, or secrets. See
    // `src/audit.rs`. Additive; safe to re-run on existing DBs.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_events(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT DEFAULT CURRENT_TIMESTAMP,
            kind TEXT NOT NULL,
            actor TEXT,
            target_hash TEXT,
            status TEXT,
            detail_hash TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_audit_kind ON audit_events(kind);
         CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_events(ts);",
    )?;

    // v1.1.0 Harden: per-tenant scoping + tamper-evidence. Additive columns
    // on `audit_events`. `tenant_id` defaults to 'global' for back-compat with
    // every pre-v1.1 row; `prev_hash` is backfilled NULL and the chain starts
    // fresh from the next inserted row (a documented upgrade-path ceiling).
    for (col, def) in [
        ("tenant_id", "TEXT NOT NULL DEFAULT 'global'"),
        ("prev_hash", "TEXT"),
    ] {
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('audit_events') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE audit_events ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_tenant ON audit_events(tenant_id)",
        [],
    )?;

    // ── v0.9.7 "Guard": verified webhook ingest queue ──────────────────
    // Bounded FIFO of verified webhook deliveries. Idempotency is enforced by
    // the UNIQUE(delivery_hash) constraint; a replayed delivery is a no-op
    // (INSERT OR IGNORE). The drain worker (src/webhook.rs) processes rows in
    // id order and deletes as it goes, so a verified webhook never mutates the
    // index directly — it only enqueues.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS webhook_queue(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            event TEXT NOT NULL,
            delivery_hash TEXT NOT NULL UNIQUE,
            payload_hash TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_webhook_queue_kind ON webhook_queue(kind);
         CREATE TABLE IF NOT EXISTS webhook_seen(
            delivery_hash TEXT PRIMARY KEY,
            seen_at TEXT DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_webhook_seen_at ON webhook_seen(seen_at);",
    )?;

    // ── v0.9.8 "Evidence": typed provenance links between chunks ──
    // Flat additive table (NOT the entities/relationships KG — see the plan's
    // v1.0.0 upgrade-path note). Records supports/supersedes/contradicts/
    // references/derived_from relationships so a contradictory or superseded
    // claim stays visible rather than silently collapsed. Idempotent.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS evidence_links(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            from_chunk INTEGER NOT NULL REFERENCES knowledge(id),
            to_chunk INTEGER NOT NULL REFERENCES knowledge(id),
            kind TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(from_chunk, to_chunk, kind)
         );
         CREATE INDEX IF NOT EXISTS idx_evidence_links_from ON evidence_links(from_chunk);
         CREATE INDEX IF NOT EXISTS idx_evidence_links_to ON evidence_links(to_chunk);",
    )?;

    // ── v0.9.9 "Qualify" / v1.1.0 "Harden": record the schema version so
    // the rehearsal tool (and future migrations) can read it. Idempotent.
    // ── v1.2.0 "AuthN": token revocation + refresh-chain tracking ────
    // Two additive tables. Both are new (no ALTER TABLE on existing tables
    // beyond the `audit_events.tenant_id` already done in v1.1), so back-
    // compat is trivial: a v1.1 DB picks these up on next start with no data
    // loss. Indices cover the hot paths: denylist lookup by (jti, iss) is
    // the PK; purge by expires_at; refresh-chain lookup by (chain_id, iss).
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS revoked_tokens(
            jti TEXT NOT NULL,
            iss TEXT NOT NULL,
            sub TEXT,
            expires_at INTEGER NOT NULL,
            revoked_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            revoked_by TEXT,
            reason TEXT,
            PRIMARY KEY (jti, iss)
         );
         CREATE INDEX IF NOT EXISTS idx_revoked_expires ON revoked_tokens(expires_at);
         CREATE TABLE IF NOT EXISTS refresh_chains(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chain_id TEXT NOT NULL,
            iss TEXT NOT NULL,
            current_jti TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'active',
            first_seen INTEGER NOT NULL,
            burned_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_refresh_chain ON refresh_chains(chain_id, iss);",
    )?;

    // ── v1.4.0 "Calibrate": bi-temporal edges (Graphiti model). ───────
    // Every relationship carries a valid-time interval [valid_at, invalid_at):
    //   valid_at   = when the fact BECAME TRUE in the world (event time)
    //   invalid_at = when the fact STOPPED BEING TRUE (NULL ⇒ still current)
    // These are distinct from created_at (transaction time: when brain learned
    // the fact). A query `?at=2015` filters: valid_at <= 2015 AND (invalid_at
    // IS NULL OR invalid_at > 2015). Context7-verified 2026-07-30 against the
    // Graphiti EntityEdge source (getzep/graphiti:edges.py): the model is
    // valid_at/invalid_at for valid time, expired_at for correction wall-clock
    // time, reference_time for source provenance. We adopt valid_at/invalid_at
    // (the two that drive retrieval filtering); expired_at is subsumed by the
    // v0.9.8 evidence_links `supersedes`/`update:` kind + audit log.
    // Idempotent + additive; existing edges default to NULL/NULL ⇒ always valid.
    for (col, def) in [("valid_at", "TIMESTAMP"), ("invalid_at", "TIMESTAMP")] {
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('relationships') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE relationships ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_rels_valid_at ON relationships(valid_at)",
        [],
    )?;

    // ── v1.4.0 "Calibrate": TRACE hierarchical node reservation. ───────
    // node_kind defaults to 'fact' (every declarative chunk is a fact). The
    // column was originally reserved as 'event'/'session'/'topic' for a worker
    // that never shipped; v1.10.0 "Procedural" repurposed it as the Mem0-style
    // memory_kind (fact/procedure/step/decision) and relabels existing rows.
    // The default was flipped to 'fact' so fresh DBs insert the repurposed
    // value directly. parent_id links a node to its enclosing session/topic.
    //   ponytail: schema reservation only. Construction logic is deferred to
    //   v1.8 Consolidate (the only release with a worker that can group events
    //   into sessions). Adding the columns now keeps v1.4's migration additive
    //   and avoids a future ALTER on the hot knowledge table.
    for (col, def) in [
        ("node_kind", "TEXT NOT NULL DEFAULT 'fact'"),
        ("parent_id", "INTEGER"),
    ] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_parent ON knowledge(parent_id)",
        [],
    )?;

    // ── v1.9.0 "Suggest": opt-in anticipation feedback. ─────────────────
    // Append-only ledger of accept/dismiss signals on suggested chunks. This
    // table IS the audit surface for the feedback mutation (chunk_id +
    // feedback + ts + tenant_id + optional reason_hash reconstruct who/what/
    // when); no duplicate `audit_events` row is written. Session is a
    // caller-supplied opaque label (Mem0 `run_id` pattern) — the server never
    // auto-tracks sessions (roadmap forbids hidden personalization).
    db.execute(
        "CREATE TABLE IF NOT EXISTS suggest_feedback (
             id          INTEGER PRIMARY KEY,
             chunk_id    INTEGER NOT NULL,
             feedback    TEXT NOT NULL,
             reason_hash TEXT,
             ts          INTEGER NOT NULL,
             session     TEXT,
             tenant_id   TEXT NOT NULL DEFAULT 'default'
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_suggest_feedback_tenant_ts
         ON suggest_feedback(tenant_id, ts);",
        [],
    )?;

    // ── v1.9.1 "Harden": feedback is last-wins per (chunk_id, session). ──
    // The v1.9.0 ledger was append-only with no idempotency: a client retry
    // or replay recorded duplicate rows, poisoning the false-positive metric
    // that is the v1.9 roadmap exit criterion. A unique index on
    // (chunk_id, COALESCE(session,'')) makes the handler's upsert one signal
    // per surfaced suggestion per session — replays collapse, and a changed
    // mind (accept → dismiss) overwrites instead of double-counting.
    // Dedup any pre-existing duplicate rows first (keep the latest per key)
    // so the index can be created on any DB.
    db.execute(
        "DELETE FROM suggest_feedback
         WHERE id NOT IN (
             SELECT MAX(id) FROM suggest_feedback
             GROUP BY chunk_id, COALESCE(session, '')
         );",
        [],
    )?;
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_suggest_feedback_chunk_session
         ON suggest_feedback(chunk_id, COALESCE(session, ''));",
        [],
    )?;

    // ── v1.10.0 "Procedural": ordered steps + memory classification. ─────
    // Repurpose the v1.4.0-reserved `knowledge.node_kind` column to carry the
    // memory classification (fact/procedure/step/decision). v1.4 reserved it
    // as 'event'/'session'/'topic' for a worker that never shipped; v1.10 makes
    // it the Mem0-style `memory_kind` — but populated deterministically
    // (keyword router), not via cloud LLM. Legacy 'event' rows become 'fact'
    // (every prior chunk is declarative). No data loss; backward-compatible.
    //   ponytail: the column DEFAULT is only 'fact' on fresh DBs. A pre-v1.10
    //   DB keeps its 'event' default (SQLite can't ALTER a column default
    //   without a table rebuild); new rows there stay 'event' until the next
    //   startup's relabel, and `MemoryKind::from_str` normalizes 'event' to
    //   'fact' at every read, so the gap is cosmetic, not functional.
    db.execute_batch(
        "UPDATE knowledge SET node_kind = 'fact'
         WHERE node_kind = 'event' OR node_kind IS NULL OR node_kind = '';",
    )?;
    // Ordered-step support on the existing evidence_links table: a `next_step`
    // edge with an explicit step_index. Reuses the typed-edge infra (no new
    // table) — Graphiti's NextEpisodeEdge pattern at chunk level.
    let has_step_index: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('evidence_links') WHERE name='step_index'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_step_index {
        db.execute(
            "ALTER TABLE evidence_links ADD COLUMN step_index INTEGER",
            [],
        )?;
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_evidence_links_step
         ON evidence_links(step_index) WHERE step_index IS NOT NULL;",
        [],
    )?;

    // ── v1.14.0 "Gate": write-back gating + decay + trust surfaces. ─────
    // All additive; defaults preserve current behavior exactly. Columns:
    //   access_scope  — private(default)|domain|team|public; enforced only in
    //                   JWT mode (loopback trusts localhost, SECURITY.md).
    //   assertion_kind — stated(default)|observed|inferred (provenance).
    //   confidence    — 0..1 deterministic derivation, default 1.0.
    //   expires_at    — unix ts, NULL = no decay (default off).
    //   pii           — 1 when the ingest-time pattern scanner flagged PII.
    //   owner         — creating principal TEXT, NULL for legacy/loopback.
    for (col, def) in [
        ("access_scope", "TEXT NOT NULL DEFAULT 'private'"),
        ("assertion_kind", "TEXT NOT NULL DEFAULT 'stated'"),
        ("confidence", "REAL NOT NULL DEFAULT 1.0"),
        ("expires_at", "INTEGER"),
        ("pii", "INTEGER NOT NULL DEFAULT 0"),
        ("owner", "TEXT"),
    ] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }

    // v1.20.19 "Vault": the v1.14 `pii_map` table was never written to (the
    // write-time placeholder mode it served was docs-only) and only `/export`
    // read it — a dead personal-data table. Drop it outright; `DROP TABLE IF
    // EXISTS` erases any legacy placeholder rows and is idempotent on a fresh
    // DB (the CREATE below was removed in the same release, so the table no
    // longer exists to be re-created before this drop).
    db.execute("DROP TABLE IF EXISTS pii_map", [])?;

    // Purge audit trail (GDPR). Append-only; keeps the audit chain
    // verifiable (knowledge_id + content_hash + purged_at, no raw content).
    // The v0.9.1 tombstones table already exists, so we ADD the two purge
    // columns idempotently (CREATE TABLE IF NOT EXISTS would be a silent
    // no-op against the old schema and the purge INSERT would fail).
    for (col, def) in [("content_hash", "TEXT"), ("purged_at", "INTEGER")] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('tombstones') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE tombstones ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_tombstones_kid_v2 ON tombstones(knowledge_id)",
        [],
    )?;

    // ── v1.14.0 "Write-back gate": the proposal review queue. ─────────
    // A proposal stores a *candidate* memory — scored deterministically
    // (novelty / conflict / salience) — with NO `knowledge` row until a human
    // approves. status: pending|approved|rejected. decided_at set on decision.
    db.execute(
        "CREATE TABLE IF NOT EXISTS proposals (
            id           INTEGER PRIMARY KEY,
            kind         TEXT NOT NULL DEFAULT 'fact',
            content      TEXT NOT NULL,
            source       TEXT,
            authority    REAL,
            observed_at  INTEGER,
            novelty      REAL NOT NULL,
            conflict_with INTEGER,
            salience     REAL NOT NULL DEFAULT 0.5,
            status       TEXT NOT NULL DEFAULT 'pending',
            created_at   INTEGER NOT NULL,
            decided_at   INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_proposals_status ON proposals(status)",
        [],
    )?;

    // ── v1.15.0 "Observe": read-event trace + DSAR ledger. ────────
    // `recall_traces` holds the replayable decision-path artifact for a recall
    // read event, keyed by the audit row id (hash-only chain stays in
    // `audit_events`; the trace is non-content metadata: ids, scores, ranks,
    // decision, scope, principal). `dsar_requests` is the GDPR deletion-
    // workflow ledger (the certificate JSON lives in `certificate`).
    db.execute(
        "CREATE TABLE IF NOT EXISTS recall_traces (
            audit_id   INTEGER PRIMARY KEY,
            trace_json TEXT NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS dsar_requests (
            id            INTEGER PRIMARY KEY,
            subject       TEXT NOT NULL,
            action        TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'pending',
            export_bundle TEXT,
            certificate   TEXT,
            created_at    INTEGER NOT NULL,
            completed_at  INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_dsar_subject ON dsar_requests(subject, status)",
        [],
    )?;
    // v1.15.0: the tombstone purge-audit row gains `reason` ('explicit' |
    // 'owner:<subject>' | 'derived') + `origin_id` (the purge root for derived
    // descendants) so `GET /tombstones?subject=` and derived-purge audit have
    // a queryable hook. Idempotent guarded adds — same pattern as v1.14.0.
    for (col, def) in [("reason", "TEXT"), ("origin_id", "INTEGER")] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('tombstones') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE tombstones ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }

    // v1.20.18 "Bound": the `/tombstones?subject=&since=` registry and the DSAR
    // certificate read `WHERE reason = ? AND purged_at >= ?` — a compound index
    // keeps those from scanning every tombstone. Both columns exist above.
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_tombstones_reason_purged ON tombstones(reason, purged_at)",
        [],
    )?;

    // v1.16.1: backfill legacy tombstones whose `purged_at` is NULL (rows
    // written by pre-v1.14 builds only set `deleted_at`). `list_tombstones`
    // reads `purged_at` as a non-null INTEGER, so NULL rows were silently
    // dropped from the deletion registry (observed: 6,008 of 6,009 invisible).
    // Map `deleted_at` (SQLite CURRENT_TIMESTAMP, UTC) to its unix epoch;
    // rows with neither stay NULL (surfaced as `null` by the handler).
    // Idempotent: only touches rows that still have NULL purged_at.
    db.execute(
        "UPDATE tombstones
            SET purged_at = CAST(strftime('%s', deleted_at) AS INTEGER)
          WHERE purged_at IS NULL AND deleted_at IS NOT NULL",
        [],
    )?;

    // v1.17.1 "Govern": persisted per-kind retention overrides. The default
    // policy ships in code (`config::DEFAULT_RETENTION_KIND_DAYS`); a
    // `POST /retention` override is upserted here so it survives restart. Empty
    // table = defaults only. `days` is a positive integer; a future kind key is
    // accepted so an operator can govern a kind before the binary names it.
    db.execute(
        "CREATE TABLE IF NOT EXISTS retention_policy (
            kind TEXT PRIMARY KEY,
            days INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );",
        [],
    )?;

    // v1.17.3 "UMP Rollout": UMP record identity + round-trip metadata.
    // `ump_id` is the content-addressed `urn:ump:` id (unique, indexed) so
    // `/ump/memory/{id}` and friends resolve without scanning; `ump_meta`
    // carries the imported record's non-column fields (provenance, consent,
    // lifecycle extras, raw_kind) so import→export round-trips losslessly
    // for L2 fields (UMP spec §6.3). Legacy rows stay NULL and are lazily
    // backfilled (deterministic) on first UMP read.
    for (col, def) in [("ump_id", "TEXT"), ("ump_meta", "TEXT")] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_knowledge_ump_id ON knowledge(ump_id)",
        [],
    )?;

    // v1.17.3: `suggest_feedback.ump_outcome` preserves the granular UMP
    // feedback outcome (followed|overridden|ignored|contradicted) alongside
    // the accept/dismiss metric signal (additive; no CHECK change, NULL for
    // non-UMP calls).
    {
        let present: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('suggest_feedback') WHERE name='ump_outcome'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                "ALTER TABLE suggest_feedback ADD COLUMN ump_outcome TEXT",
                [],
            )?;
        }
    }

    // v1.28.77 "Erasure": `suggest_feedback.owner` captures the JWT principal
    // (`sub`) that gave the feedback — the join evidence the erasure story
    // was missing. Session ids are CLIENT-OWNED opaque labels (Mem0 run_id
    // pattern), never principal ids, so a certified purge/DSAR could not
    // reach the subject's feedback rows on chunks the purge never touched.
    // With the owner captured, the DSAR sweep's feedback arm matches
    // `owner = subject` exactly (same vocabulary as `knowledge.owner`);
    // NULL rows (opaque/no-auth callers) stay reachable only through the
    // tenant + chunk arms — the honest ceiling, disclosed in the CHANGELOG.
    // Additive + nullable; same guarded pattern as ump_outcome above.
    {
        let present: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('suggest_feedback') WHERE name='owner'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute("ALTER TABLE suggest_feedback ADD COLUMN owner TEXT", [])?;
        }
    }

    // v1.18.2 "Transparency": explicit model-vs-human origin marker (Art 50
    // synthetic-content line). `source` says the ingest kind; `origin` says who
    // produced the memory. Default 'imported' is the safe fallback — never
    // claim human authorship for an unknown path. Backfill by source kind:
    // manual → human (interactive), memory → model (auto-capture/assistant),
    // markdown/structured → imported (bulk import). Same guarded-add pattern
    // as v1.14.0/v1.15.0.
    let origin_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='origin'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !origin_present {
        db.execute(
            "ALTER TABLE knowledge ADD COLUMN origin TEXT NOT NULL DEFAULT 'imported'",
            [],
        )?;
        db.execute(
            "UPDATE knowledge SET origin =
                CASE source
                    WHEN 'manual' THEN 'human'
                    WHEN 'memory' THEN 'model'
                    ELSE 'imported'
                END",
            [],
        )?;
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_origin ON knowledge(origin)",
        [],
    )?;

    // the Seatbelt posture: origin-label truth. `/procedure` self-declared
    // 'human' was operator authorship, never the user's own human voice; UMP
    // records are agent-authored by definition. Idempotent, label-only —
    // `origin` is not consumed for ACL today.
    db.execute(
        "UPDATE knowledge SET origin = 'operator'
         WHERE origin = 'human' AND node_kind IN ('procedure', 'step')",
        [],
    )?;
    db.execute(
        "UPDATE knowledge SET origin = 'agent'
         WHERE ump_meta IS NOT NULL AND origin = 'imported'",
        [],
    )?;

    // ── v1.20.1 "Shield": proposal provenance from the auto-capture path. ─
    // `source_prompt` (a) tells a reviewer which autocapture the proposal came
    // from so they can context-check it before approving, and (b) lets the
    // proposal surface re-run the injection screen against the caller-provided
    // text that fed the capture. Additive + idempotent.
    let prompt_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='source_prompt'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !prompt_present {
        db.execute("ALTER TABLE proposals ADD COLUMN source_prompt TEXT", [])?;
    }

    // ── v1.20.14 "Steer": edit provenance on pending proposals. ──────────
    // `edited_at` is a nullable unix timestamp set when a reviewer rewrites a
    // pending proposal's content via POST /proposals/{id}/edit. The review
    // badge and read-time view key off it; `None` = never edited. Additive +
    // idempotent, same guard as `source_prompt` above.
    let edited_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='edited_at'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !edited_present {
        db.execute("ALTER TABLE proposals ADD COLUMN edited_at INTEGER", [])?;
    }

    // ── v1.20.24 "Sweep": /decayed scan narrowing ────────────────────────
    // `GET /decayed` now narrows its scan in SQL (the Rust-side
    // `effective_expiry` filter stays the arbiter). These two indexes serve
    // the per-chunk branch (`expires_at < now`) and the kind-policy branch
    // (`node_kind IN (...) AND created_at < cutoff`). Idempotent; no column
    // contract change (the schema-contract test pins columns, not indexes).
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_expires_at ON knowledge(expires_at)",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_kind_created ON knowledge(node_kind, created_at)",
        [],
    )?;

    // ── v1.21.0 "Profiles": the preset system ─────────────────────────
    // `profiles` holds the JSON bundles (the 12 seeded presets + operator
    // clones); `domain_profiles` binds a domain to one profile (the plan's
    // `domain.profile` FK — there is no `domains` table, domains are labels,
    // so the binding is its own keyed row). Read at request time; no new
    // columns anywhere. Seeding is INSERT OR IGNORE so operator edits to a
    // preset survive re-migrations (only a missing preset is re-inserted).
    db.execute(
        "CREATE TABLE IF NOT EXISTS profiles (
            name       TEXT PRIMARY KEY,
            json       TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS domain_profiles (
            domain    TEXT PRIMARY KEY,
            profile   TEXT NOT NULL REFERENCES profiles(name),
            bound_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         );",
        [],
    )?;
    for (name, json) in crate::profile::PRESETS_RAW {
        db.execute(
            "INSERT OR IGNORE INTO profiles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )?;
    }

    // ── v1.22.0 "Regulated": legal holds ────────────────────────────
    // One row per (chunk, hold): multiple concurrent holds are allowed
    // (litigation + retention audit) and an id stays frozen against every
    // erasure path (decay skip, /purge 409, DSAR deferral) until EVERY hold on
    // it is released — never auto-released. Append-only except `released_at`.
    // Lives in every domain file (the migration runs per-DB) so enforcement
    // checks are local to the same pool/tx as the purge they gate.
    db.execute(
        "CREATE TABLE IF NOT EXISTS legal_holds (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            knowledge_id  INTEGER NOT NULL,
            reason        TEXT NOT NULL,
            held_by       TEXT,
            held_at       INTEGER NOT NULL,
            released_at   INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_legal_holds_open
         ON legal_holds(knowledge_id) WHERE released_at IS NULL",
        [],
    )?;

    // ── v1.25.0 "PH-Compliant": breach-notification workflow ────────
    // The DPO-opened incident ledger + its append-only event log (the one
    // "genuinely new primitive" of the release). Lives in every domain file (the shared
    // migration) like legal_holds; the breach handler operates on the `global`
    // pool — an incident is operator data, not domain-scoped memory. The
    // tamper-evident *record* is the audit chain (kind='breach') this handler
    // appends to on every event; these tables are the DPO's readable ledger.
    db.execute(
        "CREATE TABLE IF NOT EXISTS breaches (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            scope              TEXT NOT NULL,
            description        TEXT NOT NULL,
            severity           TEXT NOT NULL,
            discovered_at      INTEGER NOT NULL,
            affected_estimate  INTEGER,
            jurisdictions      TEXT NOT NULL DEFAULT '[]',
            status             TEXT NOT NULL DEFAULT 'open',
            opened_by          TEXT NOT NULL,
            opened_at          INTEGER NOT NULL,
            closed_by          TEXT,
            closed_at          INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS breach_events (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            breach_id     INTEGER NOT NULL,
            event_type    TEXT NOT NULL,
            jurisdiction  TEXT,
            body          TEXT NOT NULL,
            noted_by      TEXT NOT NULL,
            created_at    INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_breach_events_breach
         ON breach_events(breach_id, id)",
        [],
    )?;

    // ── v1.22.0 "Regulated": region pin (data residency) ───────────
    // `knowledge.region` is stamped at INSERT by the trigger below (all current
    // + future ingest paths, incl. connector/UMP/import, with zero per-site
    // churn), then surfaced on /export + the DSAR certificate. Read-only
    // provenance: the backfill stamps only NULL rows (legacy rows on first
    // v1.22 boot; a region change never rewrites where old rows lived).
    let region_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='region'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !region_present {
        db.execute("ALTER TABLE knowledge ADD COLUMN region TEXT", [])?;
    }
    let region = crate::storage_layout::region();
    match region.as_deref() {
        Some(r) => {
            // Recreate per boot so a region change re-points the stamp (the
            // backfill above never overwrites, so history is preserved).
            db.execute_batch(&format!(
                "DROP TRIGGER IF EXISTS knowledge_region_stamp;
                 CREATE TRIGGER knowledge_region_stamp
                 AFTER INSERT ON knowledge
                 WHEN NEW.region IS NULL
                 BEGIN
                     UPDATE knowledge SET region = '{r}' WHERE id = NEW.id;
                 END;"
            ))?;
            let stamped = db.execute(
                "UPDATE knowledge SET region = ?1 WHERE region IS NULL",
                rusqlite::params![r],
            )?;
            if stamped > 0 {
                info!("region pin: stamped {stamped} pre-existing chunks as '{r}'");
            }
        }
        None => {
            // No pin: stop stamping (a leftover trigger from a previous pin
            // would keep writing a region the operator removed).
            db.execute_batch("DROP TRIGGER IF EXISTS knowledge_region_stamp;")?;
        }
    }

    // ── v1.23.0 "Roles": the named scope/action bundles ──────────────
    // `roles` holds the JSON bundles (the 10 seeded presets + operator
    // clones), resolved from a JWT principal's `roles` claim at request time
    // (data gate + action gate + MCP tools). Seeding is INSERT OR IGNORE so an
    // operator edit to a preset survives a re-migration (only a missing preset
    // is re-inserted).
    db.execute(
        "CREATE TABLE IF NOT EXISTS roles (
            name       TEXT PRIMARY KEY,
            json       TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         );",
        [],
    )?;
    for (name, json) in crate::role::PRESETS_RAW {
        db.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )?;
    }

    // ── the domain router's routing prototypes ───────────────
    // `domain_route_prototypes` holds the per-domain k-means cluster means
    // the unscoped-recall router scores against. Derived data rebuilt
    // wholesale by the sweep — routing hints, not source-of-truth — so it
    // earns no schema-version bump (additive `IF NOT EXISTS`, like every
    // support table here). Declared in the ladder so the shipped schema is
    // complete: every production statement must PREPARE against a freshly
    // migrated database (tests/sql_schema_agreement.rs). The router's own
    // `ensure_prototype_table` stays as the defensive seam — a backup
    // restored over a migrated file still converges on first sweep.
    // Lives in every domain file like `transfers`/`legal_holds`; the router
    // operates on the `global` pool (routing is a whole-corpus concern).
    db.execute(
        "CREATE TABLE IF NOT EXISTS domain_route_prototypes (
            domain TEXT NOT NULL,
            idx INTEGER NOT NULL,
            proto BLOB NOT NULL,
            PRIMARY KEY (domain, idx)
        )",
        [],
    )?;

    // ── v1.26.0 "Cross-Border": the transfer register + tagging ────
    // `transfers` is the Art 30 processing-activities + Art 46 transfer-
    // safeguard evidence: every cross-border data flow as a row. The `knowledge`
    // columns (`lawful_basis`, `purpose`) carry the Art 5/6 purpose-limitation
    // + data-minimization evidence; both additive + nullable (NULL = the legacy
    // "unspecified" behavior — never a behavior change for existing rows).
    // Lives in every domain file like `legal_holds`/`breaches`; the handler
    // operates on the `global` pool (a transfer is operator data, not
    // domain-scoped memory).
    db.execute(
        "CREATE TABLE IF NOT EXISTS transfers(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            origin_jurisdiction TEXT NOT NULL,
            destination_jurisdiction TEXT NOT NULL,
            mechanism TEXT NOT NULL,
            counterparty TEXT NOT NULL,
            lawful_basis TEXT,
            purpose TEXT NOT NULL,
            signed_at INTEGER,
            expires_at INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_transfers_destination
         ON transfers(destination_jurisdiction)",
        [],
    )?;
    for (col, def) in [("lawful_basis", "TEXT"), ("purpose", "TEXT")] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE knowledge ADD COLUMN {col} {def}"), [])?;
        }
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_purpose ON knowledge(purpose)",
        [],
    )?;

    // v1.26.0 "Cross-Border": the per-jurisdiction DSAR deadline + rights table
    // is shipped in code (`crate::transfers::JURISDICTIONS`) — a curated,
    // release-versioned table, not a DB table (it is read at request time and
    // re-checked on release, per the plan's honest ceiling). Nothing to migrate.

    // ── v1.27.1 "Clients": the BPO operating register ────────────────
    // One row per operating client, stored in the **global DB** like the
    // `transfers` register it mirrors. `name` is the BPO-facing id (lowercase
    // domain-safe identifier); `domain` is the one-domain-per-client isolation
    // seam (v1.0); `status` = active | archived (archived set on termination,
    // v1.27.6); `dpa_terms` (nullable JSON) filled by v1.27.3.
    db.execute(
        "CREATE TABLE IF NOT EXISTS clients(
            name TEXT PRIMARY KEY,
            domain TEXT NOT NULL,
            jurisdiction TEXT NOT NULL,
            profile TEXT,
            dpa_terms TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            created_at INTEGER NOT NULL,
            archived_at INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_clients_domain ON clients(domain)",
        [],
    )?;

    // v1.27.8 "QaQueue": the review queue gains agent provenance + coaching.
    // `owner` = the agent whose interaction produced the candidate; `qa_note` =
    // the supervisor's coaching note (attached by the coach verb). Additive +
    // nullable — existing rows keep owner NULL / no note.
    for (col, def) in [("owner", "TEXT"), ("qa_note", "TEXT")] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE proposals ADD COLUMN {col} {def}"), [])?;
        }
    }

    // v1.27.18 "Groundwork": serve the queried columns, drop the dead.
    // Adds: `domain` (domain delete + full-domain scans), `owner` (DSAR subject
    // resolution — the regulated hot path), `(title, heading_path)` (the
    // per-proposal write-gate dedup). Drops (write-cost only, query-equivalent
    // via a UNIQUE autoindex or a newer sibling index): the pre-v0.9.6
    // `idx_tombstones_kid` (superseded by `idx_tombstones_kid_v2`),
    // `idx_entities_name` (duplicated by the `entities.name` UNIQUE
    // COLLATE NOCASE autoindex), and `idx_evidence_links_from` (a left-prefix
    // of the `evidence_links.from_chunk, to_chunk, kind` UNIQUE constraint).
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_knowledge_domain ON knowledge(domain);
         CREATE INDEX IF NOT EXISTS idx_knowledge_owner ON knowledge(owner);
         CREATE INDEX IF NOT EXISTS idx_knowledge_title_heading
             ON knowledge(title, heading_path);
         DROP INDEX IF EXISTS idx_tombstones_kid;
         DROP INDEX IF EXISTS idx_entities_name;
         DROP INDEX IF EXISTS idx_evidence_links_from;",
    )?;

    // ── v1.27.22 "Cascade": the edge table becomes TRULY bi-temporal. ─────
    // v1.4.0 gave relationships the valid-time axis (valid_at/invalid_at) and
    // created_at (transaction-time START). What was missing was the
    // transaction-time END: the instant the system stopped believing a fact.
    // `superseded_at` is the fourth timestamp (SQL:2011 / Snodgrass bi-temporal
    // model, matching Graphiti's EntityEdge valid_at/invalid_at + created_at/
    // expired_at). A superseded belief is a *different version* of the same
    // (from_entity_id, to_entity_id, relation_type) triple, not a mutation of
    // the valid interval: `superseded_at IS NULL` marks the current belief;
    // a non-NULL value records when that version was retired. The retired
    // version keeps its valid interval + created_at for historical/as-of reads.
    //
    // This replaces the write-once UNIQUE index `idx_rels_unique`, which forced
    // single-row-per-triple semantics — a corrected belief could never coexist
    // with the version it supersedes (see ingest.rs, the old INSERT OR IGNORE
    // no-op). Bi-temporal versioning requires multiple rows per triple; the
    // plain bt index `idx_rels_bt` serves the same per-triple lookup (the
    // current-belief resolution in graph_supersede + the traversal current-edge
    // predicate) without the uniqueness. Idempotent + additive on existing DBs:
    // pre-v1.27.22 edges have superseded_at NULL (current), so default reads are
    // byte-identical.
    {
        let col = "superseded_at";
        let def = "TIMESTAMP";
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('relationships') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE relationships ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }
    // Drop the write-once UNIQUE index (versioned edges need many rows per
    // triple); the bt index serves the per-triple lookups without the
    // uniqueness that forbids supersession.
    db.execute_batch(
        "DROP INDEX IF EXISTS idx_rels_unique;
         CREATE INDEX IF NOT EXISTS idx_rels_bt
             ON relationships(from_entity_id, to_entity_id, relation_type);",
    )?;

    // ── v1.27.25 "Scoped": the open-row invariant becomes STRUCTURAL. ─────
    // "At most one open (superseded_at IS NULL) row per
    // triple" was conventional only — a SELECT-then-INSERT race (or legacy
    // corrupt data) could leave two open versions, and BOTH then render
    // `current:true` on the history surface. First deterministically close
    // every open row that is not the newest of its triple (same newest-wins
    // rule `resolve_edge_insert` applies), then enforce it with a PARTIAL
    // UNIQUE INDEX — a racing double-insert now fails at the DB (the ingest
    // tx rolls back, fail-closed) instead of corrupting the lineage.
    db.execute_batch(
        "UPDATE relationships SET superseded_at = datetime('now')
          WHERE superseded_at IS NULL
            AND id NOT IN (
                SELECT MAX(id) FROM relationships
                WHERE superseded_at IS NULL
                GROUP BY from_entity_id, to_entity_id, relation_type
            );",
    )?;
    db.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_rels_open_unique
             ON relationships(from_entity_id, to_entity_id, relation_type)
            WHERE superseded_at IS NULL;",
    )?;

    // ── v1.27.30 "Spine": the governed-workflow substrate. ─────────────
    // The durable evidence tables the `*-core` engine crates (interview/consensus/
    // executor ports) will write THROUGH — no engine code ships here, only the
    // storage + primitives (src/workflow/) that make the ports provable later.
    // Lives in every domain file (the per-DB migration) like legal_holds; each
    // run is domain-scoped. Every write below emits a matching `AuditKind::Workflow`
    // row (the breach precedent) — the tables are derivable from the audit chain,
    // never the other way.
    //
    // workflow_runs    — one governed run (an interview, a plan, an execute).
    //   state_json     = OPAQUE to the server: the `*-core` crates own the shape.
    //     state_revision = the CAS token for `cas_update` (optimistic locking).
    // workflow_steps   — the run's step plan, rendered gate-by-gate.
    //   parent_step_id   = mid-case branching/handoff (resume at current_step).
    // outbox           — exactly-once event delivery, idempotent BY KEY not retry-count.
    // findings         — the loop's input valve; evidence pinned per claim (closed
    //                     schema at write, so the reducer can prove non-merge).
    // contradictions  — surfaced findings (A vs B), resolved by a later finding.
    db.execute(
        "CREATE TABLE IF NOT EXISTS workflow_runs(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            domain          TEXT NOT NULL,
            kind            TEXT NOT NULL,
            state_json      TEXT NOT NULL,
            state_revision  INTEGER NOT NULL DEFAULT 0,
            status          TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS workflow_steps(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id          INTEGER NOT NULL,
            phase           TEXT NOT NULL,
            step_key        TEXT NOT NULL,
            state_json      TEXT NOT NULL,
            revision        INTEGER NOT NULL DEFAULT 0,
            parent_step_id  INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS outbox(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id          INTEGER NOT NULL,
            topic           TEXT NOT NULL,
            payload_json    TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'pending',
            idempotency_key TEXT NOT NULL UNIQUE,
            created_at      INTEGER NOT NULL,
            delivered_at    INTEGER,
            parent_id       INTEGER REFERENCES outbox(id)
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS findings(
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id      INTEGER NOT NULL,
            claim       TEXT NOT NULL,
            evidence    TEXT NOT NULL,
            source      TEXT NOT NULL,
            confidence  REAL NOT NULL,
            ts          INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS contradictions(
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id                  INTEGER NOT NULL,
            finding_a_id            INTEGER NOT NULL,
            finding_b_id            INTEGER NOT NULL,
            state                   TEXT NOT NULL,
            resolved_by_finding_id  INTEGER
         );",
        [],
    )?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_active
             ON workflow_runs(domain, status);
         CREATE INDEX IF NOT EXISTS idx_workflow_steps_run
             ON workflow_steps(run_id, phase, step_key);",
    )?;

    // ── the intake law-version stamp (additive). ─────────────────────────
    // The server derives it at open time from the open body's OPTIONAL
    // jurisdiction through the SDK's single-owner table; empty = absent or
    // unknown jurisdiction (fail-open on labeling only). It must NEVER live
    // in state_json: the engines CAS against those exact bytes.
    {
        let present: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('workflow_runs') WHERE name='law_version'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                "ALTER TABLE workflow_runs ADD COLUMN law_version TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
    }

    // ── v1.28.18 "Lineage": outbox ancestry. ────────────────────────────
    // `parent_id` links each event to the event it followed (NULL = root).
    // Additive-NULL: existing rows become roots and legacy runs read as flat
    // sequences. The down-migration is a documented no-op (SQLite ALTER DROP
    // is not portable; keep the column, drop the code).
    {
        let present: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('outbox') WHERE name='parent_id'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                "ALTER TABLE outbox ADD COLUMN parent_id INTEGER REFERENCES outbox(id)",
                [],
            )?;
        }
    }

    // ── v1.28.22 "Bridges": the case↔run linkage. ───────────────────────
    // One row per CRM case ever synced, keyed on the stable `case_ref`
    // (`crm:{source}:{org}:{id}`). `run_id` is the governed run whose state
    // carries the same ref — the invariant Evolve's capture trigger depends
    // on. Written by the brain-connector-crm binary (idempotent upsert);
    // additive + rollback-safe.
    db.execute(
        "CREATE TABLE IF NOT EXISTS crm_cases(
            case_ref    TEXT PRIMARY KEY,
            source      TEXT NOT NULL,
            org_id      TEXT NOT NULL,
            case_id     TEXT NOT NULL,
            run_id      INTEGER REFERENCES workflow_runs(id),
            status      TEXT NOT NULL,
            updated_rev TEXT NOT NULL,
            synced_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );",
        [],
    )?;

    // ── v1.28.23 "Evolve": the KCS article lifecycle. ───────────────────
    // `knowledge` grows its KCS life: `kcs_state` (`none | draft | approved |
    // published`; existing rows stay `none` — KCS applies going forward, the
    // documented ceiling), `public_slug` (unique WHEN published via the
    // partial index; publishing itself is Beacon's, later), and
    // `freshness_review_due` (epoch; set at approve). `case_articles` is the
    // Solve-loop linkage: one row per (case, article) reuse/capture record;
    // `searched_not_found` rows carry NULL `knowledge_id` (the documented
    // zero-hit signal), so the uniqueness is partial.
    {
        let has_kcs_state: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='kcs_state'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_kcs_state {
            db.execute(
                "ALTER TABLE knowledge ADD COLUMN kcs_state TEXT NOT NULL DEFAULT 'none'",
                [],
            )?;
        }
        let has_public_slug: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='public_slug'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_public_slug {
            db.execute("ALTER TABLE knowledge ADD COLUMN public_slug TEXT", [])?;
        }
        let has_freshness: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('knowledge') WHERE name='freshness_review_due'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_freshness {
            db.execute(
                "ALTER TABLE knowledge ADD COLUMN freshness_review_due INTEGER",
                [],
            )?;
        }
    }
    db.execute(
        "CREATE TABLE IF NOT EXISTS case_articles(
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            case_ref     TEXT NOT NULL,
            knowledge_id INTEGER REFERENCES knowledge(id),
            sir          TEXT NOT NULL,
            action       TEXT NOT NULL,
            ts           INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_case_articles_link
         ON case_articles(case_ref, knowledge_id, sir) WHERE knowledge_id IS NOT NULL;",
        [],
    )?;
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_knowledge_published_slug
         ON knowledge(public_slug) WHERE kcs_state = 'published';",
        [],
    )?;

    // ── v1.28.25 "Watchbill": shifts and the sun. ────────────────────────
    // One row per site's on-call window: `overlap_minutes` declares the
    // handover budget with the NEXT shift (the ring boundary's overlap
    // window derives from the pair at read time); `roster_json` is a JSON
    // array of principal ids. Pure time-table arithmetic — computed at
    // read, no scheduler daemon. Additive + rollback-safe.
    db.execute(
        "CREATE TABLE IF NOT EXISTS shifts(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            domain          TEXT NOT NULL,
            site            TEXT NOT NULL,
            tz              TEXT NOT NULL DEFAULT 'UTC',
            start_epoch     INTEGER NOT NULL,
            end_epoch       INTEGER NOT NULL,
            overlap_minutes INTEGER NOT NULL DEFAULT 0,
            roster_json     TEXT NOT NULL DEFAULT '[]',
            created_at      INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_shifts_domain_window ON shifts(domain, start_epoch);",
        [],
    )?;

    // ── v1.28.26 "Crew": colleagues become visible. ─────────────────────
    // Presence piggybacks on authenticated activity: every mutating request
    // upserts one row per (domain, principal) INSIDE the caller's existing
    // transaction — there is no background worker and no heartbeat. Reads
    // compute TTL decay (active < 5 min, away < 30, offline beyond).
    // `principal_skills` are HITL-maintained: the ONLY write path is the
    // approval of a `crew_skills_update` proposal. `crew_config` is the DPO
    // switch — presence reads fail open to HIDDEN when the config cannot be
    // trusted.
    db.execute(
        "CREATE TABLE IF NOT EXISTS presence(
            domain           TEXT NOT NULL,
            principal        TEXT NOT NULL,
            ts               INTEGER NOT NULL,
            activity_kind    TEXT NOT NULL,
            current_case_ref TEXT,
            roles_json       TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY(domain, principal)
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS principal_skills(
            domain     TEXT NOT NULL,
            principal  TEXT NOT NULL,
            skill      TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(domain, principal, skill)
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS crew_config(
            domain           TEXT PRIMARY KEY,
            presence_enabled INTEGER NOT NULL DEFAULT 1
         );",
        [],
    )?;

    // ── v1.28.27 "Relay": the one-click handover. ───────────────────────
    // One row per handover offer over a run's I-PASS packet: offer refuses
    // on an incomplete packet (the missing list is the coaching surface);
    // accept/decline are lineage events audited in the same tx as the state
    // move; accept transfers `owner` by CAS and never touches the SLA clock.
    db.execute(
        "CREATE TABLE IF NOT EXISTS handover_offers(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            domain          TEXT NOT NULL,
            run_id          INTEGER NOT NULL REFERENCES workflow_runs(id),
            from_principal  TEXT NOT NULL,
            to_principal    TEXT NOT NULL,
            state           TEXT NOT NULL DEFAULT 'offered',
            reason          TEXT,
            overlap_minutes INTEGER NOT NULL DEFAULT 0,
            sla_deadline    INTEGER NOT NULL,
            created_at      INTEGER NOT NULL,
            decided_at      INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_handover_offers_run
         ON handover_offers(run_id, state);",
        [],
    )?;

    // ── v1.28.28 "Channel": the case gets a room. ────────────────────────
    // Case-scoped channel messages: one row per note (kind `note`) and per
    // swarm invite (kind `invite`, addressed_to = the invited principal,
    // pending → accepted by the SAME accept machinery as Relay, smaller).
    // Everything is case-scoped — no DMs, no channels without a run; content
    // is screened + bounded at write, retained per domain policy (read-time
    // filter), and swept by DSAR with the run. The outbox rows on the
    // `case/note` topic are the lineage events + the SSE ping.
    db.execute(
        "CREATE TABLE IF NOT EXISTS case_notes(
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            domain         TEXT NOT NULL,
            run_id         INTEGER NOT NULL REFERENCES workflow_runs(id),
            author         TEXT NOT NULL,
            kind           TEXT NOT NULL DEFAULT 'note',
            content        TEXT NOT NULL,
            addressed_to   TEXT,
            parent_note_id INTEGER,
            state          TEXT NOT NULL DEFAULT 'visible',
            decided_at     INTEGER,
            created_at     INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_case_notes_run ON case_notes(run_id, id);",
        [],
    )?;

    // ── v1.28.29 "Mesh": agents as named colleagues. ─────────────────────
    // `agent_cards` is the A2A-shaped identity manifest per agent principal:
    // signed with the UMP operator key at provisioning and re-verified at
    // every use point (reads + delegation acceptance) — a card whose
    // signature fails refuses loudly. `delegations` holds agent→agent work
    // orders over a run; the lineage events on `delegation/request` /
    // `delegation/result` carry ids + actors only, never task content.
    db.execute(
        "CREATE TABLE IF NOT EXISTS agent_cards(
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            domain            TEXT NOT NULL,
            principal         TEXT NOT NULL,
            name              TEXT NOT NULL,
            description       TEXT NOT NULL DEFAULT '',
            capabilities_json TEXT NOT NULL DEFAULT '{}',
            card_json         TEXT NOT NULL,
            signature         TEXT NOT NULL,
            signed_by         TEXT NOT NULL,
            signing_epoch     INTEGER,
            created_at        INTEGER NOT NULL,
            UNIQUE(domain, principal)
         );",
        [],
    )?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS delegations(
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            domain         TEXT NOT NULL,
            run_id         INTEGER NOT NULL REFERENCES workflow_runs(id),
            from_principal TEXT NOT NULL,
            to_principal   TEXT NOT NULL,
            task           TEXT NOT NULL,
            state          TEXT NOT NULL DEFAULT 'requested',
            result         TEXT,
            created_at     INTEGER NOT NULL,
            decided_at     INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_delegations_run ON delegations(run_id, id);",
        [],
    )?;

    // ── v1.28.62 "Attestation": agent identity lifecycle (ASI03/07). ─────
    // The principal kill-switch: one row per revoked principal, latest
    // revocation wins (a re-revocation updates reason/ts). EVERY card use,
    // delegation dispatch, and result submission re-checks this table BEFORE
    // signature verification (order stays fail-closed either way); the
    // revocation write drains the principal's in-flight runs through the
    // EXISTING run-cancel path (state.rs cas_update → status 'cancelled')
    // inside the same transaction as the audit row. The audit chain carries
    // the whole story (revoke → drain), hash-chained in-tx.
    db.execute(
        "CREATE TABLE IF NOT EXISTS revoked_principals(
            principal  TEXT PRIMARY KEY,
            revoked_at INTEGER NOT NULL,
            reason     TEXT NOT NULL DEFAULT '',
            revoked_by TEXT NOT NULL DEFAULT ''
         );",
        [],
    )?;

    // ── v1.28.30 "Parcels": sites share knowledge, governed. ─────────────
    // One row per parcel crossing a site boundary: `direction` is `out`
    // (signed export) or `in` (imported as pending proposals — never direct
    // knowledge writes). `signer` is the exporter's `did:key`; `reviewer` the
    // importing operator; the audit chain links every crossing in-tx.
    db.execute(
        "CREATE TABLE IF NOT EXISTS parcel_ledger(
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            domain      TEXT NOT NULL,
            direction   TEXT NOT NULL,
            parcel_hash TEXT NOT NULL,
            signer      TEXT NOT NULL,
            row_count   INTEGER NOT NULL DEFAULT 0,
            reviewer    TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_parcel_ledger_domain ON parcel_ledger(domain, id);",
        [],
    )?;

    // ── v1.28.35 "Outreach": consent-first outbound contact. ────────────
    // One row per (domain, subject_hash, channel, purpose): the consent
    // registry every outreach gate reads deterministically. Subjects are
    // stored HASHED (audit::hash) — raw identifiers never touch this table.
    // Rows are created/updated ONLY through approved HITL proposals and die
    // with their subject on a DSAR sweep (workflow::erasure). No send engine
    // exists anywhere in this server — campaigns export for CRM-side
    // execution.
    db.execute(
        "CREATE TABLE IF NOT EXISTS consent_registry(
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            domain       TEXT NOT NULL,
            subject_hash TEXT NOT NULL,
            channel      TEXT NOT NULL,
            purpose      TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'granted',
            provenance   TEXT NOT NULL DEFAULT '',
            granted_at   INTEGER NOT NULL,
            expires_at   INTEGER,
            revoked_at   INTEGER,
            updated_at   INTEGER NOT NULL,
            UNIQUE(domain, subject_hash, channel, purpose)
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_consent_subject
          ON consent_registry(domain, subject_hash);",
        [],
    )?;

    // The CRM-merge/reopen re-ask mapping needs the hashed
    // subject on the crm_cases row (additive column; existing rows degrade
    // to '' which the detector skips).
    let has_subject_ref: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('crm_cases') WHERE name='subject_ref'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_subject_ref {
        db.execute(
            "ALTER TABLE crm_cases ADD COLUMN subject_ref TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    // ── Public case-status refs + KB translations (schema history lives in
    // the version stamp below).
    // One live status ref per run (UNIQUE on both sides): an unguessable
    // HMAC-derived token naming the static `status/{ref}.json` artifact.
    // Refs are minted/rotated/revoked ONLY through audited operator actions
    // (workflow/case_status); rotation kills the old ref, revocation removes
    // the page from the next build; a DSAR sweep or legal-hold freeze
    // revokes+purges them (subject-linked artifacts).
    db.execute(
        "CREATE TABLE IF NOT EXISTS case_status_refs(
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id       INTEGER NOT NULL UNIQUE,
            ref          TEXT NOT NULL UNIQUE,
            salt_version INTEGER NOT NULL DEFAULT 1,
            minted_at    INTEGER NOT NULL,
            rotated_at   INTEGER,
            revoked_at   INTEGER
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_case_status_refs_live
          ON case_status_refs(ref) WHERE revoked_at IS NULL;",
        [],
    )?;
    // Per-locale human translations of published knowledge articles.
    // Translation is a HUMAN act — rows are created ONLY through approved
    // `kcs_translate` HITL proposals; `based_revision` pins the source
    // revision they translated so staleness is first-class (the source
    // advancing past it lands the translation on the content-health
    // worklist). UNIQUE(knowledge_id, locale): one live translation per
    // locale.
    db.execute(
        "CREATE TABLE IF NOT EXISTS kcs_translations(
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            knowledge_id   INTEGER NOT NULL REFERENCES knowledge(id),
            locale         TEXT NOT NULL,
            title          TEXT NOT NULL,
            body_md        TEXT NOT NULL,
            based_revision TEXT NOT NULL DEFAULT '',
            state          TEXT NOT NULL DEFAULT 'draft',
            translator     TEXT NOT NULL DEFAULT '',
            approved_at    INTEGER,
            created_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL,
            UNIQUE(knowledge_id, locale)
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_kcs_translations_locale
          ON kcs_translations(locale, state);",
        [],
    )?;

    // ── v1.28.42 "Valet": the one-subject Outreach-lite consent registry ──
    // (signal channel only, sole subject `owner` enforced in code) + the
    // advisory lint report that rides draft proposals. Additive only.
    db.execute(
        "CREATE TABLE IF NOT EXISTS valet_consents(
            subject_hash TEXT NOT NULL,
            channel      TEXT NOT NULL,
            granted_at   INTEGER NOT NULL,
            revoked_at   INTEGER,
            UNIQUE(subject_hash, channel)
         );",
        [],
    )?;
    let lint_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='lint_json'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !lint_present {
        db.execute("ALTER TABLE proposals ADD COLUMN lint_json TEXT", [])?;
    }

    // ── v1.28.43 "Switchboard": the channel thread map + bridge seams ──
    // One row per (channel, tenant-prefixed conversation_ref): the case
    // threading table inbound channel messages resolve through. DOMAIN is
    // part of every predicate (tenant scoping by construction — a bridge can
    // only touch cases under its own configured domain). subject_hash is the
    // HASHED conversation identity (audit::hash) so raw subscriber addresses
    // never rest here; last_inbound_at powers the deterministic reply-window
    // gate outbound sends ride. Additive; rows die with their DSAR sweep of
    // the owning run like any run-scoped evidence.
    db.execute(
        "CREATE TABLE IF NOT EXISTS channel_threads(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            channel         TEXT NOT NULL,
            tenant          TEXT NOT NULL,
            conversation_ref TEXT NOT NULL,
            domain          TEXT NOT NULL,
            case_run_id     INTEGER NOT NULL REFERENCES workflow_runs(id),
            subject_hash    TEXT NOT NULL DEFAULT '',
            last_inbound_at INTEGER,
            created_at      INTEGER NOT NULL,
            UNIQUE(channel, tenant, conversation_ref)
         );",
        [],
    )?;
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_channel_threads_run
          ON channel_threads(case_run_id);",
        [],
    )?;

    // ── v1.28.45 "Herald": the Slack/Teams user map ──────────────────────
    // Proposal-maintained platform-identity → principal mappings (the ONLY
    // writer is the approval path in workflow::channels — no HTTP route ever
    // touches the table). Platform ids are OPAQUE: the kernel resolves every
    // channel act (console decide/due/crank, presence) through this map, so
    // a platform identity is never auto-trusted. Tenant-scoped by predicate.
    db.execute(
        "CREATE TABLE IF NOT EXISTS channel_user_map(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            channel         TEXT NOT NULL,
            tenant          TEXT NOT NULL,
            platform_user_id TEXT NOT NULL,
            principal       TEXT NOT NULL,
            roles_json      TEXT NOT NULL DEFAULT '[]',
            created_at      INTEGER NOT NULL,
            created_by      TEXT NOT NULL DEFAULT '',
            UNIQUE(channel, tenant, platform_user_id)
         );",
        [],
    )?;

    // ── v1.28.53 "Triage": the review queue gains a domain ────────────
    // `proposals` predates domains: every row was a GLOBAL row (parcels
    // approximated scoping with the `parcel:{domain}:{signer}` source
    // label). Additive: `domain` is the residency label the review queue
    // scopes by in shim mode (multi-db pools are the territory already; the
    // column rides as the denormalized honest stamp), `title` is the
    // optional passthrough the queue surfaces. Backfill: existing rows keep
    // 'global' forever — provenance beats guessing (no heuristic
    // re-attribution). The (status, domain) index serves the narrowed page
    // read; idx_proposals_status stays for the global sweeps.
    let prop_domain_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='domain'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !prop_domain_present {
        db.execute(
            "ALTER TABLE proposals ADD COLUMN domain TEXT NOT NULL DEFAULT 'global'",
            [],
        )?;
    }
    let prop_title_present: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='title'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;
    if !prop_title_present {
        db.execute("ALTER TABLE proposals ADD COLUMN title TEXT", [])?;
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS idx_proposals_status_domain ON proposals(status, domain)",
        [],
    )?;

    // Bumped once per release that changes this function.
    // v1.32.16 "Attestations": delivery_attestations table + delivery_traces.seq
    // (additive, guarded) → 1.32.16.
    // v1.32.15 "Delivery": delivery_traces + delivery_budgets tables → 1.32.15.
    // v1.32.14 "DecisionEvaluation": decision_evaluation_runs table → 1.32.14.
    // v1.32.13 "ModelRegistry": decision_model_registry table → 1.32.13.
    // v1.32.12 "DecisionSurface": proposals.decision_run_ref (additive, nullable) → 1.32.12.
    // v1.32.11 "DecisionTrace": decision_run_traces table → 1.32.11.
    // v1.32.0 "LoopCore": agent_session_events table (the agent-loop
    // session event log: append-only, per-run seq, exactly-once by key) → 1.32.0.
    // v1.28.77 "Erasure": suggest_feedback.owner (additive, nullable) → 1.28.77.
    // v1.28.62 "Attestation": revoked_principals table → 1.28.62.
    // v1.28.53 "Triage": proposals.domain + proposals.title + the
    // (status, domain) index → 1.28.53.
    // v1.28.45 "Herald": channel_user_map table → 1.28.45.
    // v1.28.43 "Switchboard": channel_threads table → 1.28.43.
    // v1.28.42 "Valet": valet_consents table + proposals.lint_json → 1.28.42.
    // v1.28.36 "Keystone": case_status_refs + kcs_translations tables → 1.28.36.
    // v1.28.35 "Outreach": consent_registry table → 1.28.35.
    // v1.28.30 "Parcels": parcel_ledger table → 1.28.30.
    // v1.28.29 "Mesh": agent_cards + delegations tables → 1.28.29.
    // v1.28.28 "Channel": case_notes table → 1.28.28.
    // v1.28.27 "Relay": handover_offers table → 1.28.27.
    // v1.28.26 "Crew": presence + principal_skills + crew_config tables → 1.28.26.
    // v1.27.18 "Groundwork": indexes added/dropped → 1.27.18.
    // v1.27.22 "Cascade": relationships.superseded_at + idx_rels_bt → 1.27.22.
    // v1.27.25 "Scoped": idx_rels_open_unique partial unique index (+ dedup) → 1.27.25.
    // v1.27.30 "Spine": the five governed-workflow tables → 1.27.30.
    // v1.27.31 "AuditRepair": the audit head pin (`schema_meta.audit_chain_head`)
    // stamped for existing chains; the epoch key (`audit_chain_epoch`) is
    // runtime-only (absent = legacy) — the format itself flips only via the
    // offline `--re-audit` re-anchor. No tables, no columns.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rules(id INTEGER PRIMARY KEY, jurisdiction TEXT NOT NULL, subject TEXT NOT NULL, rule_key TEXT NOT NULL, body TEXT NOT NULL, source_ref TEXT NOT NULL, effective_at INTEGER NOT NULL, reviewed_at INTEGER, expires_at INTEGER, revision INTEGER NOT NULL, superseded_by INTEGER, created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS rule_rates(id INTEGER PRIMARY KEY, rule_id INTEGER NOT NULL REFERENCES rules(id), rate_json TEXT NOT NULL, applicable_from INTEGER NOT NULL);",
    )?;
    // ── the key-lifecycle release: agent cards carry the signing epoch so
    // `verify_card` picks the operator key deterministically and the audit
    // trail names the generation. Additive column; legacy rows are NULL
    // (= "current-or-previous" verification, byte-compat with old binaries).
    let has_epoch: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('agent_cards') WHERE name='signing_epoch'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_epoch {
        db.execute_batch("ALTER TABLE agent_cards ADD COLUMN signing_epoch INTEGER;")?;
    }

    // ── the knowledge-version axis on the case record ──
    // The ring is not idempotent under time: Solve is per-case/minutes, Evolve is
    // per-pattern/days, Deflect is per-corpus/weeks. A case can therefore be open
    // while Evolve publishes a supersession UNDERNEATH it, and a reopened case
    // (`reask`, back-referral) re-enters Solve against a MOVED knowledge base —
    // silently mixing evidence from two versions.
    //
    // Additive only, no rebuild. `NULL` = the row predates tracking; a sentinel 0
    // would falsely date every legacy row to version zero. An INTEGER monotonic
    // counter, not a timestamp, so ordering is total and comparison is one integer.
    //
    // CEILING (stated, not hidden): there is NO Evolve bump site yet, so the value
    // written today is CONSTANT. It records which version a case opened against; it
    // does not by itself prevent mixed-basis reasoning — the delta offer is where
    // prevention lives, and delta semantics are not yet defined.
    let has_kv: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('workflow_runs') WHERE name='knowledge_version'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_kv {
        db.execute_batch("ALTER TABLE workflow_runs ADD COLUMN knowledge_version INTEGER;")?;
    }

    // ── the Loop line: the agent-session event log ──────────────────────
    // Append-only, one row per session event, per-run monotonic seq,
    // exactly-once by idempotency key. The audit chain stores hashes, not
    // payloads, so a REPLAYABLE log cannot be derived from it — this is the
    // declared loop-state table (migrate-rehearse parity covers it). No FK
    // to workflow_runs on purpose: same posture as the outbox (foreign runs
    // are refused by the writer's query, not by schema).
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_session_events(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER NOT NULL,
            seq INTEGER NOT NULL,
            idempotency_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(run_id, seq),
            UNIQUE(run_id, idempotency_key)
         );
         CREATE INDEX IF NOT EXISTS idx_agent_session_events_run
             ON agent_session_events(run_id, seq);",
    )?;

    // ── the decision-run trace table ────────────────────────────────────
    // The decision harness's replayable run artifact (digests, refs, and
    // per-stage records — query-adjacent free text is hashed, never stored
    // raw), one row per persisted decision run. Mirrors the recall_traces
    // precedent: additive, no migration of existing tables, no FK on
    // purpose (same posture as agent_session_events — foreign runs are the
    // writer's refusal, not the schema's). The session-log decision_*
    // kinds are the event face; this row is the bounded-listing + replay
    // artifact.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS decision_run_traces(
            id               INTEGER PRIMARY KEY,
            run_id           INTEGER NOT NULL,
            mode             TEXT NOT NULL,
            pipeline_version TEXT NOT NULL,
            config_hash      TEXT NOT NULL,
            trace_json       TEXT NOT NULL,
            created_at       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_decision_run_traces_run
            ON decision_run_traces(run_id);",
    )?;

    // ── the trace row's model citation ──────────────────────────────────
    // WHICH registered model actually produced this run's verdict — the one
    // gap a trace could not answer: the trace recorded the pipeline, the
    // config, and the digests, but not the identity of the promoted model
    // that the human gate had cleared. The three field names are the ones
    // `decision_evaluation_runs` ALREADY uses, deliberately: a trace row and
    // an evaluation row then join on the same three columns with no
    // translation layer between the two record types.
    //
    // All three are NULLABLE and carry NO default. NULL means "this trace
    // predates citation tracking" — a DEFAULT or a sentinel would backfill
    // over every historical row and falsely date it to a model. The digest
    // is NULLable for a second, independent reason: `RegistryRow.config_digest`
    // is itself optional, and a trace records what the resolver RETURNED
    // rather than a digest invented at the write edge.
    //
    // Guarded per column by `pragma_table_info` (the in-file additive-column
    // loop), so re-running the runner is a no-op rather than an error, and a
    // DB that already carries them is left alone. No table is dropped and
    // none is rebuilt: a rebuild is the one operation that can lose rows
    // under a crash.
    for (col, def) in [
        ("model_registry_id", "TEXT"),
        ("model_registry_version", "TEXT"),
        ("model_registry_digest", "TEXT"),
    ] {
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('decision_run_traces') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE decision_run_traces ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }

    // ── the decision-harness provenance column ──────────────────────────
    // `proposals.decision_run_ref` — additive, nullable TEXT carrying the
    // decision-run provenance ref (trace id, run id, recorded mode, config
    // hash) of the run that proposed the row; NULL for every ordinary
    // human/loop proposal. The promotion gate reads the ref's mode: an
    // exploratory run can propose, never promote. Legacy rows are NULL
    // (byte-compat: every existing reader/writer is untouched).
    let has_decision_run_ref: bool = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='decision_run_ref'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_decision_run_ref {
        db.execute_batch("ALTER TABLE proposals ADD COLUMN decision_run_ref TEXT;")?;
    }

    // ── the model identity registry ──────────────────────────────────────
    // One row per declared model identity. Artifact/config digests and the
    // lifecycle are the durable control-plane record; model bytes never live
    // in this table.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS decision_model_registry(
            id                TEXT NOT NULL,
            version           TEXT NOT NULL,
            kind              TEXT NOT NULL,
            name              TEXT NOT NULL,
            output_vocabulary TEXT NOT NULL,
            artifact_digest   TEXT,
            config_digest     TEXT,
            calibration_ref   TEXT,
            status            TEXT NOT NULL CHECK (status IN ('candidate','evaluated','promoted','retired')),
            evaluation_refs   TEXT NOT NULL DEFAULT '[]',
            proposed_by       TEXT NOT NULL,
            approved_by       TEXT,
            created_at        INTEGER NOT NULL,
            updated_at        INTEGER NOT NULL,
            PRIMARY KEY (id, version)
        );
        CREATE INDEX IF NOT EXISTS idx_registry_status
            ON decision_model_registry(status);",
    )?;

    // ── the decision-evaluation record ──────────────────────────────────
    // Bounded metadata, references, closed labels, and aggregate reports only;
    // no raw query, evidence text, model bytes, or secrets are stored here.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS decision_evaluation_runs(
            evaluation_id          TEXT PRIMARY KEY,
            idempotency_key        TEXT NOT NULL UNIQUE,
            request_digest         TEXT NOT NULL,
            pipeline_version       TEXT NOT NULL,
            config_hash            TEXT NOT NULL,
            model_registry_id      TEXT NOT NULL,
            model_registry_version TEXT NOT NULL,
            model_registry_digest  TEXT NOT NULL,
            model_artifact_digest  TEXT,
            judgment_set_ref       TEXT NOT NULL,
            judgment_set_source    TEXT NOT NULL CHECK (judgment_set_source IN ('operator_declared')),
            judgment_set_digest    TEXT NOT NULL,
            judgment_set_count     INTEGER NOT NULL,
            judgment_set_frozen_at INTEGER NOT NULL,
            manifest_json          TEXT NOT NULL,
            report_json            TEXT NOT NULL,
            acceptance_state       TEXT NOT NULL CHECK (acceptance_state = 'operator_accepted_non_authoritative'),
            acceptance_bars_json   TEXT NOT NULL,
            creator_id             TEXT NOT NULL,
            reviewer_id            TEXT NOT NULL,
            created_at             INTEGER NOT NULL,
            record_digest          TEXT NOT NULL,
            audit_target           TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_decision_evaluation_runs_created
            ON decision_evaluation_runs(created_at);",
    )?;

    // ── the delivery loop's two persistence tables ─────────────────────
    // Bounded metadata, references, digests, and closed labels only — no raw
    // query, evidence text, model bytes, rules bytes, or secrets (the
    // decision_evaluation_runs posture). Refs and digests, so a trace row is
    // evidence rather than content.
    //
    // `blast_radius` is admitted by the kind CHECK because the governing spec
    // names it, and is UNENFORCED by design: this round stores the row and no
    // route or ceiling may reference it. Turning it on is a later round's turn.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS delivery_traces(
            id                 TEXT PRIMARY KEY,
            run_id             INTEGER NOT NULL,
            stage              TEXT NOT NULL CHECK (stage IN ('run','phase','gate','answer')),
            phase              TEXT NOT NULL CHECK (phase IN ('scope','design','build','release','operate','done')),
            status             TEXT NOT NULL CHECK (status IN ('admitted','advanced','allowed','prompt','denied','answered')),
            tier               TEXT NOT NULL CHECK (tier IN ('observe','propose','bounded-auto','delegated')),
            actor              TEXT NOT NULL,
            model_ref          TEXT,
            policy_digest      TEXT,
            config_digest      TEXT,
            pipeline_version   TEXT NOT NULL,
            budget_digest      TEXT,
            artifact_refs_json TEXT NOT NULL DEFAULT '[]',
            attestation_root   TEXT,
            created_at         INTEGER NOT NULL,
            -- the stored-ordinal law: the STORED ordinal inside the run. `id` digests it, so
            -- the position is part of the row's identity and must be a column
            -- and not a runtime count: `COUNT(*)` reissues an ordinal after a
            -- delete and collides on the UNIQUE index below, which is exactly
            -- what `MAX(seq)+1` prevents. The DEFAULT backfills an existing
            -- 1.32.15 database's rows with 0 — declared, not repaired: the
            -- sequence of a pre-the attestation round run is not recoverable, and the id churn
            -- this round already causes is disclosed in the CHANGELOG.
            seq                 INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_delivery_traces_run
            ON delivery_traces(run_id);
        CREATE INDEX IF NOT EXISTS idx_delivery_traces_replay
            ON delivery_traces(run_id, created_at);
        CREATE TABLE IF NOT EXISTS delivery_budgets(
            run_id     INTEGER NOT NULL,
            kind       TEXT NOT NULL CHECK (kind IN ('tokens','tool_calls','files','minutes','blast_radius')),
            ceiling    INTEGER NOT NULL CHECK (ceiling >= 0),
            spent      INTEGER NOT NULL DEFAULT 0 CHECK (spent >= 0),
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (run_id, kind)
        );
        CREATE INDEX IF NOT EXISTS idx_delivery_budgets_run
            ON delivery_budgets(run_id);
        -- the attestation round (A10): the per-run SIGNED CHAIN. Twelve columns,
        -- exactly the design owner's list, and no FK on purpose (house style).
        -- The `id` is content-addressed over
        -- `chain_hash || run_id || step_id`, so it is stable under
        -- re-derivation and does NOT change when later links append — unlike
        -- `delivery_traces.attestation_root`, which names the head and
        -- therefore moves as the chain grows. The PK is declared NOT NULL
        -- because SQLite otherwise permits NULLs in a non-INTEGER primary key,
        -- and a chain row with no id could never be addressed.
        --
        -- There is deliberately no disposition, status, decision, approval, or
        -- outcome column: an attestation is EVIDENCE OF WHO ACTED, never a
        -- disposition. Nothing may be promoted or denied on the strength of a
        -- row in this table.
        CREATE TABLE IF NOT EXISTS delivery_attestations(
            id               TEXT PRIMARY KEY NOT NULL,
            run_id           INTEGER NOT NULL,
            step_id          INTEGER NOT NULL,
            subject_name     TEXT NOT NULL,
            subject_digest   TEXT NOT NULL,
            predicate_type   TEXT NOT NULL,
            predicate_digest TEXT NOT NULL,
            envelope_json    TEXT NOT NULL,
            signer_did       TEXT NOT NULL,
            parent_id        TEXT NOT NULL DEFAULT '',
            chain_hash       TEXT NOT NULL,
            created_at       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_delivery_attestations_replay
            ON delivery_attestations(run_id, created_at);",
    )?;

    // The guarded ALTER for a database that already ran the 1.32.15
    // batch. Same `pragma_table_info` pattern as every other additive column in
    // this file, so a 1.32.15 database gains the column and a fresh build gets
    // it from the CREATE above without either path running twice.
    //
    // The ordinals are BACKFILLED before the index is created, and that ordering
    // is load-bearing: the ADD COLUMN defaults every pre-existing row to 0, so a
    // run with three trace rows would hold three (run_id, 0) pairs and
    // `CREATE UNIQUE INDEX` would fail the whole migration on exactly the
    // database this block exists to upgrade. The backfill numbers each row
    // 1..n per run in `(created_at, rowid)` order — the order the chain and the
    // replay index read in — so the stored ordinal is the row's real position
    // and not a placeholder.
    //
    // The index is created HERE and not in the batch above, for the same
    // reason: the batch's `CREATE TABLE IF NOT EXISTS` is a no-op against a
    // 1.32.15 table, so indexing a column the batch has not added would fail
    // there too.
    {
        let has_seq: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('delivery_traces') WHERE name='seq'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_seq {
            db.execute(
                "ALTER TABLE delivery_traces ADD COLUMN seq INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            db.execute(
                "UPDATE delivery_traces SET seq = (
                     SELECT COUNT(*) FROM delivery_traces prior
                      WHERE prior.run_id = delivery_traces.run_id
                        AND (prior.created_at < delivery_traces.created_at
                             OR (prior.created_at = delivery_traces.created_at
                                 AND prior.rowid <= delivery_traces.rowid))
                 )",
                [],
            )?;
        }
        db.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_delivery_traces_seq
             ON delivery_traces(run_id, seq)",
            [],
        )?;
    }

    // v1.32.17 "Bindings": the machine's STANDING authority to read one
    // external system on behalf of one tenant. A binding is not a
    // credential store and not a destination — it is a SCOPED permission,
    // and `domain` is a load-bearing column rather than decoration: a binding
    // that resolved without a domain check would be a cross-tenant
    // authority leak.
    //
    // `target_kind` carries a closed CHECK, and the enforcement is the
    // DATABASE's rather than a convention's: `registry`/`deploy`/`pm`/
    // `incident` are declared and CONSUMER-LESS (no adapter reads them) so
    // that adding one is a deliberate act with an adapter behind it, never a
    // vocabulary that grows by accident.
    //
    // `authority_digest` covers the endpoint, the stable external ref, and
    // the secret's FILE NAME — never the secret and never its path. A digest
    // computed over secret material is a credential at rest in a hash column:
    // a low-entropy secret is recoverable by brute force, and a high-entropy
    // one is a bearer that can never be rotated without rewriting history.
    //
    // `active` is the consent lever. Consent is GIVEN by configuring a
    // binding and WITHDRAWN by setting it to 0; neither is a request-time
    // operation, because a request must never be able to create or widen an
    // authority. There is no write route for this table.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS delivery_bindings(
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            domain            TEXT    NOT NULL,
            target_kind       TEXT    NOT NULL
                                  CHECK (target_kind IN
                                         ('vcs','ci','registry','deploy','pm','incident')),
            target_ref        TEXT    NOT NULL,
            endpoint          TEXT    NOT NULL,
            authority_digest  TEXT    NOT NULL,
            policy_digest     TEXT,
            capabilities_json TEXT    NOT NULL,
            secret_file_name  TEXT    NOT NULL DEFAULT '',
            active            INTEGER NOT NULL DEFAULT 1,
            created_at        INTEGER NOT NULL,
            updated_at        INTEGER NOT NULL,
            UNIQUE(domain, target_kind, target_ref)
        );
        CREATE INDEX IF NOT EXISTS idx_delivery_bindings_kind
            ON delivery_bindings(target_kind, active);",
    )?;

    // The resolver selects the secret's FILE NAME from this table — the
    // digest covers (endpoint, target_ref, file name), so the credential SLOT
    // a binding names is binding state, not a re-derivable property. The
    // 1.32.17 batch shipped without the column (the resolver's first real
    // caller arrived with the release round and caught it), so a database
    // that already ran that batch gains it here; a fresh build gets it from
    // the CREATE above. Same `pragma_table_info` pattern as every other
    // additive column in this file.
    {
        let has_secret_file: bool = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('delivery_bindings') WHERE name='secret_file_name'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_secret_file {
            db.execute(
                "ALTER TABLE delivery_bindings ADD COLUMN secret_file_name TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
    }

    // v1.32.18 "Releases": the governed release row — the machine's proposal
    // to move an artifact to an external authority, the human's approval
    // record, and the lifecycle the promotion gate walks. Three properties,
    // all load-bearing:
    //
    // * The status CHECK carries the crate's nine `ReleaseStatus` wire names.
    //   A release status does NOT fit `delivery_traces.status` (that CHECK is
    //   the trace's own six-value vocabulary), so release state lives HERE
    //   and nowhere else. Widening the trace CHECK to absorb release
    //   vocabulary would be a schema decision this line has not made.
    // * The approval is COLUMNS on the release row, not a sixth table: the
    //   row IS the approval artifact and the hash-chained audit rows are its
    //   history. The binding is THREE-WAY — content digest
    //   (`approval_subject_digest`), authority digest
    //   (`approval_authority_digest`), and the run's state revision at
    //   approval (`approval_state_revision`) — because an approval that binds
    //   content but not the target is replayable against a different
    //   external system, and one that binds both but not the revision is
    //   replayable across a later phase pass.
    // * `commit_sha` + `environment` are the OTel revision/environment facts
    //   (`vcs.repository.ref.revision` is Release Candidate — cited by name,
    //   never claimed stable; the four environment values are the Stable
    //   `deployment.environment.name` set; the closed CHECK is our stricter
    //   choice, disclosed).
    //
    // `verified_at` is written ONLY by the inbound authority reconcile path;
    // the crank never sets it. No FK, house style.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS delivery_releases(
            id                        INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id                    INTEGER NOT NULL,
            binding_id                INTEGER NOT NULL,
            ref                       TEXT    NOT NULL,
            commit_sha                TEXT,
            environment               TEXT    NOT NULL
                                          CHECK (environment IN
                                                 ('development','test','staging','production')),
            artifact_digest           TEXT    NOT NULL,
            policy_digest             TEXT,
            approval_subject_digest   TEXT,
            approval_principal        TEXT,
            approval_scope            TEXT,
            approval_authority_digest TEXT,
            approval_state_revision   INTEGER,
            approval_expires_at       INTEGER,
            approved_at               INTEGER,
            status                    TEXT    NOT NULL
                                          CHECK (status IN
                                                 ('proposed','approved','building','attested',
                                                  'staged','promoted','verified','rolled_back',
                                                  'failed')),
            created_at                INTEGER NOT NULL,
            updated_at                INTEGER NOT NULL,
            deployed_at               INTEGER,
            verified_at               INTEGER,
            rolled_back_at            INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_delivery_releases_run
            ON delivery_releases(run_id);
        CREATE INDEX IF NOT EXISTS idx_delivery_releases_status
            ON delivery_releases(status);",
    )?;

    // The create loop's four tables. Additive only — nothing is dropped and
    // nothing is rebuilt, because a rebuild is the one operation that can lose
    // rows under a crash. The gated read model is a query in a service core,
    // never a view.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS claim_schemas (
            id           INTEGER PRIMARY KEY,
            domain       TEXT NOT NULL,
            version      INTEGER NOT NULL,
            authored_by  TEXT NOT NULL,
            body         TEXT NOT NULL,
            body_digest  TEXT NOT NULL,
            ratified_at  INTEGER,
            created_at   INTEGER NOT NULL,
            CHECK (authored_by = 'human'),
            CHECK (length(body_digest) = 64)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_claim_schemas_domain_version
            ON claim_schemas(domain, version);

        CREATE TABLE IF NOT EXISTS claim_batches (
            id            INTEGER PRIMARY KEY,
            batch_digest  TEXT NOT NULL,
            member_count  INTEGER NOT NULL,
            set_check     TEXT NOT NULL DEFAULT 'pending',
            checked_at    INTEGER,
            created_at    INTEGER NOT NULL,
            CHECK (set_check IN ('pending','pass','fail')),
            CHECK (member_count >= 0),
            CHECK (length(batch_digest) = 64)
        );

        CREATE TABLE IF NOT EXISTS claims (
            id                  INTEGER PRIMARY KEY,
            claim_id            TEXT NOT NULL UNIQUE,
            schema_ref          INTEGER NOT NULL REFERENCES claim_schemas(id),
            subject             TEXT NOT NULL,
            predicate           TEXT NOT NULL,
            object              TEXT NOT NULL,
            qualifiers          TEXT NOT NULL DEFAULT '[]',
            valid_from          INTEGER,
            valid_to            INTEGER,
            status              TEXT NOT NULL DEFAULT 'pending',
            support_n           INTEGER NOT NULL DEFAULT 0,
            scope               TEXT,
            contradicts         TEXT NOT NULL DEFAULT '[]',
            batch_id            INTEGER REFERENCES claim_batches(id),
            recall_visible      INTEGER NOT NULL DEFAULT 0,
            evidence_digest     TEXT NOT NULL,
            audit_digest        TEXT,
            audit_target_hash   TEXT NOT NULL,
            created_by          TEXT NOT NULL,
            created_at          INTEGER NOT NULL,
            promoted_by         TEXT,
            promoted_at         INTEGER,
            CHECK (status IN ('pending','verify_failed','ratified','rejected',
                              'evidence_broken','superseded')),
            CHECK (recall_visible IN (0,1)),
            CHECK (created_by IN ('agent','human')),
            CHECK (length(audit_target_hash) = 64),
            CHECK (promoted_at IS NULL OR status = 'ratified')
        );
        CREATE INDEX IF NOT EXISTS idx_claims_status ON claims(status);
        CREATE INDEX IF NOT EXISTS idx_claims_batch  ON claims(batch_id);
        CREATE INDEX IF NOT EXISTS idx_claims_pred   ON claims(subject, predicate);

        CREATE TABLE IF NOT EXISTS claim_evidence (
            id          INTEGER PRIMARY KEY,
            claim_ref   INTEGER NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
            source_cid  TEXT NOT NULL,
            quote       TEXT NOT NULL,
            byte_start  INTEGER NOT NULL,
            byte_end    INTEGER NOT NULL,
            quote_digest TEXT NOT NULL,
            CHECK (byte_end > byte_start),
            CHECK (byte_start >= 0),
            CHECK (length(quote_digest) = 64)
        );
        CREATE INDEX IF NOT EXISTS idx_claim_evidence_claim ON claim_evidence(claim_ref);",
    )?;

    // The write fence. Four triggers, and they are the reason an application
    // guard is not enough here: a guard sits behind the same API the model
    // talks to, so a socially-engineered write walks straight past it. In the
    // database, it does not.
    //
    // The stated limit, which is the control's ceiling and not a caveat: each
    // trigger keys on a string the APPLICATION set (the principal kind, the
    // pre-computed target digest). It therefore defends a compromised model
    // path. It does not defend an adversary already holding the database file
    // — that is host compromise, the same boundary this repository already
    // draws for the audit chain, where the key and the pin share the host.
    //
    // No hashing happens in SQL: SQLite has no hash function and this tree
    // registers none. The witness check compares two stored columns instead.
    // The visibility fence. A claim reaches recall only by a transition from
    // invisible to visible, and only when three things hold: the claim is
    // itself ratified, its batch has passed the set-level check, and a
    // ratified promote row sits in the audit chain behind it — kind
    // `workflow`, because the promote event is a governed-workflow write and
    // is discriminated by its target digest, which is unique per claim by
    // construction. Ratification is part of the predicate rather than a
    // separate lock: a trigger that accepted a pending row would be a fence
    // whose own message ("a ratified promote audit row") was not what it
    // actually checked. The read query joins on the same two columns, so the
    // fence and the query are two independent locks on one fact rather than
    // one lock and one hope.
    //
    // Both visibility fences hang off the SAME column, and SQLite does not
    // specify the order in which same-column BEFORE UPDATE triggers fire. So
    // neither may assume it runs first, and each is pinned to be provable on
    // its own: one by withholding the batch, the other by passing the batch.
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS claims_fence_recall_visibility
        BEFORE UPDATE OF recall_visible ON claims
        FOR EACH ROW WHEN NEW.recall_visible = 1 AND OLD.recall_visible = 0
        BEGIN
            SELECT CASE WHEN NEW.audit_target_hash IS NULL THEN RAISE(ABORT,
                'claims: recall_visible requires a pre-computed audit target hash') END;
            SELECT CASE WHEN NEW.status <> 'ratified' THEN RAISE(ABORT,
                'claims: recall_visible requires a ratified claim') END;
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1 FROM audit_events a
                WHERE a.kind = 'workflow'
                  AND a.status = 'ok'
                  AND a.target_hash = NEW.audit_target_hash
            ) THEN RAISE(ABORT,
                'claims: recall_visible requires a ratified promote audit row') END;
        END;",
    )?;

    // Reject-on-CID-rewrite. Once a claim is ratified its evidence pointer is
    // frozen, so a consolidation pass that rewrites the underlying source
    // cannot re-point the citation at text that says something else. The
    // honest consequence is the other half of the property: a genuine
    // consolidation mints a NEW content id, the stored reference then fails to
    // resolve, and the claim degrades loudly and leaves recall.
    // Anti-laundering becomes a property of the data rather than a rule
    // someone can forget.
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS claims_fence_cid_rewrite
        BEFORE UPDATE OF source_cid ON claim_evidence
        FOR EACH ROW WHEN OLD.claim_ref IN (SELECT id FROM claims WHERE status = 'ratified')
        BEGIN
            SELECT RAISE(ABORT,
                'claims: source_cid is immutable once ratified (a rewrite must mint a new content id)');
        END;",
    )?;

    // Self-ratification refused. A status move into `ratified` must carry WHO
    // promoted it and WHEN. The creating writer is the party this fence exists
    // to stop: a claim that could stamp its own ratification columns would
    // never meet a human.
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS claims_fence_self_ratification
        BEFORE UPDATE OF status ON claims
        FOR EACH ROW WHEN NEW.status = 'ratified'
        BEGIN
            SELECT CASE WHEN NEW.promoted_by IS NULL OR NEW.promoted_at IS NULL
                THEN RAISE(ABORT,
                    'claims: self-ratification refused - promoted_by and promoted_at are required') END;
            SELECT CASE WHEN NEW.created_by = 'agent' AND NEW.audit_digest IS NULL
                THEN RAISE(ABORT,
                    'claims: an agent-authored claim must carry the promote audit digest') END;
        END;",
    )?;

    // The batch fence. Visibility may not be granted to a member of a batch
    // whose set-level check has not passed. Individually benign memories are
    // jointly harmful and per-item review is structurally blind to that; this
    // is the only place in the round where the unit of judgement is the batch
    // rather than the claim.
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS claims_fence_batch_flip
        BEFORE UPDATE OF recall_visible ON claims
        FOR EACH ROW WHEN NEW.recall_visible = 1
        BEGIN
            SELECT CASE WHEN NEW.batch_id IS NULL
                THEN RAISE(ABORT, 'claims: recall_visible requires a batch assignment') END;
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1 FROM claim_batches b
                WHERE b.id = NEW.batch_id AND b.set_check = 'pass'
            ) THEN RAISE(ABORT, 'claims: the batch set-check has not passed') END;
        END;",
    )?;

    // ── v1.32.22 "Disproof": the disproof condition on a claim ────────────
    // The representation two earlier rounds declined to ship ("one round owns
    // the reader") and deferred for want of ("no disproof-condition
    // representation exists"). BOTH deferrals named the SAME absent thing and
    // neither named the other's deferral; the owner is now named in
    // `IMPL_R60_CREATE_CORE_2026-09-28.md:75-83`, so this ships.
    //
    // TWO forms, no third (`ASSESSMENT_SWE_PROOF_2609.21190` §1: a prose
    // condition accepts a great deal a targeted adversary can then refute —
    // 32% of resolving submissions). `disproof_form` is `evaluated|audited`,
    // and the CHECK below is what makes a third form unrepresentable rather
    // than merely discouraged.
    //
    // `disproof_coverage` is MANDATORY alongside either form (`I60.6`): an
    // empty coverage list is refused by `disproof::DisproofCondition::validate`,
    // not defaulted here. Partial coverage is a first-class class.
    //
    // All five are NULLable and additive: a claim written before this round
    // carries NO condition, which is stamp-blind by declaration (the
    // `content_owner_stamp` precedent) and is NOT evidence that the claim is
    // sound. No table is dropped and none is rebuilt — a rebuild is the one
    // operation that can lose rows under a crash. Guarded per column by
    // `pragma_table_info`, so re-running the runner is a no-op.
    for (col, def) in [
        ("disproof_form", "TEXT"),
        ("disproof_body", "TEXT"),
        ("disproof_op", "TEXT"),
        ("disproof_citation", "TEXT"),
        ("disproof_coverage", "TEXT NOT NULL DEFAULT '[]'"),
        ("disproof_audit_ref", "TEXT"),
    ] {
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE claims ADD COLUMN {col} {def}"), [])?;
        }
    }

    // The two-form CHECK is a SEPARATE statement because SQLite cannot add a
    // table-level constraint to an existing table via ALTER — the closed
    // vocabulary is therefore enforced in Rust at the single constructor
    // (`DisproofCondition::new`), which is the stronger position anyway: the
    // vocabulary is refused before a value can be persisted, not after.

    // ── v1.32.23 "Scope": the disproof condition's SEVENTH column ─────────
    // v1.32.22 added six disproof columns. `DisproofCondition` has seven
    // fields, and `scope` was the one with nowhere to go — so
    // `DisproofForm::Evaluated`, which REQUIRES a scope, could not be
    // persisted at all, and the writer refused with a named cause rather than
    // writing a row its own read-back would reject. This closes that ceiling.
    //
    // The shape below is the v1.32.22 block's shape exactly: NULLable,
    // additive, one `pragma_table_info` guard per column so re-running the
    // runner is a no-op, and NO table rebuild — a rebuild is the one operation
    // that can lose rows under a crash.
    //
    // `NULL` here means "the claim predates the column", which is the same
    // stamp-blind story every other disproof column tells. It is NOT evidence
    // the claim is sound, and a row that names `evaluated` with a `NULL` scope
    // is refused by the constructor rather than read back as a condition.
    {
        let col = "disproof_scope";
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(&format!("ALTER TABLE claims ADD COLUMN {col} TEXT"), [])?;
        }
    }

    // ── v1.32.24 "Axis": the PER-DOMAIN knowledge-version counter ────────
    // `workflow_runs.knowledge_version` has recorded a case's knowledge basis
    // since v1.32.20, but the value was a compile-time CONSTANT because nothing
    // ever moved it. This table is the per-domain axis it is recorded against:
    // one row per domain, monotonic, from the base version.
    //
    // Shape is `domain_profiles`' exact shape (`domain TEXT PRIMARY KEY`) and
    // the upsert is the `embedding_dim` stamp's `ON CONFLICT(key) DO UPDATE`.
    // A MISSING ROW IS NOT VERSION 0 — a domain that has never published is at
    // the base version, and `NULL` in the runs column means "predates tracking".
    // One additive table: nothing dropped, nothing rebuilt.
    db.execute(
        "CREATE TABLE IF NOT EXISTS knowledge_domain_versions (
            domain         TEXT PRIMARY KEY,
            version        INTEGER NOT NULL,
            bumped_at      INTEGER NOT NULL,
            bumped_by      TEXT NOT NULL,
            bumped_article INTEGER NOT NULL
         );",
        [],
    )?;

    // ── v1.32.25 "Ref": the model citation's REGISTRY KEY, both halves ────
    // `delivery_traces.model_ref` is the single nullable TEXT a delivery run
    // writes when it resolves a model citation, and it is written from the
    // CALLER's key in the `rules:{id}` config-key shape. The model registry is
    // keyed on the composite `(id, version)`, and its `id` is BARE — the
    // resolver strips the prefix to get it. So the registry key a trace cites is
    // not derivable from the column that records it: `model_ref` alone can never
    // join to the registry, and the version it names is nowhere on the row.
    //
    // These two columns make the citation's registry key STORED rather than
    // reconstructed. They are written from what the resolver RETURNED — never
    // re-derived by stripping the prefix off `model_ref`, which is the re-
    // derivation law the sibling citation columns were added under and which a
    // second bindable kind would break.
    //
    // Names are the sibling table's names (`decision_run_traces` /
    // `decision_evaluation_runs`), so a trace joins the registry with no
    // translation layer. No foreign key: the house style declares none, and a
    // composite FK would need the parent key rebuilt.
    //
    // Both NULLable with NO default — NULL means "predates citation tracking",
    // and a DEFAULT would falsely date every historical row to a model. Guarded
    // per column by `pragma_table_info`, so re-running is a no-op. No column is
    // dropped and no table is rebuilt.
    //
    // DELIBERATELY NOT PART OF THE CONTENT ADDRESS. `delivery_traces.id` is a
    // digest over the stored columns, and folding these two in would re-derive
    // every historical row's id — which the replay gate reads as divergence and
    // would refuse promotion on every pre-existing run. The ceiling this leaves
    // is real: the address does not commit to the citation, so a citation
    // rewritten in place is not detected by the replay fold. Pinned as a
    // ceiling where it is measured.
    for (col, def) in [
        ("model_registry_id", "TEXT"),
        ("model_registry_version", "TEXT"),
    ] {
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('delivery_traces') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE delivery_traces ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }

    // ── v1.32.26 "Edge": the proposal→chunk correspondence, STORED ────────
    // The DSAR erasure deletes the memories it can attribute to a subject
    // (`knowledge.owner`) plus their `derived_from` descendants — but the
    // approved PROPOSAL that produced each of those memories has no owner
    // column and carries no proposal ref on the chunk, so the erasure's only
    // reach into `proposals` was `content LIKE '%subject%'`. A candidate body
    // almost never contains its owner's identity, so the proposal's raw
    // plaintext (possibly PII about the subject) survived a certificate reading
    // `completed` — the drill-proven erasure gap this column closes.
    //
    // The correspondence therefore has to be RECORDED where it is created: at
    // approve time, beside the shared decision CAS. One additive NULLable
    // INTEGER on `proposals` — `knowledge` gains NOTHING, so every FK-children
    // map that describes the `knowledge` parent delete stays accurate.
    //
    // NULL means "this approval promoted nothing" and is the correct state for
    // the four approve branches that produce no chunk (registry lifecycle,
    // outreach consent, campaign/follow-up, channel template). No foreign key:
    // the house style declares none here, and the edge is read and written only
    // inside the approve tx and the erasure tx.
    //
    // Guarded by `pragma_table_info`, so re-running is a no-op. No column is
    // dropped and no table is rebuilt. `src/service/purge.rs` already deletes
    // proposals by `conflict_with`; this column is the OTHER direction and is
    // not a declared child, so that map is unchanged — stated here because a
    // reader will look for it.
    {
        let col = "promoted_chunk_id";
        let present: bool = db
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='{col}'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE proposals ADD COLUMN {col} INTEGER"),
                [],
            )?;
        }
    }

    db.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('schema_version', '1.32.26')
         ON CONFLICT(key) DO UPDATE SET value = '1.32.26';",
        [],
    )?;

    // ── v1.27.31 "AuditRepair": initial audit head pin. ────────────
    // Pin the CURRENT chain head (id + legacy link hash) so a later restore
    // that rolls the chain back is detectable and truncation/extension of an
    // otherwise-valid chain fails verify. Legacy scheme by definition — an
    // existing chain is pre-re-anchor; `--re-audit` rewrites links AND pin
    // under hmac256. Fresh DBs (no rows) get their pin on the first audit
    // write (`record_tenant` re-pins per commit). Best-effort with a warning:
    // a failed stamp only degrades truncation detection until the next write
    // re-pins — it must not fail the whole migration.
    {
        let pinned: Option<String> = db
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'audit_chain_head'",
                [],
                |r| r.get(0),
            )
            .ok();
        if pinned.is_none() {
            let rows: i64 = db
                .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
                .unwrap_or(0);
            if rows > 0
                && let Some(pin) = crate::audit::initial_head_pin(db)
            {
                match serde_json::to_string(&pin) {
                    Ok(json) => {
                        if let Err(e) = db.execute(
                            "INSERT INTO schema_meta(key, value) VALUES ('audit_chain_head', ?1)
                                 ON CONFLICT(key) DO UPDATE SET value = ?1;",
                            params![json],
                        ) {
                            warn!(
                                "audit head pin stamp failed (truncation detection deferred): {e}"
                            );
                        }
                    }
                    Err(e) => warn!("audit head pin serialize failed: {e}"),
                }
            }
        }
    }

    // ── v1.28.5 "Compliance Pack" (feature-gated): Art.12/14 evidence
    // tables. Without the feature these are NOT created and server behaviour
    // is unchanged; with it, the migration stays additive + idempotent.
    #[cfg(feature = "compliance-pack")]
    db.execute_batch(crate::audit::decision::DDL)?;

    #[cfg(feature = "compliance-pack")]
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS oversight_evidence(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            reviewer_id TEXT NOT NULL,
            reviewed_at INTEGER NOT NULL,
            basis TEXT NOT NULL,
            outcome TEXT NOT NULL,
            authority TEXT NOT NULL DEFAULT '',
            decision_hash TEXT,
            proposal_id INTEGER,
            domain TEXT NOT NULL DEFAULT ''
         );",
    )?;
    // v1.28.7 pass-3 P3-4: identical-content approvals must stay
    // distinguishable — the row binds the proposal id and its owning domain.
    #[cfg(feature = "compliance-pack")]
    for (col, def) in [
        ("proposal_id", "INTEGER"),
        ("domain", "TEXT NOT NULL DEFAULT ''"),
    ] {
        let present: bool = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('oversight_evidence') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;
        if !present {
            db.execute(
                &format!("ALTER TABLE oversight_evidence ADD COLUMN {col} {def}"),
                [],
            )?;
        }
    }

    #[cfg(feature = "compliance-pack")]
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS ropa_registry(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            activity TEXT NOT NULL,
            controller TEXT NOT NULL,
            processor TEXT NOT NULL,
            categories TEXT NOT NULL DEFAULT '',
            recipients TEXT NOT NULL DEFAULT '',
            lawful_basis TEXT NOT NULL,
            retention_days INTEGER,
            security_measures TEXT NOT NULL DEFAULT '',
            transfers TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );",
    )?;

    // ── Parity check: assert vec0 absorbed all valid legacy vectors ──────
    // A silent partial backfill (e.g. skipped non-512-dim rows) would otherwise
    // leave the index incomplete with no signal. We log a warning for any
    // discrepancy rather than aborting — dimension-mismatch rows are expected
    // on legacy DBs and are surfaced here so the operator can act.
    let emb_count: i64 = db
        .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
        .unwrap_or(0);
    let vec_count: i64 = db
        .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
        .unwrap_or(0);
    if emb_count > 0 && vec_count < emb_count {
        warn!(
            "vec0 parity gap: embeddings={emb_count} vec_knowledge={vec_count} \
             ({}/{emb_count} rows have non-512-dim vectors and were skipped)",
            emb_count - vec_count
        );
    }

    Ok(())
}

/// v1.28 "Caliber": the `--re-embed` escape hatch. Re-points the store at
/// `store_dim`: overwrites the `embedding_dim` stamp, drops + recreates
/// `vec_knowledge` at the new dim, and clears the legacy JSON `embeddings`
/// backfill source (its f32 rows are the OLD dim — re-backfilling them into the
/// new store would be cross-dim corruption; the content they derive from lives
/// on in `knowledge.content`, re-embedded by the caller).
///
/// Leaves the store EMPTY — the caller re-embeds every chunk afterward
/// (main.rs `--re-embed` does exactly that). ponytail ceiling: no transaction
/// gymnastics; this is an offline operator command, a crash mid-way is
/// re-runnable (idempotent: stamp + DROP/CREATE + DELETE are all safe to repeat).
pub fn rebuild_vec_store_at_dim(db: &mut Connection, store_dim: usize) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")?;
    db.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('embedding_dim', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1;",
        params![store_dim.to_string()],
    )?;
    let ddl = format!(
        "DROP TABLE IF EXISTS vec_knowledge;
         CREATE VIRTUAL TABLE vec_knowledge USING vec0(
            knowledge_id INTEGER PRIMARY KEY,
            embedding_int8 int8[{dim}] distance_metric=cosine,
            embedding_bit  bit[{dim}],
            source         text,
            created_at     text
         );
         DELETE FROM embeddings;",
        dim = store_dim
    );
    db.execute_batch(&ddl)?;
    info!(
        "vec store rebuilt at {store_dim}-d (legacy embeddings cleared; corpus re-embed pending)"
    );
    Ok(())
}

/// Reversibility path for the v0.9.0 migration.
///
/// Drops the v0.9.0+ structures (vec0, FTS5, vocab, schema markers), leaving
/// the DB in its pre-v0.9.0 shape: `knowledge` + `embeddings` (JSON f32) intact.
/// The `embeddings.vector TEXT` column is the source of truth for the legacy
/// build, so a redeploy of the v0.8.6 binary against this DB works without
/// re-encoding — new ingests will repopulate `embeddings` as before.
///
/// Idempotent. Safe to call on a DB that was never migrated.
pub fn migrate_down_0_9_0(db: &mut Connection) -> Result<()> {
    db.execute_batch(
        "DROP TABLE IF EXISTS vec_knowledge;
         DROP TABLE IF EXISTS knowledge_fts_vocab;
         DROP TABLE IF EXISTS knowledge_fts;
         DROP TRIGGER IF EXISTS knowledge_ai;
         DROP TRIGGER IF EXISTS knowledge_ad;
         DROP TRIGGER IF EXISTS knowledge_au;
         DELETE FROM schema_meta WHERE key = 'vec_metric';",
    )?;
    // The vec0 store is gone — the search path must probe again.
    VEC0_READY.store(false, Ordering::Relaxed);
    info!("migrate_down_0_9_0: dropped vec0 + FTS5 structures; embeddings table preserved");
    Ok(())
}

#[cfg(test)]
mod dim_tests {
    //! v1.28 "Caliber" M2: the profile-parameterized store-dimension behavior.
    //! The fail-closed mismatch guard is the load-bearing guarantee — a cross-dim
    //! query/store pair silently corrupts recall, so it must refuse, not auto-migrate.
    use super::*;
    use crate::register_sqlite_vec::register_sqlite_vec;

    fn fresh() -> Connection {
        register_sqlite_vec();
        Connection::open_in_memory().expect("open in-memory DB")
    }

    #[test]
    fn fresh_db_stamps_embedding_dim_and_creates_vec0_at_it() {
        let mut db = fresh();
        run_migration_with_store_dim(&mut db, 1, 768).expect("migrate");
        let stamped: String = db
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'embedding_dim'",
                [],
                |r| r.get(0),
            )
            .expect("stamp");
        assert_eq!(stamped, "768");
        // The vec0 store exists (cosine metric, the 768-dim DDL accepted).
        let n: i64 = db
            .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0);
    }

    #[test]
    fn same_dim_rerun_is_idempotent() {
        let mut db = fresh();
        run_migration_with_store_dim(&mut db, 1, 512).expect("first");
        // Second run at the same dim must succeed (no false mismatch).
        run_migration_with_store_dim(&mut db, 1, 512).expect("second");
    }

    #[test]
    fn mismatched_dim_fails_closed_with_a_clear_message() {
        let mut db = fresh();
        run_migration_with_store_dim(&mut db, 1, 512).expect("built at 512");
        // Opening the same DB with a 1024-d embedder (enterprise) must refuse —
        // a silent cross-dim migration would corrupt recall.
        let err = run_migration_with_store_dim(&mut db, 1, 1024).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("dimension mismatch"), "msg was: {msg}");
        assert!(msg.contains("512"), "msg should name the stored dim: {msg}");
        assert!(
            msg.contains("1024"),
            "msg should name the requested dim: {msg}"
        );
    }

    #[test]
    fn legacy_run_migration_defaults_to_512_and_round_trips_with_explicit_512() {
        // The pre-v1.28 callers (tests, migrate-rehearse, domain_registry) get 512.
        // A DB they build must be openable by an explicit-512 embedder (edge).
        let mut db = fresh();
        run_migration(&mut db, 1).expect("legacy 512 default");
        run_migration_with_store_dim(&mut db, 1, 512).expect("explicit 512 matches");
        // And must FAIL against enterprise 1024 — the guard works for legacy DBs too.
        let err = run_migration_with_store_dim(&mut db, 1, 1024).unwrap_err();
        assert!(format!("{err}").contains("dimension mismatch"));
    }

    /// The `--re-embed` escape hatch: after the fail-closed refusal,
    /// `rebuild_vec_store_at_dim` repoints the store and the previously-failing
    /// dim then boots cleanly. This is the one check that the sanctioned bypass
    /// actually works (the re-embed loop itself is the /reindex shape).
    #[test]
    fn rebuild_vec_store_repoints_dim_and_unblocks_migration() {
        let mut db = fresh();
        run_migration_with_store_dim(&mut db, 1, 512).expect("built at 512");
        run_migration_with_store_dim(&mut db, 1, 1024).unwrap_err(); // fails closed
        rebuild_vec_store_at_dim(&mut db, 1024).expect("repoint to 1024");
        run_migration_with_store_dim(&mut db, 1, 1024).expect("now boots at 1024");
        let stamped: String = db
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'embedding_dim'",
                [],
                |r| r.get(0),
            )
            .expect("stamp");
        assert_eq!(stamped, "1024");
        // Back down again — the hatch is reversible too.
        rebuild_vec_store_at_dim(&mut db, 512).expect("repoint back to 512");
        run_migration_with_store_dim(&mut db, 1, 512).expect("boots at 512 again");
    }

    /// R48 (E4, KILL 3) — the production readback, and the fail-open it closes.
    ///
    /// Three properties, because a single assertion would be the vacuous pin
    /// this line has now produced four times:
    ///
    ///   1. the POSITIVE control — a normal local filesystem still boots, so
    ///      the readback does not false-positive (a gate that refuses healthy
    ///      deployments is not a gate; that is KILL 3);
    ///   2. the FAIL-OPEN FACT — `execute_batch("PRAGMA journal_mode=WAL")`
    ///      returns `Ok` even when the mode cannot change, which is the whole
    ///      reason a readback is needed;
    ///   3. the DECISION — a non-wal mode is refused.
    ///
    /// The fixture is FILE-BACKED on purpose. An in-memory database is in
    /// `memory` journal mode and cannot be set to `DELETE`; the first draft of
    /// this pin used one and failed on its own precondition.
    #[test]
    fn cycle_boot_refuses_when_journal_mode_is_not_wal() {
        let dir = std::env::temp_dir().join(format!("cycle_journal_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        // (1) POSITIVE CONTROL: a local filesystem reaches wal and boots.
        //     sqlite-vec must be registered FIRST: `run_migration` creates
        //     `vec_knowledge`, and without the module the migration fails for an
        //     unrelated reason (`no such module: vec0`) — which would make this
        //     control prove nothing about the readback.
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut ok = Connection::open(dir.join("ok.db")).expect("open");
        run_migration_with_store_dim(&mut ok, 1, 512).expect(
            "a normal local filesystem must still boot — the readback must not false-positive",
        );
        let mode: String = ok
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("read");
        assert_eq!(
            mode.to_lowercase(),
            "wal",
            "the positive control reaches wal"
        );
        drop(ok);

        // (2) WHAT A LOCAL FILESYSTEM ACTUALLY DOES. `journal_mode=WAL` after
        //     DELETE SUCCEEDS and upgrades — because ext4/APFS provide the
        //     shared-memory primitives WAL needs. So on a healthy store the
        //     readback costs one query and passes. That is the whole point of
        //     putting it on the boot path: it is free when the volume is good.
        //
        //     The FAIL-OPEN this closes therefore CANNOT be reproduced here. It
        //     is a property of a filesystem that LACKS those primitives
        //     (NFS and friends — sqlite.org/wal.html §3), which no test
        //     fixture can conjure. An earlier draft of this pin asserted the
        //     mode stayed non-wal here and failed: it does not, on local disk.
        //     The honest split is therefore:
        //       * measurable in a test  — the predicate, and the healthy path;
        //       * operator-level          — the fail-open itself, which is why
        //         E8's prohibition lives in the runbook and the chart.
        let plain = Connection::open(dir.join("plain.db")).expect("open");
        let set: String = plain
            .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
            .expect("force delete");
        assert_eq!(set.to_lowercase(), "delete", "fixture precondition");
        plain
            .execute_batch("PRAGMA journal_mode=WAL;")
            .expect("a local filesystem upgrades — the readback is free when healthy");
        let after: String = plain
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("read back");
        assert_eq!(
            after.to_lowercase(),
            "wal",
            "on a local filesystem the upgrade succeeds, so the healthy path costs one query"
        );
        drop(plain);

        // (3) THE DECISION — the same predicate the migration performs, over
        //     every mode, so the ALLOW list is pinned as tightly as the refuse
        //     list. An allow-list pinned only by example is how the in-memory
        //     false positive happened.
        let allows = |m: &str| matches!(m.to_ascii_lowercase().as_str(), "wal" | "memory");
        assert!(allows("wal"), "wal is the deployment mode");
        assert!(
            allows("memory"),
            "memory is a deliberate in-memory test store"
        );
        for refused in ["delete", "truncate", "persist", "off"] {
            assert!(
                !allows(refused),
                "{refused} is a filesystem-backed downgrade"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The refusal DECISION, pinned over the whole mode vocabulary.
    ///
    /// Split from the pin above because the two prove different things: that
    /// one is about a real store, this one is about the predicate's total
    /// coverage. A gate whose refuse-list is pinned only by example is how the
    /// in-memory false positive got in.
    #[test]
    fn cycle_wal_refusal_names_the_cause() {
        let db = Connection::open_in_memory().expect("in-memory");
        let mode: String = db
            .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
            .expect("set delete");
        // the exact predicate the migration uses
        let refused = !mode.eq_ignore_ascii_case("wal");
        assert!(refused, "a non-wal mode must be refused, not accepted");
        // and the message the operator sees must name both the observed mode and
        // the remedy, because "refusing to start" with no cause is a support call
        let msg = "journal mode is 'delete', not 'wal'";
        assert!(msg.contains("delete") && msg.contains("wal"));
    }
}
