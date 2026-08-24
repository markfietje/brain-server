//! The Beacon public-KB build.
//!
//! The public knowledge base is a **generated static artifact**, never a live
//! data path: [`build_files`] turns `kcs_state='published'` articles into a
//! byte-deterministic file map (same DB state ⇒ identical bytes), the operator
//! hosts it, and [`write_artifact`] lands it on disk with a SHA-256
//! `kb_manifest.json` so the hosted artifact is verifiable against the DB it
//! came from.
//!
//! Sanitize posture: every field passes the strict public seam
//! ([`sanitize_public`]) — PII redaction UNCONDITIONAL (no operator bypass,
//! stricter than the internal read gate), invisible-Unicode strip, markdown-ref
//! strip. Superseded articles emit redirect pages to their survivor, reusing
//! the existing `supersedes` evidence chain.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The manifest filename inside every artifact.
pub const MANIFEST_NAME: &str = "kb_manifest.json";

/// A published article shaped for the public site. Every text field has
/// already passed [`sanitize_public`] — this struct is safe to render verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct KbArticle {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub body: String,
    pub updated_at: i64,
    pub origin: Option<String>,
    pub revision: String,
}

/// Public slug vocabulary: lowercase alnum + hyphen, 1..=80, no leading or
/// trailing or doubled hyphen (URL-path safe by construction).
pub fn is_valid_slug(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
}

/// The STRICT public render seam. Unconditional PII redaction (`pii=true`,
/// no principal ⇒ `has_pii_read` can never pass), invisible-Unicode strip,
/// markdown-ref strip — in the same order as [`crate::gate::sanitize_read`].
/// There is deliberately NO privileged variant: what the approver previews is
/// byte-identical to what ships.
pub fn sanitize_public(s: &str) -> String {
    crate::fence::strip_markdown_refs(&crate::strip_invisible::strip_invisible(
        &crate::pii_mask::redact_unconditional(s),
    ))
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn iso_date(epoch_secs: i64) -> String {
    chrono::DateTime::from_timestamp(epoch_secs, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

const PAGE_STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:44rem;margin:2rem auto;padding:0 1rem;line-height:1.5}\
     header{border-bottom:1px solid #ccc;padding-bottom:.5rem;margin-bottom:1rem}\
     .meta{color:#666;font-size:.85rem}form{margin-top:2rem;border-top:1px solid #ccc;padding-top:1rem}";

fn page(title: &str, canonical: Option<&str>, body_html: &str) -> String {
    page_with_head(title, canonical, "", body_html)
}

/// The one HTML skeleton. `head_extra` carries per-page head elements (the
/// redirect's meta-refresh) AFTER the CSP meta — a page can relax nothing;
/// the CSP is emitted first and `default-src 'none'` cannot be overridden by
/// later tags anyway.
fn page_with_head(
    title: &str,
    canonical: Option<&str>,
    head_extra: &str,
    body_html: &str,
) -> String {
    let canon = canonical
        .map(|u| format!("<link rel=\"canonical\" href=\"{}\">\n", esc(u)))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\n\
         {canon}{head_extra}<title>{}</title>\n<style>{PAGE_STYLE}</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        esc(title),
        body_html
    )
}

/// Split a KCS article body into `(heading, text)` sections on `## ` lines.
/// Split a KCS article body into `(heading, text)` sections on `## ` lines.
/// Prose BEFORE the first heading is preserved as the intro section (empty
/// heading renders as plain paragraphs), never silently dropped.
fn sections(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            out.push((h.trim().to_string(), String::new()));
        } else {
            let entry = match out.last_mut() {
                Some(e) => e,
                None => {
                    out.push((String::new(), String::new()));
                    out.last_mut().expect("just pushed")
                }
            };
            entry.1.push_str(line);
            entry.1.push('\n');
        }
    }
    out.retain(|(_, text)| !text.trim().is_empty());
    out
}

fn body_html(body: &str) -> String {
    let mut html = String::new();
    for (h, text) in sections(body) {
        if !h.is_empty() {
            html.push_str(&format!("<h2>{}</h2>\n", esc(&h)));
        }
        let mut is_list = false;
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if let Some(item) = t.strip_prefix("- ") {
                if !is_list {
                    html.push_str("<ul>\n");
                    is_list = true;
                }
                html.push_str(&format!("<li>{}</li>\n", esc(item)));
            } else {
                if is_list {
                    html.push_str("</ul>\n");
                    is_list = false;
                }
                html.push_str(&format!("<p>{}</p>\n", esc(t)));
            }
        }
        if is_list {
            html.push_str("</ul>\n");
        }
    }
    html
}

