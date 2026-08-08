//! Subjects panel — the DSAR console (DESIGN §4.3). Locate → export → purge →
//! deletion certificate with a client-side chain-verify badge. The defining
//! screenshot: a subject, a found-count, and a green "chain verified" cert.
//!
//! v1.16.0 M5: a structured certificate CARD (found_count, purged_ids,
//! tombstone_root, chain_head, certified_at) with a live green/red chain badge,
//! replacing the old freeform status line. Confirmation-first (DESIGN §1.7).

use crate::api::{ApiClient, DsarCertificate};
use crate::panels::{use_document_title, PageTitle};
use crate::{Route, UiState};
use dioxus::prelude::*;

const MAX_SUBJECT: usize = 2000; // mirrors the backend's bound

/// M5 pure: the chain badge state — green "chain verified" or red "CHAIN
/// TAMPERED". Extracted so the card rendering is plumbing.
pub fn chain_badge(chain_verifies: bool) -> (&'static str, &'static str) {
    if chain_verifies {
        ("text-ok", "✓ chain verified")
    } else {
        ("text-danger", "✗ CHAIN TAMPERED")
    }
}

/// The post-purge result rendered as a structured card (M5.1). Owned so the
/// certificate fields are typed, not raw-JSON-indexed.
#[derive(Clone, PartialEq)]
struct DsarResult {
    id: i64,
    subject: String,
    cert: DsarCertificate,
}

pub fn panel() -> Element {
    use_document_title(|| "Subjects (DSAR) — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let mut subject = use_signal(String::new);
    let mut result = use_signal(|| None::<Result<DsarResult, String>>);
    let mut busy = use_signal(|| false);

    rsx! {
        PageTitle { "Subjects (DSAR)" }
        div { class: "card mt-2",
            div { class: "card-body space-y-3",
                div { class: "flex gap-2",
                    input {
                        class: "input flex-1",
                        maxlength: MAX_SUBJECT,
                        placeholder: "subject / owner / principal…",
                        value: "{subject}",
                        oninput: move |e| subject.set(e.value()),
                        "aria-label": "subject to action",
                    }
                }
                div { class: "flex gap-2",
                    button {
                        class: "btn btn-outline btn-md",
                        disabled: busy() || !writes,
                        onclick: move |_| async move {
                            let s = subject().trim().to_string();
                            if s.is_empty() { result.set(Some(Err("enter a subject first".into()))); return; }
                            busy.set(true);
                            result.set(Some(run_dsar(api(), s, "export").await));
                            busy.set(false);
                        },
                        "Locate & export"
                    }
                    button {
                        class: "btn btn-destructive btn-md",
                        disabled: busy() || !writes,
                        onclick: move |_| async move {
                            let s = subject().trim().to_string();
                            if s.is_empty() { result.set(Some(Err("enter a subject first".into()))); return; }
                            busy.set(true);
                            result.set(Some(run_dsar(api(), s, "both").await));
                            busy.set(false);
                        },
                        "Locate, export & purge"
                    }
                }
                if busy() {
                    p { class: "text-muted-foreground", "running…" }
                }
                match &*result.read() {
                    Some(Ok(r)) => rsx! { CertificateCard { result: r.clone() } },
                    Some(Err(msg)) => rsx! { p { class: "text-danger mt-2", "{msg}" } },
                    None => rsx! {},
                }
            }
        }
        p { class: "text-ink-faint mt-4 text-sm",
            "Purge is irreversible: it writes a tombstone + hash-chain entry. "
            "The deletion certificate re-verifies the chain head live."
        }
    }
}

/// v1.16.7 M1: subject shown on the certificate card header — read straight
/// off the cert object (the `/dsar/{id}/certificate` response carries it).
/// Pure so the deep-link detail can rebuild the `DsarResult` from a fetch.
fn subject_of(cert: &DsarCertificate) -> String {
    cert.certificate
        .get("subject")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string()
}

/// v1.16.7 M1: the deep-linkable deletion certificate (`/subjects/certificate/:dsar_id`).
/// Re-fetches `GET /dsar/{id}/certificate` and renders the SAME card the
/// Subjects panel shows, with the LIVE chain badge — GDPR Art 17 evidence a
/// subject (or auditor) can be pointed at directly.
pub fn detail(dsar_id: i64) -> Element {
    use_document_title(move || format!("Deletion certificate #{dsar_id} — brain"));
    let api = use_context::<Signal<ApiClient>>();
    let cert = use_resource(move || {
        let api = api();
        async move { api.dsar_certificate(dsar_id).await }
    });
    rsx! {
        PageTitle { "Deletion certificate #{dsar_id}" }
        p { class: "text-xs text-muted-foreground mb-3",
            Link { to: Route::Subjects {}, "← back to subjects" } }
        match &*cert.read() {
            Some(Ok(v)) => {
                let c = DsarCertificate::from_value(v.clone());
                rsx! { CertificateCard { result: DsarResult { id: dsar_id, subject: subject_of(&c), cert: c } } }
            }
            Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "certificate failed: {e}" } },
            None => rsx! { p { class: "text-muted-foreground mt-2", "loading…" } },
        }
    }
}

