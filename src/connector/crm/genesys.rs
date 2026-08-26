//! Genesys Cloud connector — workitems by worktype + external-contact
//! identity resolution.
//!
//! Cases are modeled as **workitems** (`/api/v2/conversations/workitems`);
//! customer identity resolves through `/api/v2/externalcontacts` (the
//! `externalContactId` on participants). Only the contact ID is stored —
//! the `subject_ref` is its salted hash, never PII beyond the hash policy.
//!
//! Hosts derive from the configured region (`{region}` = e.g.
//! `mypurecloud.com` or a regional domain like
//! `us-east-1.mypurecloud.com`): API at `api.{region}`, OAuth at
//! `login.{region}`. Both pinned; server cursors ride as query parameters.

use super::{CaseStatus, CrmCase, SOURCE_GENESYS};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Validate + build the API base for a region.
pub fn api_base(region: &str) -> Result<String> {
    let r = region.trim();
    if r.is_empty()
        || !r
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        anyhow::bail!("genesys region must be lowercase host chars, got {region:?}");
    }
    Ok(format!("https://api.{r}"))
}

/// The OAuth login base for a region.
pub fn login_base(region: &str) -> Result<String> {
    Ok(format!("https://login.{}", region.trim()))
}

/// Map one workitem to the normalized case, resolving the customer identity
/// from the external-contacts cache. Unresolved contacts fall back to the raw
/// contact id as the identity input — still hashed before storage.
pub fn map_workitem(
    org_id: &str,
    v: &serde_json::Value,
    contacts: &HashMap<String, String>,
) -> Result<CrmCase> {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .context("workitem missing id")?;
    let status_raw = v
        .get("status")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("statusCategory").and_then(|s| s.as_str()))
        .unwrap_or("")
        .to_ascii_lowercase();
    let status = if status_raw == "closed" || status_raw == "complete" {
        CaseStatus::ClosedSolved
    } else {
        CaseStatus::Open
    };
    let contact_id = v
        .get("externalContactId")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let identity = if contact_id.is_empty() {
        "workitem:unknown-contact".to_string()
    } else {
        match contacts.get(contact_id) {
            Some(resolved) => format!("contact:{resolved}"),
            None => format!("contact:{contact_id}"),
        }
    };
    Ok(CrmCase {
        source: SOURCE_GENESYS.into(),
        org_id: org_id.into(),
        case_id: id.to_string(),
        title: v
            .get("name")
            .or_else(|| v.get("subject"))
            .and_then(|s| s.as_str())
            .unwrap_or("(no subject)")
            .to_string(),
        status,
        priority: v
            .get("priority")
            .and_then(|p| p.as_str())
            .map(str::to_string),
        subject_ref: super::hash_subject(SOURCE_GENESYS, org_id, &identity),
        updated_rev: v
            .get("version")
            .map(|ver| ver.to_string())
            .or_else(|| {
                v.get("date_modified")
                    .and_then(|d| d.as_str())
                    .map(str::to_string)
            })
            .context("workitem missing version/date_modified")?,
        body_markdown: format!(
            "# Genesys workitem {id}\n\n{}",
            v.get("description").and_then(|d| d.as_str()).unwrap_or("")
        ),
        // Structured symptom fields ride custom attributes when configured:
        // `attributes.seed` / `attributes.not_seed`.
        is_seed: v
            .pointer("/attributes/seed")
            .and_then(|s| s.as_str())
            .map(str::to_string),
        is_not_seed: v
            .pointer("/attributes/not_seed")
            .and_then(|s| s.as_str())
            .map(str::to_string),
        updated_at: v
            .get("date_modified")
            .and_then(|d| d.as_str())
            .unwrap_or_default()
            .to_string(),
        merged_into: None,
        reopened: false,
    })
}

/// Workitems-by-worktype URL with cursor paging (opaque `after` token).
pub fn workitems_url(base: &str, worktype: &str, after: Option<&str>) -> String {
    let mut url = format!(
        "{base}/api/v2/conversations/workitems?pageSize=100&worktype={}",
        super::salesforce::urlencode(worktype)
    );
    if let Some(a) = after {
        url.push_str("&after=");
        url.push_str(&super::salesforce::urlencode(a));
    }
    url
}

