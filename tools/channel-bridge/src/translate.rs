//! Platform-payload projection: Meta Cloud API webhook JSON → normalized
//! projections. PURE and BOUNDED — no network, no state, named errors, and
//! every list capped. Everything channel-specific about WhatsApp dies in
//! this module; the kernel only ever sees envelopes.

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Hard caps per webhook POST (bounds law; Meta batches at most a handful of
/// messages/statuses per entry anyway).
const MAX_ENTRIES: usize = 8;
const MAX_CHANGES_PER_ENTRY: usize = 8;
const MAX_MESSAGES_PER_VALUE: usize = 16;
const MAX_STATUSES_PER_VALUE: usize = 32;
const MAX_REF_LEN: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Projection {
    /// A customer message (text or attachment carrier).
    Message {
        conversation_ref: String,
        text: Option<String>,
        external_id: String,
        media_id: Option<String>,
        mime: Option<String>,
    },
    /// A delivery receipt for a message WE sent.
    Status {
        conversation_ref: String,
        state: String,
        message_ref: String,
    },
    /// A number quality-tier observation (account-scoped).
    Quality {
        number_alias: String,
        old_tier: Option<String>,
        new_tier: String,
    },
}

fn bounded(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_REF_LEN && !s.chars().any(char::is_control)
}

/// Project ONE verified webhook body into zero or more projections.
/// Unrenderable payloads bail with a named error (caller logs + drops
/// benignly); renderable payloads never exceed their caps.
pub(crate) fn project(body: &[u8]) -> Result<Vec<Projection>> {
    let v: Value = serde_json::from_slice(body).map_err(|e| anyhow::anyhow!("not json: {e}"))?;
    let entries = v
        .get("entry")
        .and_then(|x| x.as_array())
        .context("missing entry[]")?;
    let mut out = Vec::new();
    for entry in entries.iter().take(MAX_ENTRIES) {
        let changes = entry
            .get("changes")
            .and_then(|x| x.as_array())
            .context("entry missing changes[]")?;
        for change in changes.iter().take(MAX_CHANGES_PER_ENTRY) {
            let field = change.get("field").and_then(|x| x.as_str()).unwrap_or("");
            let value = change.get("value").cloned().unwrap_or(Value::Null);
            match field {
                // Messages + statuses (the conversation seam).
                "messages" => project_value(&value, &mut out)?,
                // Account-scoped quality updates. Meta ships several shapes
                // across API versions; we accept the documented envelope
                // shape {old_detail_id/new..., or flat old/new tiers}. CEILING:
                // exact taxonomy pinned against graph_api_version at deploy.
                "account_update" => project_quality(&value, &mut out),
                _ => {} // unsubscribed/unknown fields are IGNORED loudly-ish
            }
        }
    }
    Ok(out)
}

fn project_value(value: &Value, out: &mut Vec<Projection>) -> Result<()> {
    // Conversation identity: contacts[].wa_id mirrors messages[].from, but
    // the SENDER id on the message itself is authoritative per-platform-law.
    let msgs = value.get("messages").and_then(|x| x.as_array());
    if let Some(msgs) = msgs {
        for m in msgs.iter().take(MAX_MESSAGES_PER_VALUE) {
            let from = m.get("from").and_then(|x| x.as_str()).unwrap_or("");
            let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("");
            if !bounded(from) || !bounded(id) {
                bail!("message from/id unbounded or missing");
            }
            let mtype = m.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let (text, media_id, mime): (Option<String>, Option<String>, Option<String>) =
                match mtype {
                    "text" => (
                        Some(
                            m.get("text")
                                .and_then(|t| t.get("body"))
                                .and_then(|b| b.as_str())
                                .unwrap_or("")
                                .to_string(),
                        ),
                        None,
                        None,
                    ),
                    "image" | "document" | "audio" | "video" | "sticker" => {
                        let sec = m.get(mtype).cloned().unwrap_or(Value::Null);
                        let mid = sec
                            .get("id")
                            .and_then(|x| x.as_str())
                            .filter(|s| bounded(s))
                            .map(str::to_string);
                        let cap = sec
                            .get("caption")
                            .and_then(|x| x.as_str())
                            .filter(|s| bounded(s))
                            .map(str::to_string);
                        let mime_field = sec
                            .get("mime_type")
                            .and_then(|x| x.as_str())
                            .map(str::to_string);
                        (cap, mid, mime_field)
                    }
                    // Unsupported interactive types land as a MARKER note so
                    // the case still records that SOMETHING arrived.
                    other => (Some(format!("[unsupported type: {other}]")), None, None),
                };
            out.push(Projection::Message {
                conversation_ref: from.to_string(),
                text,
                external_id: id.to_string(),
                media_id,
                mime,
            });
        }
    }

    let statuses = value.get("statuses").and_then(|x| x.as_array());
    if let Some(statuses) = statuses {
        for s in statuses.iter().take(MAX_STATUSES_PER_VALUE) {
            let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let state = s.get("status").and_then(|x| x.as_str()).unwrap_or("");
            // Kernel-side closed vocabulary; anything else is dropped here.
            if !matches!(state, "sent" | "delivered" | "read" | "failed") {
                continue;
            }
            if !bounded(id) {
                bail!("status ref unbounded");
            }
            // Statuses reference OUR sends: recipient_id names the customer.
            let conv = s
                .get("recipient_id")
                .and_then(|x| x.as_str())
                .filter(|c| bounded(c))
                .context("status missing recipient_id")?;
            out.push(Projection::Status {
                conversation_ref: conv.to_string(),
                state: state.to_string(),
                message_ref: id.to_string(),
            });
        }
    }
    Ok(())
}

fn project_quality(value: &Value, out: &mut Vec<Projection>) {
    // Number alias: prefer explicit alias/display fields; fall back to the
    // phone-number-id-shaped string (an account identifier, not a customer
    // ref — metadata law holds either way).
    let alias_candidates = [
        value.get("number_alias").and_then(|x| x.as_str()),
        value
            .get("display_phone_number_id")
            .and_then(|x| x.as_str()),
        value.get("phone_number_id").and_then(|x| x.as_str()),
    ];
    let Some(alias) = alias_candidates.into_iter().flatten().next() else {
        return; // nothing identifiable: silently skip (cannot alert sanely)
    };
    let new_tier = value
        .get("new_tier")
        .or_else(|| value.get("current_tier"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_ascii_lowercase());
    let old_tier = value
        .get("old_tier")
        .or_else(|| value.get("previous_tier"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_ascii_lowercase());
    let tier_ok = |t: &Option<String>| {
        t.as_deref()
            .map(|s| matches!(s, "green" | "yellow" | "orange" | "red"))
            .unwrap_or(true)
    };
    if !tier_ok(&new_tier) || !tier_ok(&old_tier) {
        return;
    }
    let Some(new_tier) = new_tier else {
        return;
    };
    out.push(Projection::Quality {
        number_alias: alias.to_string(),
        old_tier,
        new_tier,
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn projection_is_total_and_bounded() {
        // Junk bodies name their refusal instead of panicking.
        assert!(project(b"junk").is_err());
        assert!(project(br#"{"entry":"x"}"#).is_err());

        // A full-shaped body projects messages AND statuses.
        let body = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "WABA",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {"display_phone_number_id": "111", "phone_number_id": "111"},
                        "contacts": [{"profile": {"name": "C"}, "wa_id": "4915112345678"}],
                        "messages": [
                            {"from": "4915112345678", "id": "wamid.A", "type": "text",
                             "text": {"body": "hello kernel"}},
                            {"from": "4915112345679", "id": "wamid.B", "type": "image",
                             "image": {"id": "media-1", "mime_type": "image/png"}}
                        ],
                        "statuses": [
                            {"id": "wamid.OUT1", "status": "delivered",
                             "recipient_id": "4915112345678", "timestamp": "1"}
                        ]
                    }
                }]
            }]
        });
        let p = project(serde_json::to_string(&body).unwrap().as_bytes()).unwrap();
        assert_eq!(p.len(), 3);
        match &p[0] {
            Projection::Message {
                conversation_ref,
                text,
                external_id,
                media_id,
                ..
            } => {
                assert_eq!(conversation_ref, "4915112345678");
                assert_eq!(text.as_deref(), Some("hello kernel"));
                assert_eq!(external_id, "wamid.A");
                assert!(media_id.is_none());
            }
            other => panic!("expected message projection, got {other:?}"),
        }
        match &p[1] {
            Projection::Message { media_id, mime, .. } => {
                assert_eq!(media_id.as_deref(), Some("media-1"));
                assert_eq!(mime.as_deref(), Some("image/png"));
            }
            other => panic!("expected media projection, got {other:?}"),
        }
        match &p[2] {
            Projection::Status {
                conversation_ref,
                state,
                message_ref,
            } => {
                assert_eq!(conversation_ref, "4915112345678");
                assert_eq!(state, "delivered");
                assert_eq!(message_ref, "wamid.OUT1");
            }
            other => panic!("expected status projection, got {other:?}"),
        }

        // Unknown statuses drop (closed vocabulary lives at the edge too).
        let odd = serde_json::json!({
            "entry": [{"changes": [{"field": "messages", "value": {"statuses": [
                {"id": "wamid.Z", "status": "deleted", "recipient_id": "42"}]}}]}]
        });
        let p2 = project(serde_json::to_string(&odd).unwrap().as_bytes()).unwrap();
        assert!(p2.is_empty(), "non-canonical states never cross");
    }

    #[test]
    fn quality_projection_maps_documented_shapes_and_bounds() {
        let q = serde_json::json!({
            "entry": [{"changes": [{"field": "account_update", "value": {
                "number_alias": "biz_number",
                "old_tier": "GREEN",
                "new_tier": "ORANGE"
            }}]}]
        });
        let p = project(serde_json::to_string(&q).unwrap().as_bytes()).unwrap();
        assert_eq!(
            p,
            vec![Projection::Quality {
                number_alias: "biz_number".into(),
                old_tier: Some("green".into()),
                new_tier: "orange".into(),
            }],
            "tiers lowercase-normalize"
        );

        // Invented tiers refuse by DROP (no metadata lies upstream).
        let bad = serde_json::json!({
            "entry": [{"changes": [{"field": "account_update", "value": {
                "number_alias": "b", "old_tier": "green", "new_tier": "platinum"}}]}]
        });
        assert!(
            project(serde_json::to_string(&bad).unwrap().as_bytes())
                .unwrap()
                .is_empty()
        );
    }
}