fn feedback_form(slug: &str) -> String {
    format!(
        "<form class=\"feedback\" data-slug=\"{}\">\n<label>Did this solve it? \
         <button type=\"button\" data-helpful=\"1\">Yes</button> \
         <button type=\"button\" data-helpful=\"0\">No</button></label>\n</form>\n",
        esc(slug)
    )
}

/// The article page for one slug. Shared by the build AND the approval
/// preview — what you approve is exactly what ships.
pub fn render_article_page(a: &KbArticle, base_url: Option<&str>) -> String {
    let canonical = base_url.map(|b| format!("{b}/articles/{}.html", a.slug));
    let mut body = format!(
        "<header><a href=\"../index.html\">Knowledge Base</a></header>\n<h1>{}</h1>\n\
         <p class=\"meta\">updated {} &middot; revision {}{}</p>\n",
        esc(&a.title),
        esc(&iso_date(a.updated_at)),
        esc(&a.revision),
        a.origin
            .as_deref()
            .map(|o| format!(" &middot; source {}", esc(o)))
            .unwrap_or_default(),
    );
    body.push_str(&body_html(&a.body));
    body.push_str(&feedback_form(&a.slug));
    page(&a.title, canonical.as_deref(), &body)
}

fn redirect_page(slug: &str, survivor_slug: &str, base_url: Option<&str>) -> String {
    let target = base_url
        .map(|b| format!("{b}/articles/{survivor_slug}.html"))
        .unwrap_or_else(|| format!("{}.html", survivor_slug));
    let title = format!("{slug} moved");
    // The refresh directive lives in <head> (a body-level meta is ignored by
    // browsers); the visible link is the fallback that always works.
    let head = format!(
        "<meta http-equiv=\"refresh\" content=\"0; url={}\">\n",
        esc(&target)
    );
    let body = format!(
        "<p>This article was superseded. See <a href=\"{}\">{}</a>.</p>\n",
        esc(&target),
        esc(survivor_slug)
    );
    page_with_head(&title, None, &head, &body)
}

fn index_page(articles: &[KbArticle]) -> String {
    let mut list = String::new();
    for a in articles {
        list.push_str(&format!(
            "<li><a href=\"articles/{}.html\">{}</a></li>\n",
            esc(&a.slug),
            esc(&a.title)
        ));
    }
    let body = format!("<h1>Knowledge Base</h1>\n<ul>\n{list}</ul>\n");
    page("Knowledge Base", None, &body)
}

fn not_found_page() -> String {
    page("Not found", None, "<h1>404</h1>\n<p>No such article.</p>\n")
}

fn search_index_json(articles: &[KbArticle]) -> String {
    let items: Vec<serde_json::Value> = articles
        .iter()
        .map(|a| serde_json::json!({ "title": a.title, "slug": a.slug }))
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
}

fn sitemap_xml(articles: &[KbArticle], base_url: Option<&str>) -> String {
    let Some(base) = base_url else {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"></urlset>\n".to_string();
    };
    let mut urls = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n<url><loc>{}/index.html</loc></url>\n",
        esc(base)
    );
    for a in articles {
        urls.push_str(&format!(
            "<url><loc>{}/articles/{}.html</loc></url>\n",
            esc(base),
            esc(&a.slug)
        ));
    }
    urls.push_str("</urlset>\n");
    urls
}

const ROBOTS_TXT: &str = "User-agent: *\nAllow: /\n";

/// The collected build inputs: sanitized articles + slug→survivor redirects.
pub type KbCollect = (Vec<KbArticle>, Vec<(String, String)>);

