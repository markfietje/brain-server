//! Pure render + shaping surface for the Slack and Teams edges: proposal
//! renderers (Slack Blocks / Adaptive Cards) that ALWAYS carry the digest,
//! total parsers for the actions those renderers mint, the envelope-text
//! bound, and the Relay handover ping builders.
//!
//! Everything here is PURE — no network, no state, no clock. The DIGEST LAW
//! starts at the renderer: a proposal that renders without its digest
//! rendered alongside it is a bug, so the Blocks/Adaptive Card shapes embed
//! the digest in the very payload the human later clicks. Enforcement
//! point #1 lives in `console::digest_gate`; the kernel re-verifies against
//! stored bytes server-side (two independent gates).

use serde_json::{Value, json};

/// Hard bounds (bounds law): every emitted string is length-checked.
pub(crate) const MAX_PREVIEW_CHARS: usize = 700;
pub(crate) const MAX_ENVELOPE_TEXT_CHARS: usize = 4000;
pub(crate) const MAX_REF_LEN: usize = 128;

const PREVIEW_SUFFIX: &str = "…";
const ENVELOPE_SUFFIX: &str = "…[truncated]";

/// One proposal as the kernel's console seam reports it (`action:pending`).
pub(crate) struct Proposal {
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) content: String,
    /// 64 lowercase hex chars — the binding between the rendered bytes and
    /// every approve/reject action a human can take on this proposal.
    pub(crate) digest: String,
}

/// The locked cross-kernel envelope projection (Slack/Teams half):
/// platform ids ONLY — never a display name, never a channel name.
pub(crate) struct NoteDraft {
    pub(crate) conversation_ref: String,
    pub(crate) text: String,
    pub(crate) external_id: String,
    pub(crate) actor_ref: String,
}

/// True for exactly 64 ASCII hex chars (the digest shape everywhere).
pub(crate) fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Opaque platform reference shape (user/channel/conversation/activity
/// ids): present, bounded, control-char-free. NEVER a display name.
pub(crate) fn bounded_ref(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_REF_LEN && !s.chars().any(char::is_control)
}

fn clamp_chars(s: &str, max: usize, suffix: &str) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(suffix.chars().count());
    let mut out: String = s.chars().take(keep).collect();
    out.push_str(suffix);
    out
}

/// The envelope text bound: ≤ 4000 chars, truncation NOTED INSIDE the text.
pub(crate) fn clamp_envelope_text(s: &str) -> String {
    clamp_chars(s, MAX_ENVELOPE_TEXT_CHARS, ENVELOPE_SUFFIX)
}

/// Screen + bound message text before it crosses the seam. Pores (v1.28.71
/// M5, X-R6) edge parity: the strip now matches the canonical chain —
/// invisible-Unicode strip FIRST (the synced plugin `format.ts`
/// INVISIBLE_CLASSES set, the Rust `strip_invisible.rs` canonical class),
/// then the markdown image/link-ref dereference (the EchoLeak class), then
/// the control-char scrub (except newline/tab), then the 4000-char bound.
/// The kernel screen (`channels.rs` `screen`) still runs server-side — this
/// is defense-in-depth at the operator-rendering boundary (Slack/Teams
/// previews of message TEXT previously rode the control-char strip only).
pub(crate) fn sanitize_text(s: &str) -> String {
    let invisible_free = strip_invisible_chars(s);
    let refs_stripped = strip_markdown_refs(&invisible_free);
    let scrubbed: String = refs_stripped
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    clamp_envelope_text(&scrubbed)
}

