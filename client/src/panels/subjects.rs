//! Subjects panel — the DSAR console (DESIGN §4.3). Locate → export → purge →
//! deletion certificate with a client-side chain-verify badge. The defining
//! screenshot: a subject, a found-count, and a green "chain verified" cert.
//!
//! v1.16.0 M5: a structured certificate CARD (found_count, purged_ids,
//! tombstone_root, chain_head, certified_at) with a live green/red chain badge,
//! replacing the old freeform status line. Confirmation-first (DESIGN §1.7).

use crate::api::{ApiClient, DsarCertificate};
use crate::panels::{use_document_title, PageTitle};
use crate::{DrawerContent, UiState};
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
        div { class: "flex gap-2 my-2",
            input {
                class: "border border-border-subtle surface-raised rounded px-2 py-1 flex-1",
                maxlength: MAX_SUBJECT,
                placeholder: "subject / owner / principal…",
                value: "{subject}",
                oninput: move |e| subject.set(e.value()),
                "aria-label": "subject to action",
            }
            button {
                class: "border border-border-subtle surface-raised rounded px-2 py-1 text-sm disabled:opacity-50",
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
                class: "border border-border-subtle rounded px-2 py-1 text-sm bg-danger text-white disabled:opacity-50",
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
            p { class: "text-ink-muted", "running…" }
        }
        match &*result.read() {
            Some(Ok(r)) => rsx! { CertificateCard { result: r.clone() } },
            Some(Err(msg)) => rsx! { p { class: "text-danger mt-2", "{msg}" } },
            None => rsx! {},
        }
        p { class: "text-ink-faint mt-4 text-sm",
            "Purge is irreversible: it writes a tombstone + hash-chain entry. "
            "The deletion certificate re-verifies the chain head live."
        }
    }
}

/// M5.1: the structured certificate card — the defining screenshot. Renders
/// found_count, purged_ids (monospace), tombstone_root, certified_at, chain_head,
/// and the LIVE green/red chain badge (re-checked via GET /dsar/{id}/certificate).
#[component]
fn CertificateCard(result: DsarResult) -> Element {
    let ui = use_context::<UiState>();
    let cert = result.cert.clone();
    let mut ui = ui;
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
        div { class: "surface-raised border hairline rounded p-4 mt-2",
            div { class: "flex justify-between items-start",
                h2 { class: "text-sm font-semibold", "Deletion certificate #{result.id}" }
                button {
                    class: "font-mono text-xs text-accent hover:underline",
                    onclick: move |_| ui.drawer.set(Some(DrawerContent::Certificate(cert.clone()))),
                    "subject: {result.subject}"
                }
            }
            dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 mt-2 text-sm",
                dt { class: "text-ink-muted", "found" }
                dd { class: "font-mono tabular", "{cert.found_count}" }
                dt { class: "text-ink-muted", "purged" }
                dd { class: "font-mono tabular", "{purged}" }
                dt { class: "text-ink-muted", "tombstone root" }
                dd { class: "font-mono", "{root}" }
                dt { class: "text-ink-muted", "certified" }
                dd { "{cert.certified_at}" }
                dt { class: "text-ink-muted", "chain head" }
                dd { class: "font-mono text-xs", "{cert.chain_head}" }
            }
            p { class: "{badge_class} font-semibold mt-2 text-sm", "{badge_text}" }
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
}
