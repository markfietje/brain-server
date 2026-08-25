//! The Parcels surfaces: signed site-to-site knowledge, human-gated.
//!
//! - `POST /parcels/export {domain, since?}` — build + sign a parcel of the
//!   domain's approved knowledge (Admin on the domain); the export crossing
//!   is ledgered + audited in-tx.
//! - `POST /parcels/import {domain, parcel{manifest, signature, signed_by},
//!   expected_signer?}` — verify FIRST (fail closed), then land rows as
//!   PENDING proposals; never direct knowledge writes (Write on the domain).
//! - `GET /parcels?domain=` — the bounded parcel ledger view.

use axum::{
    Json,
    extract::{Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::parcels::{self, ParcelError};

fn parcels_err(e: ParcelError) -> HandlerError {
    match e {
        ParcelError::NoOperatorKey => HandlerError::conflict_with(
            "operator_key_missing",
            "no operator signing key — parcels refuse to sign or verify",
            serde_json::json!({ "note": "provision BRAIN_UMP_KEY_DIR to enable Parcels" }),
        ),
        ParcelError::Unsigned => HandlerError::bad_request(
            "parcel_unsigned",
            "the parcel carries no usable signature or signer identity",
        ),
        ParcelError::Tampered(why) => HandlerError::bad_request_with(
            "parcel_tampered",
            "the parcel fails signature verification",
            serde_json::json!({ "why": why }),
        ),
        ParcelError::SignerMismatch { expected, got } => HandlerError::bad_request_with(
            "signer_mismatch",
            "the parcel's signer is not the expected publisher",
            serde_json::json!({ "expected": expected, "got": got }),
        ),
        ParcelError::InvalidInput(what, why) => {
            HandlerError::bad_request("input_invalid", format!("invalid {what}: {why}"))
        }
        ParcelError::TooManyRows(cap) => HandlerError::bad_request(
            "parcel_too_large",
            format!("selection exceeds the {cap}-row parcel cap — narrow the since cursor"),
        ),
        ParcelError::Database(m) => HandlerError::internal(m),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub domain: String,
    #[serde(default)]
    pub since: Option<i64>,
}

/// `POST /parcels/export`
pub async fn post_export(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<ExportRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Admin, "", &body.domain)?;
    // Role gate note: no role preset carries an `export` capability yet, so
    // the Admin scope IS the export gate (documented honest ceiling) — adding
    // a new capability verb would rewrite the role seeds and is not Parcels'
    // scope.
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();

    let bundle_domain = body.domain.clone();
    let bundle = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let bundle =
            parcels::build_parcel(tx.tx(), &bundle_domain, body.since, now).map_err(parcels_err)?;
        parcels::record_export(tx.tx(), &bundle, &actor, now).map_err(parcels_err)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(bundle)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let manifest: serde_json::Value =
        serde_json::from_str(&bundle.manifest_json).unwrap_or(serde_json::json!({}));
    Ok(Json(serde_json::json!({
        "parcel": {
            "manifest": manifest,
            "signature": bundle.signature_hex,
            "signed_by": bundle.signed_by,
        },
        "parcel_hash": bundle.parcel_hash,
        "source_domain": crate::gate::sanitize_read(&body.domain, false, &principal),
        "region": bundle.region,
        "row_count": bundle.row_count,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct ParcelWire {
    pub manifest: serde_json::Value,
    pub signature: String,
    pub signed_by: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub domain: String,
    pub parcel: ParcelWire,
    #[serde(default)]
    pub expected_signer: Option<String>,
}

/// `POST /parcels/import`
pub async fn post_import(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<ImportRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", &body.domain)?;
    let reviewer = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let manifest_json = body.parcel.manifest.to_string();
    let import_domain = body.domain.clone();

    let out = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let out = parcels::import_parcel(
            tx.tx(),
            &parcels::ImportDraft {
                target_domain: &body.domain,
                manifest_json: &manifest_json,
                signature_hex: &body.parcel.signature,
                claimed_signer: &body.parcel.signed_by,
                expected_signer: body.expected_signer.as_deref(),
                reviewer: &reviewer,
                now,
            },
        )
        .map_err(parcels_err)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(out)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    Ok(Json(serde_json::json!({
        "domain": crate::gate::sanitize_read(&import_domain, false, &principal),
        "ledger_id": out.ledger_id,
        "parcel_hash": out.parcel_hash,
        "signed_by": crate::gate::sanitize_read(&out.signer, false, &principal),
        "proposals_created": out.proposals_created.len(),
        "proposal_ids": out.proposals_created,
        "duplicates": out.duplicates,
        "screened_out": out.screened_out,
        "status": "pending_review",
    })))
}

/// `GET /parcels?domain=&limit=&offset=`
pub async fn get_ledger(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".to_string());
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let parse_num = |k: &str| -> Result<Option<i64>, HandlerError> {
        match params.get(k).map(|s| s.parse::<i64>()) {
            Some(Ok(n)) => Ok(Some(n)),
            Some(Err(_)) => Err(HandlerError::bad_request(
                "param_invalid",
                format!("{k} must be an integer"),
            )),
            None => Ok(None),
        }
    };
    let limit = parse_num("limit")?.unwrap_or(200);
    let offset = parse_num("offset")?.unwrap_or(0);

    let ledger_domain = domain.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        parcels::list_ledger(&conn, &ledger_domain, offset, limit).map_err(parcels_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;

    let payload: Vec<serde_json::Value> = rows?
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "direction": r.direction,
                "parcel_hash": r.parcel_hash,
                "signer": crate::gate::sanitize_read(&r.signer, false, &principal),
                "row_count": r.row_count,
                "reviewer": crate::gate::sanitize_read(&r.reviewer, false, &principal),
                "created_at": r.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "domain": crate::gate::sanitize_read(&domain, false, &principal),
        "count": payload.len(),
        "parcels": payload,
    })))
}