/// M5.1: the structured certificate card — the defining screenshot. Renders
/// found_count, purged_ids (monospace), tombstone_root, certified_at, chain_head,
/// and the LIVE green/red chain badge (re-checked via GET /dsar/{id}/certificate).
#[component]
fn CertificateCard(result: DsarResult) -> Element {
    let cert = result.cert.clone();
    let (badge_class, badge_text) = chain_badge(cert.chain_verifies);
    let purged = cert
        .purged_ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let root = cert
        .tombstone_root
        .map(|r| format!("chunk #{r}"))
        .unwrap_or_else(|| "—".into());
    rsx! {
        div { class: "card mt-2",
            div { class: "card-header",
                h2 { class: "card-title", "Deletion certificate #{result.id}" }
                Link {
                    class: "font-mono text-xs text-accent hover:underline",
                    to: Route::DsarDetail { dsar_id: result.id },
                    "subject: {result.subject}"
                }
            }
            dl { class: "card-body grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm",
                dt { class: "text-muted-foreground", "found" }
                dd { class: "font-mono tabular", "{cert.found_count}" }
                dt { class: "text-muted-foreground", "purged" }
                dd { class: "font-mono tabular", "{purged}" }
                dt { class: "text-muted-foreground", "tombstone root" }
                dd { class: "font-mono", "{root}" }
                dt { class: "text-muted-foreground", "certified" }
                dd { "{cert.certified_at}" }
                dt { class: "text-muted-foreground", "chain head" }
                dd { class: "font-mono text-xs", "{cert.chain_head}" }
            }
            div { class: "card-footer",
                span { class: "{badge_class} font-semibold text-sm", role: "status", "aria-live": "polite",
                    "{badge_text}" }
            }
        }
    }
}

/// Run one DSAR action against the server, returning the typed result.
/// Confirmation-first: every field derives from the actual server response;
/// the chain badge is the chain STILL holding (live re-verify), not cert-time.
async fn run_dsar(
    api: ApiClient,
    subject: String,
    action: &'static str,
) -> Result<DsarResult, String> {
    let resp = api
        .dsar(&subject, action)
        .await
        .map_err(|e| format!("dsar {action} failed: {e}"))?;
    // Live chain re-verify: the cert-time head is a snapshot; the badge must
    // reflect the chain holding NOW (DESIGN §8 defining-screenshot rule).
    let cert = match action {
        "purge" | "both" => api
            .dsar_certificate(resp.id)
            .await
            .map(DsarCertificate::from_value)
            .map_err(|e| format!("certificate fetch failed: {e}"))?,
        _ => {
            DsarCertificate::from_value(resp.certificate.clone().unwrap_or(serde_json::Value::Null))
        }
    };
    Ok(DsarResult {
        id: resp.id,
        subject: resp.subject,
        cert,
    })
}

// ---------------------------------------------------------------------------
// M5 tests — the chain badge + certificate field extraction.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// The chain badge reflects the live verify state (the defining screenshot).
    #[test]
    fn chain_badge_reflects_live_verify() {
        let (cls, txt) = chain_badge(true);
        assert_eq!(cls, "text-ok");
        assert!(txt.contains("verified"));
        let (cls, txt) = chain_badge(false);
        assert_eq!(cls, "text-danger");
        assert!(txt.contains("TAMPERED"));
    }

    /// The certificate card fields read straight off the server JSON (typed).
    #[test]
    fn certificate_card_fields_render_from_server_json() {
        let v = serde_json::json!({
            "certificate": {
                "subject": "u", "action": "purge", "found_count": 3,
                "purged_ids": [10, 20, 30], "tombstone_root": 10,
                "chain_head": "deadbeef", "certified_at": "2026-08-08T01:02:03Z"
            },
            "chain_verifies": true
        });
        let cert = DsarCertificate::from_value(v);
        assert_eq!(cert.found_count, 3);
        assert_eq!(cert.purged_ids, vec![10, 20, 30]);
        assert_eq!(cert.tombstone_root, Some(10));
        assert_eq!(cert.chain_head, "deadbeef");
        assert_eq!(cert.certified_at, "2026-08-08T01:02:03Z");
        assert!(cert.chain_verifies);
    }

    /// v1.16.7 M1: the deep-link cert header subject reads off the cert object;
    /// absent subject → empty string (the card renders it as the id).
    #[test]
    fn subject_of_reads_cert_object_subject() {
        let v = serde_json::json!({
            "certificate": { "subject": "alice", "action": "purge", "found_count": 1 },
            "chain_verifies": true
        });
        let cert = DsarCertificate::from_value(v);
        assert_eq!(subject_of(&cert), "alice");
        let bare = DsarCertificate::from_value(serde_json::json!({ "chain_verifies": true }));
        assert_eq!(subject_of(&bare), "");
    }
}
