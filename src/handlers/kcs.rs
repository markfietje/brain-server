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
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// Freshness review horizon set at approval/publish — the shared constant.
const KCS_FRESHNESS_SECS: i64 = crate::workflow::kcs::KCS_FRESHNESS_SECS;

#[derive(serde::Deserialize)]
pub(crate) struct TranslateRequest {
    pub knowledge_id: i64,
    pub locale: String,
    pub title: String,
    pub body_md: String,
}

/// `POST /kcs/translate` — file a pending `kcs_translate` HITL proposal
/// (role `workflow`). Translation is a HUMAN act: the tool files, only an
/// approval promotes. Audited inside the tx.
pub(crate) async fn post_kcs_translate(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<TranslateRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    crate::handlers::authorize_role(&principal, &pool, "workflow")?;
    let translator = super::recall::principal_label(&principal);
    let proposal_id = tokio::task::spawn_blocking(move || -> Result<i64, rusqlite::Error> {
        let draft = crate::workflow::kcs::TranslationDraft {
            knowledge_id: body.knowledge_id,
            locale: &body.locale,
            title: &body.title,
            body_md: &body.body_md,
            translator: &translator,
        };
        let mut conn = pool
            .get()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("pool: {e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("tx: {e}")))?;
        let now = chrono::Utc::now().timestamp();
        let id = crate::workflow::kcs::propose_translation(tx.tx(), &draft, now)?;
        crate::audit::record_tenant(
            tx.tx(),
            crate::audit::AuditKind::Workflow,
            &translator,
            &format!("proposal/{id}"),
            crate::audit::AuditStatus::Ok,
            format!(
                "workflow/kcs/translate article:{} locale:{}",
                body.knowledge_id, body.locale
            )
            .as_str(),
            "global",
        );
        tx.commit()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("commit: {e}")))?;
        Ok(id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(|e| HandlerError::bad_request("translation_invalid", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "proposal_id": proposal_id,
        "status": "pending",
        "kind": crate::workflow::kcs::KIND_TRANSLATE,
    })))
}

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
        let (domain, kcs_state): (String, String) =
            crate::workflow::kcs::article_lifecycle_row(&conn, id)
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
        let n = crate::workflow::kcs::approve_article(&conn, id, now + KCS_FRESHNESS_SECS)
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
    // THE RECORD GATE. This listing applied a domain predicate but no
    // `(owner, access_scope)` predicate, so a role-less read token — which the
    // composite gate makes owner-bound on `/recall`, `/search`, `/get`,
    // `/multi-get`, `/verify` and `/graph/*` — saw every KCS-tracked row's
    // title plus 200 characters of body here. The gate is resolved OUTSIDE the
    // blocking task (it reads the role store through the pool) and bound INTO
    // the core's SQL, because rows must be narrowed before they are read.
    let record_gate = super::gate::record_read_gate(&principal, &state.pool);
    let gate_scopes = record_gate.access_scopes.clone();
    let gate_owners = record_gate.owner_in.clone();
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let rows: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
        let Ok(conn) = pool.get() else { return vec![] };
        let Ok(rows) = crate::workflow::kcs::article_worklist(
            &conn,
            gate_scopes.as_deref(),
            gate_owners.as_deref(),
        ) else {
            return vec![];
        };
        let now = chrono::Utc::now().timestamp();
        let mut out = Vec::new();
        for (id, title, content, kcs_state, fresh_due, domain, flags, pii_flag) in rows {
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
            // The row's OWN pii flag. It was hardcoded `false`, and
            // `redact_content` short-circuits on `!pii` — so the mask could not
            // run even for a row the scanner had flagged, and the corpus's
            // KCS content went out unmasked to every non-admin reader.
            out.push(serde_json::json!({
                "id": id,
                "title": title
                    .as_deref()
                    .map(|t| crate::gate::sanitize_read(t, pii_flag != 0, &principal)),
                "snippet": crate::gate::sanitize_read(&content, pii_flag != 0, &principal)
                    .chars()
                    .take(200)
                    .collect::<String>(),
                "kcs_state": kcs_state,
                "freshness_review_due": fresh_due,
                "open_flags": flags,
                "stale": stale,
            }));
        }
        // Approved translations whose source revision advanced
        // past their pinned `based_revision` land on the SAME worklist — one
        // freshness discipline, no second mechanism. Only when the caller is
        // asking for stale rows (the content-health view).
        // Same two gates as the worklist, and the JSON + read seam are built
        // HERE with the caller's principal. The projection used to be built in
        // the core with `sanitize_read(..., &None)` — the loopback posture,
        // which disables the PII arm — and it was `extend`ed into `out` AFTER
        // the per-row `can_read_domain` loop, so that loop never touched it.
        if want_stale
            && let Ok(trs) = crate::workflow::kcs::stale_translations(
                &conn,
                None,
                gate_scopes.as_deref(),
                gate_owners.as_deref(),
            )
        {
            // THE MISSING PER-ROW DOMAIN FILTER. `None` above means
            // the core scans every domain; the worklist's `can_read_domain`
            // loop never saw these rows because they were extended AFTER it.
            // A principal scoped to one tenant therefore saw every domain's
            // stale translation title. Filter each row here, before it is
            // serialized.
            out.extend(
                trs.into_iter()
                    .filter(|t| super::can_read_domain(&principal, &t.domain))
                    .map(|t| {
                        serde_json::json!({
                            "translation_id": t.translation_id,
                            "article_id": t.article_id,
                            "locale": t.locale,
                            "title": crate::gate::sanitize_read(
                                &t.title,
                                crate::gate::pii_flag("", &t.title),
                                &principal
                            ),
                            "domain": t.domain,
                            "kind": "translation_stale",
                        })
                    }),
            );
        }
        out
    })
    .await
    .unwrap_or_default();
    Ok(Json(serde_json::json!({ "articles": rows })))
}

