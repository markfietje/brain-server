//! Private memory — a no-role JWT may read its own private memory,
//! never another principal's private memory.
//!
//! Exercises the shared gate-plus-search path (record_read_gate + perform_search),
//! not a hand-constructed owner_in value.

use std::sync::Arc;

use brain_server::auth::{Principal, PrincipalKind, Scope};
use brain_server::pool::SqliteConnectionManager;
use zerocopy::IntoBytes;

fn test_pool() -> (tempfile::NamedTempFile, brain_server::Pool) {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let mgr = SqliteConnectionManager::file(tmp.path());
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(2).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().unwrap(),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    // Seed the roles table so role-based gates resolve (agent shared pool).
    {
        let conn = pool.get().unwrap();
        for (name, json) in brain_server::role::PRESETS_RAW {
            conn.execute(
                "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
                rusqlite::params![name, json],
            )
            .unwrap();
        }
    }
    (tmp, pool)
}

fn no_role(sub: &str) -> Principal {
    Principal {
        sub: sub.to_string(),
        tenant: "team-alpha".to_string(),
        scopes: vec![Scope::parse("read:team-alpha/global").unwrap()],
        jti: "jti-private-memory".to_string(),
        roles: vec![],
        manages: vec![],
        kind: PrincipalKind::Jwt,
    }
}

fn seed_private(pool: &brain_server::Pool, owner: &str, content: &str) -> i64 {
    let v = vec![0.5f32; 512];
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope)
         VALUES (?1, ?2, ?3, 'structured', 'global', ?4, 'private')",
        rusqlite::params![content, content, format!("h-{content}"), owner],
    )
    .unwrap();
    let kid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO vec_knowledge (knowledge_id, embedding_int8, embedding_bit, source, created_at)
         VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'structured', datetime('now'))",
        rusqlite::params![kid, v.as_bytes()],
    )
    .unwrap();
    kid
}

fn filters_from_gate(
    gate: &brain_server::handlers::gate::RecordReadGate,
) -> brain_server::search::SearchFilters {
    brain_server::search::SearchFilters {
        access_scopes: gate.access_scopes.clone().map(Arc::new),
        owner_in: gate.owner_in.clone().map(Arc::new),
        ..Default::default()
    }
}

/// No-role JWT sees its own private row and is denied another subject's
/// private row — through the shared gate (by-id parity) and through search.
#[test]
fn no_role_private_is_owner_bound() {
    let (_tmp, pool) = test_pool();
    let own_id = seed_private(&pool, "ana", "own private content alpha");
    let foreign_id = seed_private(&pool, "bob", "own private content beta");

    let ana = no_role("ana");
    let gate = brain_server::handlers::gate::record_read_gate(&Some(ana), &pool);

    // By-id parity: the same (owner, scope) pair /get/{id} enforces.
    assert!(
        gate.admits(&Some("ana".to_string()), &Some("private".to_string())),
        "no-role JWT reads its own private row"
    );
    assert!(
        !gate.admits(&Some("bob".to_string()), &Some("private".to_string())),
        "no-role JWT must not read another subject's private row"
    );

    // Search path: gate-derived filters drive the shared SQL predicates.
    let model =
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model");
    let filters = filters_from_gate(&gate);
    let hits = brain_server::search::perform_search(
        &pool,
        &model,
        "own private content".to_string(),
        10,
        &filters,
    )
    .expect("search");
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&own_id),
        "own private row surfaces in search: {ids:?}"
    );
    assert!(
        !ids.contains(&foreign_id),
        "foreign private row must not surface in search: {ids:?}"
    );
}

/// Absent/empty subject cannot read private records (fail closed).
#[test]
fn no_role_empty_subject_denies_private() {
    let (_tmp, pool) = test_pool();
    seed_private(&pool, "ana", "empty sub content");

    for sub in ["", "   "] {
        let p = no_role(sub);
        let gate = brain_server::handlers::gate::record_read_gate(&Some(p), &pool);
        assert!(
            !gate.admits(&Some("ana".to_string()), &Some("private".to_string())),
            "empty subject must not read private (sub={sub:?})"
        );
    }
}

/// Neighboring allowed cases are unchanged: admin + loopback unrestricted,
/// role-based agent shared pool unchanged, own domain/team readable.
#[test]
fn neighboring_allowed_cases_unchanged() {
    let (_tmp, pool) = test_pool();
    seed_private(&pool, "other", "neighbor shared content");

    // Admin (no roles, admin scope) stays unrestricted.
    let admin = Principal {
        sub: "root".to_string(),
        tenant: "team-alpha".to_string(),
        scopes: vec![Scope::parse("admin:team-alpha/*").unwrap()],
        jti: "jti-admin".to_string(),
        roles: vec![],
        manages: vec![],
        kind: PrincipalKind::Jwt,
    };
    let admin_gate = brain_server::handlers::gate::record_read_gate(&Some(admin), &pool);
    assert!(
        admin_gate.admits(&Some("other".to_string()), &Some("private".to_string())),
        "admin stays unrestricted"
    );

    // Loopback/opaque stays unrestricted.
    let loop_gate = brain_server::handlers::gate::record_read_gate(&None, &pool);
    assert!(
        loop_gate.admits(&Some("other".to_string()), &Some("private".to_string())),
        "loopback stays unrestricted"
    );

    // Role-based agent preset still reads the shared pool.
    let agent = Principal {
        sub: "ana".to_string(),
        tenant: "team-alpha".to_string(),
        scopes: vec![Scope::parse("read:team-alpha/*").unwrap()],
        jti: "jti-agent".to_string(),
        roles: vec!["agent".to_string()],
        manages: vec![],
        kind: PrincipalKind::Jwt,
    };
    let agent_gate = brain_server::handlers::gate::record_read_gate(&Some(agent), &pool);
    assert!(
        agent_gate.admits(&Some("other".to_string()), &Some("private".to_string())),
        "agent preset keeps the shared pool"
    );
}
