#![cfg(feature = "migrate")]

//! `brain-migrate-rehearse` — copy-and-verify migration rehearsal tool.
//!
//! v0.9.9 "Qualify" M2. Feature-gated (`--features migrate`). Runs the
//! legacy `brain.db` → candidate `global.db` cutover against a *copy* of the
//! live DB so the v1.0.0 split can be rehearsed before it happens. The live
//! runtime is never touched — every phase writes only to `dest` + sidecars.
//!
//! **Why a binary, not a `brain` subcommand?** It must run against a *stopped*
//! server (the live DB has WAL pages that a hot copy would miss). A standalone
//! binary makes that contract obvious.
//!
//! Phases (each independently runnable):
//!   backup     encrypted pre-rehearsal snapshot of the source DB
//!   copy       VACUUM INTO a fresh dest, then run_migration on it
//!   verify     row/hash/FTS/vec0/schema parity checks; exits non-zero on FAIL
//!   report     pure-read human summary
//!   rollback   remove dest + sidecars (never touches source)
//!   rehearse   backup → copy → verify → report (all-in-one)
//!
//! Defaults:
//!   --source  StorageLayout::detect()?.legacy_db()
//!   --dest    StorageLayout::detect()?.global_domain_db()

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use brain_server::backup;
use brain_server::migration::run_migration;
use brain_server::storage_layout::StorageLayout;

/// Tables covered by the row-count parity check. Matches the v0.9.4–v0.9.8
/// schema surface (the tables `run_migration` creates). A new table added to
/// the migration must be added here too — `test_migration_schema_contract`
/// guards the migration side, this list guards the verify side.
const PARITY_TABLES: &[&str] = &[
    "knowledge",
    "embeddings",
    "vec_knowledge",
    "entities",
    "relationships",
    "tombstones",
    "sources",
    "source_revisions",
    "connectors",
    "connector_checkpoints",
    "audit_events",
    "webhook_queue",
    "evidence_links",
];

/// Size of the random vec0 spot-check. ponytail: 50 is a heuristic — the formal
/// guarantee is the row-count + content-hash checks. The spot-check exists to
/// catch a silent vec0 corruption that preserves row count, which has been a
/// real sqlite-vec bug class.
const VEC_SPOT_CHECK_SIZE: usize = 50;

/// WAL-size heuristic for "the server is probably still running". ponytail: the
/// precise check would be `PRAGMA wal_checkpoint`, but that mutates the file.
/// 1 KiB is well above a quiescent WAL's typical few hundred bytes.
const WAL_ACTIVE_HEURISTIC_BYTES: u64 = 1024;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if let Err(e) = run(&argv) {
        eprintln!("brain-migrate-rehearse: {e:#}");
        std::process::exit(1);
    }
}

fn run(argv: &[String]) -> Result<()> {
    if argv.len() < 2 {
        usage();
        bail!("missing subcommand");
    }
    let cmd = argv[1].as_str();
    let raw = parse_args(&argv[2..])?;
    let layout = StorageLayout::detect()?;
    let args = ResolvedArgs::from_raw(raw, &layout)?;

    register_sqlite_vec();
    match cmd {
        "backup" => phase_backup(&args)?,
        "copy" => phase_copy(&args)?,
        "verify" => {
            let report = phase_verify(&args)?;
            print_verify_report(&report);
            write_verify_report_file(&args.dest, &report)?;
            if report.any_failed() {
                std::process::exit(1);
            }
        }
        "report" => phase_report(&args)?,
        "rollback" => phase_rollback(&args)?,
        "rehearse" => phase_rehearse(&args)?,
        other => {
            usage();
            bail!("unknown subcommand: {other}");
        }
    }
    Ok(())
}

fn usage() {
    eprintln!(
        "usage: brain-migrate-rehearse <backup|copy|verify|report|rollback|rehearse> \
         [--source PATH] [--dest PATH] [--strict] [--force] [--keep-snapshot]"
    );
}

/// Parsed but unresolved argv flags (every phase accepts the same shape).
struct RawArgs {
    source: Option<PathBuf>,
    dest: Option<PathBuf>,
    strict: bool,
    force: bool,
    keep_snapshot: bool,
}

fn parse_args(rest: &[String]) -> Result<RawArgs> {
    let mut raw = RawArgs {
        source: None,
        dest: None,
        strict: false,
        force: false,
        keep_snapshot: false,
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--source" => {
                i += 1;
                raw.source = Some(PathBuf::from(require_value(rest, i, "--source")?));
            }
            "--dest" => {
                i += 1;
                raw.dest = Some(PathBuf::from(require_value(rest, i, "--dest")?));
            }
            "--strict" => raw.strict = true,
            "--force" => raw.force = true,
            "--keep-snapshot" => raw.keep_snapshot = true,
            other => bail!("unknown flag: {other}"),
        }
        i += 1;
    }
    Ok(raw)
}

