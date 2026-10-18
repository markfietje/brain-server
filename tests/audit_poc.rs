//! AUDIT PoC suite — each arm drives the REAL production code and asserts the
//! FIXED behaviour, so a red run means the defect came back.
//!
//! These were written as proof-of-concept reproductions first (each asserting
//! the BUGGY behaviour), then inverted once the fix landed. They live in
//! `tests/` rather than beside the code because the in-crate red-first pins are
//! the durable guards; these are the end-to-end re-checks.
//!
//! Run with: `cargo test --test audit_poc -- --ignored --nocapture`
//! Delete after the next quarter's audit.

use brain_server::auth::{Principal, PrincipalKind, Scope};
use brain_server::pool::SqliteConnectionManager;
use std::sync::Arc;
use zerocopy::IntoBytes;

fn test_pool() -> (tempfile::TempDir, brain_server::Pool) {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let dir = tempfile::TempDir::new().expect("tmp");
    let mgr = SqliteConnectionManager::file(dir.path().join("brain.db"));
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(&mut pool.get().unwrap(), 64).expect("migrate");
    (dir, pool)
}

fn seed_private(pool: &brain_server::Pool, owner: &str, content: &str) -> i64 {
    let v = vec![0.5f32; 512];
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge(content, content_hash, owner, access_scope, domain)
         VALUES (?1, ?2, ?3, 'private', 'global')",
        rusqlite::params![content, format!("h-{content}"), owner],
    )
    .unwrap();
    let kid = conn.last_insert_rowid();
    // the legacy leg JOINs `embeddings`, the vec0 leg reads `vec_knowledge`
    conn.execute(
        "INSERT INTO vec_knowledge (knowledge_id, embedding_int8, embedding_bit, source, created_at)
         VALUES (?1, vec_quantize_int8(?2,'unit'), vec_quantize_binary(?2), 'structured', datetime('now'))",
        rusqlite::params![kid, v.as_bytes()],
    )
    .unwrap();
    let unit = format!("[{}]", vec!["0.5"; 512].join(","));
    conn.execute(
        "INSERT INTO embeddings(knowledge_id, vector) VALUES (?1, ?2)",
        rusqlite::params![kid, unit],
    )
    .unwrap();
    kid
}

fn no_role(sub: &str) -> Principal {
    Principal {
        sub: sub.into(),
        tenant: "team-alpha".into(),
        scopes: vec![Scope::parse("read:team-alpha/global").unwrap()],
        jti: "jti-audit-poc".into(),
        roles: vec![],
        manages: vec![],
        kind: PrincipalKind::Jwt,
    }
}

fn gate_filters(
    gate: &brain_server::handlers::gate::RecordReadGate,
) -> brain_server::search::SearchFilters {
    brain_server::search::SearchFilters {
        access_scopes: gate.access_scopes.clone().map(Arc::new),
        owner_in: gate.owner_in.clone().map(Arc::new),
        ..Default::default()
    }
}

/// 1. The vector-search fallback must not be a bypass of the tenant boundary.
#[test]
#[ignore = "AUDIT PoC: drives the real code. Un-ignore to re-check; a red run means the defect returned."]
fn poc_legacy_fallback_must_not_leak_the_record_gate() {
    let (_tmp, pool) = test_pool();
    let own_id = seed_private(&pool, "ana", "own private content alpha");
    let foreign_id = seed_private(&pool, "bob", "own private content beta");

    let ana = no_role("ana");
    let gate = brain_server::handlers::gate::record_read_gate(&Some(ana), &pool);
    let model = brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).unwrap();
    let filters = gate_filters(&gate);

    let vec_ids: Vec<i64> = brain_server::search::perform_search(
        &pool,
        &model,
        "own private content".to_string(),
        10,
        &filters,
    )
    .expect("vec0 search")
    .iter()
    .map(|h| h.id)
    .collect();
    println!("VEC0 leg ids   = {vec_ids:?} (own={own_id} foreign={foreign_id})");

    // Flip the process-global readiness flag exactly the way the
    // `no such table: vec_knowledge` handler at search/mod.rs does.
    brain_server::migration::VEC0_READY.store(false, std::sync::atomic::Ordering::Relaxed);
    let legacy_ids: Vec<i64> = brain_server::search::perform_search(
        &pool,
        &model,
        "own private content".to_string(),
        10,
        &filters,
    )
    .expect("legacy search")
    .iter()
    .map(|h| h.id)
    .collect();
    println!("LEGACY leg ids = {legacy_ids:?}");

    assert!(
        !vec_ids.contains(&foreign_id),
        "the gated vec0 leg must not return bob's private row"
    );
    assert!(
        !legacy_ids.contains(&foreign_id),
        ">>> REGRESSED: the fallback returned bob's access_scope='private' row {foreign_id} \
         to a gate that denies it — the fallback is a bypass of the tenant boundary"
    );
    assert!(
        legacy_ids.contains(&own_id),
        "the fallback must still work for rows the gate DOES admit: {legacy_ids:?}"
    );
    println!(">>> CLOSED: the fallback applies the same record gate as the vec0 leg");
}

