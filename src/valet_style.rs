//! Valet style gate: the deterministic, zero-token linter whose
//! report rides draft proposals.
//!
//! **Advisory, never blocking.** The human outranks the linter: a draft with
//! a terrible lint score approves exactly like a clean one — the score is
//! information for the reviewer (console + Signal message), never a gate.
//! Taste evolves through the normal proposal gate: the banned-phrase list
//! lives in an approved knowledge row (`source = 'valet-style'`), so
//! amending it is itself a HITL decision.
//!
//! Pure function over text + memory; no model calls, no I/O in `style_check`
//! itself (`style_memory` is the only DB-reading piece).

use rusqlite::Connection;
use sha2::Digest;
use sha2::Sha256;

/// The knowledge-row source tag the style guide lives under. The ONLY writer
/// of rows with this source is the generic proposal promote path — i.e. an
/// approved HITL proposal.
pub const STYLE_MEMORY_SOURCE: &str = "valet-style";

const MAX_TEXT_LEN: usize = 200_000;
const MAX_BANNED_PHRASES: usize = 200;

/// Sentence-length ceiling (words) before a sentence is flagged "long".
const MAX_SENTENCE_WORDS: usize = 25;

/// Fixed passive-voice heuristic triggers (a closed rule list — no model).
const PASSIVE_AUX: &[&str] = &["was", "were", "is being", "been", "being"];

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LintFinding {
    pub rule: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LintReport {
    /// Deterministic integer score: 100 clean, deductions per finding.
    pub score: u32,
    pub findings: Vec<LintFinding>,
    /// sha256 of the style-memory content the check ran against
    /// (provenance: which taste ruled this draft).
    pub style_memory_hash: String,
}

impl LintReport {
    fn blank(hash: String) -> Self {
        LintReport {
            score: 100,
            findings: vec![],
            style_memory_hash: hash,
        }
    }
    fn penalize(&mut self, rule: &str, detail: String) {
        self.findings.push(LintFinding {
            rule: rule.to_string(),
            detail,
        });
        self.score = self.score.saturating_sub(10);
    }
}

fn sha256_hex(s: &str) -> String {
    crate::audit::hex_encode(&Sha256::digest(s.as_bytes()))
}

/// Load the style memory: the latest approved knowledge row tagged
/// `source='valet-style'`. Its content is JSON `{"banned_phrases": [...]}`
/// (anything unparsable degrades to defaults — the linter never blocks on a
/// malformed memory). Missing row → empty banned list, hash of "".
pub fn style_memory(conn: &Connection) -> (Vec<String>, String) {
    let raw: Option<String> = conn
        .query_row(
            "SELECT content FROM knowledge WHERE source = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![STYLE_MEMORY_SOURCE],
            |r| r.get(0),
        )
        .ok();
    let Some(raw) = raw else {
        return (vec![], sha256_hex(""));
    };
    let banned: Vec<String> = serde_json::from_str(&raw)
        .ok()
        .and_then(|v: serde_json::Value| {
            v.get("banned_phrases").and_then(|p| p.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .take(MAX_BANNED_PHRASES)
                    .collect()
            })
        })
        .unwrap_or_default();
    (banned, sha256_hex(&raw))
}

/// The deterministic lint. Zero-token, pure, bounded.
pub fn style_check(text: &str, banned_phrases: &[String], style_memory_hash: &str) -> LintReport {
    let mut rep = LintReport::blank(style_memory_hash.to_string());
    let text = &text[..text.len().min(MAX_TEXT_LEN)];

    if text.contains('\u{2014}') || text.contains("--") {
        rep.penalize("em_dash", "em-dash / double-hyphen present".into());
    }

    let lower = text.to_lowercase();
    for phrase in banned_phrases {
        let p = phrase.trim().to_lowercase();
        if !p.is_empty() && lower.contains(&p) {
            rep.penalize(
                "banned_phrase",
                format!("contains banned phrase {phrase:?}"),
            );
        }
    }

    // Filler openers: a fixed list of the classic throat-clearers.
    let trimmed = text.trim_start();
    for opener in [
        "So, ",
        "Basically, ",
        "In conclusion, ",
        "It goes without saying that ",
    ] {
        if trimmed.starts_with(opener) {
            rep.penalize("filler_opener", format!("opens with filler {opener:?}"));
            break;
        }
    }

    // Sentence-length distribution + passive heuristic, sentence-scoped.
    let sentences: Vec<&str> = text
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let long = sentences
        .iter()
        .filter(|s| s.split_whitespace().count() > MAX_SENTENCE_WORDS)
        .count();
    if long > 0 {
        rep.penalize(
            "long_sentences",
            format!("{long} sentence(s) exceed {MAX_SENTENCE_WORDS} words"),
        );
    }
    let passive_hits: usize = sentences
        .iter()
        .filter(|s| {
            let words: Vec<String> = s
                .split_whitespace()
                .map(|w| {
                    w.trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase()
                })
                .collect();
            words.windows(2).any(|w| {
                PASSIVE_AUX.contains(&w[0].as_str()) && w[1].len() > 4 && w[1].ends_with("ed")
            }) || (s.contains("is being") && s.contains("ed"))
        })
        .count();
    if passive_hits > 0 {
        rep.penalize(
            "passive_voice",
            format!("{passive_hits} sentence(s) look passive"),
        );
    }

    // Status labels: product claims need one of the fixed status labels.
    let claim_words = ["will ship", "ships soon", "coming soon", "guarantees"];
    let has_claim = claim_words.iter().any(|c| lower.contains(c));
    let has_label = ["shipped", "in progress", "planned", "spec", "idea"]
        .iter()
        .any(|l| lower.contains(l));
    if has_claim && !has_label {
        rep.penalize(
            "missing_status_label",
            "product claim without a shipped/in-progress/planned/spec/idea label".into(),
        );
    }

    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "testhash";

    #[test]
    fn style_check_flags_em_dash_and_banned_phrases() {
        let mem = vec![
            "delve".to_string(),
            "in today's fast-paced world".to_string(),
        ];
        let rep = style_check("Basically, we delve into this — it works.", &mem, HASH);
        assert!(rep.findings.iter().any(|f| f.rule == "banned_phrase"));
        assert!(rep.findings.iter().any(|f| f.rule == "em_dash"));
        assert!(rep.findings.iter().any(|f| f.rule == "filler_opener"));
        assert!(rep.score < 100);
        // Clean text scores 100 with zero findings and keeps provenance.
        let clean = style_check("I shipped it. It works.", &mem, HASH);
        assert_eq!(clean.score, 100);
        assert!(clean.findings.is_empty());
        assert_eq!(clean.style_memory_hash, HASH);
        // Advisory by construction: the report carries no verdict field a
        // gate could branch on — only score + findings + provenance.
        let json = serde_json::to_string(&clean).unwrap();
        assert!(!json.contains("verdict") && !json.contains("blocked"));
    }
}
