//! Zendesk connector — cursor-based Incremental Ticket Export.
//!
//! `GET /api/v2/incremental/tickets/cursor.json` is the documented bulk path;
//! it is rate-capped (~10 req/min), so the poll cadence floor
//! ([`super::MIN_POLL_INTERVAL_SECS`]) must hold and the cursor persists in
//! the connector's own state file, never inside brain-server.
//!
//! All URLs are built from the configured subdomain only; the server-provided
//! `after_cursor` is an opaque token appended as a query parameter to the
//! pinned base — never a followed URL.

use super::{CaseStatus, CrmCase, SOURCE_ZENDESK};
use anyhow::{Context, Result};

/// The only host family a Zendesk bearer ever goes to.
pub fn api_base(subdomain: &str) -> Result<String> {
    let s = subdomain.trim();
    if s.is_empty()
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        anyhow::bail!("zendesk subdomain must be lowercase alnum/hyphen, got {subdomain:?}");
    }
    Ok(format!("https://{s}.zendesk.com"))
}

/// Cursor-export URL for either the first page (`start_time`) or a resumed
/// page (`cursor`). Both are pinned to the configured base.
pub fn cursor_url(base: &str, cursor: Option<&str>, start_time: u64) -> String {
    match cursor {
        Some(c) => format!("{base}/api/v2/incremental/tickets/cursor.json?cursor={c}"),
        None => format!("{base}/api/v2/incremental/tickets/cursor.json?start_time={start_time}"),
    }
}