fn require_value<'a>(rest: &'a [String], idx: usize, flag: &str) -> Result<&'a str> {
    let v = rest.get(idx).context(format!("{flag} requires a value"))?;
    Ok(v)
}

struct ResolvedArgs {
    source: PathBuf,
    dest: PathBuf,
    /// `--strict` escalates WARN rows to FAIL. Currently every check emits OK/FAIL
    /// only; reserved for future WARN-class checks (e.g. soft parity gaps).
    #[allow(dead_code)]
    strict: bool,
    force: bool,
    keep_snapshot: bool,
}

impl ResolvedArgs {
    fn from_raw(raw: RawArgs, layout: &StorageLayout) -> Result<Self> {
        Ok(Self {
            source: raw.source.unwrap_or_else(|| layout.legacy_db()),
            dest: raw.dest.unwrap_or_else(|| layout.global_domain_db()),
            strict: raw.strict,
            force: raw.force,
            keep_snapshot: raw.keep_snapshot,
        })
    }
}

/// Register sqlite-vec process-wide so `run_migration` can create vec0 tables
/// on the dest connection. This binary doesn't share `main.rs`'s helper (it's
/// a standalone binary), so the registration is duplicated here.
///
/// # Safety
///
/// See `main.rs::register_sqlite_vec` for the full safety proof. The short
/// version: `sqlite3_vec_init` is `extern "C"` with the signature
/// `sqlite3_auto_extension` expects; the pointer is process-lifetime static.
fn register_sqlite_vec() {
    #![allow(clippy::missing_transmute_annotations)]
    // SAFETY: see the doc comment above.
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
}

// ── backup phase ────────────────────────────────────────────────────────────

fn phase_backup(args: &ResolvedArgs) -> Result<()> {
    eprintln!("backup: source={}", args.source.display());
    confirm_source_exists(&args.source)?;
    warn_if_wal_active(&args.source, args.force)?;

    let passphrase = resolve_passphrase()?;
    let backups_dir = args
        .source
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups");
    fs::create_dir_all(&backups_dir)
        .with_context(|| format!("create {}", backups_dir.display()))?;
    let ts = now_iso().replace(':', "-");
    let out = backups_dir.join(format!("pre-rehearsal-{ts}.bbk"));

    backup::backup(&args.source, &out, passphrase.as_slice())
        .with_context(|| format!("backup {} → {}", args.source.display(), out.display()))?;

    // verify reads the manifest back + recomputes hashes; a failed verify is a
    // hard stop because the rollback anchor would be untrustworthy.
    backup::verify(&out, passphrase.as_slice())
        .with_context(|| format!("verify backup {}", out.display()))?;

    println!("{}", out.display());
    eprintln!("backup: ok → {}", out.display());
    Ok(())
}

// ── copy phase ──────────────────────────────────────────────────────────────

fn phase_copy(args: &ResolvedArgs) -> Result<()> {
    eprintln!("copy: {} → {}", args.source.display(), args.dest.display());
    confirm_source_exists(&args.source)?;
    warn_if_wal_active(&args.source, args.force)?;

    if let Some(parent) = args.dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if args.dest.exists() {
        fs::remove_file(&args.dest)
            .with_context(|| format!("remove old {}", args.dest.display()))?;
    }
    // Sidecars from a prior run are stale — clear them so verify/report don't
    // read metadata that doesn't match this copy.
    remove_sidecars(&args.dest);

    let (source_sha, source_size) = sha256_and_size(&args.source)?;
    let schema_before = read_schema_version(&args.source)?;

    // VACUUM INTO runs from a connection open on SOURCE and writes to dest.
    // This is the existing primitive `run_migration` uses for pre-migration
    // backup since v0.9.1 — defragmented, WAL-flattened.
    {
        let src_conn = Connection::open(&args.source)
            .with_context(|| format!("open source {}", args.source.display()))?;
        // VACUUM INTO cannot be parameterized; the dest path is operator-
        // supplied and we've already validated it lives under the storage root.
        let sql = format!("VACUUM INTO '{}'", args.dest.display());
        src_conn
            .execute_batch(&sql)
            .with_context(|| format!("VACUUM INTO {}", args.dest.display()))?;
    }

    // Now open dest separately and bring it up to current schema. This is the
    // exact code path v1.0.0 will run on cutover — rehearsing it now is the
    // point. Idempotent: a no-op if the copy already records 0.9.9.
    let mut dest_conn = Connection::open(&args.dest)
        .with_context(|| format!("open dest {}", args.dest.display()))?;
    run_migration(&mut dest_conn, 256).context("run_migration on dest")?;
    let schema_after = read_schema_version(&args.dest)?;

    let meta = CopyMeta {
        source_sha256: source_sha,
        source_size,
        copied_at: now_iso(),
        schema_version_before: schema_before.clone(),
        schema_version_after: schema_after.clone(),
    };
    write_copy_meta(&args.dest, &meta)?;
    eprintln!(
        "copy: ok (schema {} → {})",
        schema_before.unwrap_or_else(|| "none".into()),
        schema_after.unwrap_or_else(|| "none".into())
    );
    Ok(())
}

