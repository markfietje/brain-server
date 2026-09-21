//! The curated legal-rules surface: a read-only window over the DPO's
//! quarterly import. The file location rides `BRAIN_LEGAL_DB_PATH`; unset or
//! unreadable is a NAMED refusal, nothing here ever writes the file, and the
//! law-version vocabulary itself stays single-owned by the SDK's policy
//! table.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use std::sync::Arc;

use super::HandlerError;
use crate::AppState;
use legal_rules_db::db::LegalDbError;

/// `GET /legal/rules?since=<law_version>` — the deterministic diff of the
/// curated rules newer than the pinned version's snapshot point (absent
/// `since` = the full ordered snapshot). Admin gate + the DPO role (the
/// scoreboard posture): the surface is DPO evidence, not a public read. The
/// DB opens READ-ONLY per request — hot-reload is exactly "the next request
/// reads the current file"; population is the DPO's import, never a route.
pub async fn get_rules(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Query(q): Query<RulesQuery>,
) -> Result<Json<legal_rules_db::db::DiffResponse>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let path = match std::env::var("BRAIN_LEGAL_DB_PATH") {
        Ok(p) if !p.trim().is_empty() => std::path::PathBuf::from(p),
        _ => {
            return Err(HandlerError::internal_with(
                "legal_db_unconfigured",
                "BRAIN_LEGAL_DB_PATH is unset — the curated legal DB is not configured; the DPO imports it per docs/legal-db-import.md",
                StatusCode::NOT_IMPLEMENTED,
            ));
        }
    };
    let since = q.since;
    let diff = tokio::task::spawn_blocking(
        move || -> Result<legal_rules_db::db::DiffResponse, LegalDbError> {
            let conn = legal_rules_db::db::open_readonly(&path)?;
            legal_rules_db::db::diff_since(&conn, since.as_deref())
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(legal_refusal)?;
    Ok(Json(diff))
}

/// Every legal-DB failure surfaces as a NAMED refusal — never a bare 500, a
/// silent empty diff, or an auto-created file.
fn legal_refusal(e: LegalDbError) -> HandlerError {
    match e {
        LegalDbError::UnknownLawVersion(v) => HandlerError::internal_with(
            "law_version_unknown",
            format!("unknown law_version: {v}"),
            StatusCode::NOT_FOUND,
        ),
        other => HandlerError::internal_with(
            "legal_db_unavailable",
            other.to_string(),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct RulesQuery {
    pub since: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LEGAL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn legal_env_lock() -> std::sync::MutexGuard<'static, ()> {
        LEGAL_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Set + restore one env var (SAFETY: single-threaded under the lock).
    struct EnvGuard(&'static str);
    impl EnvGuard {
        fn set(name: &'static str, value: &std::path::Path) -> EnvGuard {
            // SAFETY: single-threaded under LEGAL_ENV_LOCK.
            unsafe { std::env::set_var(name, value) };
            EnvGuard(name)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: single-threaded under LEGAL_ENV_LOCK.
            unsafe { std::env::remove_var(self.0) };
        }
    }

    fn temp_legal_db(tag: &str) -> std::path::PathBuf {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "brain-legal-route-{}-{}-{}.db",
            tag,
            std::process::id(),
            n
        ));
        let conn = rusqlite::Connection::open(&path).expect("open writable legal db");
        legal_rules_db::db::seed(&conn, "dpo-test", 1_700_000_000).expect("seed");
        path
    }

    async fn get_rules_bytes(
        state: &Arc<AppState>,
        since: Option<String>,
    ) -> Result<String, HandlerError> {
        let response = get_rules(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Query(RulesQuery { since }),
        )
        .await?;
        serde_json::to_string(&response.0).map_err(|e| HandlerError::internal(format!("{e}")))
    }

    /// The unset-path law: a NAMED refusal before any read; nothing else on
    /// the server is affected (this route only).
    #[test]
    fn unset_path_is_a_named_refusal_before_any_read() {
        let _lock = legal_env_lock();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(unset_path_body());
        // SAFETY: single-threaded under LEGAL_ENV_LOCK.
        unsafe { std::env::remove_var("BRAIN_LEGAL_DB_PATH") };
    }

    async fn unset_path_body() {
        let f = crate::handlers::case_run::tests::fixture();
        // SAFETY: single-threaded under LEGAL_ENV_LOCK.
        unsafe { std::env::remove_var("BRAIN_LEGAL_DB_PATH") };
        let err = get_rules_bytes(&f.state, None).await.unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
        assert!(
            err.inner.code.contains("legal_db_unconfigured"),
            "the refusal must be named: {err:?}"
        );
        // A path that does not exist refuses named too (never auto-created).
        let _g = EnvGuard::set(
            "BRAIN_LEGAL_DB_PATH",
            std::path::Path::new("/nonexistent/legal.db"),
        );
        let err = get_rules_bytes(&f.state, None).await.unwrap_err();
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.inner.code.contains("legal_db_unavailable"));
    }

    /// The C2 contract at the route layer: same DB state + same `since` →
    /// byte-identical body, twice; an unknown `since` is a named 404.
    #[test]
    fn law_version_diff_is_reproducible() {
        let _lock = legal_env_lock();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(diff_reproducible_body());
    }

    async fn diff_reproducible_body() {
        let f = crate::handlers::case_run::tests::fixture();
        let path = temp_legal_db("reproducible");
        let _g = EnvGuard::set("BRAIN_LEGAL_DB_PATH", &path);
        let body1 = get_rules_bytes(&f.state, Some("npc-advisory-2024-04".to_string()))
            .await
            .expect("first diff");
        let body2 = get_rules_bytes(&f.state, Some("npc-advisory-2024-04".to_string()))
            .await
            .expect("second diff");
        assert_eq!(body1, body2, "same since, byte-identical response, twice");
        assert!(body1.contains("\"since\":\"npc-advisory-2024-04\""));
        assert!(
            !body1.contains("\"subject\":\"dsar\""),
            "the zero-date curated snapshot rows are not 'newer' than a zero-date pin: {body1}"
        );
        assert!(
            body1.contains("\"subject\":\"vat\""),
            "the dated VAT rules are newer than the snapshot point: {body1}"
        );
        // Unknown since → named 404.
        let err = get_rules_bytes(&f.state, Some("not-a-version".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.inner.code.contains("law_version_unknown"));
    }

    /// An import newer than the pin lands in the diff — the honest
    /// "what changed" surface a buyer's DPO reads.
    #[test]
    fn diff_reports_rules_newer_than_the_pinned_version() {
        let _lock = legal_env_lock();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(diff_import_body());
    }

    async fn diff_import_body() {
        let f = crate::handlers::case_run::tests::fixture();
        let path = temp_legal_db("import");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            let rule = legal_rules_db::Rule {
                id: 0,
                law_version: brain_engine_sdk::policy::law_version_for("eu")
                    .unwrap()
                    .to_string(),
                jurisdiction: "eu".to_string(),
                subject: "dsar".to_string(),
                rule_key: "eu_dsar_curated".to_string(),
                body: "GDPR (Art 12/15/17), amended".to_string(),
                source_ref: "GDPR (Art 12/15/17)".to_string(),
                effective_at: 1_800_000_000,
                reviewed_at: Some(1_800_000_000),
                expires_at: None,
                revision: 2,
                superseded_by: None,
                created_at: 1_800_000_000,
            };
            legal_rules_db::db::insert_jurisdiction_rule(&conn, &rule, Some(30), &["access"])
                .unwrap();
            legal_rules_db::db::pin_head(&conn).unwrap();
        }
        let _g = EnvGuard::set("BRAIN_LEGAL_DB_PATH", &path);
        let body = get_rules_bytes(&f.state, Some("gdpr-consolidated-2021".to_string()))
            .await
            .expect("diff");
        assert!(
            body.contains("GDPR (Art 12/15/17), amended"),
            "the imported revision is the diff: {body}"
        );
    }
}
