//! The KCS article lifecycle surfaces (the Evolve release).
//!
//! - `POST /kcs/articles/{id}/approve` — the draft → approved transition
//!   (role `approve`, row-domain re-auth, audited in the same tx). Sets the
//!   freshness-review deadline. Publishing is Beacon's, later.
//! - `GET /kcs/articles?state=&stale=1` — the content-health worklist:
//!   every signal (stale freshness, open improve flags) in one list.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use rusqlite::OptionalExtension;
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// Freshness review horizon set at approval. Constant for now — a per-domain
/// retention-policy lookup is the documented ceiling.
const KCS_FRESHNESS_SECS: i64 = 90 * 24 * 3600;

pub async fn post_kcs_article_approve(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let actor = super::recall::principal_label(&principal);
    let outcome: Result<i64, HandlerError> = tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let (domain, kcs_state): (String, String) = conn
            .query_row(
                "SELECT domain, kcs_state FROM knowledge WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| HandlerError::internal(format!("{e}")))?
            .ok_or_else(|| HandlerError::not_found("article not found"))?;
        // Row-domain re-auth + the HITL approve role gate.
        super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
        super::authorize_role(&principal, &pool, "approve")?;
        if kcs_state != "draft" {
            return Err(HandlerError::conflict_with(
                "kcs_state_invalid",
                format!("only draft articles can be approved (state: {kcs_state})"),
                serde_json::json!([]),
            ));
        }
        let now = chrono::Utc::now().timestamp();
        let n = conn
            .execute(
                "UPDATE knowledge SET kcs_state = 'approved', freshness_review_due = ?2
                 WHERE id = ?1 AND kcs_state = 'draft'",
                rusqlite::params![id, now + KCS_FRESHNESS_SECS],
            )
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        if n == 0 {
            return Err(HandlerError::conflict("article state changed concurrently"));
        }
        // Same-tx audit evidence (autocommit single write here).
        crate::audit::record_tenant(
            &conn,
            crate::audit::AuditKind::Workflow,
            actor.trim(),
            &format!("article:{id}"),
            crate::audit::AuditStatus::Ok,
            "kcs/approve",
            &domain,
        );
        Ok(now + KCS_FRESHNESS_SECS)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let due = outcome?;
    Ok(Json(serde_json::json!({
        "id": id,
        "kcs_state": "approved",
        "freshness_review_due": due,
    })))
}

pub async fn get_kcs_articles(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let want_state = params.get("state").cloned();
    if let Some(s) = &want_state
        && !matches!(s.as_str(), "draft" | "approved" | "published")
    {
        return Err(HandlerError::bad_request(
            "state_invalid",
            "state must be draft, approved, or published",
        ));
    }
    let want_stale = params.contains_key("stale");
    // Explicit gate (guard-table contract); the per-row domain filter below
    // is defense-in-depth for principals scoped to non-global domains.
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let rows: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(mut stmt) = conn.prepare(
            "SELECT k.id, k.title, k.content, k.kcs_state, k.freshness_review_due, k.domain,
                    (SELECT COUNT(*) FROM findings f
                      WHERE f.claim = 'kcs_flag' AND f.evidence = 'article:' || k.id) AS open_flags
             FROM knowledge k
             WHERE k.kcs_state != 'none'
             ORDER BY k.id DESC LIMIT 500",
        ) else {
            return vec![];
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
            ))
        }) else {
            return vec![];
        };
        let now = chrono::Utc::now().timestamp();
        let mut out = Vec::new();
        for (id, title, content, kcs_state, fresh_due, domain, flags) in it.flatten() {
            if !super::can_read_domain(&principal, &domain) {
                continue;
            }
            if let Some(want) = &want_state
                && *want != kcs_state
            {
                continue;
            }
            let stale = flags > 0 || fresh_due.is_some_and(|d| d < now);
            if want_stale && !stale {
                continue;
            }
            out.push(serde_json::json!({
                "id": id,
                "title": title
                    .as_deref()
                    .map(|t| crate::gate::sanitize_read(t, false, &principal)),
                "snippet": crate::gate::sanitize_read(&content, false, &principal)
                    .chars()
                    .take(200)
                    .collect::<String>(),
                "kcs_state": kcs_state,
                "freshness_review_due": fresh_due,
                "open_flags": flags,
                "stale": stale,
            }));
        }
        out
    })
    .await
    .unwrap_or_default();
    Ok(Json(serde_json::json!({ "articles": rows })))
}