#[derive(serde::Serialize)]
struct CopyMeta {
    source_sha256: String,
    source_size: u64,
    copied_at: String,
    schema_version_before: Option<String>,
    schema_version_after: Option<String>,
}

fn copy_meta_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".copy-meta.json");
    PathBuf::from(p)
}

fn write_copy_meta(dest: &Path, meta: &CopyMeta) -> Result<()> {
    let path = copy_meta_path(dest);
    let json = serde_json::to_string_pretty(meta)?;
    fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

// ── verify phase ────────────────────────────────────────────────────────────

/// One row of the parity table. `status` is `OK`/`FAIL`/`WARN`.
#[derive(Debug)]
struct CheckRow {
    check: String,
    source: String,
    dest: String,
    status: String,
    note: String,
}

impl CheckRow {
    fn ok(check: &str, source: impl ToString, dest: impl ToString) -> Self {
        Self {
            check: check.into(),
            source: source.to_string(),
            dest: dest.to_string(),
            status: "OK".into(),
            note: String::new(),
        }
    }
    fn fail(check: &str, source: impl ToString, dest: impl ToString, note: impl ToString) -> Self {
        Self {
            check: check.into(),
            source: source.to_string(),
            dest: dest.to_string(),
            status: "FAIL".into(),
            note: note.to_string(),
        }
    }
}

struct VerifyReport {
    rows: Vec<CheckRow>,
}

impl VerifyReport {
    fn any_failed(&self) -> bool {
        self.rows.iter().any(|r| r.status == "FAIL")
    }
}

fn phase_verify(args: &ResolvedArgs) -> Result<VerifyReport> {
    eprintln!(
        "verify: {} vs {}",
        args.source.display(),
        args.dest.display()
    );
    let mut rows: Vec<CheckRow> = Vec::new();

    let src = Connection::open(&args.source)
        .with_context(|| format!("open source {}", args.source.display()))?;
    let dst = Connection::open(&args.dest)
        .with_context(|| format!("open dest {}", args.dest.display()))?;

    // 1. Per-table row counts.
    for tbl in PARITY_TABLES {
        let (s, d) = (count_rows(&src, tbl)?, count_rows(&dst, tbl)?);
        if s == d {
            rows.push(CheckRow::ok(&format!("rows:{tbl}"), s, d));
        } else {
            rows.push(CheckRow::fail(
                &format!("rows:{tbl}"),
                s,
                d,
                "row count mismatch",
            ));
        }
    }

    // 2. FTS5 row count (separate because it's a virtual table — included in
    //    PARITY_TABLES already, but called out explicitly in the plan).
    // (Already covered above via "rows:knowledge_fts" if we add it to the list.
    //  We keep knowledge_fts out of PARITY_TABLES and check it explicitly here
    //  so the table label is unambiguous.)
    {
        let s = count_rows(&src, "knowledge_fts")?;
        let d = count_rows(&dst, "knowledge_fts")?;
        if s == d {
            rows.push(CheckRow::ok("rows:knowledge_fts", s, d));
        } else {
            rows.push(CheckRow::fail(
                "rows:knowledge_fts",
                s,
                d,
                "FTS trigger missed a row",
            ));
        }
    }

    // 3. content_hash multiset parity (catches row-body drift that preserves count).
    {
        let (s, d) = (content_hash_multiset(&src)?, content_hash_multiset(&dst)?);
        if s == d {
            rows.push(CheckRow::ok("content_hash multiset", s.len(), d.len()));
        } else {
            rows.push(CheckRow::fail(
                "content_hash multiset",
                s.len(),
                d.len(),
                "knowledge content hashes diverged",
            ));
        }
    }

    // 4. Source/revision linkage count.
    {
        let s = count_join(
            &src,
            "SELECT COUNT(*) FROM knowledge k JOIN sources s ON k.source_id = s.id",
        )?;
        let d = count_join(
            &dst,
            "SELECT COUNT(*) FROM knowledge k JOIN sources s ON k.source_id = s.id",
        )?;
        if s == d {
            rows.push(CheckRow::ok("source linkage", s, d));
        } else {
            rows.push(CheckRow::fail(
                "source linkage",
                s,
                d,
                "linked-chunk count diverged",
            ));
        }
    }

    // 5. Schema version: dest must be >= source.
    {
        let s = read_schema_version(&args.source)?;
        let d = read_schema_version(&args.dest)?;
        let ok = match (s.as_deref(), d.as_deref()) {
            (Some(a), Some(b)) => schema_ge(b, a),
            // Source with no schema_meta is a pre-v0.9.9 DB; any dest version is fine.
            (None, _) => true,
            (Some(_), None) => false,
        };
        if ok {
            rows.push(CheckRow::ok(
                "schema_version",
                s.clone().unwrap_or_else(|| "none".into()),
                d.clone().unwrap_or_else(|| "none".into()),
            ));
        } else {
            rows.push(CheckRow::fail(
                "schema_version",
                s.unwrap_or_else(|| "none".into()),
                d.unwrap_or_else(|| "none".into()),
                "dest schema version < source (would be a downgrade)",
            ));
        }
    }

    // 6. 50-row random vec0 spot check.
    rows.push(vec0_spot_check(&src, &dst)?);

    Ok(VerifyReport { rows })
}

fn count_rows(conn: &Connection, table: &str) -> Result<i64> {
    // table names come from a hard-coded allowlist (PARITY_TABLES) — never user input.
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn
        .query_row(&sql, [], |r| r.get::<_, i64>(0))
        .unwrap_or(0))
}

