//! v1.17.1 "Govern" M4 — Universal Memory Protocol (UMP) adapter.
//!
//! A transform, not a new model: `GET /export?format=ump` re-renders the
//! portable JSON as UMP records; `POST /ingest?format=ump` lowers UMP records
//! back into the existing structured-ingest path. No schema change.
//!
//! Conformance claim: **UMP 1.0 / L0** (portable-record file binding — parse +
//! emit records; no `capabilities`/`recall`/`remember`/`get` server, no
//! consent/`redact` enforcement). Spec: the Universal Memory Protocol v1.0
//! (github.com/edihasaj/universal-memory-protocol, `SPEC.md`), §2 record shape:
//! `{"ump":"1.0","id":"urn:ump:…","kind":"semantic","body":{"text":…,
//! "structured":{}},"scope":{"owner":…,"visibility":…},"time":{"created":…,
//! "observed":…,"valid_from":…,"valid_to":…},"lifecycle":{"confidence":…,
//! "salience":…,"decay":…},"relations":[…]}`.
//!
//! Per §8 the import path rejects any record whose `ump` major version is not
//! `1` rather than silently reinterpreting it. `id` is content-addressed
//! (`urn:ump:<content_hash>`, §6.2 L2+ form) when the row has a hash — stable
//! across exports and dedup-friendly — and falls back to a stable
//! `urn:ump:brain:<domain>:<id>` for legacy hashless rows (`ponytail:` a
//! brain-scoped id is unique but not base32/content-hash shaped; backfilling
//! hashes is a v2.x operator step).
//!
//! Round-trip guarantee: [`from_ump`]∘[`to_ump`] is the identity on the row
//! fields (pinned by a test) except the numeric `id`, which is content-mapped
//! (spec: a peer may rewrite ids on import; brain rows with a content-addressed
//! id import as a fresh row, dedup is by `content_hash`). The raw brain
//! `memory_kind` survives in `body.structured.raw_kind` so the mapped UMP
//! `kind` never loses it.

use serde_json::{json, Value};

/// brain `memory_kind` → UMP `kind` (v1.0 §2.1 has exactly five kinds:
/// semantic | episodic | procedural | working | identity — no `declarative`).
/// `step` collapses to `procedural` (its parent `procedure` keeps the full
/// sequence); `decision` is a durable fact about a choice → `semantic`. The
/// raw value round-trips via `body.structured.raw_kind` either way.
pub fn um_kind(memory_kind: &str) -> &'static str {
    match memory_kind {
        "fact" | "decision" => "semantic",
        "episodic" => "episodic",
        "procedure" | "step" => "procedural",
        _ => "semantic",
    }
}

/// UMP `kind` → brain `memory_kind` (prefers `raw_kind` when present).
pub fn brain_kind(record: &Value) -> String {
    record
        .pointer("/body/structured/raw_kind")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let k = record["kind"].as_str().unwrap_or("semantic");
            match k {
                "episodic" => "episodic",
                "procedural" => "procedure",
                // "working"/"identity" have no brain equivalent; raw_kind
                // preserves the original when present.
                _ => "fact",
            }
            .to_string()
        })
}

/// A stable `urn:ump:` id for a knowledge row (§6.2): content-addressed from
/// the row's `content_hash` when present (L2+ form), else a brain-scoped
/// `urn:ump:brain:<domain>:<id>` fallback. Both are stable across exports.
pub fn record_id(domain: &str, id: i64, content_hash: Option<&str>) -> String {
    match content_hash {
        Some(h) if !h.is_empty() => format!("urn:ump:{h}"),
        _ => format!("urn:ump:brain:{domain}:{id}"),
    }
}

/// Brain naive-UTC timestamp (`"YYYY-MM-DD HH:MM:SS"`, UTC by convention) →
/// RFC 3339 (§2.3 REQUIRED string form). RFC 3339 input passes through
/// unchanged; null passes through null.
fn to_rfc3339(v: Option<&str>) -> Option<String> {
    v.map(|s| {
        if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
            s.to_string()
        } else {
            format!("{}Z", s.replace(' ', "T"))
        }
    })
}

/// RFC 3339 → brain naive-UTC string; already-naive input passes through.
fn from_rfc3339(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    Some(match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(dt) => dt
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => s.to_string(),
    })
}