/// The canonical invisible-Unicode smuggling class, ported from the synced
/// plugin `format.ts` `INVISIBLE_CLASSES` set (Meridian parity — the ONE
/// definition both trees pin): tag block U+E0000–E007F, variation selectors
/// U+FE00–FE0F + supplemental U+E0100–E01EF, zero-width set
/// (U+200B/200C/200D + word joiner U+2060–2063), the bidi controls
/// (U+200E/200F, U+202A–202E, U+2066–2069, U+061C), and the legacy/residual
/// members (BOM U+FEFF, soft hyphen U+00AD, combining grapheme joiner U+034F,
/// Mongolian vowel separator U+180E, Hangul fillers U+115F/U+1160,
/// interlinear annotation U+FFF9–FFFB).
fn is_invisible_char(c: char) -> bool {
    let cp = c as u32;
    (0xE0000..=0xE007F).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp)
        || (0xE0100..=0xE01EF).contains(&cp)
        || (0x200E..=0x200F).contains(&cp)
        || (0x202A..=0x202E).contains(&cp)
        || (0x2066..=0x2069).contains(&cp)
        || cp == 0x061C
        || matches!(cp, 0x200B | 0x200C | 0x200D | 0x2060)
        || matches!(cp, 0xFEFF | 0x2061 | 0x2062 | 0x2063 | 0x00AD | 0x034F)
        || matches!(cp, 0x180E | 0x115F | 0x1160)
        || (0xFFF9..=0xFFFB).contains(&cp)
}

fn strip_invisible_chars(s: &str) -> String {
    s.chars().filter(|&c| !is_invisible_char(c)).collect()
}

/// Neutralize the EchoLeak markdown exfil class on rendered text — the same
/// contract as the kernel's `strip_markdown_refs`: `![alt](url)` → `[alt]`,
/// `[text](url)` → `text`, bare URLs in prose left intact. Invisible chars
/// are stripped FIRST (the scanner requires `(` directly after `]`).
/// Lint-clean by construction (`.get` + checked arithmetic — this runs on
/// hostile bytes; a panic here would be an edge DoS).
fn strip_markdown_refs(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some((is_image, label_start, label_end, resume)) = scan_construct_at(bytes, i) {
            if is_image {
                // Image: keep the alt text bracketed (it renders as prose).
                out.push('[');
                if let Some(seg) = s.get(label_start..label_end) {
                    out.push_str(seg);
                }
                out.push(']');
            } else if let Some(seg) = s.get(label_start..label_end) {
                out.push_str(seg);
            }
            i = resume;
            continue;
        }
        // Copy one char (multibyte-safe); `get` on a byte offset is
        // boundary-safe by construction (i only advances on boundaries).
        match s.get(i..).and_then(|rest| rest.chars().next()) {
            Some(ch) => {
                out.push(ch);
                i = i.saturating_add(ch.len_utf8());
            }
            None => break,
        }
    }
    out
}

/// If a markdown link/image construct starts at `pos` (either `![` or `[`),
/// return `(is_image, label_start, label_end, resume_pos)`.
fn scan_construct_at(bytes: &[u8], pos: usize) -> Option<(bool, usize, usize, usize)> {
    let open = match bytes.get(pos) {
        Some(b'!') => pos.checked_add(1)?,
        Some(b'[') => pos,
        _ => return None,
    };
    let (label_start, label_end, url_close) = scan_link_construct(bytes, open)?;
    let resume = url_close.checked_add(1)?;
    Some((
        bytes.get(pos) == Some(&b'!'),
        label_start,
        label_end,
        resume,
    ))
}

/// From an opening `[` at `open_bracket`, look for the complete link
/// construct `[label](url)`. Returns `(label_start, label_end, url_close)`
/// byte offsets (label excludes the brackets; url_close is the `)` index).
/// Byte-for-byte port of the kernel's `fence::scan_link_construct` — parity
/// IS the deliverable (X-R6), so the two scanners accept the same constructs.
fn scan_link_construct(bytes: &[u8], open_bracket: usize) -> Option<(usize, usize, usize)> {
    let label_start = open_bracket.checked_add(1)?;
    let label_end_rel = bytes.get(label_start..)?.iter().position(|&b| b == b']')?;
    let label_end = label_start.checked_add(label_end_rel)?;
    let paren_open = label_end.checked_add(1)?;
    if bytes.get(paren_open) != Some(&b'(') {
        return None;
    }
    let url_start = paren_open.checked_add(1)?;
    let url_close_rel = bytes.get(url_start..)?.iter().position(|&b| b == b')')?;
    url_start
        .checked_add(url_close_rel)
        .map(|c| (label_start, label_end, c))
}