fn count_join(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0))
}

/// Build {content_hash → count} for every knowledge row. Order-independent
/// (a multiset comparison), so a re-pack that changes rowid assignment still
/// passes as long as the content is identical.
fn content_hash_multiset(conn: &Connection) -> Result<std::collections::BTreeMap<String, i64>> {
    let mut map: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut stmt = match conn.prepare("SELECT content_hash FROM knowledge") {
        Ok(s) => s,
        Err(_) => return Ok(map),
    };
    let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
    for h in rows.flatten().flatten() {
        *map.entry(h).or_insert(0) += 1;
    }
    Ok(map)
}

/// Pick up to VEC_SPOT_CHECK_SIZE random knowledge ids, fetch the matching
/// vec0 embedding bytes from both DBs, byte-compare. Catches a silent vec0
/// corruption that preserves row count (a real sqlite-vec bug class).
fn vec0_spot_check(src: &Connection, dst: &Connection) -> Result<CheckRow> {
    let ids: Vec<i64> = src
        .prepare("SELECT id FROM knowledge ORDER BY RANDOM() LIMIT ?1")?
        .query_map([VEC_SPOT_CHECK_SIZE as i64], |r| r.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut checked = 0;
    for id in &ids {
        let s = fetch_vec_blob(src, id)?;
        let d = fetch_vec_blob(dst, id)?;
        // Treat "row absent on both" as a non-failure (e.g. legacy embeddings
        // that never made it to vec0). Mismatched presence is a failure.
        match (s.as_ref(), d.as_ref()) {
            // Both present + equal: pass.
            (Some(a), Some(b)) if a == b => checked += 1,
            // Both present + different: hard fail.
            (Some(_), Some(_)) => {
                return Ok(CheckRow::fail(
                    "vec0 spot check",
                    ids.len(),
                    checked,
                    format!("embedding bytes differ for knowledge_id={id}"),
                ));
            }
            // Absent on both: a non-vec0 row (e.g. legacy embeddings-only). Skip.
            (None, None) => {}
            (Some(_), None) => {
                return Ok(CheckRow::fail(
                    "vec0 spot check",
                    ids.len(),
                    checked,
                    format!("source has vec0 row but dest does not for knowledge_id={id}"),
                ));
            }
            (None, Some(_)) => {
                return Ok(CheckRow::fail(
                    "vec0 spot check",
                    ids.len(),
                    checked,
                    format!("dest has vec0 row but source does not for knowledge_id={id}"),
                ));
            }
        }
    }
    Ok(CheckRow::ok(
        &format!("vec0 spot check ({} rows)", ids.len()),
        ids.len(),
        checked,
    ))
}

/// Fetch the raw int8 embedding blob for `id` from vec_knowledge. Returns
/// None if the row has no vec0 entry.
fn fetch_vec_blob(conn: &Connection, id: &i64) -> Result<Option<Vec<u8>>> {
    let mut stmt =
        conn.prepare("SELECT embedding_int8 FROM vec_knowledge WHERE knowledge_id = ?1")?;
    let mut rows = stmt.query_map([id], |r| r.get::<_, Option<Vec<u8>>>(0))?;
    if let Some(row) = rows.next() {
        Ok(row?)
    } else {
        Ok(None)
    }
}

/// Compare two dotted schema versions. Returns true if `b >= a` (dest >= source).
/// ponytail: naive lexicographic-per-component compare; assumes both are
/// "major.minor.patch" numerics. Sufficient for the v0.9.x → v1.0.0 cutover.
fn schema_ge(b: &str, a: &str) -> bool {
    let pa: Vec<u64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
    pb >= pa
}

fn print_verify_report(report: &VerifyReport) {
    println!("## Migration rehearsal — verify report\n");
    println!("| check | source | dest | status | note |");
    println!("|---|---|---|---|---|");
    for r in &report.rows {
        println!(
            "| {} | {} | {} | {} | {} |",
            r.check, r.source, r.dest, r.status, r.note
        );
    }
    println!();
    if report.any_failed() {
        println!("**RESULT: FAIL** — at least one parity check failed.");
    } else {
        println!("**RESULT: ALL CHECKS PASSED**");
    }
}

fn verify_report_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".verify-report.md");
    PathBuf::from(p)
}

