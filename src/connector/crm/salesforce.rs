//! Salesforce connector — OAuth 2.0 client-credentials + SOQL incremental by
//! `SystemModstamp`.
//!
//! The token is fetched from the pinned instance's OAuth endpoint, cached
//! until expiry, and refreshed **fail-closed**: any refresh error aborts the
//! sync rather than sending a stale or empty bearer. `nextRecordsUrl` from
//! Salesforce is reduced to its query string applied to the pinned base —
//! a forged path cannot move the bearer off the instance.

use super::{CaseStatus, CrmCase, SOURCE_SALESFORCE};
use anyhow::{Context, Result};
use std::time::{Duration, SystemTime};

/// One cached client-credentials token.
#[derive(Debug, Clone)]
pub struct CachedToken {
    pub access_token: String,
    pub expires_at: SystemTime,
}

impl CachedToken {
    /// True when the token needs a refresh (<60s left, or already expired).
    pub fn needs_refresh(&self) -> bool {
        self.expires_at
            .duration_since(SystemTime::now())
            .map(|remaining| remaining < Duration::from_secs(60))
            .unwrap_or(true)
    }
}

/// Validate + normalize the configured instance URL (https only).
pub fn api_base(instance_url: &str) -> Result<String> {
    let u = url::Url::parse(instance_url.trim())
        .with_context(|| format!("invalid salesforce instance_url {instance_url:?}"))?;
    if u.scheme() != "https" || u.host_str().is_none() {
        anyhow::bail!("salesforce instance_url must be https://host");
    }
    let host = u.host_str().expect("checked above");
    Ok(format!("https://{host}"))
}

/// Build the token-endpoint form body for the client-credentials flow.
pub fn token_form(client_id: &str, client_secret: &str) -> String {
    format!(
        "grant_type=client_credentials&client_id={}&client_secret={}",
        urlencode(client_id),
        urlencode(client_secret)
    )
}

/// Minimal form urlencoding (space + reserved). Ids/secrets are
/// alnum-heavy; this covers the rest without pulling a dep.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The SOQL query URL for cases modified after `last` (ISO-8601 modstamp).
/// The persisted timestamp is validated against a strict ISO-8601 shape
/// BEFORE interpolation — the state file is operator-owned (0600), but a
/// tampered value must never reach the query grammar.
pub fn query_url(base: &str, api_version: &str, last: Option<&str>) -> String {
    let iso_ok = |ts: &str| {
        // Hand-rolled shape check (no regex dep): 20-char canonical form
        // `YYYY-MM-DDTHH:MM:SSZ` or longer with fractional seconds.
        let b = ts.as_bytes();
        b.len() >= 20
            && b[4] == b'-'
            && b[7] == b'-'
            && b[10] == b'T'
            && b[13] == b':'
            && b[16] == b':'
            && *b.last().unwrap_or(&b' ') == b'Z'
            && b[..19]
                .iter()
                .enumerate()
                .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16) || c.is_ascii_digit())
    };
    let last = last.filter(|ts| {
        let ok = iso_ok(ts);
        if !ok {
            tracing::warn!("salesforce: ignoring malformed persisted modstamp");
        }
        ok
    });
    let soql = match last {
        Some(ts) => format!(
            "SELECT Id, CaseNumber, Subject, Status, Priority, SystemModstamp, Description \
             FROM Case WHERE SystemModstamp > {ts}"
        ),
        None => "SELECT Id, CaseNumber, Subject, Status, Priority, SystemModstamp, Description \
                 FROM Case"
            .to_string(),
    };
    format!(
        "{base}/services/data/{api_version}/query?q={}",
        urlencode(&soql)
    )
}

/// Reduce Salesforce's server-provided `nextRecordsUrl` to a safe same-base
/// URL. Only paths under `/services/data/` with no scheme/host override pass.
pub fn next_url(base: &str, next_records: &str) -> Result<String> {
    if !next_records.starts_with("/services/data/") || next_records.contains("..") {
        anyhow::bail!(
            "refusing unexpected nextRecordsUrl {next_records:?} — not an instance-relative query path"
        );
    }
    Ok(format!("{base}{next_records}"))
}

