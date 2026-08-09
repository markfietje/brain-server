//! v1.17.1 "Govern" M4 — Universal Memory Protocol (UMP) wire adapter.
//!
//! A transform, not a new model: `GET /export?format=ump` re-renders the
//! portable JSON as UMP records; `POST /ingest?format=ump` lowers UMP records
//! back into the existing structured-ingest path. No schema change.
//!
//! The UMP memory record (per universalmemoryprotocol.io, wire spec 0.1):
//! `{"ump":"0.1","id":"urn:ump:…","kind":"semantic","body":{"text":…,
//! "structured":{}},"scope":{"owner":…,"visibility":…},"time":{"created":…,
//! "observed":…,"valid_from":…,"valid_to":…},"lifecycle":{"confidence":…,
//! "salience":…,"decay":…}}`.
//!
//! Round-trip guarantee: [`from_ump`]∘[`to_ump`] is the identity on the row
//! JSON (pinned by a test). The raw brain `memory_kind` survives in
//! `body.structured.raw_kind` so the mapped UMP `kind` never loses it.

use serde_json::{json, Value};

/// brain `memory_kind` → UMP `kind`. `step` collapses to `procedural` (its
/// parent `procedure` keeps the full sequence); the raw value round-trips via
/// `body.structured.raw_kind` either way.
pub fn um_kind(memory_kind: &str) -> &'static str {
    match memory_kind {
        "fact" => "semantic",
        "episodic" => "episodic",
        "procedure" | "step" => "procedural",
        "decision" => "declarative",
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
                "declarative" => "decision",
                _ => "fact",
            }
            .to_string()
        })
}

/// A stable `urn:ump:` id for a knowledge row: `urn:ump:brain:<domain>:<id>`.
/// Lossless within brain-server (the domain+id pair is unique); a peer may
/// rewrite the id on import into its own namespace.
pub fn record_id(domain: &str, id: i64) -> String {
    format!("urn:ump:brain:{domain}:{id}")
}

/// Render one `/export` knowledge row as a UMP memory record. `domain` seeds
/// the id; `entities`/`relations` (name-based, the structured-ingest shape)
/// are embedded in `body.structured` so a UMP import restores the graph.
pub fn to_ump(row: &Value, domain: &str, entities: &Value, relations: &Value) -> Value {
    let id = row["id"].as_i64().unwrap_or(0);
    let memory_kind = row["memory_kind"].as_str().unwrap_or("fact");
    let visibility = row["access_scope"].as_str().unwrap_or("private");
    json!({
        "ump": "0.1",
        "id": record_id(domain, id),
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
            "created": row["created_at"].as_i64(),
            "observed": row["observed_at"],
            "valid_from": row["valid_from"],
            "valid_to": row["valid_to"],
        },
        "lifecycle": {
            "confidence": row["confidence"],
            "salience": row["salience"].as_f64(),
            "decay": row["expires_at"].as_i64(),
        },
    })
}

/// Lower a UMP record back into the `/export` knowledge-row JSON (the inverse
/// of [`to_ump`] on the row fields). `id` is derived from the UMP id's trailing
/// `:<id>`; a peer that rewrote the id keeps a fresh numeric id here (mapping
/// is by content, never by foreign ids).
pub fn from_ump(record: &Value) -> Result<Value, String> {
    if record["ump"].as_str() != Some("0.1") {
        return Err("UMP record must carry \"ump\": \"0.1\"".into());
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
        "observed_at": time.get("observed").cloned().filter(|v| !v.is_null()).and_then(|v| v.as_str().map(|x| x.to_string())),
        "valid_from": time.get("valid_from").cloned().filter(|v| !v.is_null()).and_then(|v| v.as_str().map(|x| x.to_string())),
        "valid_to": time.get("valid_to").cloned().filter(|v| !v.is_null()).and_then(|v| v.as_str().map(|x| x.to_string())),
        "content_hash": s["content_hash"].as_str().map(|x| x.to_string()),
        "created_at": time["created"].as_i64(),
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
    #[test]
    fn ump_round_trip_is_identity_on_row_fields() {
        let row = fixture_row();
        let rec = to_ump(&row, "global", &row["entities"], &row["relations"]);
        assert_eq!(rec["ump"], "0.1");
        assert_eq!(rec["id"], "urn:ump:brain:global:7");
        assert_eq!(rec["kind"], "semantic");
        let back = from_ump(&rec).unwrap();
        assert_eq!(back["id"], 7);
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

    /// M4: the raw brain memory_kind survives via body.structured.raw_kind even
    /// when the mapped UMP kind differs.
    #[test]
    fn ump_kind_mapping_keeps_raw_kind() {
        assert_eq!(um_kind("fact"), "semantic");
        assert_eq!(um_kind("episodic"), "episodic");
        assert_eq!(um_kind("procedure"), "procedural");
        assert_eq!(um_kind("step"), "procedural");
        assert_eq!(um_kind("decision"), "declarative");
        let rec = to_ump(&fixture_row(), "global", &json!([]), &json!([]));
        assert_eq!(brain_kind(&rec), "fact");
        let step = json!({ "body": { "structured": { "raw_kind": "step" } } });
        assert_eq!(brain_kind(&step), "step");
    }

    /// M4: malformed UMP is rejected, never silently ingested.
    #[test]
    fn ump_rejects_malformed_records() {
        assert!(from_ump(&json!({})).is_err());
        assert!(from_ump(&json!({ "ump": "0.2", "body": { "text": "x" } })).is_err());
        assert!(
            from_ump(&json!({ "ump": "0.1", "body": { "text": "   " } })).is_err(),
            "empty text rejected"
        );
        // Round-trips that came from a peer with an unknown/rewritten id still
        // lower to a fresh numeric id (mapping is by content, not by id).
        let rec = to_ump(&fixture_row(), "global", &json!([]), &json!([]));
        let rewritten = json!({ "ump": "0.1", "id": "urn:ump:other-peer:99", "body": rec["body"].clone(), "scope": rec["scope"].clone(), "time": rec["time"].clone(), "lifecycle": rec["lifecycle"].clone() });
        assert_eq!(from_ump(&rewritten).unwrap()["id"], 99);
    }
}