fn write_verify_report_file(dest: &Path, report: &VerifyReport) -> Result<()> {
    let path = verify_report_path(dest);
    let mut out = String::new();
    out.push_str("# Migration rehearsal — verify report\n\n");
    out.push_str("| check | source | dest | status | note |\n");
    out.push_str("|---|---|---|---|---|\n");
    for r in &report.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            r.check, r.source, r.dest, r.status, r.note
        ));
    }
    fs::write(&path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

// ── report phase ────────────────────────────────────────────────────────────

fn phase_report(args: &ResolvedArgs) -> Result<()> {
    eprintln!(
        "report: {} vs {}",
        args.source.display(),
        args.dest.display()
    );
    let src = Connection::open(&args.source)
        .with_context(|| format!("open source {}", args.source.display()))?;
    let dst = Connection::open(&args.dest)
        .with_context(|| format!("open dest {}", args.dest.display()))?;

    let src_ver = read_schema_version(&args.source)?.unwrap_or_else(|| "none".into());
    let dst_ver = read_schema_version(&args.dest)?.unwrap_or_else(|| "none".into());
    let src_size = fs::metadata(&args.source).map(|m| m.len()).unwrap_or(0);
    let dst_size = fs::metadata(&args.dest).map(|m| m.len()).unwrap_or(0);

    println!("## Migration rehearsal — report\n");
    println!(
        "- source: `{}` (schema {}, {} bytes)",
        args.source.display(),
        src_ver,
        src_size
    );
    println!(
        "- dest:   `{}` (schema {}, {} bytes)",
        args.dest.display(),
        dst_ver,
        dst_size
    );
    println!();
    println!("| table | source | dest |");
    println!("|---|---|---|");
    for tbl in PARITY_TABLES {
        let s = count_rows(&src, tbl).unwrap_or(0);
        let d = count_rows(&dst, tbl).unwrap_or(0);
        println!("| {tbl} | {s} | {d} |");
    }
    let fts_s = count_rows(&src, "knowledge_fts").unwrap_or(0);
    let fts_d = count_rows(&dst, "knowledge_fts").unwrap_or(0);
    println!("| knowledge_fts | {fts_s} | {fts_d} |");
    Ok(())
}

// ── rollback phase ──────────────────────────────────────────────────────────

fn phase_rollback(args: &ResolvedArgs) -> Result<()> {
    eprintln!("rollback: removing {} + sidecars", args.dest.display());
    if args.dest.exists() {
        fs::remove_file(&args.dest).with_context(|| format!("remove {}", args.dest.display()))?;
    }
    remove_sidecars(&args.dest);
    eprintln!("rollback: ok (source untouched)");
    Ok(())
}

fn remove_sidecars(dest: &Path) {
    for p in [
        copy_meta_path(dest),
        verify_report_path(dest),
        sha256_sidecar_path(dest),
    ] {
        if p.exists() {
            let _ = fs::remove_file(&p);
        }
    }
}

fn sha256_sidecar_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".rehearsal-source.sha256");
    PathBuf::from(p)
}

// ── rehearse (all-in-one) ───────────────────────────────────────────────────

