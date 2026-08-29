//! Per-decision evidence records (EU AI Act Art. 12 deployer-side logging).
//!
//! Each automated or human-in-the-loop decision appends a [`DecisionRecord`]
//! with a SHA-256 chain link and an optional detached Ed25519 signature over
//! the hash. The recorder lives on the host write path (`WorkflowHost::audit`
//! and the Admin compliance endpoints) — never in engine or application code —
//! so a system cannot modify its own evidence.
//!
//! Honest ceiling: a record proves existence/time/signer/immutability, not
//! fairness, lawfulness, or accuracy of the underlying decision. When
//! `BRAIN_AUDIT_SIGNING_KEY` is unset, rows are hashed + chained but carry a
//! NULL signature (disclosed on `/audit/export`, not silently trusted).

#![deny(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::sync::OnceLock;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The feature-gated evidence table. Created by the migration only under
/// `--features compliance-pack`; without it the server behaves as before.
pub const DDL: &str = "CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS decision_records(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    actor_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT '',
    policy_version TEXT NOT NULL DEFAULT '',
    prompt_class TEXT NOT NULL DEFAULT '',
    tool TEXT NOT NULL DEFAULT '',
    model_id TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL,
    hash TEXT NOT NULL,
    prev_hash TEXT NOT NULL DEFAULT '',
    sig TEXT
 );";

#[derive(Debug, Clone, Serialize)]
pub struct DecisionRecord {
    pub id: i64,
    pub ts: i64,
    pub actor_id: String,
    pub role: String,
    pub policy_version: String,
    pub prompt_class: String,
    pub tool: String,
    pub model_id: String,
    pub outcome: String,
    pub hash: String,
    #[serde(skip)]
    pub prev_hash: String,
    /// hex detached Ed25519 signature over `hash`; `None` = unsigned chain.
    pub sig: Option<String>,
}

/// Everything that identifies one decision for evidence purposes.
#[derive(Debug, Clone, Default)]
pub struct DecisionInput<'a> {
    pub actor_id: &'a str,
    pub role: &'a str,
    /// governance policy version in force at decision time
    pub policy_version: &'a str,
    /// coarse prompt/task class — never raw content (a query can be personal data)
    pub prompt_class: &'a str,
    pub tool: &'a str,
    pub model_id: &'a str,
    pub outcome: &'a str,
}

struct ChainKey(Option<SigningKey>);

static CHAIN_KEY: OnceLock<std::sync::RwLock<ChainKey>> = OnceLock::new();

fn chain_key_cell() -> &'static std::sync::RwLock<ChainKey> {
    CHAIN_KEY.get_or_init(|| std::sync::RwLock::new(load_chain_key()))
}

fn load_chain_key() -> ChainKey {
    let key = if let Ok(hex) = std::env::var("BRAIN_AUDIT_SIGNING_KEY") {
        decode_seed(&hex)
    } else if let Ok(path) = std::env::var("BRAIN_AUDIT_SIGNING_KEY_FILE") {
        std::fs::read(&path).ok().and_then(|bytes| {
            if bytes.is_empty() {
                return None;
            }
            // a wide-mode key file is refused fail-closed (the auth-secret posture)
            let ok_mode = std::fs::metadata(&path)
                .map(|m| (m.permissions().mode() & 0o077) == 0)
                .unwrap_or(false);
            match (ok_mode, decode_seed(&String::from_utf8_lossy(&bytes))) {
                (true, Some(sk)) => Some(sk),
                _ => {
                    tracing::error!(
                        "BRAIN_AUDIT_SIGNING_KEY_FILE {path} unreadable or group/world-readable \
                         — decisions are recorded UNSIGNED"
                    );
                    None
                }
            }
        })
    } else {
        None
    };
    if key.is_none() && std::env::var("BRAIN_AUDIT_SIGNING_KEY").is_ok() {
        tracing::error!("BRAIN_AUDIT_SIGNING_KEY is not valid 32-byte hex — unsigned decisions");
    }
    ChainKey(key)
}

/// Test-only override so suites can exercise the signed path without env
/// racing the process-wide cache. Also visible under `compliance-pack`: the
/// pack's evidence suites run in the BINARY's test target, where this lib
/// crate is an external dependency and `cfg(test)` is off — without the
/// feature arm (and `pub`) the pack's own pins cannot reach the seam.
#[cfg(any(test, feature = "compliance-pack"))]
pub fn install_test_signing_key(seed: [u8; 32]) {
    *chain_key_cell().write().unwrap_or_else(|e| e.into_inner()) =
        ChainKey(Some(SigningKey::from_bytes(&seed)));
}

