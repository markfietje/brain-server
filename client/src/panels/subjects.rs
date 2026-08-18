//! Subjects panel — the DSAR console (DESIGN §4.3). Locate → export → purge →
//! deletion certificate with a client-side chain-verify badge. The defining
//! screenshot: a subject, a found-count, and a green "chain verified" cert.
//!
//! v1.16.0 M5: a structured certificate CARD (found_count, purged_ids,
//! tombstone_root, chain_head, certified_at) with a live green/red chain badge,
//! replacing the old freeform status line. Confirmation-first (DESIGN §1.7).

use crate::api::{ApiClient, DsarCertificate, DsarLedgerRow, Footprint};
use crate::i18n::{t, t_fmt};
use crate::panels::{use_document_title, PageTitle};
use crate::time_budget::{format_remaining, now_unix, remaining, tier, Tier};
use crate::{Route, UiState};
use dioxus::prelude::*;

const MAX_SUBJECT: usize = 2000; // mirrors the backend's bound

/// v1.20.22 M2.1: the DSAR Art 17 countdown bands — day-scale SLA (<3d warn,
/// <1d danger), unlike the review queue's hour-scale bands. `tier()` consumes
/// them identically; only the numbers differ per surface.
const DSAR_WARN_SECS: i64 = 3 * 86400;
const DSAR_CRITICAL_SECS: i64 = 86400;

/// v1.20.22 M2.1: render one open-ledger-row's clock as `(badge_class,
/// label)`. The deadline is server-provided (`row.deadline` — the same number
/// the POST response carries), so the countdown honors an operator's
/// `BRAIN_DSAR_WINDOW_DAYS` override with no client mirror. `Some` only for an
/// open row that carries a deadline.
pub fn dsar_clock(row: &DsarLedgerRow, now: i64) -> Option<(&'static str, String)> {
    let deadline = row.deadline?;
    let t = tier(remaining(deadline, now), DSAR_WARN_SECS, DSAR_CRITICAL_SECS);
    let cls = match t {
        Tier::Critical | Tier::Expired => "badge badge-danger",
        Tier::Warn => "badge badge-warn",
        Tier::Ok => "badge badge-ok",
    };
    Some((cls, format_remaining(remaining(deadline, now))))
}