/// Basic auth header value (RFC 7617) for `{email}/token:{api_token}`.
pub fn basic_auth(email: &str, api_token: &str) -> String {
    use base64::Engine;
    let raw = format!("{email}/token:{api_token}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    )
}

/// Map one ticket JSON object to the normalized case. `status` of
/// `solved`/`closed` → [`CaseStatus::ClosedSolved`]; everything else open.
pub fn map_ticket(org_id: &str, v: &serde_json::Value) -> Result<CrmCase> {
    let id = v
        .get("id")
        .and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .context("ticket missing id")?;
    let status = match v.get("status").and_then(|s| s.as_str()).unwrap_or("") {
        "solved" | "closed" => CaseStatus::ClosedSolved,
        _ => CaseStatus::Open,
    };
    let requester = v
        .get("requester_id")
        .map(|r| r.to_string())
        .unwrap_or_default();
    Ok(CrmCase {
        source: SOURCE_ZENDESK.into(),
        org_id: org_id.into(),
        case_id: id.to_string(),
        title: v
            .get("subject")
            .and_then(|s| s.as_str())
            .unwrap_or("(no subject)")
            .to_string(),
        status,
        priority: v
            .get("priority")
            .and_then(|p| p.as_str())
            .map(str::to_string),
        subject_ref: super::hash_subject(SOURCE_ZENDESK, org_id, &format!("requester:{requester}")),
        updated_rev: v
            .get("generated_at")
            .or_else(|| v.get("updated_at"))
            .and_then(|t| t.as_str())
            .context("ticket missing generated_at")?
            .to_string(),
        body_markdown: format!(
            "# Zendesk ticket {id}\n\n{}",
            v.get("description").and_then(|d| d.as_str()).unwrap_or("")
        ),
        // Zendesk carries no structured symptom fields by default; custom
        // mappings extend via the custom-connector doc, not this mapper.
        is_seed: None,
        is_not_seed: None,
        updated_at: v
            .get("updated_at")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// One page of the incremental export.
pub struct ZendeskPage {
    pub cases: Vec<CrmCase>,
    pub after_cursor: Option<String>,
    pub end_of_stream: bool,
}

/// Fetch + translate one cursor page through any transport.
pub fn fetch_page(
    t: &dyn super::VendorTransport,
    base: &str,
    auth: &str,
    cursor: Option<&str>,
    start_time: u64,
    org_id: &str,
) -> Result<ZendeskPage> {
    let url = cursor_url(base, cursor, start_time);
    let body = t.get_json(&url, auth)?;
    let tickets = body
        .get("tickets")
        .and_then(|x| x.as_array())
        .context("incremental response missing tickets array")?;
    let mut cases = Vec::with_capacity(tickets.len());
    for tk in tickets {
        cases.push(map_ticket(org_id, tk)?);
    }
    Ok(ZendeskPage {
        cases,
        after_cursor: body
            .get("after_cursor")
            .and_then(|c| c.as_str())
            .map(str::to_string),
        end_of_stream: body
            .get("end_of_stream")
            .and_then(|e| e.as_bool())
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn ticket_json(status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": 42,
            "subject": "Cannot reset PIN",
            "description": "Customer locked out after 2FA move.",
            "status": status,
            "priority": "urgent",
            "requester_id": 991,
            "generated_at": "2026-08-24T10:00:00Z",
            "updated_at": "2026-08-24T10:00:00Z"
        })
    }

    #[test]
    fn ticket_maps_solved_to_closed() {
        let c = map_ticket("acme", &ticket_json("solved")).unwrap();
        assert_eq!(c.status, CaseStatus::ClosedSolved);
        assert_eq!(c.case_ref(), "crm:zendesk:acme:42");
        let o = map_ticket("acme", &ticket_json("open")).unwrap();
        assert_eq!(o.status, CaseStatus::Open);
    }

    struct MockZendesk {
        pages: RefCell<Vec<serde_json::Value>>,
        seen_urls: RefCell<Vec<String>>,
    }
    impl super::super::VendorTransport for MockZendesk {
        fn get_json(&self, url: &str, _auth: &str) -> Result<serde_json::Value> {
            self.seen_urls.borrow_mut().push(url.to_string());
            let mut pages = self.pages.borrow_mut();
            if pages.is_empty() {
                anyhow::bail!("no more mocked pages");
            }
            Ok(pages.remove(0))
        }
        fn post_form(
            &self,
            _url: &str,
            _form: &str,
            _auth: Option<&str>,
        ) -> Result<serde_json::Value> {
            anyhow::bail!("zendesk sync posts nothing")
        }
    }

    /// The named pin: two passes over identical mock responses produce
    /// identical case sets (idempotent translation), and every URL stays on
    /// the pinned host with the cadence floor holding at 300s.
    #[test]
    fn zendesk_cursor_sync_is_idempotent_and_respects_cadence()
    -> Result<(), Box<dyn std::error::Error>> {
        let base = api_base("acme")?;
        assert_eq!(base, "https://acme.zendesk.com");
        let mk = || MockZendesk {
            pages: RefCell::new(vec![serde_json::json!({
                "tickets": [ticket_json("open")],
                "after_cursor": "cur-1",
                "end_of_stream": false
            })]),
            seen_urls: RefCell::new(Vec::new()),
        };
        let pass_a = mk();
        let pass_b = mk();
        let a = fetch_page(&pass_a, &base, "Basic x", None, 0, "acme")?;
        let b = fetch_page(&pass_b, &base, "Basic x", None, 0, "acme")?;
        assert_eq!(
            a.cases, b.cases,
            "identical responses translate identically"
        );
        assert_eq!(a.after_cursor.as_deref(), Some("cur-1"));
        // Resumed page pins to the same base with the opaque token as query.
        let next = cursor_url(&base, a.after_cursor.as_deref(), 0);
        assert!(next.starts_with(
            "https://acme.zendesk.com/api/v2/incremental/tickets/cursor.json?cursor=cur-1"
        ));
        for url in pass_b.seen_urls.borrow().iter() {
            assert!(
                url.starts_with("https://acme.zendesk.com/"),
                "host pin: {url}"
            );
        }
        assert_eq!(
            super::super::MIN_POLL_INTERVAL_SECS,
            300,
            "poll cadence must respect the ~10 req/min incremental cap"
        );
        Ok(())
    }

    #[test]
    fn subdomain_validation_refuses_injection() {
        assert!(api_base("acme.evil.net").is_err());
        assert!(api_base("").is_err());
        assert!(api_base("../up").is_err());
        assert!(api_base("acme").is_ok());
    }

    #[test]
    fn basic_auth_shape() {
        use base64::Engine;
        let h = basic_auth("ops@acme.com", "tok");
        let raw = h.strip_prefix("Basic ").expect("basic prefix");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .unwrap();
        assert_eq!(decoded, b"ops@acme.com/token:tok");
    }
}