fn clamp_preview(s: &str) -> String {
    clamp_chars(s, MAX_PREVIEW_CHARS, PREVIEW_SUFFIX)
}

fn mrkdwn_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn digest_bookends(hex: &str) -> (String, String) {
    let first = hex.get(0..16).unwrap_or(hex).to_string();
    let last = hex
        .get(hex.len().saturating_sub(16)..)
        .unwrap_or(hex)
        .to_string();
    (first, last)
}

/// Slack Blocks for ONE proposal. The digest is shown in the context block
/// AND embedded into BOTH button values — a click IS a digest presentation.
pub(crate) fn slack_blocks(p: &Proposal) -> Value {
    let (first, last) = digest_bookends(&p.digest);
    json!([
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!(
                    "*:inbox_tray: Proposal #{}* · `{}`\n{}",
                    p.id,
                    p.kind,
                    mrkdwn_escape(&clamp_preview(&p.content))
                ),
            }
        },
        {
            "type": "context",
            "elements": [{
                "type": "mrkdwn",
                "text": format!("digest `{first}…{last}` · full: {}", p.digest),
            }]
        },
        {
            "type": "actions",
            "elements": [
                {
                    "type": "button",
                    "style": "primary",
                    "action_id": "brain:approve",
                    "text": {"type": "plain_text", "text": "Approve"},
                    "value": format!("approve:{}:{}", p.id, p.digest),
                },
                {
                    "type": "button",
                    "style": "danger",
                    "action_id": "brain:reject",
                    "text": {"type": "plain_text", "text": "Reject"},
                    "value": format!("reject:{}:{}", p.id, p.digest),
                },
            ]
        },
    ])
}

/// ONE Adaptive Card attachment for a Teams proposal — same law: the
/// digest rides the body AND both `Action.Submit` payloads.
pub(crate) fn adaptive_card(p: &Proposal) -> Value {
    json!({
        "contentType": "application/vnd.microsoft.card.adaptive",
        "content": {
            "type": "AdaptiveCard",
            "version": "1.4",
            "body": [
                {
                    "type": "TextBlock",
                    "text": format!("Proposal #{} · {}", p.id, p.kind),
                    "wrap": true,
                },
                {"type": "TextBlock", "text": clamp_preview(&p.content), "wrap": true},
                {
                    "type": "TextBlock",
                    "text": format!("digest: {}", p.digest),
                    "isSubtle": true,
                    "wrap": true,
                },
            ],
            "actions": [
                {
                    "type": "Action.Submit",
                    "title": "Approve",
                    "data": {"action": "brain:approve", "proposal_id": p.id, "digest": p.digest},
                },
                {
                    "type": "Action.Submit",
                    "title": "Reject",
                    "data": {"action": "brain:reject", "proposal_id": p.id, "digest": p.digest},
                },
            ],
        }
    })
}

/// Total parser for the Slack button value grammar
/// `approve:<id>:<64hex>` / `reject:<id>:<64hex>`. Anything else → None.
pub(crate) fn parse_slack_action_value(value: &str) -> Option<(bool, i64, String)> {
    let mut parts = value.split(':');
    let approve = match parts.next()? {
        "approve" => true,
        "reject" => false,
        _ => return None,
    };
    let id: i64 = parts.next()?.parse().ok()?;
    if id < 0 {
        return None;
    }
    let digest = parts.next()?;
    if parts.next().is_some() || !is_hex64(digest) {
        return None;
    }
    Some((approve, id, digest.to_ascii_lowercase()))
}

/// Total parser for an Adaptive Card `Action.Submit` activity `value`.
pub(crate) fn parse_card_submit(value: &Value) -> Option<(bool, i64, String)> {
    let approve = match value.get("action").and_then(|x| x.as_str())? {
        "brain:approve" => true,
        "brain:reject" => false,
        _ => return None,
    };
    let id = value.get("proposal_id").and_then(|x| x.as_i64())?;
    let digest = value
        .get("digest")
        .and_then(|x| x.as_str())
        .filter(|d| is_hex64(d))?;
    Some((approve, id, digest.to_ascii_lowercase()))
}