/// Translate one workitems page; returns cases + the next cursor.
pub fn translate_page(
    org_id: &str,
    body: &serde_json::Value,
    contacts: &HashMap<String, String>,
) -> Result<(Vec<CrmCase>, Option<String>)> {
    let rows = body
        .get("entities")
        .and_then(|e| e.as_array())
        .context("workitems response missing entities array")?;
    let cases = rows
        .iter()
        .map(|r| map_workitem(org_id, r, contacts))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        cases,
        body.get("cursor")
            .or_else(|| body.pointer("/nextUri"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::VendorTransport;
    use super::*;

    fn workitem(status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "wi-9",
            "name": "Cannot reset PIN",
            "description": "Customer locked out.",
            "status": status,
            "priority": "high",
            "version": 3,
            "date_modified": "2026-08-24T10:00:00Z",
            "externalContactId": "ec-77",
            "attributes": {"seed": "2FA migration broke PIN reset"}
        })
    }

    /// The named pin: workitem → case translation with external-contact
    /// identity resolution, structured seed passthrough, terminal mapping.
    #[test]
    fn genesys_workitem_maps_to_case_with_external_contact() {
        let mut contacts = HashMap::new();
        contacts.insert("ec-77".to_string(), "cust-uuid-42".to_string());
        let c = map_workitem("acme", &workitem("closed"), &contacts).expect("workitem maps");
        assert_eq!(c.case_ref(), "crm:genesys:acme:wi-9");
        assert_eq!(c.status, CaseStatus::ClosedSolved);
        assert_eq!(c.is_seed.as_deref(), Some("2FA migration broke PIN reset"));
        assert!(
            !format!("{c:?}").contains("cust-uuid-42"),
            "identity must appear only as its salted hash"
        );

        // Unresolved contact id still hashes (never raw into storage).
        let empty = HashMap::new();
        let c2 = map_workitem("acme", &workitem("open"), &empty).expect("unresolved maps");
        assert_eq!(c2.status, CaseStatus::Open);
        assert_ne!(c.subject_ref, c2.subject_ref);

        let complete =
            map_workitem("acme", &workitem("complete"), &contacts).expect("complete maps");
        assert_eq!(complete.status, CaseStatus::ClosedSolved);
    }

    struct MockGenesys;
    impl super::super::VendorTransport for MockGenesys {
        fn get_json(&self, url: &str, _auth: &str) -> Result<serde_json::Value> {
            if url.contains("after=page-2") {
                return Ok(serde_json::json!({"entities": [], "cursor": null}));
            }
            Ok(serde_json::json!({
                "entities": [workitem("open")],
                "cursor": "page-2"
            }))
        }
        fn post_form(&self, _: &str, _: &str, _: Option<&str>) -> Result<serde_json::Value> {
            anyhow::bail!("not used in this test")
        }
    }

    #[test]
    fn page_translation_and_cursor_paging_stay_pinned() {
        let base = api_base("us-east-1.mypurecloud.com").expect("api base");
        assert_eq!(base, "https://api.us-east-1.mypurecloud.com");
        assert_eq!(
            login_base("us-east-1.mypurecloud.com").expect("login base"),
            "https://login.us-east-1.mypurecloud.com"
        );
        let contacts = HashMap::new();
        let t = MockGenesys;
        let (cases, cursor) = translate_page(
            "acme",
            &t.get_json(&workitems_url(&base, "support", None), "Bearer x")
                .expect("page fetch"),
            &contacts,
        )
        .expect("page translates");
        assert_eq!(cases.len(), 1);
        assert_eq!(cursor.as_deref(), Some("page-2"));
        // Cursor rides as an encoded query param on the pinned base only.
        let next = workitems_url(&base, "support", cursor.as_deref());
        assert!(next.starts_with("https://api.us-east-1.mypurecloud.com/"));
        assert!(next.contains("after=page-2"));
        assert!(api_base("../evil").is_err());
    }
}