/// The Beacon publish request: `action` defaults to `publish`; `retract` is
/// the operational rollback (state back to `approved`, next build drops the
/// page). Both travel through the SAME HITL proposal — publishing is a human
/// decision, never an API side effect.
#[derive(Debug, serde::Deserialize)]
pub struct PublishBody {
    #[serde(default)]
    pub public_slug: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
}

/// `POST /kcs/articles/{id}/publish` — create a pending `kcs_publish`
/// proposal (Write gate on the row's domain; the approval itself demands the
/// `approve` + `publish` capabilities in the gate). Audited.
pub async fn post_kcs_article_publish(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<PublishBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let action = body.action.as_deref().unwrap_or("publish").to_string();
    let action_label = action.clone();
    if !matches!(action.as_str(), "publish" | "retract") {
        return Err(HandlerError::bad_request(
            "action_invalid",
            "action must be publish or retract",
        ));
    }
    if action == "publish" {
        let Some(slug) = body.public_slug.as_deref() else {
            return Err(HandlerError::bad_request(
                "public_slug_required",
                "publish requires a public_slug",
            ));
        };
        if !crate::kb::is_valid_slug(slug) {
            return Err(HandlerError::bad_request(
                "public_slug_invalid",
                "slug must be lowercase alnum + hyphen, no leading/trailing/doubled hyphen",
            ));
        }
    }
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let actor = super::recall::principal_label(&principal);
    let outcome: Result<i64, HandlerError> = tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let (domain, kcs_state): (String, String) =
            crate::workflow::kcs::article_lifecycle_row(&conn, id)
                .map_err(|e| HandlerError::internal(format!("{e}")))?
                .ok_or_else(|| HandlerError::not_found("article not found"))?;
        // Row-domain re-auth. Proposing needs Write only — the publish
        // capability is enforced at APPROVAL time, where it belongs.
        super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
        let want_state = if action == "publish" {
            "approved"
        } else {
            "published"
        };
        if kcs_state != want_state {
            return Err(HandlerError::conflict_with(
                "kcs_state_invalid",
                format!("{action} requires state {want_state} (article is {kcs_state})"),
                serde_json::json!([]),
            ));
        }
        let payload = serde_json::json!({
            "knowledge_id": id,
            "public_slug": body.public_slug,
            "action": action,
        });
        let now = chrono::Utc::now().timestamp();
        let pid = crate::workflow::kcs::file_publish_proposal(&conn, &payload.to_string(), now)
            .map_err(HandlerError::internal)?;
        crate::audit::record_tenant(
            &conn,
            crate::audit::AuditKind::Workflow,
            actor.trim(),
            &format!("article:{id}"),
            crate::audit::AuditStatus::Ok,
            &format!("workflow/kcs/{action} proposed"),
            &domain,
        );
        Ok(pid)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let proposal_id = outcome?;
    Ok(Json(serde_json::json!({
        "proposal_id": proposal_id,
        "knowledge_id": id,
        "action": action_label,
        "status": "pending",
    })))
}

/// `GET /kcs/articles/{id}/preview` — the EXACT sanitized public page for an
/// approved/published article, rendered by the same function the build uses.
/// What you approve is byte-identical to what ships.
pub async fn get_kcs_article_preview(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    // Explicit gate (guard-table contract); the row-domain filter below is
    // defense-in-depth for principals scoped to non-global domains.
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let html: Option<(String, String)> =
        tokio::task::spawn_blocking(move || -> Option<(String, String)> {
            let conn = pool.get().ok()?;
            let (domain, kcs_state, slug, title, content, created_at, origin, hash) =
                crate::workflow::kcs::publishable_article_row(&conn, id).ok()??;
            if !super::can_read_domain(&principal, &domain) {
                return None;
            }
            let slug = if slug.is_empty() {
                format!("preview-{id}")
            } else {
                slug
            };
            // The preview IS the public page: same strict seam as the build
            // (unconditional PII redact + invisible strip + markdown-ref strip).
            let article = crate::kb::KbArticle {
                id,
                slug,
                title: {
                    let t = if title.is_empty() {
                        format!("Article {id}")
                    } else {
                        title
                    };
                    crate::kb::sanitize_public(&t)
                },
                body: crate::kb::sanitize_public(&content),
                updated_at: created_at,
                origin: origin.as_deref().map(crate::kb::sanitize_public),
                revision: hash
                    .as_deref()
                    .map(crate::kb::sanitize_public)
                    .unwrap_or_default(),
            };
            Some((kcs_state, crate::kb::render_article_page(&article, None)))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let (kcs_state, page) =
        html.ok_or_else(|| HandlerError::not_found("no approved/published article with that id"))?;
    Ok(Json(serde_json::json!({
        "id": id,
        "kcs_state": kcs_state,
        "public_html": page,
    })))
}