/// Collect live published articles (sanitized) plus slug→survivor redirect
/// pairs from the supersession chain. Deterministic order (by slug).
pub fn collect_articles(conn: &Connection) -> rusqlite::Result<KbCollect> {
    let mut articles = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, public_slug, COALESCE(title, ''), content,
                CAST(COALESCE(strftime('%s', created_at), '0') AS INTEGER),
                origin, content_hash
         FROM knowledge
         WHERE kcs_state = 'published' AND valid_to IS NULL AND public_slug IS NOT NULL
         ORDER BY public_slug",
    )?;
    let rows: Vec<KbArticle> = stmt
        .query_map([], |r| {
            Ok(KbArticle {
                id: r.get(0)?,
                slug: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                body: r.get::<_, String>(3)?,
                updated_at: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                origin: r.get(5)?,
                revision: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        })?
        .filter_map(Result::ok)
        // Defense-in-depth: the DB row is DATA, not trust. The publish gate
        // validates slugs, but any out-of-band write (old tooling, a bug,
        // direct SQL) must not turn a `public_slug` into a filesystem path —
        // an invalid slug is skipped, never rendered, never written.
        .filter(|a| is_valid_slug(&a.slug))
        .collect();
    for mut a in rows {
        a.title = sanitize_public(&a.title);
        if a.title.is_empty() {
            a.title = a.slug.clone();
        }
        a.body = sanitize_public(&a.body);
        a.revision = sanitize_public(&a.revision);
        a.origin = a.origin.as_deref().map(sanitize_public);
        articles.push(a);
    }

    // Redirects: an EXPIRED published article follows its supersedes chain to
    // a LIVE published survivor. Bounded walk (chains are operator-created
    // and short; 16 hops caps pathological cycles deterministically).
    let mut redirects = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, public_slug FROM knowledge
         WHERE kcs_state = 'published' AND valid_to IS NOT NULL AND public_slug IS NOT NULL
         ORDER BY public_slug",
    )?;
    let expired: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(Result::ok)
        .collect();
    for (id, old_slug) in expired {
        if !is_valid_slug(&old_slug) {
            continue;
        }
        let mut cursor = id;
        for _ in 0..16 {
            let survivor: Option<i64> = conn
                .query_row(
                    "SELECT from_chunk FROM evidence_links
                     WHERE kind = 'supersedes' AND to_chunk = ?1",
                    params![cursor],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(next) = survivor else { break };
            cursor = next;
            if let Some(a) = articles.iter().find(|a| a.id == next) {
                if a.slug != old_slug {
                    redirects.push((old_slug, a.slug.clone()));
                }
                break;
            }
        }
    }
    redirects.sort();
    redirects.dedup();
    Ok((articles, redirects))
}

/// Build the full artifact as a path→content map (byte-deterministic:
/// same inputs ⇒ identical map, iteration order is `BTreeMap`-sorted).
pub fn build_files(
    articles: &[KbArticle],
    redirects: &[(String, String)],
    base_url: Option<&str>,
) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    files.insert("index.html".into(), index_page(articles));
    files.insert("404.html".into(), not_found_page());
    files.insert("robots.txt".into(), ROBOTS_TXT.into());
    files.insert("search.json".into(), search_index_json(articles));
    files.insert("sitemap.xml".into(), sitemap_xml(articles, base_url));
    for a in articles {
        files.insert(
            format!("articles/{}.html", a.slug),
            render_article_page(a, base_url),
        );
    }
    for (old, new) in redirects {
        files.insert(
            format!("articles/{old}.html"),
            redirect_page(old, new, base_url),
        );
    }
    files
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// The content-addressed manifest over every artifact file except itself.
/// Sorted keys (serde_json preserves BTreeMap order) — the Anchor discipline
/// applied to the KB: an operator verifies what they host.
pub fn manifest_json(files: &BTreeMap<String, String>) -> String {
    let digests: BTreeMap<&String, String> = files
        .iter()
        .map(|(p, c)| (p, sha256_hex(c.as_bytes())))
        .collect();
    let body = serde_json::json!({ "files": digests });
    serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into())
}

