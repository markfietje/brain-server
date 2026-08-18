//! Universal Memory Protocol (UMP) adapter.
//!
//! A transform, not a new model: `GET /export?format=ump` re-renders the
//! portable JSON as UMP records; `POST /ingest?format=ump` lowers UMP records
//! back into the existing structured-ingest path. No schema change.
//!
//! Conformance claim: **UMP 1.0 / L3** (self-attested; L2 when no operator
//! signing key is configured) — the full record engine: `capabilities`,
//! `remember`/`get`/`recall`/`revise`/`forget` ops (§3), consent + redact
//! enforcement (§2.7), blake3 + Ed25519 integrity (§2.8), did:key identity +
//! capability tokens (§5), content-addressed ids (§6.2), the `*.ump.md`
//! projection (§6.3). Spec: the Universal Memory Protocol v1.0
//! (github.com/edihasaj/universal-memory-protocol, `SPEC.md`).
//!
//! The L0 record codec below (`to_ump`/`from_ump`/`um_kind`/`brain_kind`) is
//! unchanged from v1.17.1 — it stays the canonical shape mapping. v1.17.3 M1
//! layers the engine on top: [`emit_record`] (L3 render: content-addressed id
//! + integrity + consent/redact + meta overlay) and [`verify_record`] (§5.3
//!
//! drop-unverifiable enforcement) and the §6.3 markdown projection
//! ([`record_to_markdown`]/[`record_from_markdown`]).
//!
//! Per §8 the import path rejects any record whose explicit `ump` major
//! version is not `1` rather than silently reinterpreting it; the field is
//! optional (absent defaults to `1.0`, per the reference suite's op requests).
//!
//! Round-trip guarantee: [`from_ump`]∘[`to_ump`] is the identity on the row
//! fields (pinned by a test) except the numeric `id`, which is content-mapped
//! (spec: a peer may rewrite ids on import; brain rows with a content-addressed
//! id import as a fresh row, dedup is by `content_hash`). The raw brain
//! `memory_kind` survives in `body.structured.raw_kind` so the mapped UMP
//! `kind` never loses it.