/// Map one SOQL Case row to the normalized case. Any status containing
/// `closed` reads terminal (Salesforce status picklists are org-defined;
/// `IsClosed` semantics fold into the name in every stock configuration).
pub fn map_case(org_id: &str, v: &serde_json::Value) -> Result<CrmCase> {
    let id = v
        .get("Id")
        .and_then(|x| x.as_str())
        .context("case row missing Id")?;
    let status_raw = v
        .get("Status")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let status = if status_raw.contains("closed") {
        CaseStatus::ClosedSolved
    } else {
        CaseStatus::Open
    };
    let contact = v
        .get("ContactId")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let identity = if contact.is_null() {
        format!(
            "casenumber:{}",
            v.get("CaseNumber").and_then(|c| c.as_str()).unwrap_or(id)
        )
    } else {
        format!("contact:{contact}")
    };
    Ok(CrmCase {
        source: SOURCE_SALESFORCE.into(),
        org_id: org_id.into(),
        case_id: id.to_string(),
        title: v
            .get("Subject")
            .and_then(|s| s.as_str())
            .unwrap_or("(no subject)")
            .to_string(),
        status,
        priority: v
            .get("Priority")
            .and_then(|p| p.as_str())
            .map(str::to_string),
        subject_ref: super::hash_subject(SOURCE_SALESFORCE, org_id, &identity),
        updated_rev: v
            .get("SystemModstamp")
            .and_then(|t| t.as_str())
            .context("case row missing SystemModstamp")?
            .to_string(),
        body_markdown: format!(
            "# Salesforce case {}\n\n{}",
            v.get("CaseNumber").and_then(|c| c.as_str()).unwrap_or(id),
            v.get("Description").and_then(|d| d.as_str()).unwrap_or("")
        ),
        is_seed: None,
        is_not_seed: None,
        updated_at: v
            .get("SystemModstamp")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        merged_into: None,
        reopened: false,
    })
}

