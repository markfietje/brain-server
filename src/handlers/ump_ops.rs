//! the HTTP ops binding (`/ump/*` + `/.well-known/ump.json`).
//! Handler layer over the record codec (`ump.rs`); every operation delegates to
//! an existing engine (`run_recall`, `ingest_one`, `resolve_supersession`,
//! `purge_chunk_ids`, `record_feedback`, the audit chain) — this file adds no
//! new storage logic, only the UMP 1.0 wire contract (§3/§4.2).

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, KeepAliveStream, Sse};
use axum::Json;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::StreamExt;

use crate::handlers::auth::{OptCapability, OptPrincipal};
use crate::handlers::gate::{load_knowledge_row, principal_to_owner};
use crate::handlers::ingest::{ingest_one, lower_ump};
use crate::handlers::recall::{RecallRequest, RecallSourceQuery};
use crate::handlers::suggest::record_feedback;
use crate::handlers::ump::{self, UmpMeta};
use crate::handlers::{audit_scope, HandlerError};
use crate::AppState;

/// §3.1 `max_recall`: the UMP-side cap on a recall request. Callers are
/// clamped, never rejected — the brain's own `MAX_LIMIT` stays authoritative.
pub const MAX_RECALL: u32 = 50;

/// §3.1: the UMP 1.0 negotiation handshake (also served at
/// `/.well-known/ump.json`). `conformance` reflects operator-key presence:
/// L3 (signed) when a key is configured, L2 (hash-only integrity) otherwise.
pub fn capabilities_payload() -> Value {
    let conformance = if ump::operator_signing_key().is_some() {
        "L3"
    } else {
        "L2"
    };
    json!({
        "server": { "name": "brain-server", "version": env!("CARGO_PKG_VERSION") },
        "ump": "1.0",
        "conformance": conformance,
        "kinds": ["semantic", "episodic", "procedural", "working", "identity"],
        "bindings": ["http", "mcp", "file"],
        "retrieval_signals": ["similarity", "recency", "salience", "scope_match", "provenance_depth"],
        "max_recall": MAX_RECALL,
        "writable": true,
        "audit": true,
    })
}

pub async fn capabilities() -> Json<Value> {
    Json(capabilities_payload())
}

/// record a `Denied` auth audit row for a §3.7 consent mismatch.
/// Extractable for a test (pass an on-disk/in-memory conn); the handler uses a
/// fresh connection to the live DB. Best-effort: a failure is ignored so a
/// missing audit log never fails the request.
fn record_forbidden_scope(
    conn: &rusqlite::Connection,
    principal_sub: &str,
    declared_owner: &str,
) -> bool {
    let detail = format!("ump.remember scope.owner={declared_owner} does not match principal");
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Auth,
        principal_sub,
        &detail,
        crate::audit::AuditStatus::Denied,
        "api",
    )
    .is_some()
}

/// §3.3 `remember` — lower a partial record through the structured-ingest
/// path (`ingest_one`), with the §3.7 consent/signature gates applied first.
pub async fn remember(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(body): Json<Value>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    super::cap_gate(&cap.0, "write")?;
    // §3.3: a partial record; tolerate both `{record: …}` and a bare record.
    let rec = body.get("record").unwrap_or(&body);
    let (req, mut meta) =
        lower_ump(rec).map_err(|e| HandlerError::bad_request("invalid_record", e))?;
    // §3.7 consent: a declared `scope.owner` must equal the principal's
    // identity; absent → owned by the principal. (`None` principal = loopback
    // superuser: the record keeps its declared owner.)
    if let Some(owner) = principal_to_owner(&principal.0) {
        if let Some(declared) = &meta.owner {
            if declared != &owner {
                // authorize()-style consent denials on the UMP
                // surface must be audited like any other authz denial
                // (COMPLIANCE.md §3.5 promised this; it never happened). Fresh
                // connection, best-effort — a missing audit log must not fail
                // the request. Detail carries only the declared owner (it is
                // the handler's own identity assertion, not a secret).
                if let Ok(audit_conn) = rusqlite::Connection::open(crate::config::brain_db_path()) {
                    let _ = record_forbidden_scope(&audit_conn, &owner, declared);
                }
                return Err(HandlerError::bad_request_with(
                    "forbidden_scope",
                    "record scope.owner does not match the authenticated principal",
                    json!({ "declared": declared, "principal": owner }),
                ));
            }
        } else {
            meta.owner = Some(owner);
        }
    }
    // §5.3 L3: a signed record must verify against the operator key before
    // it is stored. (Hash-only records always pass — L2.) Both the reference
    // (`signature`/`signer`/`content_hash`) and legacy v1.17.3 (`sig`/`key`/
    // `hash`) integrity shapes count as signed; `verify_record` dual-reads.
    if rec["integrity"]["signature"].is_string() || rec["integrity"]["sig"].is_string() {
        let Some((_, sk)) = ump::operator_signing_key() else {
            return Err(HandlerError::bad_request(
                "signature_invalid",
                "signed record rejected: no operator key configured (L2 mode)",
            ));
        };
        let pk = sk.verifying_key().to_bytes();
        if !ump::verify_record(rec, Some(&pk)) {
            return Err(HandlerError::bad_request(
                "signature_invalid",
                "record signature does not verify",
            ));
        }
    }
    let resp = ingest_one(&state, &principal.0, req).await?;
    // §3.3 `created | merged | rejected` — brain's dedup reports "duplicate".
    let result = match resp.status {
        "created" => "created",
        "duplicate" => "merged",
        _ => "rejected",
    };
    let id = stored_ump_id(&state.pool, resp.id).await?;
    publish(&state, "remember", resp.id);
    Ok(Json(json!({ "id": id, "result": result })))
}