use base64::Engine;
use ed25519_dalek::SigningKey;
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
/// is by content, never by foreign ids). Per §8 an explicit unknown `ump` major
/// version is rejected, never reinterpreted; the field is OPTIONAL in op
/// requests (the reference suite sends none) and defaults to `1.0`, with the
/// legacy `0.1` accepted for import.
pub fn from_ump(record: &Value) -> Result<Value, String> {
    match record["ump"].as_str() {
        None | Some("1.0") | Some("0.1") => {}
        _ => return Err("UMP record carries an unsupported \"ump\" version".into()),
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

// ---------------------------------------------------------------------------
// the record engine (L3)
// ---------------------------------------------------------------------------

/// The persisted per-row UMP overlay (`knowledge.ump_meta`, JSON). Only fields
/// UMP needs beyond the brain columns: the raw UMP kind when it has no brain
/// equivalent (`working`/`identity`), the owner DID (authoritative over
/// `knowledge.owner`, which is the JWT `sub` string), `visibility`, the
/// origin record id for `provenance`, and the record's `provenance`/`consent`
/// blocks (§2.7) — stored verbatim so import → export round-trips them.
/// Empty object = pure brain columns.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UmpMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<Value>,
}

impl UmpMeta {
    /// Parse the stored JSON overlay; corrupt/absent → default (never fails).
    pub fn parse(raw: Option<&str>) -> Self {
        raw.and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

/// Render a knowledge row as a fully-conformant UMP 1.0 memory record:
/// the L0 shape + a content-addressed id (§6.2 L2: `urn:ump:<base32(blake3)>`
/// over `domain \0 text` — stable per (domain, content) and unique per row via
/// the domain salt) + the §2.8 `integrity` field in the reference format
/// (`content_hash: "blake3:<base32>"` over the canonical record minus
/// `integrity`, `signature: "ed25519:<base64>"` = Ed25519 over BLAKE3 of the
/// content-hash STRING, `signer` did:key; signed by the operator key when a
/// signer is configured). `superseded_by` carries the content-addressed ids
/// of the chunks that superseded this one (L2 bi-temporal; empty = current).
/// `redact=true` replaces the body text with the shape-preserving placeholder
/// *before* the integrity is computed (§2.7 consent: the integrity then
/// authenticates the redacted view).
/// `ponytail:` 8 args — bundling the render options into a struct is ceremony
/// for the two production callers (`/ump/*` ops + export), same precedent as
/// `write_markdown_ingest`.
#[allow(clippy::too_many_arguments)]
pub fn emit_record(
    row: &Value,
    domain: &str,
    entities: &Value,
    relations: &Value,
    meta: &UmpMeta,
    redact: bool,
    superseded_by: &[String],
    signer: Option<(&str, &SigningKey)>,
) -> Value {
    let mut rec = to_ump(row, domain, entities, relations);
    if row["content_hash"].as_str().is_some_and(|h| !h.is_empty()) {
        let content = rec["body"]["text"].as_str().unwrap_or("");
        let hash =
            brain_server::ump_integrity::record_hash(format!("{domain}\0{content}").as_bytes());
        rec["id"] = json!(brain_server::ump_integrity::content_id(&hash));
    }
    if !superseded_by.is_empty() {
        rec["superseded_by"] = json!(superseded_by);
    }
    if let Some(kind) = &meta.kind {
        rec["kind"] = json!(kind);
        // the emitted kind IS the raw kind of this row — keep raw_kind in sync
        // so `from_ump` round-trips the exact stored memory_kind.
        rec["body"]["structured"]["raw_kind"] = json!(kind);
    }
    if let Some(owner) = &meta.owner {
        rec["scope"]["owner"] = json!(owner);
    }
    if let Some(v) = &meta.visibility {
        rec["scope"]["visibility"] = json!(v);
    }
    if let Some(p) = &meta.provenance {
        rec["provenance"] = p.clone();
    }
    if let Some(c) = &meta.consent {
        rec["consent"] = c.clone();
    }
    if redact {
        rec["body"]["text"] = json!("[redacted]");
    }
    // §2.8/§6.1: the content hash covers the canonical record minus the
    // `integrity` block ONLY — `id` stays inside the hash (the reference
    // `contentHash` omits just `integrity`). JS-flavor canonicalization is
    // required so the reference `verify()` reproduces the same bytes
    // (integral floats serialize as `1`, not `1.0`).
    let mut canonical = rec.clone();
    canonical.as_object_mut().map(|m| m.remove("integrity"));
    if let Ok(bytes) = brain_server::ump_integrity::canonical_ump(&canonical) {
        let content_hash = brain_server::ump_integrity::content_hash_string(&bytes);
        let mut integrity = json!({ "content_hash": content_hash });
        if let Some((did, sk)) = signer {
            let sig = brain_server::ump_integrity::sign_hash_string(
                integrity["content_hash"].as_str().unwrap_or(""),
                sk,
            );
            integrity["signature"] = json!(format!(
                "ed25519:{}",
                base64::engine::general_purpose::STANDARD.encode(sig)
            ));
            integrity["signer"] = json!(did);
        }
        rec["integrity"] = integrity;
    }
    rec
}

/// §5.3 mandatory read-path check: recompute the blake3 over the canonical
/// record (minus `integrity`), require it to match `integrity.content_hash`
/// (reference §2.8 format), and when a signature + operator key are both
/// present, verify the EdDSA signature over BLAKE3 of the hash string.
/// Hash-only records verify without a key. The legacy v1.17.3 format
/// (`algo`/`hash`(hex)/`key`/`sig`, hash over the record with `id` +
/// `integrity` nulled, signature over the raw hash) is still accepted so
/// records signed by a v1.17.3 peer verify — the emit side writes only the
/// reference format. Returns false on any malformed input — the read path
/// drops unverifiable records.
pub fn verify_record(record: &Value, pk: Option<&[u8; 32]>) -> bool {
    if let Some(content_hash) = record["integrity"]["content_hash"].as_str() {
        if !content_hash.starts_with("blake3:") {
            return false;
        }
        let mut canonical = record.clone();
        canonical.as_object_mut().map(|m| m.remove("integrity"));
        let Ok(bytes) = brain_server::ump_integrity::canonical_ump(&canonical) else {
            return false;
        };
        if brain_server::ump_integrity::content_hash_string(&bytes) != content_hash {
            return false;
        }
        return match (pk, record["integrity"]["signature"].as_str()) {
            (Some(pk), Some(sig)) => {
                // Reference §2.8 form carries an `ed25519:` prefix; bare
                // base64 (pre-fix emit) is still accepted.
                let b64 = sig.strip_prefix("ed25519:").unwrap_or(sig);
                let Ok(sig_bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
                    return false;
                };
                brain_server::ump_integrity::verify_hash_string(content_hash, pk, &sig_bytes)
            }
            _ => true,
        };
    }
    let Some(hash_hex) = record["integrity"]["hash"].as_str() else {
        return false;
    };
    let mut canonical = record.clone();
    canonical["id"] = Value::Null;
    canonical["integrity"] = Value::Null;
    let Ok(bytes) = brain_server::ump_integrity::canonical_jcs(&canonical) else {
        return false;
    };
    let hash = brain_server::ump_integrity::record_hash(&bytes);
    let mut want = [0u8; 32];
    if hex_decode(hash_hex, &mut want).is_err() || hash != want {
        return false;
    }
    match (pk, record["integrity"]["sig"].as_str()) {
        (Some(pk), Some(sig)) => {
            let Ok(sig_bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig) else {
                return false;
            };
            brain_server::ump_integrity::verify_hash(pk, &hash, &sig_bytes)
        }
        _ => true,
    }
}

/// §6.3 file binding: multi-record `*.ump.md` separator. The two-line form
/// (`---\n---\n`) is the record boundary: a projection's own frontmatter
/// closer is a single `---` line (`\n---\n\n`), never `\n---\n---\n`, so the
/// join and the split share one unambiguous pattern.
/// `ponytail:` a body containing two adjacent bare `---` lines splits early
/// (same ceiling as the single-`---` split, narrowed by one line).
pub const MD_RECORD_SEP: &str = "\n---\n---\n";

/// §6.3 markdown projection: YAML frontmatter carrying the L2 fields + the
/// body text. Lossless for the fields it carries (pinned by a round-trip
/// test). `ponytail:` YAML is a hand-rolled subset (like `vault.rs`) — no
/// serde_yaml dep; the subset covers exactly the fields the projection emits.
pub fn record_to_markdown(record: &Value) -> Result<String, String> {
    let mut out = String::from("---\nump: \"1.0\"\n");
    let push = |out: &mut String, key: &str, v: &Value| {
        let s = match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return,
        };
        if !s.is_empty() {
            out.push_str(&format!("{key}: {s}\n"));
        }
    };
    push(&mut out, "id", &record["id"]);
    push(&mut out, "kind", &record["kind"]);
    push(&mut out, "title", &record["body"]["structured"]["title"]);
    let scope_owner = record["scope"]["owner"].as_str().unwrap_or("");
    let scope_vis = record["scope"]["visibility"].as_str().unwrap_or("");
    if !scope_owner.is_empty() || !scope_vis.is_empty() {
        out.push_str(&format!(
            "scope: {{ owner: {scope_owner}, visibility: {scope_vis} }}\n"
        ));
    }
    let mut time_parts: Vec<String> = Vec::new();
    for k in ["observed", "valid_from", "valid_to"] {
        if let Some(v) = record["time"][k].as_str() {
            time_parts.push(format!("{k}: {v}"));
        }
    }
    if !time_parts.is_empty() {
        out.push_str(&format!("time: {{ {} }}\n", time_parts.join(", ")));
    }
    let mut lc_parts: Vec<String> = Vec::new();
    for (k, v) in [
        ("confidence", &record["lifecycle"]["confidence"]),
        ("salience", &record["lifecycle"]["salience"]),
        ("decay", &record["lifecycle"]["decay"]),
    ] {
        if !v.is_null() {
            lc_parts.push(format!("{k}: {v}"));
        }
    }
    if !lc_parts.is_empty() {
        out.push_str(&format!("lifecycle: {{ {} }}\n", lc_parts.join(", ")));
    }
    out.push_str("---\n\n");
    out.push_str(record["body"]["text"].as_str().unwrap_or(""));
    Ok(out)
}

/// §6.3: parse a `*.ump.md` projection back into a UMP record. Round-trip
/// lossless for the L2 fields the projection carries (id/kind/scope/time/
/// lifecycle + title); the body is the text.
pub fn record_from_markdown(text: &str) -> Result<Value, String> {
    let (fm, body) = crate::vault::split_frontmatter(text);
    if fm.is_empty() {
        return Err("UMP markdown must carry a frontmatter block".into());
    }
    let get = |k: &str| -> Option<String> {
        fm.lines().find_map(|l| {
            l.trim()
                .strip_prefix(k)
                .and_then(|r| r.trim_start().strip_prefix(':'))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
    };
    if get("ump").as_deref() != Some("1.0") {
        return Err("UMP markdown must carry ump: \"1.0\"".into());
    }
    // `{ k: v, k: v }` inline object → Value; plain scalars stay strings.
    let inline = |v: &str| -> Value {
        let v = v.trim();
        if let Ok(j) = serde_json::from_str::<Value>(v) {
            return j;
        }
        if let Some(inner) = v.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let mut map = serde_json::Map::new();
            for pair in inner.split(',') {
                let mut it = pair.splitn(2, ':');
                if let (Some(k), Some(v)) = (it.next(), it.next()) {
                    let v = v.trim();
                    let val = if v == "null" {
                        Value::Null
                    } else {
                        Value::String(v.trim_matches('"').to_string())
                    };
                    map.insert(k.trim().to_string(), val);
                }
            }
            return Value::Object(map);
        }
        Value::String(v.to_string())
    };
    let scope = inline(&get("scope").unwrap_or_default());
    let time = inline(&get("time").unwrap_or_default());
    let lifecycle = inline(&get("lifecycle").unwrap_or_default());
    Ok(json!({
        "ump": "1.0",
        "id": get("id").unwrap_or_default(),
        "kind": get("kind").unwrap_or_else(|| "semantic".into()),
        "body": {
            "text": body,
            "structured": {
                "title": get("title"),
            }
        },
        "scope": scope,
        "time": time,
        "lifecycle": lifecycle,
    }))
}

/// Lowercase hex, hand-rolled (the codebase carries no hex dep; 3 lines).
/// Legacy v1.17.3 format hex encoding — the decode half stays live for the
/// legacy-verify path; the encode half is exercised by the hex round-trip
/// test and kept so a future `algo`/`hash` re-emission has a peer.
#[allow(dead_code)]
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Inverse of [`hex_encode`]; `Err(())` on wrong length or non-hex input.
fn hex_decode(s: &str, out: &mut [u8]) -> Result<(), ()> {
    if s.len() != out.len() * 2 {
        return Err(());
    }
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).ok_or(())?;
        let lo = (pair[1] as char).to_digit(16).ok_or(())?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Ok(())
}

/// `did:key` for an Ed25519 public key (multicodec 0xed01 + base58btc) — the
/// §2.8 identity form the operator's signatures are verified against.
pub fn did_key(pk: &[u8; 32]) -> String {
    let mut mc = Vec::with_capacity(34);
    mc.push(0xed);
    mc.push(0x01);
    mc.extend_from_slice(pk);
    format!("did:key:z{}", bs58::encode(mc).into_string())
}

/// The operator's Ed25519 signing key, if configured: a raw 32-byte seed file
/// in [`crate::config::ump_key_dir`] (any file — convention: `operator.key`).
/// Returns `(did, key)`; `None` → L2 conformance (hash-only integrity).
/// `ponytail:` raw-seed files only — no PKCS#8/PEM parsing (ed25519-dalek
/// ships without the `pkcs8` feature here); `openssl genpkey` interop is an
/// operator convenience, not a compat requirement. Read errors are swallowed —
/// a missing/unreadable key degrades to L2, never a boot failure.
pub fn operator_signing_key() -> Option<(String, SigningKey)> {
    let dir = crate::config::ump_key_dir();
    let seed = std::fs::read_dir(&dir).ok()?.find_map(|e| {
        let e = e.ok()?;
        if !e.path().is_file() {
            return None;
        }
        // the seed is a signing secret —
        // same 0600 owner-only enforcement the JWT keys / token file /
        // webhook secret get. A group/world-readable seed would let any local
        // user mint capability tokens; refuse it (fail closed to L2
        // hash-only integrity).
        if crate::auth::check_secret_permissions(&e.path()).is_err() {
            return None;
        }
        std::fs::read(e.path()).ok()
    })?;
    let bytes: [u8; 32] = seed.try_into().ok()?;
    let key = SigningKey::from_bytes(&bytes);
    Some((did_key(&key.verifying_key().to_bytes()), key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

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

    // -----------------------------------------------------------------------
    // record engine
    // -----------------------------------------------------------------------

    /// M1: `emit_record` ids are content-addressed (§6.2 L2) — deterministic
    /// per (domain, content), different for different content, base32-shaped.
    #[test]
    fn emit_record_ids_are_content_addressed() {
        let row = fixture_row();
        let a = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        let b = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        assert_eq!(a["id"], b["id"]);
        assert!(a["id"].as_str().unwrap().starts_with("urn:ump:"));
        let mut other = row.clone();
        other["content"] = Value::String("Dave works at Acme since 2022.".into());
        let c = emit_record(
            &other,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        assert_ne!(a["id"], c["id"], "different content → different id");
        let d = emit_record(
            &row,
            "beta",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        assert_ne!(a["id"], d["id"], "domain salt keeps per-domain ids unique");
        assert_eq!(a["id"].as_str().unwrap().len(), "urn:ump:".len() + 52);
    }

    /// M1 §2.8: the integrity field authenticates the canonical record — a
    /// hash-only emit verifies without a key, any text change breaks it.
    #[test]
    fn emit_record_integrity_detects_tampering() {
        let row = fixture_row();
        let rec = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        assert!(rec["integrity"]["content_hash"]
            .as_str()
            .unwrap()
            .starts_with("blake3:"));
        assert!(verify_record(&rec, None));
        let mut tampered = rec.clone();
        tampered["body"]["text"] = json!("Dave does NOT work at Acme.");
        assert!(!verify_record(&tampered, None));
        assert!(
            !verify_record(&json!({ "ump": "1.0" }), None),
            "no integrity → unverifiable"
        );
    }

    /// M1 §2.8: with an operator key the record is EdDSA-signed; verification
    /// requires the right key.
    #[test]
    fn emit_record_signed_and_verified_with_operator_key() {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes();
        let did = brain_server::ump_integrity::did_key_from_ed25519(&pk);
        let row = fixture_row();
        let rec = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            Some((&did, &sk)),
        );
        assert_eq!(rec["integrity"]["signer"], did);
        assert!(rec["integrity"]["signature"].is_string());
        assert!(
            rec["integrity"]["signature"]
                .as_str()
                .is_some_and(|s| s.starts_with("ed25519:")),
            "reference §2.8 signature carries the ed25519: prefix"
        );
        assert!(rec["integrity"]["content_hash"].is_string());
        assert!(verify_record(&rec, Some(&pk)));
        assert!(
            !verify_record(&rec, Some(&[0u8; 32])),
            "wrong key must not verify"
        );
    }

    /// M1 §2.7: redaction is shape-preserving and applied before the
    /// visibility boundary — the redacted view still verifies, the id and
    /// metadata survive.
    #[test]
    fn emit_record_redacts_for_non_owner() {
        let row = fixture_row();
        let rec = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            true,
            &[],
            None,
        );
        assert_eq!(rec["body"]["text"], "[redacted]");
        assert_eq!(rec["kind"], "semantic");
        assert_eq!(rec["scope"]["visibility"], "private");
        assert!(rec["id"].as_str().unwrap().starts_with("urn:ump:"));
        assert!(verify_record(&rec, None), "the redacted view authenticates");
        // The id identifies the underlying memory; redaction is a view over it.
        assert_eq!(
            rec["id"],
            emit_record(
                &row,
                "global",
                &json!([]),
                &json!([]),
                &UmpMeta::default(),
                false,
                &[],
                None
            )["id"]
        );
    }

    /// M1: the UMP-only overlay (kind with no brain equivalent, owner DID,
    /// visibility) round-trips onto the record and back through `from_ump`.
    #[test]
    fn emit_record_meta_overlay_round_trips() {
        let row = fixture_row();
        let meta = UmpMeta {
            kind: Some("working".into()),
            owner: Some("did:key:z6MkFake".into()),
            visibility: Some("shared".into()),
            origin: Some("urn:ump:peer1:42".into()),
            provenance: None,
            consent: None,
        };
        let rec = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &meta,
            false,
            &[],
            None,
        );
        assert_eq!(rec["kind"], "working");
        assert_eq!(rec["scope"]["owner"], "did:key:z6MkFake");
        assert_eq!(rec["scope"]["visibility"], "shared");
        let back = from_ump(&rec).unwrap();
        assert_eq!(back["memory_kind"], "working", "raw_kind preserved");
        assert_eq!(back["owner"], "did:key:z6MkFake");
        assert_eq!(back["access_scope"], "shared");
    }

    /// M1 §6.3: the markdown projection round-trips losslessly for the fields
    /// it carries (id/kind/scope/time/lifecycle/title + body).
    #[test]
    fn record_markdown_round_trip_is_lossless_for_carried_fields() {
        let row = fixture_row();
        let rec = emit_record(
            &row,
            "global",
            &json!([]),
            &json!([]),
            &UmpMeta::default(),
            false,
            &[],
            None,
        );
        let md = record_to_markdown(&rec).unwrap();
        assert!(md.starts_with("---\nump: \"1.0\"\n"));
        assert!(md.contains("id: urn:ump:"));
        let back = record_from_markdown(&md).unwrap();
        assert_eq!(back["ump"], "1.0");
        assert_eq!(back["id"], rec["id"]);
        assert_eq!(back["kind"], rec["kind"]);
        assert_eq!(back["scope"]["visibility"], rec["scope"]["visibility"]);
        assert_eq!(back["time"]["observed"], rec["time"]["observed"]);
        assert_eq!(back["lifecycle"]["decay"], rec["lifecycle"]["decay"]);
        assert_eq!(back["body"]["text"], rec["body"]["text"]);
        assert_eq!(back["body"]["structured"]["title"], "dave-acme");
    }

    /// M1 §6.3/§8: the projection rejects missing or wrong-version headers.
    #[test]
    fn record_from_markdown_rejects_bad_headers() {
        assert!(record_from_markdown("just body text").is_err());
        assert!(record_from_markdown("---\nump: \"2.0\"\n---\nbody").is_err());
    }

    /// M1: hex helpers round-trip and reject malformed input.
    #[test]
    fn hex_helpers_round_trip() {
        let bytes = [0u8, 1, 15, 16, 255, 170];
        let hex = hex_encode(&bytes);
        assert_eq!(hex, "00010f10ffaa");
        let mut out = [0u8; 6];
        assert!(hex_decode(&hex, &mut out).is_ok());
        assert_eq!(out, bytes);
        assert!(hex_decode("abc", &mut out).is_err(), "odd length");
        assert!(hex_decode("zzzz", &mut out).is_err(), "non-hex");
    }

    /// M2: did:key derivation matches the multibase/multicodec spec form
    /// (`did:key:z` + base58btc of 0xed01 ‖ pk).
    #[test]
    fn did_key_derives_multicodec_form() {
        let pk = [7u8; 32];
        let dk = did_key(&pk);
        assert!(dk.starts_with("did:key:z"));
        let decoded = bs58::decode(&dk[9..]).into_vec().expect("base58btc");
        assert_eq!(&decoded[..2], &[0xed, 0x01], "ed25519 multicodec prefix");
        assert_eq!(&decoded[2..], &pk);
        assert_eq!(dk.len(), 56, "did:key:z (9) + 34 bytes base58btc (47)");
    }
}
