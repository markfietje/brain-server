//! The knowledge-version axis core — the per-domain version a case's
//! `knowledge_version` is recorded against, bumped at publication.
//!
//! Extracted verbatim from `brain-server`'s `service::gate`:
//! the two functions and their SQL moved; the migration and schema, the
//! handler wiring, the audit row, and the base-version constant STAYED in the
//! server. The crate holds no DDL — `knowledge_domain_versions` is created by
//! the server's migration; this core only reads and bumps it, inside the
//! caller's transaction.
//!
//! **The base version is a PARAMETER, not a copy.** This crate does not define
//! the base-version constant — the house law for exactly this shape: the
//! caller supplies the vocabulary it operates against, and this crate holds no
//! copy of its own. A second copy here would be a hand-typed duplicate that
//! drifts silently when the server's base moves.
//!
//! Error `Display` carries the exact pre-move message text (the
//! `Database(String)` convention); the server maps [`EvolveError`] to its own
//! gate error at the call site and wraps it unchanged in its internal-error
//! form.

use rusqlite::Connection;

/// A storage failure. `Database`'s `Display` carries the exact pre-move
/// message text; the server maps it to its own gate error at the call site
/// and wraps it unchanged in its internal-error form.
#[derive(Debug)]
pub enum EvolveError {
    Database(String),
}

impl std::fmt::Display for EvolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvolveError::Database(m) => f.write_str(m),
        }
    }
}

/// Bump the domain of the article named by `article_id`, inside the CALLER'S
/// transaction — the per-domain axis a case's `knowledge_version` is recorded
/// against.
///
/// **The domain is resolved HERE, from the article's own `knowledge.domain`,
/// and never accepted from the caller.** That is deliberate: the publish branch
/// audits under the literal tenant `"global"`, which is an audit label and not a
/// domain, so a caller-supplied domain would let one shared counter wear a
/// per-domain name. Taking the article id alone makes that unrepresentable
/// rather than merely discouraged — there is no argument to get wrong.
///
/// **Monotonic by construction, and by definition.** The KCS state machine has
/// a BACKWARD edge — `retract` moves `published → approved` — so counting state
/// transitions would be confidently wrong: a retraction would tell a reopened
/// case its basis had moved when the world had in fact reverted. This therefore
/// reads the CURRENT version and writes current+1, and callers must invoke it
/// only on a publication, never on a retraction.
///
/// A domain with no row is at `base` — the caller-supplied pre-axis base; this
/// crate holds no copy of that constant — so the first publication of a domain
/// writes `base + 1`, the version that publication itself moved the basis to.
/// This never returns the base version as a result of a bump.
pub fn bump_article_knowledge_version(
    conn: &Connection,
    article_id: i64,
    bumped_by: &str,
    now: i64,
    base: i64,
) -> Result<i64, EvolveError> {
    let domain: String = conn
        .query_row(
            "SELECT domain FROM knowledge WHERE id = ?1",
            rusqlite::params![article_id],
            |r| r.get(0),
        )
        .map_err(|e| EvolveError::Database(format!("article domain read failed: {e}")))?;
    conn.execute(
        "INSERT INTO knowledge_domain_versions(domain, version, bumped_at, bumped_by, bumped_article)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(domain) DO UPDATE SET
             version        = knowledge_domain_versions.version + 1,
             bumped_at      = ?3,
             bumped_by      = ?4,
             bumped_article = ?5",
        rusqlite::params![
            domain,
            base + 1,
            now,
            bumped_by,
            article_id
        ],
    )
    .map_err(|e| EvolveError::Database(format!("domain version bump failed: {e}")))?;
    Ok(current_domain_knowledge_version(conn, &domain, base))
}

/// A domain's CURRENT version, or the base version when it has no row.
///
/// A missing row is NOT version 0: zero would falsely date a never-published
/// domain to "version zero" and make it comparable to the `NULL` sentinel that
/// means "predates tracking". Cross-domain comparison is meaningless by
/// construction; a case's stored version is comparable to its OWN domain's
/// current version and to nothing else.
pub fn current_domain_knowledge_version(conn: &Connection, domain: &str, base: i64) -> i64 {
    conn.query_row(
        "SELECT version FROM knowledge_domain_versions WHERE domain = ?1",
        rusqlite::params![domain],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::EvolveError;

    /// The error's `Display` carries the EXACT inner message — the pre-move
    /// convention, where the server wraps `e.to_string()` unchanged in its
    /// internal-error form. Any decoration here would silently change every
    /// internal-error body the bump can produce.
    #[test]
    fn evolve_error_display_carries_the_exact_message() {
        let e = EvolveError::Database("domain version bump failed: boom".into());
        assert_eq!(e.to_string(), "domain version bump failed: boom");
    }

    /// The crate holds NO copy of the base version — the caller supplies it.
    /// A second declaration here would be the hand-typed duplicate the
    /// no-second-copy law forbids, and it would drift silently when the
    /// server's base moves. (The DB-dependent behavioural pins — two domains
    /// bumping independently, monotonicity, retract-does-not-bump, the
    /// rollback twin — live in the server's `tests/r61p_per_domain_axis.rs`:
    /// they need the server's migration, and the crate holds no DDL.)
    #[test]
    fn the_crate_declares_no_base_version_of_its_own() {
        // Scan the PRODUCTION region only — the house `.split("#[cfg(test)]")`
        // idiom — so the pin does not fire on its own assertion text.
        let src = include_str!("lib.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        assert!(
            !production.contains("pub const KNOWLEDGE_BASE_VERSION"),
            "the base must stay a PARAMETER. The crate must not declare its own \
             base-version constant — the caller supplies the vocabulary."
        );
    }
}