/// The ping target pure-decision: the case channel wins; the config's
/// `handover_channel` is the caller-side fallback for roomless cases; a
/// roomless case WITHOUT a configured handover channel is dropped (None).
pub(crate) fn ping_target(case_channel: &str, handover: Option<&str>) -> Option<String> {
    if !case_channel.is_empty() {
        return Some(case_channel.to_string());
    }
    handover.filter(|h| !h.is_empty()).map(str::to_string)
}

fn ping_text(ping: &Value, mention: fn(&str) -> String) -> Option<(Option<String>, String)> {
    let offer_id = ping.get("offer_id").and_then(|x| x.as_i64())?;
    let case_run_id = ping
        .get("case_run_id")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let complete = ping
        .get("complete")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let case_channel = ping
        .get("case_channel")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let missing: Vec<String> = ping
        .get("missing")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.as_str())
                .filter(|m| !m.is_empty() && m.len() <= 200)
                .take(16)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let refs: Vec<String> = ping
        .get("platform_refs")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.as_str())
                .filter(|m| bounded_ref(m))
                .take(8)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut text = format!(
        ":handshake: Handover offer #{offer_id} for case #{case_run_id} — I-PASS complete: {}",
        if complete { "yes" } else { "no" }
    );
    for m in &missing {
        text.push_str("\n- ");
        text.push_str(m);
    }
    if !refs.is_empty() {
        let mentions: Vec<String> = refs.iter().map(|r| mention(r)).collect();
        text.push('\n');
        text.push_str(&mentions.join(" "));
    }
    let target = ping_target(case_channel, None);
    Some((target, text))
}

/// Slack variant: platform_refs render as `<@U…>` mentions.
pub(crate) fn build_ping_message_slack(ping: &Value) -> Option<(Option<String>, String)> {
    ping_text(ping, |u| format!("<@{u}>"))
}