/// The signing key is ONE process-global; every test that records+verifies
/// decisions must hold this lock for the whole record→verify sequence or a
/// sibling's `install_test_signing_key` races mid-test and signatures are
/// verified under the wrong key (the tip_truncation CI flake).
#[cfg(any(test, feature = "compliance-pack"))]
pub static DECISION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Poison-tolerant acquisition — a panicking sibling must not cascade
/// PoisonErrors through unrelated decision tests.
#[cfg(any(test, feature = "compliance-pack"))]
pub fn decision_test_lock() -> std::sync::MutexGuard<'static, ()> {
    DECISION_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Re-exported test seam: other modules' tests install the same fixed key so
/// cross-module evidence tests verify signatures deterministically.
#[cfg(test)]
pub mod tests_seed {
    pub fn install_test_key() {
        super::install_test_signing_key([7u8; 32]);
    }
}

fn signing_key() -> Option<SigningKey> {
    chain_key_cell()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .0
        .clone()
}

fn decode_seed(s: &str) -> Option<SigningKey> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    (0..32)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()
        .and_then(|b| SigningKey::from_bytes(&b.try_into().ok()?).into())
}

/// The verifying half of the configured signing key, for out-of-band checks.
pub fn verifying_key() -> Option<VerifyingKey> {
    signing_key().map(|sk| sk.verifying_key())
}

/// Canonical preimage of the chain link: prev hash binds the row into the
/// decision chain; every committed field participates so mutating any of them
/// breaks verification.
fn link_prehash(prev_hash: &str, r: &DecisionInput<'_>, ts: i64) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(r.actor_id.as_bytes());
    h.update([0]);
    h.update(r.role.as_bytes());
    h.update([0]);
    h.update(r.policy_version.as_bytes());
    h.update([0]);
    h.update(r.prompt_class.as_bytes());
    h.update([0]);
    h.update(r.tool.as_bytes());
    h.update([0]);
    h.update(r.model_id.as_bytes());
    h.update([0]);
    h.update(r.outcome.as_bytes());
    h.update([0]);
    h.update(ts.to_le_bytes());
    h.finalize().to_vec()
}

/// Append one decision record. Best-effort like the audit chain: evidence must
/// never fail the primary action; a dropped row reads as a gap, never a fork.
/// Returns the stored record on success.
pub fn record_decision(conn: &Connection, input: &DecisionInput<'_>) -> Option<DecisionRecord> {
    let ts = chrono::Utc::now().timestamp();
    let autocommit = conn.is_autocommit();
    let (begin, end, rollback) = if autocommit {
        ("BEGIN IMMEDIATE", "COMMIT", "ROLLBACK")
    } else {
        (
            "SAVEPOINT decision_link",
            "RELEASE SAVEPOINT decision_link",
            "ROLLBACK TO SAVEPOINT decision_link",
        )
    };
    if conn.execute(begin, []).is_err() {
        return None;
    }
    // Engine-controlled fields must never carry NUL: the link preimage
    // separates fields with a single 0 byte, so an embedded NUL would make
    // distinct field combinations hash identically (dispute ambiguity).
    let nul_free = [
        input.actor_id,
        input.role,
        input.policy_version,
        input.prompt_class,
        input.tool,
        input.model_id,
        input.outcome,
    ]
    .iter()
    .all(|f| !f.as_bytes().contains(&0));
    if !nul_free {
        tracing::warn!("decision record refused: NUL byte in engine-controlled field");
        let _ = conn.execute(rollback, []);
        return None;
    }
    let prev_hash: String = conn
        .query_row(
            "SELECT hash FROM decision_records ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
        .unwrap_or_default();
    let hash_hex = hex(&link_prehash(&prev_hash, input, ts));
    let sig = signing_key()
        .as_ref()
        .map(|sk| hex(&sk.sign(hash_hex.as_bytes()).to_bytes()));
    let row = conn.execute(
        "INSERT INTO decision_records(ts, actor_id, role, policy_version, prompt_class,
                                          tool, model_id, outcome, hash, prev_hash, sig)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            ts,
            input.actor_id,
            input.role,
            input.policy_version,
            input.prompt_class,
            input.tool,
            input.model_id,
            input.outcome,
            hash_hex,
            prev_hash,
            sig
        ],
    );
    if row.is_err() {
        let _ = conn.execute(rollback, []);
        return None;
    }
    // Re-pin the head in the same tx as the append (the audit-chain-head
    // posture): truncating the tip later is detected at verify time.
    if conn
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES ('decision_chain_head', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![hash_hex],
        )
        .is_err()
    {
        let _ = conn.execute(rollback, []);
        return None;
    }
    if conn.execute(end, []).is_err() && autocommit {
        return None;
    }
    let id = conn.last_insert_rowid();
    // Extend the EXISTING audit_events chain with one row per decision: the
    // decision ledger keeps its rich fields locally, but its tamper-evidence
    // root is the audit chain (extended, never replaced).
    super::record(
        conn,
        super::AuditKind::Decision,
        input.actor_id,
        input.tool,
        super::AuditStatus::Ok,
        &format!("decision:{hash_hex}"),
    );
    Some(DecisionRecord {
        id,
        ts,
        actor_id: input.actor_id.into(),
        role: input.role.into(),
        policy_version: input.policy_version.into(),
        prompt_class: input.prompt_class.into(),
        tool: input.tool.into(),
        model_id: input.model_id.into(),
        outcome: input.outcome.into(),
        hash: hash_hex,
        prev_hash,
        sig,
    })
}

