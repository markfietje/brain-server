//! AUDIT PoC 5 — attaching a shipped READ-ONLY role (`dpo`, `client-auditor`,
//! `bpo-ops`) REMOVES the record-level owner/`access_scope` fence that a
//! role-less principal holds. The role is an IdP-supplied JWT claim, never a
//! server-side user record. Delete after triage.

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
    let conn = pool.get().unwrap();
    for (name, json) in brain_server::role::PRESETS_RAW {
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )
        .unwrap();
    }
    (tmp, pool)
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

fn principal(sub: &str, roles: &[&str]) -> Principal {
    Principal {
        sub: sub.to_string(),
        tenant: "team-alpha".to_string(),
        scopes: vec![Scope::parse("read:team-alpha/global").unwrap()],
        jti: "jti-audit-poc".to_string(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        manages: vec![],
        kind: PrincipalKind::Jwt,
    }
}

#[test]
#[ignore = "AUDIT PoC: asserts the FIXED behaviour. A RED run here means the defect came back. Run with --ignored."]
fn poc_readonly_role_removes_the_owner_fence() {
    let (_tmp, pool) = test_pool();
    seed_private(&pool, "alice", "alice private salary row");

    // BASELINE: the same token with NO roles is owner-bound.
    let no_roles = principal("alice-tenant-user", &[]);
    let g0 = brain_server::handlers::gate::record_read_gate(&Some(no_roles), &pool);
    println!(
        "no roles      -> owner_in={:?} access_scopes={:?}",
        g0.owner_in, g0.access_scopes
    );
    assert!(
        !g0.admits(&Some("alice".into()), &Some("private".into())),
        "baseline: a role-less principal must not read alice's private row"
    );

    // The three read-only compliance presets. `dpo` and `bpo-ops` are read-only
    // BOARD roles whose duties are verb gates (dsar_export, the ops board), so
    // they must stay owner-bound like a role-less principal. `client-auditor`'s
    // documented job IS a cross-owner read of exactly one client domain, so it
    // keeps the widening — but bounded by its access_scope allowlist.
    for role in ["dpo", "bpo-ops"] {
        let p = principal("alice-tenant-user", &[role]);
        let g = brain_server::handlers::gate::record_read_gate(&Some(p), &pool);
        let admits = g.admits(&Some("alice".into()), &Some("private".into()));
        println!(
            "role={role:<16} -> owner_in={:?} access_scopes={:?} admits_alice_private={admits}",
            g.owner_in, g.access_scopes
        );
        assert!(
            !admits,
            "PoC NOT closed for role `{role}`: alice's private row is still admitted"
        );
    }

    let p = principal("alice-tenant-user", &["client-auditor"]);
    let g = brain_server::handlers::gate::record_read_gate(&Some(p), &pool);
    assert!(
        g.admits(&Some("alice".into()), &Some("private".into())),
        "client-auditor's declared cross-owner read must still work"
    );
    assert!(
        g.access_scopes.is_some(),
        "even client-auditor keeps an access_scope allowlist: {:?}",
        g.access_scopes
    );
    assert!(
        !g.admits(&Some("alice".into()), &Some("legal-hold".into())),
        "the scope allowlist must not admit an access_scope no role declared"
    );
    println!(
        ">>> CLOSED: dpo + bpo-ops no longer widen; client-auditor widens only `owner` \
         and stays inside its declared access_scope set"
    );
}
