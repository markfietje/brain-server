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
            "CREATE UNIQUE INDEX idx_knowledge_hash ON knowledge(content_hash)",
            [],
        )?;
        println!("MIGRATION: Complete");
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

    // Bumped once per release that changes this function.
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
    db.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('schema_version', '1.28.29')
         ON CONFLICT(key) DO UPDATE SET value = '1.28.29';",
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
}
