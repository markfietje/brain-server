//! Database schema migration (extracted from `main.rs` for v0.9.9 "Qualify" M2).
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
use rusqlite::{params, Connection};
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;
use zerocopy::IntoBytes;

pub fn run_migration(db: &mut Connection, mmap_mib: i64) -> Result<()> {
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

    // v0.9.1: additive domain + temporal columns (P2 domain isolation + temporal
    // memory scaffold) and structure-aware chunk metadata (P1 chunking). Idempotent.
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

    // v0.9.1: per-domain centroids for centroid routing (P2). One row per
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
    // `fts5vocab='instance'` exposes one row per (term, document, column) with
    // a `cnt` (occurrence count). PRF query expansion joins this against the
    // top-K rowids to rank expansion terms by corpus-weighted frequency
    // (BM25-style signal), replacing the naive in-memory DF heuristic.
    //   ponytail: per-instance vocab; for a very large corpus switch to
    //   'row' mode (one row per term+doc). Ceiling: ~corpus-size rows.
    db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts_vocab USING fts5vocab(
            knowledge_fts, 'instance'
         );",
    )?;

    // ── v0.9.0 Phase 1: sqlite-vec vec0 virtual table ────────────────────
    // Replaces the old JSON-text vector storage in `embeddings.vector`.
    //
    // Schema per Context7-verified sqlite-vec docs (July 2026):
    //   embedding_int8  int8[512] distance_metric=cosine — default search tier
    //     (quantized f32→int8). cosine is REQUIRED: vec0 defaults to L2, but the
    //     int8-quantized vectors are not unit-normalized, so an L2 distance is
    //     meaningless for semantic similarity. cosine distance is well-defined
    //     on int8 vectors and yields similarity = 1 - distance in [0,1].
    //   embedding_bit   bit[512]   — archive / first-pass tier (binary quantized)
    //   source          text       — metadata column (enables filtered KNN)
    //   created_at      text       — metadata column (enables temporal filtering)
    db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_knowledge USING vec0(
            knowledge_id INTEGER PRIMARY KEY,
            embedding_int8 int8[512] distance_metric=cosine,
            embedding_bit  bit[512],
            source         text,
            created_at     text
        );",
    )?;

    // ── One-time migration: rebuild vec_knowledge with the cosine metric ──
    // Earlier v0.9.0 builds created vec0 WITHOUT distance_metric=cosine, so the
    // int8 index used the default L2 metric — useless for semantic similarity
    // (yielded flat ~0 scores). Rebuild the table once, then the backfill below
    // repopulates it from the f32 `embeddings` table (the source of truth). The
    // `schema_meta` marker makes this idempotent across restarts.
    db.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")?;
    let needs_rebuild: bool = db
        .query_row(
            "SELECT value IS NULL OR value <> 'cosine' FROM schema_meta WHERE key = 'vec_metric'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n != 0)
        .unwrap_or(true);
    if needs_rebuild {
        info!("Rebuilding vec_knowledge with distance_metric=cosine (one-time fix)");
        db.execute_batch(
            "DROP TABLE IF EXISTS vec_knowledge;
             CREATE VIRTUAL TABLE vec_knowledge USING vec0(
                knowledge_id INTEGER PRIMARY KEY,
                embedding_int8 int8[512] distance_metric=cosine,
                embedding_bit  bit[512],
                source         text,
                created_at     text
             );",
        )?;
        db.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('vec_metric', 'cosine')
             ON CONFLICT(key) DO UPDATE SET value = 'cosine';",
            [],
        )?;
        info!("vec_knowledge rebuilt; backfill will repopulate from embeddings");
    }

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

    // ── v0.9.4: canonical sources + revisions (M1) ─────────────────────
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

    // v1.1.0 Harden M2: per-tenant scoping + tamper-evidence. Additive columns
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

    // ── v0.9.8 "Evidence" M2.2: typed provenance links between chunks ──
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

    // ── v0.9.9 "Qualify" M1.2 / v1.1.0 "Harden": record the schema version so
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

    // ── v1.4.0 "Calibrate" M1: bi-temporal edges (Graphiti model). ───────
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

    // ── v1.4.0 "Calibrate" M3: TRACE hierarchical node reservation. ───────
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
    //   assertion_kind — stated(default)|observed|inferred (M3 provenance).
    //   confidence    — 0..1 deterministic derivation, default 1.0 (M3).
    //   expires_at    — unix ts, NULL = no decay (M2; default off).
    //   pii           — 1 when the ingest-time pattern scanner flagged PII (M4).
    //   owner         — creating principal TEXT, NULL for legacy/loopback (M4).
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

    // Purge audit trail (M2/GDPR). Append-only; keeps the audit chain
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

    // ── v1.14.0 M1 "Write-back gate": the proposal review queue. ─────────
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

    // ── v1.15.0 "Observe" M1/M3: read-event trace + DSAR ledger. ────────
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

    // v1.17.1 "Govern" M2: persisted per-kind retention overrides. The default
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

    // ── v1.20.1 "Shield" M2: proposal provenance from the auto-capture path. ─
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

    // ── v1.20.14 "Steer" M1: edit provenance on pending proposals. ──────────
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

    // Bumped once per release that changes this function.
    db.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('schema_version', '1.20.19')
         ON CONFLICT(key) DO UPDATE SET value = '1.20.19';",
        [],
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

/// Reversibility path for the v0.9.0 migration (plan M4).
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
    info!("migrate_down_0_9_0: dropped vec0 + FTS5 structures; embeddings table preserved");
    Ok(())
}