/// List records newer than `since` (unix seconds), newest first, bounded.
pub fn list_decisions(
    conn: &Connection,
    since: Option<i64>,
    limit: i64,
) -> Result<Vec<DecisionRecord>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor_id, role, policy_version, prompt_class, tool, model_id,
                outcome, hash, prev_hash, sig
         FROM decision_records WHERE (?1 IS NULL OR ts >= ?1)
         ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![since, limit], |r| {
        Ok(DecisionRecord {
            id: r.get(0)?,
            ts: r.get(1)?,
            actor_id: r.get(2)?,
            role: r.get(3)?,
            policy_version: r.get(4)?,
            prompt_class: r.get(5)?,
            tool: r.get(6)?,
            model_id: r.get(7)?,
            outcome: r.get(8)?,
            hash: r.get(9)?,
            prev_hash: r.get(10)?,
            sig: r.get(11)?,
        })
    })?;
    rows.collect()
}

/// Full-chain verification: every link recomputes from its predecessor AND
/// every present signature verifies against the configured key. Unsigned
/// chains verify structurally; the absence of signatures is disclosed by the
/// exporter, not hidden here.
pub fn verify_decisions(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let vk = verifying_key();
    verify_decisions_with(conn, vk.as_ref())
}

/// The verification core with the verifying key injected (test seam; also
/// lets a caller distinguish "no key" from "key mismatch" upstream).
pub fn verify_decisions_with(
    conn: &Connection,
    vk: Option<&VerifyingKey>,
) -> Result<bool, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor_id, role, policy_version, prompt_class, tool, model_id,
                outcome, hash, prev_hash, sig
         FROM decision_records ORDER BY id",
    )?;
    let rows: Vec<DecisionRecord> = stmt
        .query_map([], |r| {
            Ok(DecisionRecord {
                id: r.get(0)?,
                ts: r.get(1)?,
                actor_id: r.get(2)?,
                role: r.get(3)?,
                policy_version: r.get(4)?,
                prompt_class: r.get(5)?,
                tool: r.get(6)?,
                model_id: r.get(7)?,
                outcome: r.get(8)?,
                hash: r.get(9)?,
                prev_hash: r.get(10)?,
                sig: r.get(11)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut prev = String::new();
    for r in &rows {
        let input = DecisionInput {
            actor_id: &r.actor_id,
            role: &r.role,
            policy_version: &r.policy_version,
            prompt_class: &r.prompt_class,
            tool: &r.tool,
            model_id: &r.model_id,
            outcome: &r.outcome,
        };
        let expect = hex(&link_prehash(&prev, &input, r.ts));
        if r.prev_hash != prev || r.hash != expect {
            return Ok(false);
        }
        if let Some(sig_hex) = &r.sig {
            // Fail closed: a stored signature with no verifying key
            // configured is UNVERIFIABLE, never "ok".
            let Some(vk) = vk else {
                return Ok(false);
            };
            let Some(sig_bytes) = unhex(sig_hex) else {
                return Ok(false);
            };
            let Ok(sig) = Signature::from_slice(&sig_bytes) else {
                return Ok(false);
            };
            if vk.verify(r.hash.as_bytes(), &sig).is_err() {
                return Ok(false);
            }
        }
        prev = r.hash.clone();
    }
    // Head pin: a chain that is internally valid but truncated (or extended
    // after the fact) must still fail when the pinned head disagrees. A chain
    // with no pin yet bootstraps its pin at first verify (legacy chains —
    // truncation BEFORE that first verify is not detectable; disclosed
    // ceiling).
    let rows_empty = rows.is_empty();
    let pinned: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'decision_chain_head'",
            [],
            |r| r.get(0),
        )
        .ok();
    if let Some(p) = pinned {
        // Pinned head must equal the recomputed tip — truncation or extension
        // of an internally-valid chain fails here.
        if rows_empty {
            if !p.is_empty() {
                return Ok(false);
            }
        } else if p != prev {
            return Ok(false);
        }
    } else if !rows_empty
        && conn
            .execute(
                "INSERT INTO schema_meta(key, value) VALUES ('decision_chain_head', ?1)",
                rusqlite::params![prev],
            )
            .is_err()
    {
        // Legacy chain with no pin yet: bootstrap the pin at first verify.
        return Ok(false);
    }
    Ok(true)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Human-readable export body (Annex IV technical-documentation shape):
/// one text page-set listing each record's fields. Deliberately dependency-free
/// PDF 1.4 — text objects only, Helvetica, paginated at 40 lines per page.
pub fn render_pdf(records: &[DecisionRecord], title: &str) -> Vec<u8> {
    let refs: Vec<&DecisionRecord> = records.iter().collect();
    render_pdf_labelled(&[], &refs, title)
}

/// Same body, with one provenance label (the owning domain) printed per
/// record: ids collide across domains, so an exported bundle without the
/// label is unattributable. `labels` must be empty or `records.len()` long.
pub fn render_pdf_labelled(labels: &[&str], records: &[&DecisionRecord], title: &str) -> Vec<u8> {
    assert!(labels.is_empty() || labels.len() == records.len());
    const LINES_PER_PAGE: usize = 40;
    let mut pages: Vec<Vec<String>> = vec![Vec::new()];
    let push = |pages: &mut Vec<Vec<String>>, line: String| {
        if pages.last_mut().expect("seeded").len() >= LINES_PER_PAGE {
            pages.push(Vec::new());
        }
        pages.last_mut().expect("seeded").push(line);
    };
    push(
        &mut pages,
        format!(
            "BF1 / {} / generated {}",
            title,
            chrono::Utc::now().to_rfc3339()
        ),
    );
    for (i, r) in records.iter().enumerate() {
        if !labels.is_empty() {
            push(&mut pages, format!("domain={}", labels[i]));
        }
        push(&mut pages, format!("decision id={} ts={}", r.id, r.ts));
        push(
            &mut pages,
            format!("  actor={} role={}", r.actor_id, r.role),
        );
        push(
            &mut pages,
            format!(
                "  policy={} class={} tool={} model={}",
                r.policy_version, r.prompt_class, r.tool, r.model_id
            ),
        );
        push(&mut pages, format!("  outcome={}", r.outcome));
        push(
            &mut pages,
            format!(
                "  hash={} sig={}",
                r.hash,
                r.sig.as_deref().unwrap_or("<unsigned>")
            ),
        );
    }

    fn esc(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                '(' | ')' => format!("\\{c}"),
                '\\' => "\\\\".into(),
                c if (c as u32) < 32 || (c as u32) > 126 => '?'.into(),
                c => c.into(),
            })
            .collect()
    }

    // Assemble the PDF object graph. Offsets computed as we emit.
    let mut objects: Vec<String> = Vec::new();
    let n_pages = pages.len();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..n_pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {n_pages} >>",
        kids.join(" ")
    ));
    for (i, lines) in pages.iter().enumerate() {
        let content_ref = 4 + i * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 792 612] \
             /Resources << /Font << /F1 {} 0 R >> >> /Contents {content_ref} 0 R >>",
            3 + n_pages * 2
        ));
        let mut stream = String::from("BT /F1 9 Tf 12 TL 36 576 Td\n");
        for line in lines {
            // field text is escaped once, here (parens/backslash/control chars)
            stream.push_str(&format!("({}) Tj T*\n", esc(line)));
        }
        stream.push_str("ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DDL).unwrap();
        conn
    }

    fn input<'a>(actor: &'a str, outcome: &'a str) -> DecisionInput<'a> {
        DecisionInput {
            actor_id: actor,
            role: "dpo",
            policy_version: "gov-2026.08",
            prompt_class: "review",
            tool: "proposal:7",
            model_id: "test-model",
            outcome,
        }
    }

    #[test]
    fn chain_and_signatures_verify_roundtrip() {
        let _g = decision_test_lock();
        install_test_signing_key([7u8; 32]);
        let conn = db();
        let first = record_decision(&conn, &input("alice", "accept")).unwrap();
        record_decision(&conn, &input("bob", "override")).unwrap();
        assert!(verify_decisions(&conn).unwrap());
        assert_eq!(first.prev_hash, "", "the genesis record has no predecessor");
        assert!(
            first.sig.is_some(),
            "an unsigned env would still be disclosed here via None"
        );
        // tamper with a committed field → verification fails
        conn.execute(
            "UPDATE decision_records SET outcome='forged' WHERE id=1",
            [],
        )
        .unwrap();
        assert!(!verify_decisions(&conn).unwrap());
    }

    #[test]
    fn genesis_link_binds_the_first_row_too() {
        // An empty-string prev means an attacker cannot prepend a fabricated
        // earlier history without breaking the genesis link check.
        let conn = db();
        record_decision(&conn, &input("a", "ok"));
        conn.execute("UPDATE decision_records SET prev_hash='x'", [])
            .unwrap();
        assert!(!verify_decisions(&conn).unwrap());
    }

    #[test]
    fn list_respects_since_and_limit() {
        let conn = db();
        for i in 0..5 {
            record_decision(&conn, &input("a", &format!("o{i}")));
        }
        assert_eq!(list_decisions(&conn, None, 100).unwrap().len(), 5);
        assert_eq!(list_decisions(&conn, None, 2).unwrap().len(), 2);
        assert!(
            list_decisions(&conn, Some(i64::MAX), 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pdf_is_wellformed_and_escapes_parens() {
        let conn = db();
        record_decision(&conn, &input("al(ice)", "ac\"cept"));
        let recs = list_decisions(&conn, None, 10).unwrap();
        let pdf = render_pdf(&recs, "decision ledger");
        let body = String::from_utf8(pdf).unwrap();
        assert!(body.starts_with("%PDF-1.4"));
        assert!(body.trim_end().ends_with("%%EOF"));
        let hit = body.lines().any(|l| l.contains("al\\(ice\\)"));
        assert!(
            hit,
            "paren escaping missing in: {}",
            &body[..body.len().min(600)]
        );
        assert!(body.contains("/Count 1"));
        assert!(!body.contains("unsigned>") || recs.iter().any(|r| r.sig.is_none()));
    }

    #[test]
    fn multipage_pdf_counts_pages() {
        let conn = db();
        for i in 0..95 {
            record_decision(&conn, &input("a", &format!("o{i}")));
        }
        let recs = list_decisions(&conn, None, 200).unwrap();
        let recs: Vec<_> = recs.into_iter().rev().collect();
        let pdf = render_pdf(&recs, "t");
        let body = String::from_utf8(pdf).unwrap();
        let count = body
            .match_indices("/Count ")
            .next()
            .map(|i| &body[i.0..i.0 + 12])
            .unwrap_or_default();
        assert!(
            body.contains("/Count 12"),
            "476 lines paginate to 12 pages, got {count}"
        );
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DDL).unwrap();
        conn
    }

    fn input<'a>(actor: &'a str, outcome: &'a str) -> DecisionInput<'a> {
        DecisionInput {
            actor_id: actor,
            role: "dpo",
            policy_version: "gov-2026.08",
            prompt_class: "review",
            tool: "proposal:7",
            model_id: "test-model",
            outcome,
        }
    }

    #[test]
    fn signed_chain_without_key_fails_closed() {
        let _g = decision_test_lock();
        // A stored signature with NO verifying key configured must read as
        // NOT OK — never as structurally-valid silence. Exercised via the
        // injection seam (no process-global key mutation).
        install_test_signing_key([9u8; 32]);
        let conn = db();
        record_decision(&conn, &input("a", "ok")).unwrap();
        assert!(!verify_decisions_with(&conn, None).unwrap());
        assert!(verify_decisions(&conn).unwrap());
    }

    #[test]
    fn tip_truncation_is_detected_by_the_head_pin() {
        let _g = decision_test_lock();
        // Deterministic signing: never inherit a sibling's key mid-test.
        install_test_signing_key([9u8; 32]);
        let conn = db();
        record_decision(&conn, &input("a", "o1")).unwrap();
        record_decision(&conn, &input("b", "o2")).unwrap();
        assert!(verify_decisions(&conn).unwrap());
        let pinned: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key='decision_chain_head'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute("DELETE FROM decision_records WHERE id = 2", [])
            .unwrap();
        // The chain is internally valid after truncation; the pin catches it.
        assert!(!verify_decisions(&conn).unwrap());
        assert_ne!(
            pinned,
            conn.query_row(
                "SELECT hash FROM decision_records ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap()
        );
    }

    #[test]
    fn nul_in_engine_field_refuses_the_record() {
        let conn = db();
        let i = input("eng\u{0}ine", "ok");
        assert!(record_decision(&conn, &i).is_none(), "NUL refused");
        assert_eq!(list_decisions(&conn, None, 10).unwrap().len(), 0);
    }
}