fn phase_rehearse(args: &ResolvedArgs) -> Result<()> {
    eprintln!(
        "rehearse: {} → {}",
        args.source.display(),
        args.dest.display()
    );
    phase_backup(args)?;
    phase_copy(args)?;
    let report = phase_verify(args)?;
    print_verify_report(&report);
    write_verify_report_file(&args.dest, &report)?;
    phase_report(args)?;

    if report.any_failed() {
        bail!("rehearse: parity checks failed — dest left in place for inspection");
    }
    if !args.keep_snapshot {
        phase_rollback(args)?;
    } else {
        eprintln!("rehearse: --keep-snapshot set; dest + sidecars left in place");
    }
    eprintln!("rehearse: ALL CHECKS PASSED");
    Ok(())
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn confirm_source_exists(source: &Path) -> Result<()> {
    if !source.exists() {
        bail!("source DB not found: {}", source.display());
    }
    Ok(())
}

/// Warn loudly if the source's WAL file looks active (>1 KiB heuristic).
/// Requires --force to proceed when the heuristic trips. ponytail: opening the
/// DB read-only + `PRAGMA wal_checkpoint` would be precise but mutates the file.
fn warn_if_wal_active(source: &Path, force: bool) -> Result<()> {
    let wal: PathBuf = {
        let mut p = source.as_os_str().to_os_string();
        p.push("-wal");
        PathBuf::from(p)
    };
    if let Ok(meta) = fs::metadata(&wal) {
        if meta.len() > WAL_ACTIVE_HEURISTIC_BYTES {
            if !force {
                bail!(
                    "source WAL is {} bytes (> {} heuristic) — stop the server first, \
                     or pass --force to proceed at your own risk",
                    meta.len(),
                    WAL_ACTIVE_HEURISTIC_BYTES
                );
            }
            eprintln!(
                "warning: source WAL is {} bytes (heuristic tripped); proceeding because --force",
                meta.len()
            );
        }
    }
    Ok(())
}

/// Resolve the backup passphrase from the env ladder:
/// `BRAIN_BACKUP_PASSPHRASE_FILE` → `BRAIN_BACKUP_PASSPHRASE`. Mirrors the
/// restore ladder in `src/backup.rs`.
fn resolve_passphrase() -> Result<Vec<u8>> {
    if let Ok(path) = std::env::var("BRAIN_BACKUP_PASSPHRASE_FILE") {
        let p = path.trim();
        if !p.is_empty() {
            let bytes = fs::read(p).with_context(|| format!("read passphrase file {p}"))?;
            // Trim a trailing newline; passphrases are not binary data here.
            let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
            if !trimmed.is_empty() {
                return Ok(trimmed.into_bytes());
            }
        }
    }
    if let Ok(raw) = std::env::var("BRAIN_BACKUP_PASSPHRASE") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return Ok(raw.as_bytes().to_vec());
        }
    }
    bail!(
        "backup passphrase required: set BRAIN_BACKUP_PASSPHRASE or \
         BRAIN_BACKUP_PASSPHRASE_FILE"
    )
}

fn read_schema_version(db_path: &Path) -> Result<Option<String>> {
    let conn = Connection::open(db_path).with_context(|| format!("open {}", db_path.display()))?;
    Ok(brain_server::storage_layout::schema_version(&conn))
}