/// Render one `/export` knowledge row as a UMP memory record. `domain` seeds
/// the fallback id; `entities`/`relations` (name-based, the structured-ingest
/// shape) are embedded in `body.structured` so a UMP import restores the graph
/// losslessly, and projected to the spec's top-level `relations` (§2.5) as
/// `{type, target}` links (`about` = the from-entity, typed link = the
/// to-entity) for knowledge-graph peers that ignore the opaque payload.
pub fn to_ump(row: &Value, domain: &str, entities: &Value, relations: &Value) -> Value {
    let id = row["id"].as_i64().unwrap_or(0);
    let memory_kind = row["memory_kind"].as_str().unwrap_or("fact");
    let visibility = row["access_scope"].as_str().unwrap_or("private");
    let rels = relations.as_array().filter(|a| !a.is_empty()).map(|a| {
        a.iter()
            .flat_map(|r| {
                let ty = r["type"].as_str().unwrap_or("relates_to");
                let from = r["from"].as_str().unwrap_or_default();
                let to = r["to"].as_str().unwrap_or_default();
                vec![
                    json!({ "type": "about", "target": format!("entity:{from}") }),
                    json!({ "type": ty, "target": format!("entity:{to}") }),
                ]
            })
            .collect::<Vec<_>>()
    });
    let created = row["created_at"]
        .as_i64()
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|d| d.to_rfc3339());
    let mut rec = json!({
        "ump": "1.0",
        "id": record_id(domain, id, row["content_hash"].as_str()),
        "kind": um_kind(memory_kind),
        "body": {
            "text": row["content"].as_str().unwrap_or(""),
            "structured": {
                "title": row["title"].as_str(),
                "raw_kind": memory_kind,
                "source": row["source"].as_str(),
                "authority": row["authority"],
                "assertion_kind": row["assertion_kind"].as_str(),
                "confidence": row["confidence"],
                "content_hash": row["content_hash"].as_str(),
                "entities": entities,
                "relations": relations,
            }
        },
        "scope": {
            "owner": row["owner"],
            "visibility": visibility,
        },
        "time": {
            "created": created,
            "observed": to_rfc3339(row["observed_at"].as_str()),
            "valid_from": to_rfc3339(row["valid_from"].as_str()),
            "valid_to": to_rfc3339(row["valid_to"].as_str()),
        },
        "lifecycle": {
            "confidence": row["confidence"],
            "salience": row["salience"].as_f64(),
            "decay": row["expires_at"].as_i64(),
        },
    });
    if let Some(rels) = rels {
        rec["relations"] = Value::Array(rels);
    }
    rec
}

