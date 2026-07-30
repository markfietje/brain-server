//! Obsidian vault semantics: YAML frontmatter + `[[wikilink]]` extraction.
//!
//! Pure, allocation-only, no I/O, no `unsafe`, no YAML dependency. Only the
//! four keys brain-server cares about are parsed: `title`, `tags`, `aliases`,
//! `domain`. Anything more complex is out of scope (upgrade path: a real YAML
//! parser if a corpus needs nested frontmatter).
//!
//! ponytail ceiling: hand-rolled line-oriented YAML. Handles the inline and
//! block list forms Obsidian emits (`tags: [a, b]` and `tags:\n  - a`). Does
//! NOT handle nested mappings, flow maps, multi-doc, or quoted scalars with
//! embedded colons. Every real Obsidian vault uses one of the two supported
//! forms for these keys, so this is sufficient for v0.9.2 / v1.4.x.

#![deny(unsafe_code)]

/// Parsed frontmatter for the keys brain-server cares about.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub domain: Option<String>,
}

/// Split a document into `(frontmatter_yaml, body)`.
///
/// Frontmatter is a leading block delimited by `---` lines:
/// ```text
/// ---
/// title: Foo
/// ---
/// body...
/// ```
/// Returns `("", content)` when no valid frontmatter is present. The opening
/// `---` must be the very first line and the closing `---` its own line.
pub fn split_frontmatter(content: &str) -> (String, String) {
    let mut lines = content.split_inclusive('\n');
    let first = lines.next();
    // The opening `---` must be the very first line. If it's absent or
    // different, there's no frontmatter — return the whole content as body.
    let first_line = match first {
        Some(l) if l.trim_end() == "---" => l,
        _ => return (String::new(), content.to_string()),
    };
    let mut yaml = String::new();
    let mut consumed = first_line.len();
    for line in lines {
        consumed += line.len();
        if line.trim_end() == "---" {
            let body = content[consumed..].trim_start_matches('\n').to_string();
            return (yaml, body);
        }
        yaml.push_str(line);
    }
    // No closing delimiter → not frontmatter; treat whole doc as body.
    (String::new(), content.to_string())
}

/// Parse the three supported keys from a frontmatter YAML string.
pub fn parse_frontmatter(yaml: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut lines = yaml.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, val)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        match key {
            "title" => {
                if !val.is_empty() {
                    fm.title = Some(strip_quotes(val).to_string());
                }
            }
            "domain" => {
                if !val.is_empty() {
                    fm.domain = Some(strip_quotes(val).to_string());
                }
            }
            "tags" | "aliases" => {
                let list = if val.is_empty() {
                    // Block list form: following indented lines starting with `-`.
                    collect_block_list(&mut lines)
                } else {
                    // Inline form: `[a, b]` or bare scalar.
                    parse_inline_list(val)
                };
                if key == "tags" {
                    fm.tags.extend(list);
                } else {
                    fm.aliases.extend(list);
                }
            }
            _ => {}
        }
    }
    fm.tags = fm.tags.into_iter().flat_map(expand_tag).collect();
    fm
}