/// Write the artifact (including `kb_manifest.json`) under `out_dir`,
/// creating parent directories. Returns the number of files written.
pub fn write_artifact(out_dir: &Path, files: &BTreeMap<String, String>) -> std::io::Result<usize> {
    std::fs::create_dir_all(out_dir)?;
    let articles_dir = out_dir.join("articles");
    std::fs::create_dir_all(&articles_dir)?;
    let mut n = 0;
    for (path, content) in files {
        let dest = out_dir.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, content)?;
        n += 1;
    }
    std::fs::write(out_dir.join(MANIFEST_NAME), manifest_json(files))?;
    Ok(n + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    fn insert_published(conn: &Connection, slug: &str, title: &str, body: &str) -> i64 {
        conn.execute(
            "INSERT INTO knowledge(content, title, source, content_hash, node_kind,
                                   assertion_kind, confidence, domain, kcs_state, public_slug, created_at)
             VALUES (?1, ?2, 'agent', ?3, 'fact', 'stated', 0.8, 'global', 'published', ?4, 1756000000)",
            params![format!("# {title}\n\n{body}"), title, format!("hash-{slug}"), slug],
        )
        .expect("insert");
        conn.last_insert_rowid()
    }

    #[test]
    fn kb_build_is_deterministic_byte_for_byte() {
        let conn = db();
        insert_published(
            &conn,
            "laptop-wont-boot",
            "Laptop won't boot",
            "## Issue\nNo power.",
        );
        let (a1, r1) = collect_articles(&conn).expect("collect");
        let f1 = build_files(&a1, &r1, Some("https://kb.example.com"));
        let m1 = manifest_json(&f1);
        let (a2, r2) = collect_articles(&conn).expect("collect");
        let f2 = build_files(&a2, &r2, Some("https://kb.example.com"));
        assert_eq!(f1, f2, "same DB state ⇒ byte-identical file map");
        assert_eq!(m1, manifest_json(&f2));
    }

    #[test]
    fn public_pages_pass_strict_sanitize_with_no_operator_bypass() {
        // A privileged principal would see raw PII on internal reads; the
        // public seam takes NO principal — output is identical either way.
        let raw = "contact admin@corp.com or 555-123-4567";
        let sanitized = sanitize_public(raw);
        assert!(!sanitized.contains("admin@corp.com"));
        assert_eq!(
            sanitized,
            crate::fence::strip_markdown_refs(&crate::strip_invisible::strip_invisible(
                &crate::pii_mask::redact_unconditional(raw)
            ))
        );
        // invisible bidi smuggling never reaches HTML
        let smuggled = "safe\u{202E}evil";
        assert!(!sanitize_public(smuggled).contains('\u{202E}'));
        // markdown image refs are stripped (EchoLeak class)
        let refs = "![x](https://evil.invalid/pixel)";
        assert!(!sanitize_public(refs).contains("https://evil.invalid/pixel"));
    }

    #[test]
    fn pii_never_reaches_public_html() {
        let conn = db();
        insert_published(
            &conn,
            "billing-issue",
            "Billing issue",
            "## Issue\nCustomer email jane@example.com phone 5558675309.",
        );
        let (articles, redirects) = collect_articles(&conn).expect("collect");
        let files = build_files(&articles, &redirects, None);
        for (path, content) in &files {
            assert!(
                !content.contains("jane@example.com") && !content.contains("5558675309"),
                "raw PII leaked into {path}"
            );
        }
    }

    #[test]
    fn superseded_slug_redirects_to_survivor() {
        let conn = db();
        let old = insert_published(&conn, "old-fix", "Old fix", "## Issue\nx");
        let new_id = insert_published(&conn, "new-fix", "New fix", "## Issue\ny");
        conn.execute(
            "UPDATE knowledge SET valid_to = datetime('now') WHERE id = ?1",
            params![old],
        )
        .expect("expire");
        conn.execute(
            "INSERT INTO evidence_links(from_chunk, to_chunk, kind) VALUES (?1, ?2, 'supersedes')",
            params![new_id, old],
        )
        .expect("link");
        let (articles, redirects) = collect_articles(&conn).expect("collect");
        assert_eq!(
            redirects,
            vec![("old-fix".to_string(), "new-fix".to_string())]
        );
        let files = build_files(&articles, &redirects, None);
        let redirect = &files["articles/old-fix.html"];
        assert!(
            redirect.contains("new-fix.html"),
            "redirect points at survivor"
        );
        assert!(
            !redirect.contains("## Issue"),
            "superseded page carries no body"
        );
    }

    #[test]
    fn kb_manifest_digests_match_files() {
        let conn = db();
        insert_published(&conn, "slug-one", "One", "## Issue\nbody");
        let (articles, redirects) = collect_articles(&conn).expect("collect");
        let files = build_files(&articles, &redirects, None);
        let manifest = manifest_json(&files);
        let parsed: serde_json::Value = serde_json::from_str(&manifest).expect("manifest json");
        for (path, digest) in parsed["files"].as_object().expect("files obj") {
            let content = files
                .get(path.as_str())
                .unwrap_or_else(|| panic!("manifest names unknown file {path}"));
            let mut h = Sha256::new();
            h.update(content.as_bytes());
            let expect: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(
                digest.as_str().expect("str"),
                expect,
                "digest mismatch for {path}"
            );
        }
        assert!(
            !parsed["files"]
                .as_object()
                .expect("obj")
                .contains_key(MANIFEST_NAME)
        );
    }

    #[test]
    fn redirect_refresh_lives_in_head_with_csp_first() {
        // A body-level meta refresh is ignored by browsers — the directive
        // must be in <head>, and the CSP meta must precede it (nothing a
        // page contains can relax default-src 'none').
        let survivor = KbArticle {
            id: 2,
            slug: "new-fix".into(),
            title: "New fix".into(),
            body: String::new(),
            updated_at: 0,
            origin: None,
            revision: String::new(),
        };
        let html = redirect_page("old-fix", &survivor.slug, None);
        let head = html.split("</head>").next().unwrap_or("");
        assert!(
            head.contains("http-equiv=\"refresh\""),
            "refresh is in head"
        );
        let csp_pos = html.find("Content-Security-Policy").expect("csp");
        let refresh_pos = html.find("http-equiv=\"refresh\"").expect("refresh");
        assert!(
            csp_pos < refresh_pos,
            "CSP emitted before any per-page head"
        );
    }

    #[test]
    fn intro_prose_before_first_section_is_preserved() {
        let body = "# Title\n\nIntro paragraph that must survive.\n\n## Issue\nthing\n";
        let secs = sections(body);
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].0, "", "intro has no heading");
        assert!(secs[0].1.contains("must survive"));
        assert_eq!(secs[1].0, "Issue");
    }

    #[test]
    fn slug_vocabulary_is_strict() {
        assert!(is_valid_slug("laptop-wont-boot"));
        assert!(is_valid_slug("fix-2"));
        assert!(!is_valid_slug("../etc"));
        assert!(!is_valid_slug("-lead"));
        assert!(!is_valid_slug("trail-"));
        assert!(!is_valid_slug("double--hyphen"));
        assert!(!is_valid_slug("UPPER"));
        assert!(!is_valid_slug(""));
    }

    #[test]
    fn kb_build_refuses_traversal_slug() {
        // Defense-in-depth: a hostile/out-of-band public_slug in the DB must
        // never become a filesystem path (the publish gate validates, but the
        // build reads the DB as untrusted data).
        let conn = db();
        conn.execute(
            "INSERT INTO knowledge(content, title, source, content_hash, node_kind,
                                   assertion_kind, confidence, domain, kcs_state, public_slug)
             VALUES ('c', 'Evil', 'agent', 'h-evil', 'fact', 'stated', 0.8,
                     'global', 'published', '../../escape')",
            [],
        )
        .unwrap();
        let (articles, redirects) = collect_articles(&conn).expect("collect");
        assert!(
            articles.iter().all(|a| is_valid_slug(&a.slug)),
            "invalid slugs are never collected"
        );
        let files = build_files(&articles, &redirects, None);
        assert!(
            !files.keys().any(|p| p.contains("..")),
            "no traversal path reaches the artifact: {files:?}"
        );
    }

    #[test]
    fn write_artifact_lands_manifest_and_pages() {
        let conn = db();
        insert_published(&conn, "art", "Art", "## Issue\nbody");
        let (articles, redirects) = collect_articles(&conn).expect("collect");
        let files = build_files(&articles, &redirects, None);
        let dir = std::env::temp_dir().join(format!("brain-kb-test-{}", std::process::id()));
        let n = write_artifact(&dir, &files).expect("write");
        assert!(dir.join(MANIFEST_NAME).exists());
        assert!(dir.join("articles/art.html").exists());
        assert_eq!(n, files.len() + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