/// Teams variant: platform_refs render as `<at>U…</at>` mentions.
pub(crate) fn build_ping_message_teams(ping: &Value) -> Option<(Option<String>, String)> {
    ping_text(ping, |u| format!("<at>{u}</at>"))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::panic,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;

    fn hex64() -> String {
        // Exactly 64 hex chars, built (not hand-typed) so the law's shape
        // holds no matter how many times the group is repeated in edits.
        "aa55aa55".repeat(8)
    }

    fn proposal() -> Proposal {
        Proposal {
            id: 42,
            kind: "draft".to_string(),
            content: "release notes body & <details>".to_string(),
            digest: hex64(),
        }
    }

    // ── HERALD PIN: renderers always mint digest-carrying payloads and the
    //    parsers invert them exactly.
    #[test]
    fn renderers_embed_digest_and_parsers_round_trip() {
        let hex = hex64();
        let p = proposal();

        let blocks = slack_blocks(&p);
        let value = blocks[2]["elements"][0]["value"].as_str().unwrap();
        assert_eq!(
            parse_slack_action_value(value),
            Some((true, 42, hex.clone())),
            "approve button round-trips id+digest"
        );
        let reject_value = blocks[2]["elements"][1]["value"].as_str().unwrap();
        assert_eq!(
            parse_slack_action_value(reject_value),
            Some((false, 42, hex.clone()))
        );
        // mrkdwn escaping keeps hostile content inert in the preview.
        let preview = blocks[0]["text"]["text"].as_str().unwrap();
        assert!(preview.contains("&amp;") && preview.contains("&lt;details&gt;"));

        let card = adaptive_card(&p);
        assert_eq!(
            parse_card_submit(&card["content"]["actions"][0]["data"]),
            Some((true, 42, hex.clone()))
        );
        assert_eq!(
            parse_card_submit(&card["content"]["actions"][1]["data"]),
            Some((false, 42, hex))
        );
        // The card body SHOWS the digest (the human binds to visible bytes).
        let body = card["content"]["body"][2]["text"].as_str().unwrap();
        assert!(body.contains(&hex64()), "card body shows the digest");
    }

    #[test]
    fn action_parsers_are_total() {
        assert_eq!(parse_slack_action_value(""), None);
        assert_eq!(parse_slack_action_value("approve:42"), None);
        assert_eq!(parse_slack_action_value("approve:42:zz"), None);
        assert_eq!(parse_slack_action_value("approve:-1:xx"), None);
        assert_eq!(parse_slack_action_value("approve:42:nothex"), None);
        assert_eq!(
            parse_slack_action_value("approve:42:extra:parts"),
            None,
            "exactly three segments"
        );
        assert_eq!(parse_slack_action_value("frog:42:xx"), None);
        assert_eq!(parse_card_submit(&json!({"action": "brain:approve"})), None);
        assert_eq!(
            parse_card_submit(&json!({"action": "other", "proposal_id": 1})),
            None
        );

        // Short-digest values refuse (the law wants exactly 64 hex).
        assert_eq!(
            parse_card_submit(
                &json!({"action": "brain:approve", "proposal_id": 1, "digest": "abcd"})
            ),
            None
        );
    }

    #[test]
    fn envelope_text_clamps_at_4000_with_marker() {
        let short = "hello";
        assert_eq!(sanitize_text(short), "hello");
        let long = "x".repeat(6000);
        let clamped = sanitize_text(&long);
        let chars = clamped.chars().count();
        assert!(chars <= MAX_ENVELOPE_TEXT_CHARS, "bound holds");
        assert!(clamped.ends_with("…[truncated]"), "truncation noted INSIDE");
        // Control junk (except \n\t) is scrubbed at the seam.
        assert_eq!(sanitize_text("a\u{0}b\u{7}c"), "abc");
        assert_eq!(sanitize_text("a\nb\tc"), "a\nb\tc");
    }

    // ── HERALD PIN (M3): Relay handover pings carry the I-PASS state,
    //    mention platform refs per-platform, and fall back purely.
    #[test]
    fn ping_messages_render_completeness_mentions_and_fallback() {
        let complete = json!({
            "event_id": 9,
            "to_principal": "op2",
            "platform_refs": ["U0PING1", "U0PING2"],
            "case_channel": "C0CASE1",
            "case_run_id": 7,
            "offer_id": 3,
            "complete": true,
            "missing": []
        });
        let (target, text) = build_ping_message_slack(&complete).unwrap();
        assert_eq!(target.as_deref(), Some("C0CASE1"), "case channel wins");
        assert!(text.contains("Handover offer #3 for case #7"));
        assert!(text.contains("I-PASS complete: yes"));
        assert!(
            text.contains("<@U0PING1>") && text.contains("<@U0PING2>"),
            "slack mentions"
        );

        let (_, teams_text) = build_ping_message_teams(&complete).unwrap();
        assert!(
            teams_text.contains("<at>U0PING1</at>"),
            "teams mention variant"
        );
        assert!(!teams_text.contains("<@U0PING1>"));

        let incomplete = json!({
            "offer_id": 4,
            "case_run_id": 8,
            "complete": false,
            "missing": ["id_rsa_key", "customer_consents"],
            "case_channel": "",
            "platform_refs": ["U0PING9"]
        });
        let (none_target, text2) = build_ping_message_slack(&incomplete).unwrap();
        assert_eq!(none_target, None, "roomless case defers to the caller");
        assert!(text2.contains("I-PASS complete: no"));
        assert!(text2.contains("\n- id_rsa_key"));
        assert!(text2.contains("\n- customer_consents"));

        // No offer id → nothing renderable.
        assert_eq!(
            build_ping_message_slack(&json!({"case_channel": "C1"})),
            None
        );

        // Fallback pure-decision: case wins, handover second, else drop.
        assert_eq!(
            ping_target("C0CASE1", Some("C0HANDOV1")).as_deref(),
            Some("C0CASE1")
        );
        assert_eq!(
            ping_target("", Some("C0HANDOV1")).as_deref(),
            Some("C0HANDOV1")
        );
        assert!(ping_target("", Some("")).is_none());
        assert!(ping_target("", None).is_none());
    }

    // ── PORES PIN (v1.28.71 M5, X-R6): the bridge edge strips what the
    //    kernel screen + plugin fence strip — the canonical invisible class
    //    and the markdown-ref dereference — before the 4000 clamp.

    /// Tag-block (U+E0000–E007F) and bidi/zero-width smuggling is stripped
    /// at the bridge edge: the rendered message text the operator previews
    /// (and the kernel receives) carries none of the class.
    #[test]
    fn edge_strips_tag_unicode() {
        // The Meridian canary shape: tag-encoded bytes + a bidi override.
        let tagged = "keep\u{E0000}\u{E0010}secret\u{E007F} this \u{202E}noitarod\u{202C} visible";
        let out = sanitize_text(tagged);
        assert!(
            !out.chars().any(is_invisible_char),
            "no invisible class survives: {out:?}"
        );
        assert!(out.contains("keepsecret"));
        // The RLO-wrapped word's MARKS are gone; the char order is the
        // author's problem (stripping never reorders) — the kernel screen's
        // typoglycemia tier is what reads scrambled forms.
        assert!(out.contains("this noitarod visible"));
        // The residual members of the canonical set too.
        let residual = "a\u{FEFF}b\u{00AD}c\u{200B}d\u{2060}e\u{061C}f";
        assert_eq!(sanitize_text(residual), "abcdef");
    }

    /// The EchoLeak markdown class is dereferenced at the bridge edge:
    /// image URLs drop, link URLs drop (label kept), bare URLs in prose
    /// stay (the documented shipped contract — linkified-but-inert).
    #[test]
    fn edge_strips_markdown_image_refs() {
        assert_eq!(
            sanitize_text("see ![alt](http://x.example/pixel.png) now"),
            "see [alt] now"
        );
        assert_eq!(
            sanitize_text("read [the doc](http://x.example/a) today"),
            "read the doc today"
        );
        // Bare URL in prose survives (kernel parity — not this seam's job).
        assert_eq!(
            sanitize_text("visit http://x.example/a for more"),
            "visit http://x.example/a for more"
        );
    }

    /// The 4000-char envelope bound is unchanged: clamping still lands on
    /// the marker, measured AFTER the strip (an invisible-padded blob
    /// clamps the same as its visible content).
    #[test]
    fn clamp_unchanged() {
        let long = "x".repeat(6000);
        let clamped = sanitize_text(&long);
        assert!(clamped.chars().count() <= MAX_ENVELOPE_TEXT_CHARS);
        assert!(clamped.ends_with("…[truncated]"));
        // Invisible padding does not buy extra room: 3000+3000 tag chars
        // around 10 visible chars renders to just the visible content.
        let padded = "\u{E0000}".repeat(3000) + "short note" + &"\u{E007F}".repeat(3000);
        let out = sanitize_text(&padded);
        assert_eq!(out, "short note");
    }

    /// End-to-end (bridge half): a tagged-unicode message lands at the
    /// kernel ALREADY stripped — the kernel screen (`channels.rs`
    /// `screen`) stays authoritative server-side either way, so the text
    /// the bridge emits is exactly the text the screen's layer-1 sees
    /// stripped-form. The parity property: whatever the kernel strips at
    /// its boundary is a no-op on bridge-rendered bytes.
    #[test]
    fn kernel_screen_still_authoritative() {
        let hostile = "\u{2066}\u{E0000}hidden\u{E007F}instruction\u{2069} ![x](http://y)";
        let rendered = sanitize_text(hostile);
        // Idempotent: re-sanitizing rendered bytes is a no-op.
        assert_eq!(sanitize_text(&rendered), rendered);
        // No invisible class, no live ref — the kernel sees shaped text.
        assert!(!rendered.chars().any(is_invisible_char));
        assert!(!rendered.contains("](http://"));
    }
}