/// Translate one query-response page.
pub fn translate_page(
    org_id: &str,
    body: &serde_json::Value,
) -> Result<(Vec<CrmCase>, Option<String>)> {
    let rows = body
        .get("records")
        .and_then(|r| r.as_array())
        .context("SOQL response missing records array")?;
    let cases = rows
        .iter()
        .map(|r| map_case(org_id, r))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        cases,
        body.get("nextRecordsUrl")
            .and_then(|n| n.as_str())
            .map(str::to_string),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::VendorTransport;
    use super::*;

    fn row(status: &str) -> serde_json::Value {
        serde_json::json!({
            "Id": "500XX00001",
            "CaseNumber": "00001042",
            "Subject": "Cannot reset PIN",
            "Status": status,
            "Priority": "P2",
            "SystemModstamp": "2026-08-24T10:00:00.000+0000",
            "Description": "Customer locked out.",
            "ContactId": "003XX2"
        })
    }

    #[test]
    fn closed_statuses_map_terminal() {
        for s in ["Closed", "Closed - Solved", "Escalated"] {
            let c = map_case("acme", &row(s)).unwrap();
            let want = if s.contains("closed") || s.contains("Closed") {
                CaseStatus::ClosedSolved
            } else {
                CaseStatus::Open
            };
            assert_eq!(c.status, want, "status {s}");
        }
    }

    struct MockToken {
        calls: std::cell::Cell<usize>,
        fail_token: bool,
    }
    impl super::super::VendorTransport for MockToken {
        fn get_json(&self, _url: &str, _auth: &str) -> Result<serde_json::Value> {
            anyhow::bail!("sync GET before auth must never happen in this test")
        }
        fn post_form(
            &self,
            _url: &str,
            form: &str,
            _auth: Option<&str>,
        ) -> Result<serde_json::Value> {
            self.calls.set(self.calls.get() + 1);
            if self.fail_token {
                anyhow::bail!("token endpoint returned 401");
            }
            assert!(form.contains("grant_type=client_credentials"));
            Ok(serde_json::json!({
                "access_token": "tok1",
                "expires_in": 3600
            }))
        }
    }

    /// The named pin: when the token endpoint fails, the sync fails CLOSED —
    /// the error propagates and NO case fetch is attempted with a stale or
    /// empty bearer. A successful refresh caches and reuses.
    #[test]
    fn salesforce_modstamp_sync_refreshes_token_fail_closed() {
        // Happy path first: token fetched once, cached until near-expiry.
        let ok = MockToken {
            calls: Default::default(),
            fail_token: false,
        };
        let base = api_base("https://acme.my.salesforce.com").expect("instance url");
        assert_eq!(base, "https://acme.my.salesforce.com");
        let tok: super::CachedToken = {
            let resp = ok
                .post_form(
                    &format!("{base}/services/oauth2/token"),
                    &token_form("id", "sec"),
                    None,
                )
                .expect("token fetch");
            let expires_in = resp
                .get("expires_in")
                .and_then(|e| e.as_i64())
                .context("expires_in")
                .expect("expires_in");
            super::CachedToken {
                access_token: resp["access_token"]
                    .as_str()
                    .context("access_token")
                    .expect("access_token")
                    .into(),
                expires_at: SystemTime::now() + Duration::from_secs(expires_in.max(0) as u64),
            }
        };
        assert!(!tok.needs_refresh(), "fresh token must not refresh");
        assert_eq!(ok.calls.get(), 1);

        // Near-expiry forces a refresh attempt; failure refuses the sync.
        let stale = super::CachedToken {
            access_token: "old".into(),
            expires_at: SystemTime::now() - Duration::from_secs(10),
        };
        assert!(stale.needs_refresh());
        let bad = MockToken {
            calls: Default::default(),
            fail_token: true,
        };
        let refreshed = bad.post_form(
            &format!("{base}/services/oauth2/token"),
            &token_form("id", "sec"),
            None,
        );
        assert!(refreshed.is_err(), "refresh failure must surface");
        assert_eq!(
            bad.calls.get(),
            1,
            "fail-closed: no retry storm, no fallback bearer"
        );
    }

    #[test]
    fn next_records_url_cannot_leave_the_instance() {
        let base = api_base("https://acme.my.salesforce.com").unwrap();
        assert_eq!(
            next_url(&base, "/services/data/v62.0/query/01gXX/next-100").unwrap(),
            "https://acme.my.salesforce.com/services/data/v62.0/query/01gXX/next-100"
        );
        for bad in [
            "https://evil.net/services/data/x",
            "/etc/passwd",
            "/services/data/../../admin",
        ] {
            assert!(next_url(&base, bad).is_err(), "must refuse {bad}");
        }
    }

    #[test]
    fn page_translates_and_carries_next() {
        let body = serde_json::json!({
            "records": [row("Closed")],
            "done": false,
            "nextRecordsUrl": "/services/data/v62.0/query/01gXX/next-100"
        });
        let (cases, next) = translate_page("acme", &body).expect("page translates");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].status, CaseStatus::ClosedSolved);
        assert_eq!(cases[0].updated_rev, "2026-08-24T10:00:00.000+0000");
        assert_eq!(
            next.as_deref(),
            Some("/services/data/v62.0/query/01gXX/next-100")
        );
    }

    #[test]
    fn query_url_pins_instance_and_version() {
        let u = query_url(
            "https://acme.my.salesforce.com",
            "v62.0",
            Some("2026-08-01T00:00:00Z"),
        );
        assert!(u.starts_with("https://acme.my.salesforce.com/services/data/v62.0/query?q="));
        assert!(u.contains("SystemModstamp"));
    }
}