/// Resolve a UMP id to the numeric row id. Plain integers pass through;
/// `urn:ump:` content-addressed ids resolve via the indexed `ump_id` column
/// (ingest computes them from `domain \0 content`, so the id a peer sends
/// round-trips exactly); the legacy `urn:ump:brain:<domain>:<id>` shape
/// resolves by its trailing numeric id. Unknown ids are 404s.
fn resolve_row_id(conn: &rusqlite::Connection, id: &str) -> Result<i64, HandlerError> {
    if let Ok(n) = id.parse::<i64>() {
        return Ok(n);
    }
    if let Some(rid) = conn
        .query_row(
            "SELECT id FROM knowledge WHERE ump_id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| HandlerError::internal(format!("resolve ump_id failed: {e}")))?
    {
        return Ok(rid);
    }
    if let Some(tail) = id.rsplit(':').next().and_then(|t| t.parse::<i64>().ok()) {
        return Ok(tail);
    }
    Err(HandlerError::not_found(format!("no chunk with id {id}")))
}

/// The UMP urns of the chunks that superseded this one (L2 bi-temporal
/// `superseded_by`): `supersedes` evidence links pointing AT this chunk,
/// resolved to the successor's content-addressed id. Empty when current.
fn superseded_by_for(conn: &rusqlite::Connection, id: i64) -> Result<Vec<String>, HandlerError> {
    let mut stmt = conn
        .prepare(
            "SELECT k.ump_id FROM evidence_links el
               JOIN knowledge k ON k.id = el.from_chunk
              WHERE el.kind = 'supersedes' AND el.to_chunk = ?1
                AND k.ump_id IS NOT NULL AND k.ump_id != ''
              ORDER BY el.id",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| HandlerError::internal(e.to_string()))
}

/// Resolve a row's stored UMP record id: the persisted `ump_meta.origin` URN
/// (peer-authored ids round-trip), else the content-addressed `ump_id` column,
/// else the legacy row-based `urn:ump:brain:…` shape for rows written before
/// the overlay existed.
async fn stored_ump_id(pool: &crate::Pool, id: i64) -> Result<String, HandlerError> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || -> Result<String, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let row = load_knowledge_row(&conn, id)
            .map_err(|e| HandlerError::internal(e.to_string()))?
            .ok_or_else(|| HandlerError::not_found(format!("no chunk with id {id}")))?;
        let meta = UmpMeta::parse(row["ump_meta"].as_str());
        if let Some(origin) = &meta.origin {
            return Ok(origin.clone());
        }
        if let Some(ump_id) = row["ump_id"].as_str().filter(|s| !s.is_empty()) {
            return Ok(ump_id.to_string());
        }
        Ok(ump::record_id("global", id, row["content_hash"].as_str()))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?
}

/// sanitize a knowledge row's stored-text fields
/// for the *interactive* UMP read surface — the same read boundary the HTTP
/// recall/search surface applies. The export path (`render_ump` in gate.rs)
/// deliberately stays byte-faithful (DSAR portability), so this helper is only
/// used by `/ump/memory/{id}` + `/ump/recall`. A later pass extends the field set
/// to `assertion_kind` (free text at ingest — the one text column the v1.27.14
/// pass missed). It mutates a CLONE of the row, then `emit_record` hashes the
/// sanitized text — so the emitted record's `integrity.content_hash` stays
/// self-consistent and `verify_record` still passes (§2.8/§5.3 unaffected).
fn sanitize_ump_row_for_read(row: Value) -> Value {
    let mut row = row;
    let none: Option<crate::auth::Principal> = None;
    for field in ["content", "title", "source", "assertion_kind"] {
        if let Some(v) = row[field].as_str() {
            row[field] = json!(crate::gate::sanitize_stored(v, false, &none));
        }
    }
    row
}

/// the `ump_meta` overlay is client-controlled at
/// `/ump/remember` and re-emitted verbatim — its string fields and every
/// string leaf of the free-form `provenance`/`consent` JSON are stored text on
/// an LLM-facing surface, so they pass the same read seam. The RAW owner still
/// drives the redact decision before this runs (a sanitized owner must not
/// change who sees what).
fn sanitize_ump_meta_for_read(meta: &ump::UmpMeta) -> ump::UmpMeta {
    let none: Option<crate::auth::Principal> = None;
    let s = |o: &Option<String>| {
        o.as_ref()
            .map(|v| crate::gate::sanitize_stored(v, false, &none))
    };
    ump::UmpMeta {
        kind: s(&meta.kind),
        owner: s(&meta.owner),
        visibility: s(&meta.visibility),
        origin: s(&meta.origin),
        provenance: meta.provenance.as_ref().map(sanitize_json_strings),
        consent: meta.consent.as_ref().map(sanitize_json_strings),
    }
}

/// Recursively apply the read seam to every string leaf of a stored JSON blob
/// (`provenance` is arbitrary client JSON). Structure is preserved verbatim.
fn sanitize_json_strings(v: &Value) -> Value {
    match v {
        Value::String(s) => {
            let none: Option<crate::auth::Principal> = None;
            json!(crate::gate::sanitize_stored(s, false, &none))
        }
        Value::Array(a) => Value::Array(a.iter().map(sanitize_json_strings).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, val)| (k.clone(), sanitize_json_strings(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `GET /ump/memory/{id}` — one record by numeric row id OR `urn:ump:…`
/// content-addressed id (the form `/ump/remember` returns and peers send),
/// integrity-verified on read (§5.3); a row whose stored integrity no longer
/// verifies is treated as absent.
pub async fn get_memory(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Path(id): Path<String>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    super::cap_gate(&cap.0, "read")?;
    let owner = principal_to_owner(&principal.0);
    let signer = ump::operator_signing_key();
    let pk: Option<[u8; 32]> = signer.as_ref().map(|(_, sk)| sk.verifying_key().to_bytes());
    let id_arg = id.clone();
    let pool = state.pool.clone();
    let record = tokio::task::spawn_blocking(move || -> Result<Option<Value>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let rid = resolve_row_id(&conn, &id_arg)?;
        let Some(row) =
            load_knowledge_row(&conn, rid).map_err(|e| HandlerError::internal(e.to_string()))?
        else {
            return Ok(None);
        };
        let meta = UmpMeta::parse(row["ump_meta"].as_str());
        // §2.7: a principal sees other owners' rows redacted (mirrors export).
        // Owned copy: `sanitize_ump_row_for_read(row)` moves `row` below.
        let row_owner: Option<String> = row["owner"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| meta.owner.clone());
        let redact = owner.is_some()
            && row_owner
                .as_deref()
                .map(|o| Some(o) != owner.as_deref())
                .unwrap_or(true);
        let superseded = superseded_by_for(&conn, rid)?;
        // interactive read → sanitize a clone of
        // the row before emit, so bidi/ZW/markdown-ref never reach the LLM
        // boundary. `emit_record` hashes the sanitized text (self-consistent).
        // the meta overlay rides the same seam (raw owner already
        // decided `redact` above).
        let rec = ump::emit_record(
            &sanitize_ump_row_for_read(row),
            "global",
            &json!([]),
            &serde_json::Value::Array(relations_for_chunk(&conn, rid)?),
            &sanitize_ump_meta_for_read(&meta),
            redact,
            &superseded,
            signer.as_ref().map(|(did, sk)| (did.as_str(), sk)),
        );
        if !ump::verify_record(&rec, pk.as_ref()) {
            return Ok(None);
        }
        Ok(Some(rec))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    record
        .map(|rec| Json(json!({ "record": rec })))
        .ok_or_else(|| HandlerError::not_found(format!("no chunk with id {id}")))
}

/// The chunk's outgoing relations as `{from, to, type}` rows (the shape the
/// record engine renders into `about`/typed `body.structured.relations`).
fn relations_for_chunk(conn: &rusqlite::Connection, id: i64) -> Result<Vec<Value>, HandlerError> {
    let mut stmt = conn
        .prepare(
            "SELECT e1.name, e2.name, r.relation_type
               FROM relationships r
               JOIN entities e1 ON r.from_entity_id = e1.id
               JOIN entities e2 ON r.to_entity_id = e2.id
              WHERE r.knowledge_id = ?1",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![id], |r| {
            // entity names arrive from vault
            // wikilinks/frontmatter with no vocabulary gate, and the relation
            // type is linker text — stored text, same seam.
            let none: Option<crate::auth::Principal> = None;
            Ok(json!({
                "from": crate::gate::sanitize_stored(&r.get::<_, String>(0)?, false, &none),
                "to": crate::gate::sanitize_stored(&r.get::<_, String>(1)?, false, &none),
                "type": crate::gate::sanitize_stored(&r.get::<_, String>(2)?, false, &none),
            }))
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| HandlerError::internal(e.to_string()))
}

/// §3.2 `recall` — the shared `run_recall` core rendered as UMP records with
/// the five standard signals per result.
#[derive(Debug, Deserialize)]
pub struct UmpRecallRequest {
    pub query: String,
    #[serde(default = "default_recall_limit")]
    pub limit: u32,
    #[serde(default)]
    pub scope: Option<Value>,
    #[serde(default)]
    pub filter: Option<UmpRecallFilter>,
    /// Accepted for contract completeness; the brain has no rank steering
    /// (`ponytail:` no learned rerank — `prefer` is a no-op).
    #[serde(default)]
    pub ranking_hints: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UmpRecallFilter {
    #[serde(default)]
    pub kind: Vec<String>,
    #[serde(default)]
    pub valid_at: Option<String>,
}

fn default_recall_limit() -> u32 {
    10
}

/// UMP kind → brain `memory_kind` filter (mirrors `brain_kind`'s table).
pub fn filter_kind_to_brain(kind: &str) -> Option<&'static str> {
    match kind {
        "episodic" => Some("episodic"),
        "procedural" => Some("procedure"),
        "semantic" | "working" | "identity" => Some("fact"),
        _ => None,
    }
}

pub async fn recall(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(req): Json<UmpRecallRequest>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    super::cap_gate(&cap.0, "read")?;
    let limit = req.limit.min(MAX_RECALL);
    let filter = req.filter.unwrap_or_default();
    let filter_kind = filter.kind.iter().find_map(|k| filter_kind_to_brain(k));
    let brain_req = RecallRequest {
        query: req.query,
        limit,
        domain: None,
        strict: false,
        provenance: false,
        source: None,
        since: None,
        lex: Default::default(),
        vec: None,
        hyde: None,
        intent: None,
        sources: Vec::new(),
        profile: None,
        include_flagged: false,
        as_of: None,
        evidence: false,
        at: filter.valid_at,
        max_context_tokens: None,
        gold_answer: None,
        graph: false,
        include_decayed: false,
        memory_kind: filter_kind.map(String::from),
        min_relevance: None,
        trace: false,
    };
    let outcome = crate::handlers::recall::run_recall(
        &state,
        &principal.0,
        brain_req,
        RecallSourceQuery::default(),
    )
    .await?;
    let scope_owner = req
        .scope
        .and_then(|s| s["owner"].as_str().map(|o| o.to_string()));
    let owner = principal_to_owner(&principal.0);
    let signer = ump::operator_signing_key();
    let pk: Option<[u8; 32]> = signer.as_ref().map(|(_, sk)| sk.verifying_key().to_bytes());
    let pool = state.pool.clone();
    let now_unix = chrono::Utc::now().timestamp();
    let results = tokio::task::spawn_blocking(move || -> Result<Vec<Value>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut out = Vec::with_capacity(outcome.tagged.len());
        for (r, domain) in &outcome.tagged {
            // §5.3: unverifiable rows are dropped, never served.
            let Some(row) = load_knowledge_row(&conn, r.id)
                .map_err(|e| HandlerError::internal(e.to_string()))?
            else {
                continue;
            };
            let meta = UmpMeta::parse(row["ump_meta"].as_str());
            // Owned copy: `sanitize_ump_row_for_read(row)` moves `row` below, so
            // the owner comparison must not borrow it (F-10 read sanitization).
            let row_owner: Option<String> = row["owner"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| meta.owner.clone());
            let redact = owner.is_some()
                && row_owner
                    .as_deref()
                    .map(|o| Some(o) != owner.as_deref())
                    .unwrap_or(true);
            let superseded = superseded_by_for(&conn, r.id)?;
            let record = ump::emit_record(
                &sanitize_ump_row_for_read(row),
                domain,
                &json!([]),
                &serde_json::Value::Array(relations_for_chunk(&conn, r.id)?),
                &sanitize_ump_meta_for_read(&meta),
                redact,
                &superseded,
                signer.as_ref().map(|(did, sk)| (did.as_str(), sk)),
            );
            if !ump::verify_record(&record, pk.as_ref()) {
                continue;
            }
            // §3.2 signals from the existing telemetry (plan M2): the fused
            // score, decay (recency), stored confidence (salience), owner
            // match, and evidence-link depth.
            let score = r.provenance.fused_score.unwrap_or(r.score);
            let signals = json!({
                "similarity": score,
                "recency": if crate::gate::is_decayed(r.expires_at, now_unix) { 0.0 } else { 1.0 },
                "salience": r.confidence.unwrap_or(0.0),
                "scope_match": match &scope_owner {
                    Some(want) => f32::from(row_owner.as_deref() == Some(want.as_str())),
                    None => 1.0,
                },
                "provenance_depth": r.evidence.as_ref().map(|e| e.links.len() as u32).unwrap_or(0),
            });
            out.push(json!({ "record": record, "signals": signals, "score": score }));
        }
        Ok(out)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(json!({ "results": results })))
}

/// §3.5 `revise` — patch a record: the patched record lowers through the
/// ingest path as a NEW chunk, then `resolve_supersession` expires the old
/// one (default recall returns the new revision; `?at=<past>` still finds
/// the old). `id` is the numeric row id or the `urn:ump:…` content id.
#[derive(Debug, Deserialize)]
pub struct ReviseRequest {
    pub id: String,
    #[serde(default)]
    pub patch: Value,
}

/// Deep merge: object patches recurse; anything else overwrites. `id`/
/// `integrity` in a patch are ignored — the server is authoritative for
/// those (content-addressing + signing are never client-controlled).
fn deep_merge(base: Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(mut b), Value::Object(p)) => {
            for (k, v) in p {
                if k == "id" || k == "integrity" {
                    continue;
                }
                let existing = b.get(k).cloned().unwrap_or(Value::Null);
                b.insert(k.clone(), deep_merge(existing, v));
            }
            Value::Object(b)
        }
        (_, p) => p.clone(),
    }
}

pub async fn revise(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(req): Json<ReviseRequest>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    super::cap_gate(&cap.0, "write")?;
    let id_arg = req.id.clone();
    let owner = principal_to_owner(&principal.0);
    let pool = state.pool.clone();
    let old_id = tokio::task::spawn_blocking(
        move || -> Result<(i64, crate::handlers::ingest::IngestRequest), HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let old_id = resolve_row_id(&conn, &id_arg)?;
            let row = load_knowledge_row(&conn, old_id)
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .ok_or_else(|| HandlerError::not_found(format!("no chunk with id {id_arg}")))?;
            let meta = UmpMeta::parse(row["ump_meta"].as_str());
            let row_owner = row["owner"].as_str().or(meta.owner.as_deref());
            let redact = owner.is_some()
                && row_owner
                    .map(|o| Some(o) != owner.as_deref())
                    .unwrap_or(true);
            let superseded = superseded_by_for(&conn, old_id)?;
            let base = ump::emit_record(
                &row,
                "global",
                &json!([]),
                &serde_json::Value::Array(relations_for_chunk(&conn, old_id)?),
                &meta,
                redact,
                &superseded,
                None,
            );
            let merged = deep_merge(base, &req.patch);
            let (mut req, _meta) =
                lower_ump(&merged).map_err(|e| HandlerError::bad_request("invalid_record", e))?;
            // The revision is a NEW record with its own content-addressed id:
            // drop the carried origin so the new row reports its own urn
            // (otherwise `superseded_by` on the old row points at itself).
            if let Some(meta_json) = &mut req.ump_meta {
                if let Ok(mut m) = serde_json::from_str::<Value>(meta_json) {
                    m.as_object_mut().map(|o| o.remove("origin"));
                    *meta_json = m.to_string();
                }
            }
            Ok((old_id, req))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let (old_id, new_req) = old_id;
    let resp = ingest_one(&state, &principal.0, new_req).await?;
    let new_id = resp.id;
    if new_id != old_id {
        // Expire the old chunk so current recall returns the new revision.
        // `ponytail:` `ingest_one` commits its own transaction first, so the
        // insert + supersession are not one atomic unit — a failure here
        // leaves the new chunk created but unlinked (still retrievable).
        let pool = state.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let now_utc = chrono::Utc::now().to_rfc3339();
            crate::consolidate::resolve_supersession(&tx, new_id, old_id, &now_utc)
                .map_err(|e| HandlerError::internal(format!("supersession failed: {e}")))?;
            tx.commit()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    }
    let supersedes = stored_ump_id(&state.pool, old_id).await?;
    let id = stored_ump_id(&state.pool, new_id).await?;
    publish(&state, "revise", new_id);
    Ok(Json(json!({ "id": id, "supersedes": vec![supersedes] })))
}

/// §3.4 `forget` — soft (quarantine-style flag + tombstone, still retrievable
/// with `include_flagged`) or hard (`purge_chunk_ids` — the v1.14 erase path).
/// `id` is the numeric row id or the `urn:ump:…` content id.
#[derive(Debug, Deserialize)]
pub struct ForgetRequest {
    pub id: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub hard: bool,
}

pub async fn forget(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(req): Json<ForgetRequest>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    super::cap_gate(&cap.0, "write")?;
    let id_arg = req.id.clone();
    let reason = req.reason.unwrap_or_else(|| "ump_forget".to_string());
    let hard = req.hard;
    let pool = state.pool.clone();
    let id = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let id = resolve_row_id(&conn, &id_arg)?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM knowledge WHERE id = ?1)",
                rusqlite::params![id],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| HandlerError::internal(format!("existence check failed: {e}")))?
            != 0;
        if !exists {
            return Err(HandlerError::not_found(format!("no chunk with id {id}")));
        }
        let now = chrono::Utc::now().timestamp();
        if hard {
            // the hard path IS an erasure — the legal-hold
            // fence that guards `/purge`/DSAR/forget must guard it here too,
            // inside the same tx (the 409 envelope matches `/purge`). This is
            // the seam an MCP `ump.forget` reaches with only Write scope.
            crate::legal_hold::refuse_if_held(&tx, &[id])?;
            crate::handlers::gate::purge_chunk_ids(&tx, &[id], now, &reason, None)?;
        } else {
            // Soft: quarantine-style flag; tombstone + audit row (hash-only).
            let hash: Option<String> = tx
                .query_row(
                    "SELECT content_hash FROM knowledge WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            tx.execute(
                "UPDATE knowledge SET flagged = 1 WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
            tx.execute(
                "INSERT INTO tombstones(knowledge_id, content_hash, purged_at, reason, origin_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, hash, now, reason, None::<i64>],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
            crate::audit::record(
                &tx,
                crate::audit::AuditKind::Ingest,
                "ump",
                &id.to_string(),
                crate::audit::AuditStatus::Ok,
                "ump-forget-soft",
            );
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    publish(&state, "forget", id);
    let result = if hard { "erased" } else { "tombstoned" };
    Ok(Json(json!({ "result": result })))
}

/// §3.6 `feedback` outcomes → the suggest-feedback last-wins upsert
/// (followed → accept; the rest are dismissals for the false-positive metric).
pub const UMP_OUTCOMES: [&str; 4] = ["followed", "overridden", "ignored", "contradicted"];

pub fn feedback_for_outcome(outcome: &str) -> &'static str {
    if outcome == "followed" {
        "accept"
    } else {
        "dismiss"
    }
}

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    /// Numeric row id or `urn:ump:…` content id.
    pub id: String,
    pub outcome: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
}

pub async fn feedback(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(req): Json<FeedbackRequest>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    super::cap_gate(&cap.0, "write")?;
    if !UMP_OUTCOMES.contains(&req.outcome.as_str()) {
        return Err(HandlerError::bad_request_with(
            "feedback_invalid",
            "outcome must be one of followed|overridden|ignored|contradicted",
            json!({ "allowed": UMP_OUTCOMES }),
        ));
    }
    let id_arg = req.id.clone();
    let outcome = req.outcome.clone();
    let feedback = feedback_for_outcome(&req.outcome);
    let session = req.session.clone();
    let reason_hash = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(crate::audit::hash);
    let tenant = principal
        .0
        .as_ref()
        .map(|p| p.tenant.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| crate::audit::DEFAULT_TENANT.to_string());
    let ts = chrono::Utc::now().timestamp();
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let id = resolve_row_id(&conn, &id_arg)?;
        record_feedback(
            &conn,
            id,
            feedback,
            reason_hash,
            ts,
            session,
            tenant,
            Some(&outcome),
        )
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(json!({ "ok": true })))
}

/// §3.8 `subscribe` — SSE change signal over a tokio broadcast channel.
/// `pending` handshake first, then `{kind, id}` events on remember/revise/
/// forget — never record bodies (the stream is a change signal, not a data
/// channel). Lagged subscribers drop missed events; a closed channel ends
/// the stream (client reconnects).
pub async fn subscribe(
    State(state): State<Arc<AppState>>,
    OptPrincipal(principal): OptPrincipal,
    cap: OptCapability,
) -> Sse<KeepAliveStream<Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>>>>
{
    let rx = state.ump_events.subscribe();
    let handshake = tokio_stream::once(Ok::<Event, Infallible>(
        Event::default().event("pending").data("{\"ump\":\"1.0\"}"),
    ));
    let gate = super::authorize(&principal, crate::auth::Action::Read, "", "global")
        .and_then(|()| super::cap_gate(&cap.0, "read"));
    let stream: Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>> =
        if let Err(e) = gate {
            // No body can be sent on the stream; surface the denial as a
            // single `error` event and close.
            Box::pin(tokio_stream::once(Ok::<Event, Infallible>(
                Event::default().event("error").data(format!("{e:?}")),
            )))
        } else {
            Box::pin(
                handshake.chain(tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
                    |item| {
                        match item {
                            Ok(v) => Some(Ok::<Event, Infallible>(
                                Event::default()
                                    .event("change")
                                    .json_data(v)
                                    .unwrap_or_default(),
                            )),
                            Err(_) => None,
                        }
                    },
                )),
            )
        };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub(crate) fn publish(state: &Arc<AppState>, kind: &'static str, id: i64) {
    let _ = state.ump_events.send(json!({ "kind": kind, "id": id }));
}

/// §9 `POST /ump/audit` — the reference audit facility: recent hash-chained
/// audit rows (Admin, tenant-scoped — same semantics as `/audit`).
#[derive(Debug, Default, Deserialize)]
pub struct AuditRequest {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

pub async fn audit(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Json(req): Json<AuditRequest>,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    // Admin surface: a capability token can never grant it (no admin verb
    // exists in the §5.2 vocabulary); a capability bearer is always denied.
    super::cap_gate(&cap.0, "admin")?;
    let tenant = audit_scope(&principal.0, &None)?;
    let limit = req.limit.unwrap_or(100).min(crate::config::MAX_MULTI_GET);
    let kind = req.kind;
    let offset = req.offset.unwrap_or(0);
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::audit::AuditRow>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            crate::audit::recent_tenant(&conn, kind.as_deref(), tenant.as_deref(), limit, offset)
                .map_err(|e| HandlerError::internal(e.to_string()))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(json!({ "rows": rows, "count": rows.len() })))
}

/// §9 `GET /ump/audit/verify` — fresh full-chain verification (authoritative,
/// unlike the TTL-cached `/metrics` signal).
pub async fn audit_verify(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
) -> Result<Json<Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    // Admin surface: capability tokens can never grant it.
    super::cap_gate(&cap.0, "admin")?;
    let pool = state.pool.clone();
    let ok = tokio::task::spawn_blocking(move || -> bool {
        match pool.get() {
            Ok(conn) => crate::audit::verify_chain(&conn),
            Err(_) => false,
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    Ok(Json(json!({ "ok": ok })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_shape_and_conformance_level() {
        let c = capabilities_payload();
        assert_eq!(c["ump"], "1.0");
        assert_eq!(c["server"]["name"], "brain-server");
        assert!(matches!(c["conformance"].as_str(), Some("L2" | "L3")));
        assert_eq!(c["max_recall"], 50);
        assert_eq!(c["writable"], true);
        assert_eq!(c["audit"], true);
        let kinds: Vec<&str> = c["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(kinds.contains(&"procedural") && kinds.contains(&"identity"));
        assert!(c["bindings"].as_array().unwrap().len() == 3);
        assert_eq!(
            c["retrieval_signals"].as_array().unwrap().len(),
            5,
            "the five §3.2 signal names"
        );
    }

    #[test]
    fn filter_kinds_map_to_brain_memory_kinds() {
        assert_eq!(filter_kind_to_brain("semantic"), Some("fact"));
        assert_eq!(filter_kind_to_brain("episodic"), Some("episodic"));
        assert_eq!(filter_kind_to_brain("procedural"), Some("procedure"));
        assert_eq!(filter_kind_to_brain("working"), Some("fact"));
        assert_eq!(filter_kind_to_brain("identity"), Some("fact"));
        assert_eq!(filter_kind_to_brain("bogus"), None);
    }

    #[test]
    fn feedback_followed_is_the_only_accept() {
        assert_eq!(feedback_for_outcome("followed"), "accept");
        for o in ["overridden", "ignored", "contradicted"] {
            assert_eq!(feedback_for_outcome(o), "dismiss");
        }
    }

    #[test]
    fn deep_merge_patches_nested_fields_and_ignores_id_integrity() {
        let base = json!({
            "kind": "semantic",
            "id": "urn:ump:old",
            "body": { "text": "old", "structured": { "title": "t" } },
            "time": { "valid_from": "2026-01-01T00:00:00Z" },
        });
        let patched = deep_merge(
            base,
            &json!({
                "id": "urn:ump:evil",
                "integrity": { "hash": "faked" },
                "body": { "text": "new" },
                "time": { "valid_from": "2026-02-01T00:00:00Z", "valid_to": null },
            }),
        );
        assert_eq!(patched["id"], "urn:ump:old", "server-authoritative id kept");
        assert!(
            patched["integrity"].is_null(),
            "server-authoritative integrity kept"
        );
        assert_eq!(patched["body"]["text"], "new");
        assert_eq!(
            patched["body"]["structured"]["title"], "t",
            "untouched nest survives"
        );
        assert_eq!(patched["time"]["valid_from"], "2026-02-01T00:00:00Z");
        assert!(
            patched["time"]["valid_to"].is_null(),
            "explicit null patch wins"
        );
    }

    /// a §3.7 consent mismatch is audited as a `Denied` auth
    /// event on the read-event-capable audit chain (COMPLIANCE.md §3.5
    /// promised it; the gap this closes was a silent 400 with no footprint).
    #[test]
    fn forbidden_scope_is_audited_as_denied_auth_event() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        assert!(
            record_forbidden_scope(&conn, "alice", "eve"),
            "the audit write succeeds on the migrated chain"
        );
        let (kind, status, target_hash): (String, String, Option<String>) = conn
            .query_row(
                "SELECT kind, status, target_hash FROM audit_events WHERE kind = 'auth' ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "auth");
        assert_eq!(status, "denied");
        // The scope detail rides in the target slot (record(kind, actor,
        // target, status, detail)) as a bounded xxh3 hash, never the raw
        // string. Derive from the helper's OWN format so the test cannot
        // drift; asserting the exact hash also proves nothing raw persisted.
        let expected = crate::audit::hash("ump.remember scope.owner=eve does not match principal");
        assert_eq!(target_hash.as_deref(), Some(expected.as_str()));
        // The chain still verifies (the denial joined the tamper-evident log).
        assert!(crate::audit::verify_chain(&conn));
    }
}