/// M5 pure: the chain badge state — returns the class + the i18n KEY of the
/// status text (the render site resolves it so the certificate card speaks the
/// operator's locale). Extracted so the card rendering is plumbing.
pub fn chain_badge(chain_verifies: bool) -> (&'static str, &'static str) {
    if chain_verifies {
        ("text-ok", "chain_verified")
    } else {
        ("text-danger", "chain_tampered")
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

/// v1.20.0 M3: a DSAR outcome. `Queued` is an offline window (the action is
/// enqueued for replay) — rendered as an amber note, never a red error.
enum DsarOutcome {
    Done(DsarResult),
    Queued,
    Failed(String),
}

pub fn panel() -> Element {
    use_document_title(|| "Subjects (DSAR) — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let mut subject = use_signal(String::new);
    let mut result = use_signal(|| None::<DsarOutcome>);
    let mut busy = use_signal(|| false);
    // v1.20.21 M2: the dry-run footprint preview — see-before-erase.
    let mut prev_subject = use_signal(String::new);
    let mut prev = use_signal(|| None::<Result<Footprint, String>>);
    let mut prev_busy = use_signal(|| false);
    // v1.28.1 M4 (F-12): the two-step purge gate. The erasing hand is armed
    // only after the footprint for the CURRENT subject has rendered; the
    // confirm button below stays frozen otherwise (`purge_preview_ready`).
    let mut pending_purge = use_signal(|| None::<String>);
    // The ready-state MUST be computed outside the rsx (branches hold nodes
    // only); recomputed every render, so a subject edit re-freezes.
    let pending_ui = pending_purge().map(|s| {
        let ready =
            crate::panels::subjects::purge_preview_ready(&s, &subject(), &prev_subject(), &prev());
        (s, ready)
    });
    // v1.20.22 M2.1: the DSAR request ledger + its Art 17 countdown. A single
    // ~30s on-load ticker (the ops.rs idiom) re-renders every countdown from a
    // fresh `now_unix()` — the honest near-real-time approximation.
    let ledger = use_signal(Vec::<DsarLedgerRow>::new);
    let ledger_total = use_signal(|| 0i64);
    let ledger_loaded = use_signal(|| false);
    let tick = use_signal(|| 0u64);
    use_future(move || {
        let mut tick = tick;
        async move {
            loop {
                crate::probe_sleep(30).await;
                tick += 1;
            }
        }
    });
    use_future(move || {
        let api = api();
        let mut ledger = ledger;
        let mut ledger_total = ledger_total;
        let mut ledger_loaded = ledger_loaded;
        let tick = tick;
        async move {
            loop {
                let _ = tick(); // subscribe → refetch once (and each tick)
                if let Ok(l) = api.dsar_ledger().await {
                    ledger.set(l.requests);
                    ledger_total.set(l.total);
                }
                ledger_loaded.set(true);
                crate::probe_sleep(30).await;
            }
        }
    });

    let dsar_running = crate::i18n::t("dsar_running");
    rsx! {
        PageTitle { {crate::i18n::t("subjects_title")} }
        div { class: "card mt-2",
            div { class: "card-body space-y-3",
                div { class: "flex gap-2",
                    input {
                        class: "input flex-1",
                        maxlength: MAX_SUBJECT,
                        placeholder: t("dsar_subject_placeholder"),
                        value: "{subject}",
                        oninput: move |e| subject.set(e.value()),
                        "aria-label": t("dsar_subject_aria"),
                    }
                }
                div { class: "flex gap-2",
                    button {
                        class: "btn btn-outline btn-md",
                        disabled: busy() || !writes,
                        onclick: move |_| async move {
                            let s = subject().trim().to_string();
                            if s.is_empty() { result.set(Some(DsarOutcome::Failed("enter a subject first".into()))); return; }
                            busy.set(true);
                            result.set(Some(run_dsar(api(), s, "export").await));
                            busy.set(false);
                        },
                        {t("dsar_locate_export")}
                    }
                    button {
                        class: "btn btn-destructive btn-md",
                        disabled: busy() || !writes,
                        onclick: move |_| async move {
                            let s = subject().trim().to_string();
                            if s.is_empty() { result.set(Some(DsarOutcome::Failed("enter a subject first".into()))); return; }
                            if crate::panels::subjects::purge_preview_ready(
                                &s, &s, &prev_subject(), &prev()
                            ) {
                                // Step 2 already rendered for THIS subject →
                                // the erasure may proceed.
                                busy.set(true);
                                result.set(Some(run_dsar(api(), s, "both").await));
                                busy.set(false);
                                pending_purge.set(None);
                            } else {
                                // Step 1: render the footprint for the current
                                // subject first (the confirm card then arms).
                                prev_subject.set(s.clone());
                                prev_busy.set(true);
                                let out = match api().dsar_preview(&s).await {
                                    Ok(fp) => Ok(fp),
                                    Err(e) => Err(t_fmt("dsar_preview_failed", &[e.to_string()])),
                                };
                                prev.set(Some(out));
                                prev_busy.set(false);
                                pending_purge.set(Some(s));
                            }
                        },
                        {t("dsar_locate_export_purge")}
                    }
                }
                // v1.28.1 M4: the confirm card — renders after step 1 and stays
                // frozen until the CAREFULLY previewed footprint is on screen.
                // `pending_ui` is precomputed (rsx branches hold nodes, not
                // statements) and re-arms each render: editing the subject
                // input after arming freezes the confirm (`ready` goes false).
                if let Some((pending, ready)) = pending_ui.clone() {
                    div { class: "card mt-2 border-danger/40",
                        div { class: "card-body space-y-2",
                            p { class: "text-sm text-danger", {crate::i18n::t("dsar_purge_confirm_title")} }
                            match &*prev.read() {
                                Some(Ok(fp)) => rsx! { FootprintCard { fp: fp.clone() } },
                                _ => rsx! { p { class: "text-sm text-warn", {crate::i18n::t("dsar_purge_need_preview")} } },
                            }
                            div { class: "flex items-center gap-2",
                                button {
                                    class: "btn btn-outline btn-sm",
                                    disabled: busy(),
                                    onclick: move |_| pending_purge.set(None),
                                    {t("cancel")}
                                }
                                button {
                                    class: "btn btn-destructive btn-sm",
                                    disabled: busy() || !writes || !ready,
                                    onclick: move |_| {
                                        let s = pending.clone();
                                        spawn(async move {
                                            busy.set(true);
                                            result.set(Some(run_dsar(api(), s, "both").await));
                                            busy.set(false);
                                            pending_purge.set(None);
                                        });
                                    },
                                    {crate::i18n::t("dsar_purge_confirm")}
                                }
                            }
                        }
                    }
                }
                if busy() {
                    p { class: "text-muted-foreground", "{dsar_running}" }
                }
                match &*result.read() {
                    Some(DsarOutcome::Done(r)) => rsx! { CertificateCard { result: r.clone() } },
                    Some(DsarOutcome::Queued) => rsx! { p { class: "text-warn mt-2", {t("dsar_queued")} } },
                    Some(DsarOutcome::Failed(msg)) => rsx! { p { class: "text-danger mt-2", "{msg}" } },
                    None => rsx! {},
                }
            }
        }
        div { class: "card mt-2",
            div { class: "card-header",
                h2 { class: "card-title", {crate::i18n::t("dsar_clock_title")} }
                span { class: "text-sm text-muted-foreground", "{ledger_total}" }
            }
            div { class: "card-body space-y-1",
                if ledger_loaded() && ledger().iter().all(|r| r.status == "completed") {
                    p { class: "text-sm text-muted-foreground", {crate::i18n::t("dsar_clock_empty")} }
                } else {
                    ul { class: "space-y-1",
                        for row in ledger() {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                // i18n-exempt: ledger row data — the id + subject verbatim (the subject IS
                                // signed data; the id is an autoincrement).
                                span { class: "font-mono text-xs", "#{row.id} {row.subject}" }
                                span { class: "flex items-center gap-2",
                                    if row.status == "completed" {
                                        span { class: "text-muted-foreground text-xs",
                                        {t_fmt("dsar_completed_retained", &[t("dsar_clock_completed"), t("dsar_clock_retained")])} }
                                    } else if let Some((cls, label)) = dsar_clock(&row, now_unix()) {
                                        span { class: "text-muted-foreground text-xs", "{row.action}" }
                                        span { class: "{cls} tabular",
                                            {t_fmt("dsar_clock_deadline", &[label])} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        div { class: "card mt-2",
            div { class: "card-header",
                h2 { class: "card-title", {crate::i18n::t("dsar_preview_title")} }
                p { class: "text-xs text-muted-foreground", {crate::i18n::t("dsar_preview_sub")} }
            }
            div { class: "card-body space-y-2",
                div { class: "flex gap-2",
                    input {
                        class: "input flex-1",
                        maxlength: MAX_SUBJECT,
                        placeholder: crate::i18n::t("dsar_preview_placeholder"),
                        value: "{prev_subject}",
                        oninput: move |e| prev_subject.set(e.value()),
                        "aria-label": crate::i18n::t("dsar_preview_placeholder"),
                    }
                    button {
                        class: "btn btn-outline btn-md",
                        disabled: prev_busy(),
                        onclick: move |_| async move {
                            let s = prev_subject().trim().to_string();
                            if s.is_empty() {
                                prev.set(Some(Err(t("dsar_subject_required"))));
                                return;
                            }
                            prev_busy.set(true);
                            let out = match api().dsar_preview(&s).await {
                                Ok(fp) => Ok(fp),
                                Err(e) => Err(t_fmt("dsar_preview_failed", &[e.to_string()])),
                            };
                            prev.set(Some(out));
                            prev_busy.set(false);
                        },
                        {crate::i18n::t("dsar_preview_button")}
                    }
                }
                if prev_busy() {
                    p { class: "text-muted-foreground", {t("dsar_previewing")} }
                }
                match &*prev.read() {
                    Some(Ok(fp)) => rsx! { FootprintCard { fp: fp.clone() } },
                    Some(Err(msg)) => rsx! { p { class: "text-danger mt-2", "{msg}" } },
                    None => rsx! {},
                }
            }
        }
        p { class: "text-ink-faint mt-4 text-sm",
            {t("dsar_purge_note")}
        }
    }
}

/// v1.20.21 M2: the footprint preview card — the exact would-be deletion counts
/// with an explicit "nothing deleted" note. No purge button here: seeing and
/// erasing stay one click apart.
#[component]
fn FootprintCard(fp: Footprint) -> Element {
    rsx! {
        div { class: "card mt-2 border-dashed",
            div { class: "card-body space-y-1",
                p { role: "status", class: "text-sm text-muted-foreground",
                    {crate::i18n::t("dsar_preview_note")} }
                dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm",
                    dt { class: "text-muted-foreground", {crate::i18n::t("dsar_preview_owners")} }
                    dd { class: "font-mono tabular", "{fp.roots}" }
                    dt { class: "text-muted-foreground", {crate::i18n::t("dsar_preview_derived")} }
                    dd { class: "font-mono tabular", "{fp.derived}" }
                    dt { class: "text-muted-foreground", {crate::i18n::t("dsar_preview_export_rows")} }
                    dd { class: "font-mono tabular", "{fp.export_rows}" }
                    dt { class: "text-muted-foreground", {crate::i18n::t("dsar_preview_tombstones")} }
                    dd { class: "font-mono tabular", "{fp.tombstones}" }
                    dt { class: "text-muted-foreground", {crate::i18n::t("dsar_preview_ledger_rows")} }
                    dd { class: "font-mono tabular", "{fp.dsar_rows}" }
                }
            }
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
    let page_title = t_fmt("deletion_certificate", &[format!("#{dsar_id}")]);
    let back_label = t("dsar_back_link");
    rsx! {
        PageTitle { "{page_title}" }
        p { class: "text-xs text-muted-foreground mb-3",
            Link { to: Route::Subjects {}, "{back_label}" } }
        match &*cert.read() {
            Some(Ok(v)) => {
                let c = DsarCertificate::from_value(v.clone());
                rsx! { CertificateCard { result: DsarResult { id: dsar_id, subject: subject_of(&c), cert: c } } }
            }
            Some(Err(e)) => rsx! { p { class: "text-danger mt-2", {t_fmt("cert_fetch_failed", &[e.to_string()])} } },
            None => rsx! { p { class: "text-muted-foreground mt-2", {t("dsar_loading")} } },
        }
    }
}

/// M5.1: the structured certificate card — the defining screenshot. Renders
/// found_count, purged_ids (monospace), tombstone_root, certified_at, chain_head,
/// and the LIVE green/red chain badge (re-checked via GET /dsar/{id}/certificate).
#[component]
fn CertificateCard(result: DsarResult) -> Element {
    let cert = result.cert.clone();
    let (badge_class, badge_key) = chain_badge(cert.chain_verifies);
    let badge_text = t(badge_key);
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
    let cert_title = t_fmt("deletion_certificate", &[format!("#{}", result.id)]);
    let cert_subject = t_fmt("dsar_subject_line", &[result.subject.clone()]);
    let cert_found = t("cert_found");
    let cert_purged = t("cert_purged");
    let cert_tombstone_root = t("cert_tombstone_root");
    let cert_certified = t("cert_certified");
    let cert_chain_head = t("cert_chain_head");
    rsx! {
        div { class: "card mt-2",
            div { class: "card-header",
                // i18n-exempt: the certificate letter-id is the wire id (#n) —
                // "Deletion certificate" itself is the translated key.
                h2 { class: "card-title", "{cert_title}" }
                Link {
                    class: "font-mono text-xs text-accent hover:underline",
                    to: Route::DsarDetail { dsar_id: result.id },
                    "{cert_subject}"
                }
            }
            dl { class: "card-body grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm",
                dt { class: "text-muted-foreground", "{cert_found}" }
                dd { class: "font-mono tabular", "{cert.found_count}" }
                dt { class: "text-muted-foreground", "{cert_purged}" }
                dd { class: "font-mono tabular", "{purged}" }
                dt { class: "text-muted-foreground", "{cert_tombstone_root}" }
                dd { class: "font-mono", "{root}" }
                dt { class: "text-muted-foreground", "{cert_certified}" }
                dd { "{cert.certified_at}" }
                dt { class: "text-muted-foreground", "{cert_chain_head}" }
                dd { class: "font-mono text-xs", "{cert.chain_head}" }
            }
            div { class: "card-footer",
                span { class: "{badge_class} font-semibold text-sm", role: "status", "aria-live": "polite",
                    "{badge_text}" }
            }
        }
    }
}

/// v1.28.1 M4 (F-12) pure: may the erasure confirm fire? Three conditions:
/// the armed subject matches the CURRENT input (an edit after arming freezes
/// the confirm), the preview on screen is for that same subject, and the
/// preview actually succeeded. The confirm button reads this every render.
pub fn purge_preview_ready(
    pending: &str,
    subject_input: &str,
    prev_subject: &str,
    prev: &Option<Result<Footprint, String>>,
) -> bool {
    let s = subject_input.trim();
    pending.trim() == s && prev_subject.trim() == s && matches!(prev, Some(Ok(_)))
}

/// Run one DSAR action against the server, returning the typed result.
/// Confirmation-first: every field derives from the actual server response;
/// the chain badge is the chain STILL holding (live re-verify), not cert-time.
/// v1.20.0 M3: an unreachable backend enqueues the action instead of failing.
async fn run_dsar(api: ApiClient, subject: String, action: &'static str) -> DsarOutcome {
    let resp = match api.dsar(&subject, action).await {
        Err(e) if crate::queue::is_offline(&e) => {
            // v1.28.1 M4: the queue persists only the subject's SHA-256 hash —
            // the raw email never touches site-local storage; replay re-prompts.
            // v1.27.21 (N7): the digest is salted per install, so the stored
            // hash is useless as a precomputed/rainbow-table target; replay
            // verifies the retyped subject against the same salt.
            let (subject_hash, salted) = crate::queue::dsar_subject_hash(&subject);
            crate::queue::enqueue(crate::queue::QueuedAction::Dsar {
                subject_hash,
                action: action.to_string(),
                salted,
                queued_at: crate::queue::now_ts(),
                retries: 0,
            });
            return DsarOutcome::Queued;
        }
        Err(e) => {
            return DsarOutcome::Failed(t_fmt(
                "dsar_action_failed",
                &[action.to_string(), e.to_string()],
            ))
        }
        Ok(resp) => resp,
    };
    // Live chain re-verify: the cert-time head is a snapshot; the badge must
    // reflect the chain holding NOW (DESIGN §8 defining-screenshot rule).
    let cert = match action {
        "purge" | "both" => match api.dsar_certificate(resp.id).await {
            Ok(v) => DsarCertificate::from_value(v),
            Err(e) => return DsarOutcome::Failed(t_fmt("cert_fetch_failed", &[e.to_string()])),
        },
        _ => {
            DsarCertificate::from_value(resp.certificate.clone().unwrap_or(serde_json::Value::Null))
        }
    };
    DsarOutcome::Done(DsarResult {
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
    /// The badge text is an i18n KEY — the card translates it at render.
    #[test]
    fn chain_badge_reflects_live_verify() {
        let (cls, txt) = chain_badge(true);
        assert_eq!(cls, "text-ok");
        assert_eq!(txt, "chain_verified");
        let (cls, txt) = chain_badge(false);
        assert_eq!(cls, "text-danger");
        assert_eq!(txt, "chain_tampered");
        // both keys resolve in the default locale (never a blank badge).
        assert!(crate::i18n::resolve_fmt("chain_verified", "en", &[]).contains("verified"));
        assert!(crate::i18n::resolve_fmt("chain_tampered", "en", &[]).contains("TAMPERED"));
    }

    /// v1.28.1 M4 (F-12): the two-step purge — the confirm stays frozen until
    /// the footprint for the CURRENT subject has rendered.
    #[test]
    fn dsar_purge_two_step_with_fresh_preview() {
        let fp = Some(Ok(Footprint {
            roots: 1,
            derived: 0,
            export_rows: 1,
            tombstones: 0,
            dsar_rows: 0,
            dry_run: true,
        }));
        let none = None;
        // Step 2 armed: pending == input == previewed subject, preview Ok.
        assert!(purge_preview_ready("alice@x", "alice@x", "alice@x", &fp));
        // Preview on screen for a DIFFERENT subject → frozen.
        assert!(!purge_preview_ready("alice@x", "alice@x", "bob@y", &fp));
        // Subject edited after arming → frozen until re-previewed.
        assert!(!purge_preview_ready("alice@x", "alice@z", "alice@x", &fp));
        // No preview, or a failed preview → frozen.
        assert!(!purge_preview_ready("alice@x", "alice@x", "alice@x", &none));
        assert!(!purge_preview_ready(
            "alice@x",
            "alice@x",
            "alice@x",
            &Some(Err("preview failed: down".into()))
        ));
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

    /// v1.20.22 M2.1: the Art 17 clock badge tiers the deadline by the
    /// day-scale bands and labels the remaining window; an open row with no
    /// server deadline yields `None` (nothing to count down).
    #[test]
    fn dsar_clock_tiers_and_labels_the_art17_deadline() {
        let now = 1_750_000_000i64;
        let row = |deadline: Option<i64>| DsarLedgerRow {
            id: 1,
            subject: "alice@x".into(),
            action: "both".into(),
            status: "pending".into(),
            created_at: Some(now - 86400),
            deadline,
            completed_at: None,
        };
        let (cls, _) = dsar_clock(&row(Some(now + 10 * 86400)), now).unwrap();
        assert_eq!(cls, "badge badge-ok", ">3d left is ok");
        let (cls, _) = dsar_clock(&row(Some(now + 2 * 86400)), now).unwrap();
        assert_eq!(cls, "badge badge-warn", "<3d left warns");
        let (cls, _) = dsar_clock(&row(Some(now + 12 * 3600)), now).unwrap();
        assert_eq!(cls, "badge badge-danger", "<1d left is danger");
        let (cls, label) = dsar_clock(&row(Some(now - 100)), now).unwrap();
        assert_eq!(cls, "badge badge-danger");
        assert_eq!(label, "expired");
        // No deadline → the caller shows nothing.
        assert!(dsar_clock(&row(None), now).is_none());
    }
}