/// Parse `[[wikilink]]` targets from markdown body.
///
/// Recognizes the three Obsidian forms:
///   - `[[Target]]`
///   - `[[Target|Alias]]`       → Target
///   - `[[Target#Heading]]`     → Target (heading dropped)
///   - `[[#Heading]]`           → skipped (no target note; links within page)
///
/// Returns the de-duplicated, trimmed target note names in order of first
/// appearance. Embedded code spans / fences are not special-cased; a link
/// inside a code fence is still emitted (cheap and rare — upgrade path: a
/// fence-aware pass if a noisy corpus needs it).
pub fn parse_wikilinks(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();

    while i + 1 < len {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            // Find the closing `]]`: first occurrence of two consecutive `]`.
            let mut j = start;
            let mut closed = None;
            while j + 1 < len {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    closed = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(j) = closed {
                let raw = std::str::from_utf8(&bytes[start..j]).unwrap_or("").trim();
                if !raw.is_empty() && !raw.starts_with('#') {
                    let target = raw
                        .split('#')
                        .next()
                        .unwrap_or(raw)
                        .split('|')
                        .next()
                        .unwrap_or(raw)
                        .trim();
                    if !target.is_empty() && seen.insert(target.to_lowercase()) {
                        out.push(target.to_string());
                    }
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ── helpers ──────────────────────────────────────────────────────────────

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse `[a, b, c]` or a bare scalar `a`.
fn parse_inline_list(val: &str) -> Vec<String> {
    let v = val.trim();
    let inner = v
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(v);
    inner
        .split([','])
        .map(|s| strip_quotes(s.trim()).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Collect a YAML block list (`- item` lines) following a `key:` line.
fn collect_block_list<'a, I: Iterator<Item = &'a str>>(
    lines: &mut std::iter::Peekable<I>,
) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(next) = lines.peek() {
        let t = next.trim_start();
        if t.starts_with("- ") || t == "-" {
            let item = t.trim_start_matches('-').trim();
            out.push(strip_quotes(item).to_string());
            lines.next();
        } else if next.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    out
}

/// Obsidian tag shorthand: `#foo/bar` → also emit `#foo`. Leading `#` stripped.
fn expand_tag(tag: String) -> Vec<String> {
    let t = tag.trim_start_matches('#').trim().to_string();
    if t.is_empty() {
        return Vec::new();
    }
    let mut out = vec![t.clone()];
    if let Some((parent, _)) = t.rsplit_once('/') {
        out.push(parent.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_frontmatter() {
        let doc = "---\ntitle: My Note\ntags: [a, b]\n---\n# Body\nText.";
        let (yaml, body) = split_frontmatter(doc);
        assert!(yaml.contains("title: My Note"));
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn no_frontmatter_returns_empty_yaml() {
        let (yaml, body) = split_frontmatter("just body\n");
        assert!(yaml.is_empty());
        assert_eq!(body, "just body\n");
    }

    #[test]
    fn unterminated_delimiter_is_body() {
        let (yaml, body) = split_frontmatter("---\ntitle: X\nno close");
        assert!(yaml.is_empty());
        assert!(!body.is_empty());
    }

    #[test]
    fn parses_inline_tags_and_aliases() {
        let fm = parse_frontmatter("title: Foo\naliases: [Bar, Baz]\ntags: [rust, lang]\n");
        assert_eq!(fm.title.as_deref(), Some("Foo"));
        assert_eq!(fm.aliases, vec!["Bar".to_string(), "Baz".to_string()]);
        assert!(fm.tags.contains(&"rust".to_string()));
        assert!(fm.tags.contains(&"lang".to_string()));
    }

    #[test]
    fn parses_block_list_tags() {
        let yaml = "tags:\n  - alpha\n  - beta\n";
        let fm = parse_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn tag_shorthand_strips_hash_and_emits_parent() {
        let fm = parse_frontmatter("tags:\n  - fruit/tropical\n");
        assert_eq!(
            fm.tags,
            vec!["fruit/tropical".to_string(), "fruit".to_string()]
        );
    }

    #[test]
    fn quoted_title_is_unquoted() {
        let fm = parse_frontmatter("title: \"A Title: With Colon\"\n");
        assert_eq!(fm.title.as_deref(), Some("A Title: With Colon"));
    }

    #[test]
    fn parses_plain_wikilink() {
        let links = parse_wikilinks("see [[Bignay]] and [[Mangosteen]]");
        assert_eq!(links, vec!["Bignay".to_string(), "Mangosteen".to_string()]);
    }

    #[test]
    fn parses_alias_and_heading_forms() {
        let links = parse_wikilinks("[[Target|display]] [[Target#Section]]");
        assert_eq!(links, vec!["Target".to_string()]);
    }

    #[test]
    fn skips_intra_page_heading_links() {
        let links = parse_wikilinks("see [[#Internal]] only");
        assert!(links.is_empty());
    }

    #[test]
    fn dedups_repeated_targets_case_insensitively() {
        let links = parse_wikilinks("[[Foo]] [[foo]] [[FOO|alt]]");
        assert_eq!(links, vec!["Foo".to_string()]);
    }

    #[test]
    fn closes_at_first_double_bracket() {
        // Obsidian does not nest `[[`; the first `]]` closes the link.
        let links = parse_wikilinks("[[unclosed and [[ok]]");
        assert_eq!(links, vec!["unclosed and [[ok".to_string()]);
    }

    #[test]
    fn ignores_truly_unclosed_link() {
        // No closing `]]` at all → nothing emitted, but a later valid link still works.
        let links = parse_wikilinks("[[unclosed text here then [[ok]]");
        assert_eq!(links, vec!["unclosed text here then [[ok".to_string()]);
    }

    #[test]
    fn frontmatter_parses_domain_key() {
        let fm = parse_frontmatter("title: Test\ndomain: proxmox\n");
        assert_eq!(fm.title.as_deref(), Some("Test"));
        assert_eq!(fm.domain.as_deref(), Some("proxmox"));
    }

    #[test]
    fn frontmatter_domain_defaults_when_absent() {
        let fm = parse_frontmatter("title: Test\ntags: [a]\n");
        assert!(fm.domain.is_none());
    }
}