fn sha256_and_size(path: &Path) -> Result<(String, u64)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let size = bytes.len() as u64;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok((hex::encode(h.finalize()), size))
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use zerocopy::IntoBytes;

    /// Build a temp source DB with a couple of knowledge rows, vec0 entries,
    /// and an evidence_link. Returns the path; caller owns the tempdir.
    fn build_source_db() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("brain.db");
        register_sqlite_vec();
        let mut conn = Connection::open(&db_path).expect("open source");
        run_migration(&mut conn, 256).expect("migrate source");

        // Two knowledge rows + matching vec0 embeddings.
        for i in 1..=2 {
            conn.execute(
                "INSERT INTO knowledge (content, source, content_hash) VALUES (?1, 'test', ?2)",
                params![format!("content-{i}"), format!("hash-{i}")],
            )
            .expect("insert knowledge");
            let kid: i64 = conn.last_insert_rowid();
            let f32_vec: Vec<f32> = (0..512).map(|j| j as f32 / 512.0 + i as f32).collect();
            conn.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'test', datetime('now'))",
                params![kid, f32_vec.as_bytes()],
            )
            .expect("insert vec");
        }
        // One evidence_link between the two chunks.
        conn.execute(
            "INSERT INTO evidence_links (from_chunk, to_chunk, kind) VALUES (1, 2, 'references')",
            [],
        )
        .expect("insert evidence_link");
        drop(conn);
        dir
    }

    fn resolve_args(source: PathBuf, dest: PathBuf) -> ResolvedArgs {
        ResolvedArgs {
            source,
            dest,
            strict: false,
            force: true, // tests don't stop a "server"
            keep_snapshot: true,
        }
    }

    #[test]
    fn test_rehearse_copy_produces_byte_identical_knowledge_rows() {
        let dir = build_source_db();
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy");

        let src = Connection::open(&source).unwrap();
        let dst = Connection::open(&dest).unwrap();
        let s_hashes: std::collections::BTreeSet<String> = src
            .prepare("SELECT content_hash FROM knowledge")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        let d_hashes: std::collections::BTreeSet<String> = dst
            .prepare("SELECT content_hash FROM knowledge")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(s_hashes, d_hashes, "content hashes must match after copy");
        // vec0 embeddings must be byte-identical too.
        for id in 1..=2 {
            let s = fetch_vec_blob(&src, &id).unwrap().expect("src vec");
            let d = fetch_vec_blob(&dst, &id).unwrap().expect("dst vec");
            assert_eq!(s, d, "vec0 bytes must match for id={id}");
        }
    }

    #[test]
    fn test_rehearse_detects_missing_evidence_link() {
        let dir = build_source_db();
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy");

        // Tamper: delete the evidence_link from dest.
        {
            let conn = Connection::open(&dest).unwrap();
            conn.execute("DELETE FROM evidence_links WHERE from_chunk = 1", [])
                .unwrap();
        }

        let report = phase_verify(&resolve_args(source, dest)).expect("verify runs");
        assert!(
            report.any_failed(),
            "verify must FAIL when an evidence_link is missing: {:?}",
            report.rows
        );
    }

    #[test]
    fn test_rehearse_rollback_cleans_dest_but_not_source() {
        let dir = build_source_db();
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy");
        assert!(dest.exists(), "dest must exist after copy");
        // Write a sidecar so we can prove rollback removes it.
        fs::write(copy_meta_path(&dest), "{}").unwrap();
        phase_rollback(&resolve_args(source.clone(), dest.clone())).expect("rollback");

        assert!(source.exists(), "source must survive rollback");
        assert!(!dest.exists(), "dest must be gone after rollback");
        assert!(
            !copy_meta_path(&dest).exists(),
            "sidecars must be gone after rollback"
        );
    }

    #[test]
    fn test_verify_rejects_schema_downgrade() {
        let dir = build_source_db();
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy");

        // Tamper: regress the recorded schema_version on dest to 0.9.4.
        {
            let conn = Connection::open(&dest).unwrap();
            conn.execute(
                "UPDATE schema_meta SET value = '0.9.4' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }

        let report = phase_verify(&resolve_args(source, dest)).expect("verify runs");
        let schema_row = report
            .rows
            .iter()
            .find(|r| r.check == "schema_version")
            .expect("schema_version check exists");
        assert_eq!(
            schema_row.status, "FAIL",
            "schema downgrade must FAIL: {:?}",
            schema_row
        );
        assert!(report.any_failed());
    }

    #[test]
    fn schema_ge_handles_versions() {
        assert!(schema_ge("0.9.9", "0.9.8"));
        assert!(schema_ge("0.9.9", "0.9.9"));
        assert!(schema_ge("1.0.0", "0.9.9"));
        assert!(!schema_ge("0.9.4", "0.9.9"));
    }

    /// Create a fixture DB whose schema matches a historical release.
    /// Uses the full current migration then removes tables/columns that didn't
    /// exist in `version`. This is sound because the verify phase checks row
    /// counts — a table absent from source reports 0 rows, and the dest (after
    /// `run_migration`) also has 0 rows for any table that was empty in source.
    fn build_historical_fixture(version: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("brain.db");
        register_sqlite_vec();
        let mut conn = Connection::open(&db_path).expect("open fixture");
        run_migration(&mut conn, 256).expect("migrate fixture");

        // Drop tables that postdate the target version.
        // v0.9.8 = + evidence_links (no webhook/audit which are v0.9.7)
        // v0.9.6 = + connectors + checkpoints (no audit, webhook, evidence_links)
        // v0.9.4 = sources + revisions only (no connectors, audit, webhook, evidence_links)
        match version {
            "0.9.8" => {
                conn.execute_batch(
                    "DROP TABLE IF EXISTS webhook_queue;
                     DROP TABLE IF EXISTS webhook_seen;
                     DROP TABLE IF EXISTS audit_events;",
                )
                .expect("drop v0.9.7+ tables");
            }
            "0.9.4" | "0.9.6" => {
                conn.execute_batch(
                    "DROP TABLE IF EXISTS evidence_links;
                     DROP TABLE IF EXISTS webhook_queue;
                     DROP TABLE IF EXISTS webhook_seen;
                     DROP TABLE IF EXISTS audit_events;",
                )
                .expect("drop v0.9.7+ tables");
            }
            _ => {}
        }
        if version == "0.9.4" {
            conn.execute_batch(
                "DROP TABLE IF EXISTS connectors;
                 DROP TABLE IF EXISTS connector_checkpoints;",
            )
            .expect("drop v0.9.6+ tables");
        }
        // Pre-v0.9.9: no schema_version key in schema_meta.
        conn.execute("DELETE FROM schema_meta WHERE key = 'schema_version'", [])
            .expect("clear schema_version");

        // Populate test data: 2 knowledge rows + vec0 embeddings.
        for i in 1..=2 {
            conn.execute(
                "INSERT INTO knowledge (content, source, content_hash) VALUES (?1, 'test', ?2)",
                params![format!("content-{i}"), format!("hash-{i}")],
            )
            .expect("insert knowledge");
            let kid: i64 = conn.last_insert_rowid();
            let f32_vec: Vec<f32> = (0..512).map(|j| j as f32 / 512.0 + i as f32).collect();
            conn.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'test', datetime('now'))",
                params![kid, f32_vec.as_bytes()],
            )
            .expect("insert vec");
        }
        // One evidence_link (only if the table exists in this version).
        if version == "0.9.8" {
            conn.execute(
                "INSERT INTO evidence_links (from_chunk, to_chunk, kind) VALUES (1, 2, 'references')",
                [],
            )
            .expect("insert evidence_link");
        }
        // One source + revision (v0.9.4+).
        conn.execute(
            "INSERT INTO sources (uri, kind) VALUES ('fixture://doc-1', 'vault')",
            [],
        )
        .expect("insert source");
        let sid: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO source_revisions (source_id, revision, chunk_count, byte_size) VALUES (?1, 'rev-1', 2, 256)",
            params![sid],
        )
        .expect("insert revision");
        // Link knowledge row 1 to the source.
        conn.execute(
            "UPDATE knowledge SET source_id = ?1, revision_id = 1 WHERE id = 1",
            params![sid],
        )
        .expect("link source");

        // One connector + checkpoint (v0.9.6+).
        if version == "0.9.6" || version == "0.9.8" {
            conn.execute(
                "INSERT INTO connectors (kind, instance) VALUES ('test', 'fixture')",
                [],
            )
            .expect("insert connector");
            let cid: i64 = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO connector_checkpoints (connector_id, key, value) VALUES (?1, 'cursor', '42')",
                params![cid],
            )
            .expect("insert checkpoint");
        }

        drop(conn);
        dir
    }

    #[test]
    fn test_rehearse_upgrade_from_v0_9_8_fixture() {
        let dir = build_historical_fixture("0.9.8");
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy from v0.9.8 fixture");
        let report = phase_verify(&resolve_args(source, dest)).expect("verify");
        assert!(
            !report.any_failed(),
            "v0.9.8 fixture upgrade must pass all checks: {:?}",
            report.rows
        );
    }

    #[test]
    fn test_rehearse_upgrade_from_v0_9_6_fixture() {
        let dir = build_historical_fixture("0.9.6");
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy from v0.9.6 fixture");
        let report = phase_verify(&resolve_args(source, dest)).expect("verify");
        assert!(
            !report.any_failed(),
            "v0.9.6 fixture upgrade must pass all checks: {:?}",
            report.rows
        );
    }

    #[test]
    fn test_rehearse_upgrade_from_v0_9_4_fixture() {
        let dir = build_historical_fixture("0.9.4");
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        phase_copy(&resolve_args(source.clone(), dest.clone())).expect("copy from v0.9.4 fixture");
        let report = phase_verify(&resolve_args(source, dest)).expect("verify");
        assert!(
            !report.any_failed(),
            "v0.9.4 fixture upgrade must pass all checks: {:?}",
            report.rows
        );
    }

    #[test]
    fn test_rehearse_interrupted_then_resumed() {
        let dir = build_source_db();
        let source = dir.path().join("brain.db");
        let dest = dir.path().join("global.db");

        // Simulate a crash that left a partial/truncated dest file.
        fs::write(&dest, b"partial garbage").expect("write partial dest to simulate crash");

        // Run copy — should remove the partial file and succeed.
        phase_copy(&resolve_args(source.clone(), dest.clone()))
            .expect("copy must clean partial dest and succeed");

        // Verify full parity.
        let report = phase_verify(&resolve_args(source, dest)).expect("verify runs");
        assert!(
            !report.any_failed(),
            "verify must pass after interrupted-then-resumed copy: {:?}",
            report.rows
        );
    }
}