// 2. PII: the detectors must catch realistic values, and the write path must
// store the flag so the read seam actually masks.
#[test]
#[ignore = "AUDIT PoC: drives the real code. Un-ignore to re-check; a red run means the defect returned."]
fn poc_pii_detected_flagged_and_masked() {
    let realistic = [
        "alice.chen@hospital.example",
        "Card 4111 1111 1111 1111 expires 12/27",
        "charged 4111 1111 1111 1111 on 2026-10-01 ref 88213",
    ];
    for t in realistic {
        println!("scan_pii({t:?}) = {:?}", brain_server::gate::scan_pii(t));
    }
    assert!(
        !brain_server::gate::scan_pii(realistic[0]).is_empty(),
        "an email at offset 0 must be detected"
    );
    for t in &realistic[1..] {
        assert!(
            !brain_server::gate::scan_pii(t).is_empty(),
            "a card sharing the line with another number must be detected: {t:?}"
        );
    }

    // the storage half: the `/add` INSERT shape, with the flag.
    let (_tmp, pool) = test_pool();
    let conn = pool.get().unwrap();
    let payload = realistic[1];
    conn.execute(
        "INSERT INTO knowledge(content, title, source, content_hash, pii, owner, origin)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            payload,
            "billing note",
            "manual",
            "h-x",
            brain_server::gate::pii_flag("billing note", payload) as i64,
            "ana",
            "user"
        ],
    )
    .unwrap();
    let (content, pii): (String, i64) = conn
        .query_row(
            "SELECT content, pii FROM knowledge ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    println!("stored content = {content:?}\nstored pii     = {pii}");
    assert_eq!(pii, 1, "a PAN on the line must store pii=1");

    let alice = no_role("ana");
    let seam = brain_server::gate::sanitize_read(&content, pii != 0, &Some(alice));
    println!("seam           = {seam:?}");
    assert!(
        !seam.contains("4111 1111 1111 1111"),
        ">>> REGRESSED: the PAN reached a non-admin principal verbatim: {seam:?}"
    );
    println!(">>> CLOSED: detected at write, stored as pii=1, masked at read");
}

/// 3. Every `/verify` `match_ranges` entry must index `content`.
#[test]
#[ignore = "AUDIT PoC: drives the real code. Un-ignore to re-check; a red run means the defect returned."]
fn poc_verify_offsets_always_index_content() {
    let content = "\u{130}stanbul"; // 'İ' = 2 bytes, lowercases to 3.
    let ranges = brain_server::handlers::verify::verify_claim(content, "stanbul");
    println!(
        "content = {content:?} ({} bytes), ranges = {ranges:?}",
        content.len()
    );
    for r in &ranges {
        // The DOCUMENTED consumer operation: &content[start..end]. It panics
        // when the range is out of bounds, which is the point.
        let sliced = std::panic::catch_unwind(|| &content[r[0]..r[1]]);
        println!(
            "  content[{}..{}] -> {}",
            r[0],
            r[1],
            if sliced.is_ok() {
                format!("{sliced:?}")
            } else {
                "PANIC".to_string()
            }
        );
    }
    assert!(
        ranges.iter().all(|r| r[1] <= content.len()),
        ">>> REGRESSED: a range indexes past the end of `content`; openapi.yaml publishes \
         these as offsets WITHIN chunk `content`"
    );
    assert_eq!(
        &content[ranges[0][0]..ranges[0][1]],
        "stanbul",
        "the range must slice to the matched span"
    );
    println!(">>> CLOSED: every returned range indexes `content` and slices to the claim");
}