/// Lower a UMP record back into the `/export` knowledge-row JSON (the inverse
/// of [`to_ump`] on the row fields). `id` is derived from the UMP id's trailing
/// `:<id>`; a peer that rewrote the id keeps a fresh numeric id here (mapping
/// is by content, never by foreign ids). Per §8, an unknown `ump` major
/// version is rejected, never reinterpreted.
pub fn from_ump(record: &Value) -> Result<Value, String> {
    match record["ump"].as_str() {
        Some("1.0") => {}
        _ => return Err("UMP record must carry \"ump\": \"1.0\"".into()),
    }
    let text = record
        .pointer("/body/text")
        .and_then(|v| v.as_str())
        .ok_or("UMP record body.text is required")?;
    if text.trim().is_empty() {
        return Err("UMP record body.text must not be empty".into());
    }
    let s = &record["body"]["structured"];
    let id = record["id"]
        .as_str()
        .and_then(|u| u.rsplit(':').next())
        .and_then(|t| t.parse::<i64>().ok())
        .unwrap_or(0);
    let own = |p: &str| s.get(p).cloned().filter(|v| !v.is_null());
    let scope = record.get("scope").cloned().unwrap_or(Value::Null);
    let time = record.get("time").cloned().unwrap_or(Value::Null);
    let lifecycle = record.get("lifecycle").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "id": id,
        "title": s["title"].as_str().unwrap_or("untitled").to_string(),
        "content": text.to_string(),
        "memory_kind": brain_kind(record),
        "source": s["source"].as_str().map(|x| x.to_string()),
        "authority": own("authority"),
        "assertion_kind": s["assertion_kind"].as_str().map(|x| x.to_string()),
        "confidence": s["confidence"].clone(),
        "access_scope": scope["visibility"].as_str().unwrap_or("private").to_string(),
        "owner": scope.get("owner").cloned(),
        "observed_at": from_rfc3339(time.get("observed")),
        "valid_from": from_rfc3339(time.get("valid_from")),
        "valid_to": from_rfc3339(time.get("valid_to")),
        "content_hash": s["content_hash"].as_str().map(|x| x.to_string()),
        "created_at": time["created"].as_str()
            .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
            .map(|d| d.timestamp()),
        "expires_at": lifecycle["decay"].as_i64(),
        "entities": s.get("entities").cloned().filter(|v| !v.is_null()).unwrap_or(json!([])),
        "relations": s.get("relations").cloned().filter(|v| !v.is_null()).unwrap_or(json!([])),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_row() -> Value {
        json!({
            "id": 7,
            "content": "Dave works at Acme since 2021.",
            "title": "dave-acme",
            "memory_kind": "fact",
            "source": "manual",
            "authority": 0.9,
            "assertion_kind": "stated",
            "confidence": 0.8,
            "access_scope": "private",
            "owner": "user-42",
            "observed_at": "2026-01-01 10:00:00",
            "valid_from": "2021-01-01 00:00:00",
            "valid_to": null,
            "content_hash": "abc123",
            "created_at": 1735689600,
            "expires_at": null,
            "salience": 0.6,
            "entities": [{"name": "Dave", "type": "person"}],
            "relations": [{"from": "Dave", "to": "Acme", "type": "works_at"}],
        })
    }

    /// M4 exit criterion: `from_ump(to_ump(row))` restores every row field.
    /// The numeric `id` is content-mapped (content-addressed UMP ids import as
    /// a fresh row; dedup is by `content_hash`, which does round-trip).
    #[test]
    fn ump_round_trip_is_identity_on_row_fields() {
        let row = fixture_row();
        let rec = to_ump(&row, "global", &row["entities"], &row["relations"]);
        assert_eq!(rec["ump"], "1.0");
        assert_eq!(rec["id"], "urn:ump:abc123");
        assert_eq!(rec["kind"], "semantic");
        assert_eq!(rec["time"]["created"], "2025-01-01T00:00:00+00:00");
        assert_eq!(rec["time"]["observed"], "2026-01-01T10:00:00Z");
        let back = from_ump(&rec).unwrap();
        assert_eq!(back["id"], 0, "content-addressed id → fresh row");
        assert_eq!(back["content"], row["content"]);
        assert_eq!(back["memory_kind"], "fact");
        assert_eq!(back["source"], "manual");
        assert_eq!(back["confidence"], 0.8);
        assert_eq!(back["access_scope"], "private");
        assert_eq!(back["owner"], "user-42");
        assert_eq!(back["observed_at"], "2026-01-01 10:00:00");
        assert_eq!(back["valid_from"], "2021-01-01 00:00:00");
        assert_eq!(back["content_hash"], "abc123");
        assert_eq!(back["created_at"], 1735689600);
        assert_eq!(back["entities"], row["entities"]);
        assert_eq!(back["relations"], row["relations"]);
    }

    /// Legacy hashless rows keep a recoverable brain-scoped id.
    #[test]
    fn ump_hashless_rows_keep_brain_scoped_id() {
        let mut row = fixture_row();
        row["content_hash"] = Value::Null;
        let rec = to_ump(&row, "global", &json!([]), &json!([]));
        assert_eq!(rec["id"], "urn:ump:brain:global:7");
        assert_eq!(from_ump(&rec).unwrap()["id"], 7);
    }

    /// M4: the raw brain memory_kind survives via body.structured.raw_kind even
    /// when the mapped UMP kind differs. v1.0 has no `declarative` kind.
    #[test]
    fn ump_kind_mapping_keeps_raw_kind() {
        assert_eq!(um_kind("fact"), "semantic");
        assert_eq!(um_kind("episodic"), "episodic");
        assert_eq!(um_kind("procedure"), "procedural");
        assert_eq!(um_kind("step"), "procedural");
        assert_eq!(um_kind("decision"), "semantic", "v1.0 kinds only");
        let rec = to_ump(&fixture_row(), "global", &json!([]), &json!([]));
        assert_eq!(brain_kind(&rec), "fact");
        let step = json!({ "body": { "structured": { "raw_kind": "step" } } });
        assert_eq!(brain_kind(&step), "step");
    }

    /// M4: malformed UMP is rejected, never silently ingested. Per §8 an
    /// unknown major version is rejected, not reinterpreted.
    #[test]
    fn ump_rejects_malformed_records() {
        assert!(from_ump(&json!({})).is_err());
        assert!(from_ump(&json!({ "ump": "0.2", "body": { "text": "x" } })).is_err());
        assert!(from_ump(&json!({ "ump": "1.1", "body": { "text": "x" } })).is_err());
        assert!(from_ump(&json!({ "ump": "2.0", "body": { "text": "x" } })).is_err());
        assert!(
            from_ump(&json!({ "ump": "1.0", "body": { "text": "   " } })).is_err(),
            "empty text rejected"
        );
        // Round-trips that came from a peer with an unknown/rewritten id still
        // lower to a fresh numeric id (mapping is by content, not by id).
        let rec = to_ump(&fixture_row(), "global", &json!([]), &json!([]));
        let rewritten = json!({ "ump": "1.0", "id": "urn:ump:other-peer:99", "body": rec["body"].clone(), "scope": rec["scope"].clone(), "time": rec["time"].clone(), "lifecycle": rec["lifecycle"].clone() });
        assert_eq!(from_ump(&rewritten).unwrap()["id"], 99);
    }

    /// v1.0 §2.5: top-level relations are `{type, target}` links (`about` =
    /// from-entity, typed link = to-entity); the full graph stays lossless in
    /// the opaque `body.structured` payload.
    #[test]
    fn ump_relations_use_spec_shape() {
        let row = fixture_row();
        let rec = to_ump(&row, "global", &row["entities"], &row["relations"]);
        assert_eq!(
            rec["relations"],
            json!([
                { "type": "about", "target": "entity:Dave" },
                { "type": "works_at", "target": "entity:Acme" },
            ])
        );
        assert_eq!(
            rec["body"]["structured"]["relations"],
            json!([{ "from": "Dave", "to": "Acme", "type": "works_at" }])
        );
        let empty = to_ump(&fixture_row(), "global", &json!([]), &json!([]));
        assert!(empty.get("relations").is_none(), "no graph → no relations");
    }
}
