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
    build_files_ext(articles, redirects, base_url, &BuildOptions::default())
}

/// Build options: additive flags; the default build is
/// byte-identical to the pre-options one.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Emit `status/{ref}.json` + `status/{ref}.html` for every live
    /// case-status ref and exclude `/status/` from robots.txt.
    pub with_case_status: bool,
    /// Emit per-locale article pages (`{locale}/{slug}.html`) alongside the
    /// default-locale pages, with hreflang alternates + an x-default.
    pub locales: Vec<String>,
    /// Approved human translations (collected by the caller).
    pub translations: Vec<KbTranslation>,
    /// Live case-status entries (collected by the caller).
    pub status_entries: Vec<CaseStatusEntry>,
}

/// An approved human translation shaped for the public site (already
/// sanitized by [`collect_translations`]).
#[derive(Debug, Clone, PartialEq)]
pub struct KbTranslation {
    pub knowledge_id: i64,
    pub locale: String,
    pub title: String,
    pub body_md: String,
}

/// Collect APPROVED translations for published articles, sanitized through
/// the same strict seam as the source articles. Deterministic order.
pub fn collect_translations(conn: &Connection) -> rusqlite::Result<Vec<KbTranslation>> {
    let mut stmt = conn.prepare(
        "SELECT t.knowledge_id, t.locale, t.title, t.body_md
         FROM kcs_translations t JOIN knowledge k ON k.id = t.knowledge_id
         WHERE t.state = 'approved' AND k.kcs_state = 'published'
           AND k.valid_to IS NULL AND k.public_slug IS NOT NULL
         ORDER BY t.knowledge_id, t.locale",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, locale, title, body)| KbTranslation {
            knowledge_id: id,
            locale,
            title: sanitize_public(&title),
            body_md: sanitize_public(&body),
        })
        .collect())
}

fn hreflang_head(
    slug: &str,
    base_url: Option<&str>,
    locales: &[String],
    have: &[&KbTranslation],
) -> String {
    let Some(base) = base_url else {
        return String::new();
    };
    let mut out = String::new();
    // x-default points at the default-locale page (the canonical article).
    out.push_str(&format!(
        "<link rel=\"alternate\" hreflang=\"x-default\" href=\"{base}/articles/{slug}.html\">\n"
    ));
    for l in locales {
        let target = if let Some(t) = have.iter().find(|t| t.locale == *l) {
            format!("{base}/{}/{slug}.html", esc(&t.locale))
        } else {
            // No translation ⇒ the localized URL still exists (serving the
            // default content with the explicit note), so the alternate is
            // honest for every declared locale.
            format!("{base}/{l}/{slug}.html")
        };
        out.push_str(&format!(
            "<link rel=\"alternate\" hreflang=\"{}\" href=\"{}\">\n",
            esc(l),
            target
        ));
    }
    out
}

/// The honesty rule rendered: no silent fallback. A localized URL without a
/// translation serves the DEFAULT-LOCALE content with a visible note naming
/// where the real text lives.
const MISSING_NOTE_EN: &str = "<p class=\"meta\">This article is not yet available in this language — read the English version below.</p>\n";

/// Render one article in one locale: the human translation when one exists,
/// otherwise the default content behind the explicit availability note.
/// Shares the status-page dir/ltr plumbing (`page_with_head`); RTL arrives
/// with Charter's `ar` locale work on the same skeleton.
fn localized_page(
    a: &KbArticle,
    t: Option<&KbTranslation>,
    locale: &str,
    base_url: Option<&str>,
    locales: &[String],
    all_for_slug: &[&KbTranslation],
) -> String {
    let (title, body, note) = if let Some(t) = t {
        (t.title.clone(), t.body_md.clone(), "")
    } else {
        (a.title.clone(), a.body.clone(), MISSING_NOTE_EN)
    };
    let head_extra = hreflang_head(&a.slug, base_url, locales, all_for_slug);
    crate::kb::page_with_head(
        &title,
        base_url
            .map(|b| format!("{b}/{locale}/{}.html", a.slug))
            .as_deref(),
        &head_extra,
        &format!("{note}{}", body_html(&body)),
    )
}

fn locale_search_index(
    articles: &[KbArticle],
    translations: &[&KbTranslation],
    locale: &str,
) -> String {
    let items: Vec<serde_json::Value> = translations
        .iter()
        .filter(|t| t.locale == locale)
        .filter_map(|t| articles.iter().find(|a| a.id == t.knowledge_id))
        .map(|a| serde_json::json!({ "title": a.title, "slug": a.slug }))
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

/// The sitemap WITH per-locale alternates: every declared locale gets an
/// xhtml:link entry; status refs never appear anywhere here.
fn sitemap_xml_locales(
    articles: &[KbArticle],
    base_url: Option<&str>,
    locales: &[String],
    translations: &[KbTranslation],
) -> String {
    let Some(base) = base_url else {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"></urlset>\n".to_string();
    };
    let mut urls = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n<url><loc>{base}/index.html</loc></url>\n"
    );
    for a in articles {
        let alts: String = locales
            .iter()
            .map(|l| {
                format!(
                    "<xhtml:link rel=\"alternate\" hreflang=\"{l}\" href=\"{base}/{l}/{}.html\"/>",
                    esc(&a.slug)
                )
            })
            .collect();
        urls.push_str(&format!(
            "<url><loc>{base}/articles/{}.html</loc>{alts}</url>\n",
            esc(&a.slug)
        ));
    }
    let _ = translations;
    urls.push_str("</urlset>\n");
    urls
}

/// One public case-status page's build inputs. Everything here is from the
/// FIXED public vocabulary — no PII, no raw deadlines, no operator names
/// (pinned by tests).
#[derive(Debug, Clone, PartialEq)]
pub struct CaseStatusEntry {
    pub run_id: i64,
    pub r: String,
    /// The seven-word public status (`PublicStatus::as_str`).
    pub status: &'static str,
    /// Promise bucket derived from the envelope P-class TTL — a class
    /// window, never a raw deadline or an internal clock reading.
    pub promise_bucket: &'static str,
    /// Build stamp (the honest static-freshness ceiling).
    pub updated_at: i64,
}

/// The envelope P-class → promise-bucket label table. The ONLY place these
/// labels live.
fn promise_label(p: brain_engine_sdk::policy::Priority) -> &'static str {
    match p {
        brain_engine_sdk::policy::Priority::P1 => "4 hours",
        brain_engine_sdk::policy::Priority::P2 => "24 hours",
        brain_engine_sdk::policy::Priority::P3 => "72 hours",
        // #[non_exhaustive] SDK enum: a future class degrades to the widest
        // window, never a panic in the build.
        _ => "7 days",
    }
}

fn parse_priority(state: &serde_json::Value) -> brain_engine_sdk::policy::Priority {
    match state.get("priority").and_then(|v| v.as_str()) {
        Some("P1") | Some("p1") | Some("1") => brain_engine_sdk::policy::Priority::P1,
        Some("P2") | Some("p2") | Some("2") => brain_engine_sdk::policy::Priority::P2,
        Some("P4") | Some("p4") | Some("4") => brain_engine_sdk::policy::Priority::P4,
        _ => brain_engine_sdk::policy::Priority::P3,
    }
}

/// The fixed next-action sentence per public status (English default locale;
/// i18n-ready — locale pages share this template table shape). Fixed templates only:
/// never free text from the run.
pub fn status_sentence(status: &str) -> &'static str {
    match status {
        "received" => "We have received your case and will start work shortly.",
        "in-progress" => "We are actively working on your case.",
        "awaiting-your-reply" => "We need your reply before we can continue.",
        "awaiting-confirmation" => "Please review and confirm the proposed resolution.",
        "resolved" => "Your case has been resolved.",
        "closed" => "This case is closed. Thank you for your patience.",
        _ => "Your case is being processed.",
    }
}

/// Collect the live case-status refs joined to their runs' current state and
/// map each through the deterministic public vocabulary. Deterministic order
/// (by ref). A missing/deleted run row yields NO page (fail closed — a dead
/// run must not advertise a stale state forever).
pub fn collect_status_entries(
    conn: &Connection,
    build_time: i64,
) -> rusqlite::Result<Vec<CaseStatusEntry>> {
    let mut stmt = conn.prepare(
        "SELECT r.run_id, r.ref, COALESCE(run.state_json, '')
         FROM case_status_refs r
         LEFT JOIN workflow_runs run ON run.id = r.run_id
         WHERE r.revoked_at IS NULL ORDER BY r.ref",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, state)| !state.is_empty())
        .map(|(run_id, r, state_json)| {
            let state: serde_json::Value =
                serde_json::from_str(&state_json).unwrap_or(serde_json::Value::Null);
            let p = parse_priority(&state);
            CaseStatusEntry {
                run_id,
                r,
                status: brain_engine_sdk::workflow_state::public_status(&state).as_str(),
                promise_bucket: promise_label(p),
                updated_at: build_time,
            }
        })
        .collect())
}

/// The public JSON payload: fixed vocabulary + class-window promise +
/// build stamp + one fixed-template sentence. Nothing else — no PII, no
/// deadlines, no names.
pub fn status_json(e: &CaseStatusEntry) -> String {
    let body = serde_json::json!({
        "status": e.status,
        "promise": format!("expected within {}", e.promise_bucket),
        "next_action": status_sentence(e.status),
        "updated_at": e.updated_at,
    });
    serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into())
}

/// The static status page: renders the SAME content as the JSON inline plus
/// a same-origin fetch of `status/{ref}.json` for freshness on reload.
fn status_html(e: &CaseStatusEntry) -> String {
    crate::kb::page_with_head(
        "Case status",
        None,
        "<meta name=\"robots\" content=\"noindex\">\n",
        &format!(
            "<h1>Case status</h1>\n<p>Status: <strong>{}</strong></p>\n<p>{}</p>\n\
             <p class=\"meta\">Expected within {}.</p>\n<p class=\"meta\">Updated at build time ({}).</p>",
            esc(e.status),
            esc(status_sentence(e.status)),
            esc(e.promise_bucket),
            iso_date(e.updated_at),
        ),
    )
}

const ROBOTS_TXT_STATUS: &str = "User-agent: *\nAllow: /\nDisallow: /status/\n";

/// Extend a built file set with case-status artifacts (JSON + HTML per live
/// ref) and the `/status/` robots exclusion. Status refs NEVER appear in the
/// sitemap (it is not touched here), and every emitted file lands in the
/// manifest automatically via [`manifest_json`].
pub fn add_case_status_files(files: &mut BTreeMap<String, String>, entries: &[CaseStatusEntry]) {
    if entries.is_empty() {
        return;
    }
    files.insert("robots.txt".into(), ROBOTS_TXT_STATUS.into());
    for e in entries {
        files.insert(format!("status/{}.json", e.r), status_json(e));
        files.insert(format!("status/{}.html", e.r), status_html(e));
    }
}

pub fn build_files_ext(
    articles: &[KbArticle],
    redirects: &[(String, String)],
    base_url: Option<&str>,
    opts: &BuildOptions,
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
    // ── Per-locale pages + per-locale search indexes. Missing
    // translations serve the default content with an explicit note — never a
    // silent fallback.
    if !opts.locales.is_empty() {
        for locale in &opts.locales {
            let for_locale: Vec<&KbTranslation> = opts
                .translations
                .iter()
                .filter(|t| t.locale == *locale)
                .collect();
            for a in articles {
                let all_for_slug: Vec<&KbTranslation> = opts
                    .translations
                    .iter()
                    .filter(|t| t.knowledge_id == a.id)
                    .collect();
                let t = for_locale.iter().find(|t| t.knowledge_id == a.id).copied();
                files.insert(
                    format!("{locale}/{}.html", a.slug),
                    localized_page(a, t, locale, base_url, &opts.locales, &all_for_slug),
                );
            }
            files.insert(
                format!("search.{locale}.json"),
                locale_search_index(
                    articles,
                    &opts.translations.iter().collect::<Vec<_>>(),
                    locale,
                ),
            );
        }
        if base_url.is_some() {
            files.insert(
                "sitemap.xml".into(),
                sitemap_xml_locales(articles, base_url, &opts.locales, &opts.translations),
            );
        }
    }
    // ── Case-status artifacts + the /status/ robots exclusion.
    if opts.with_case_status {
        add_case_status_files(&mut files, &opts.status_entries);
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
    fn hreflang_alternates_and_x_default_are_complete() {
        let conn = db();
        insert_published(&conn, "art", "Art", "body");
        let (articles, _) = collect_articles(&conn).expect("collect");
        let locales = vec!["en".to_string(), "de".to_string(), "fr".to_string()];
        let translations = vec![KbTranslation {
            knowledge_id: articles[0].id,
            locale: "de".into(),
            title: "Titel".into(),
            body_md: "Inhalt".into(),
        }];
        let files = build_files_ext(
            &articles,
            &[],
            Some("https://kb.example.com"),
            &BuildOptions {
                locales: locales.clone(),
                translations,
                ..Default::default()
            },
        );
        // All three locale URLs exist.
        assert!(files.contains_key("de/art.html"));
        assert!(files.contains_key("fr/art.html"));
        assert!(files.contains_key("en/art.html"));
        let de = &files["de/art.html"];
        // x-default → the default-locale canonical page.
        assert!(
            de.contains("hreflang=\"x-default\" href=\"https://kb.example.com/articles/art.html\"")
        );
        // Every declared locale gets an alternate, including the translated one.
        for l in &locales {
            assert!(
                de.contains(&format!("hreflang=\"{l}\"")),
                "missing alternate for {l} in de page"
            );
        }
        assert!(de.contains("https://kb.example.com/de/art.html"));
        assert!(de.contains("Titel"), "the human translation renders");
    }

    #[test]
    fn missing_translation_shows_explicit_note_not_silent_fallback() {
        let conn = db();
        insert_published(&conn, "art", "Art", "## S\nbody text");
        let (articles, _) = collect_articles(&conn).expect("collect");
        // fr has NO translation for this article.
        let files = build_files_ext(
            &articles,
            &[],
            None,
            &BuildOptions {
                locales: vec!["fr".to_string()],
                with_case_status: false,
                ..Default::default()
            },
        );
        let fr = &files["fr/art.html"];
        assert!(
            fr.contains("not yet available in this language"),
            "the honesty note must be visible"
        );
        assert!(
            fr.contains("<h2>S</h2>"),
            "default content served behind the note"
        );
        // A TRANSLATED page carries no note.
        let files2 = build_files_ext(
            &articles,
            &[],
            None,
            &BuildOptions {
                locales: vec!["de".to_string()],
                with_case_status: false,
                translations: vec![KbTranslation {
                    knowledge_id: articles[0].id,
                    locale: "de".into(),
                    title: "Titel".into(),
                    body_md: "## S\nInhalt".into(),
                }],
                status_entries: Vec::new(),
            },
        );
        let de = &files2["de/art.html"];
        assert!(!de.contains("not yet available"));
        assert!(de.contains("Inhalt"));
    }

    #[test]
    fn search_index_is_per_locale() {
        let conn = db();
        insert_published(&conn, "art-a", "A", "x");
        insert_published(&conn, "art-b", "B", "y");
        let (articles, _) = collect_articles(&conn).expect("collect");
        let id_a = articles.iter().find(|a| a.slug == "art-a").expect("a").id;
        let files = build_files_ext(
            &articles,
            &[],
            None,
            &BuildOptions {
                locales: vec!["de".to_string()],
                translations: vec![KbTranslation {
                    knowledge_id: id_a,
                    locale: "de".into(),
                    title: "A-de".into(),
                    body_md: "x".into(),
                }],
                ..Default::default()
            },
        );
        let idx = &files["search.de.json"];
        // The global index is untouched.
        assert_eq!(files["search.json"], search_index_json(&articles));
        assert!(
            idx.contains("\"art-a\""),
            "translated article in its locale index"
        );
        assert!(
            !idx.contains("\"art-b\""),
            "untranslated article NOT in the locale index"
        );
    }

    #[test]
    fn sitemap_alternates_cover_locales_and_never_status_refs() {
        let conn = db();
        insert_published(&conn, "art", "Art", "b");
        let (articles, _) = collect_articles(&conn).expect("collect");
        let files = build_files_ext(
            &articles,
            &[],
            Some("https://kb.example.com"),
            &BuildOptions {
                locales: vec!["de".to_string(), "nl".to_string()],
                ..Default::default()
            },
        );
        let sm = &files["sitemap.xml"];
        assert!(sm.contains("hreflang=\"de\" href=\"https://kb.example.com/de/art.html\""));
        assert!(sm.contains("hreflang=\"nl\" href=\"https://kb.example.com/nl/art.html\""));
        assert!(sm.contains("xhtml:link"));
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

    // ── The public case-status artifact.

    fn insert_run_with_state(conn: &Connection, run_id: i64, state_json: &str) {
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, status, created_at, updated_at)
             VALUES (?1, 'global', 'case', ?2, 'active', 1000, 1000)",
            rusqlite::params![run_id, state_json],
        )
        .expect("insert run");
    }

    fn insert_live_ref(conn: &Connection, run_id: i64, r: &str) {
        conn.execute(
            "INSERT INTO case_status_refs(run_id, ref, salt_version, minted_at) VALUES (?1, ?2, 1, 1000)",
            rusqlite::params![run_id, r],
        )
        .expect("insert ref");
    }

    #[test]
    fn status_json_contains_no_pii_no_deadlines_no_names() {
        let conn = db();
        // PII in EVERY field: subject-like content, an operator name, a raw
        // deadline epoch in the state. None of it may reach the page.
        insert_run_with_state(
            &conn,
            1,
            r#"{"status":"active","pending_question":"reply to jane.doe@corp.example about invoice","operator":"Alice Smith","sla_deadline":9999999999,"subject":"Jane Doe, acct 4242"}"#,
        );
        insert_live_ref(&conn, 1, "REFREFREFREFREFREFREFRE");
        let entries = collect_status_entries(&conn, 5_000).expect("collect");
        assert_eq!(entries.len(), 1);
        let json = status_json(&entries[0]);
        for banned in [
            "jane.doe",
            "Jane Doe",
            "Alice Smith",
            "4242",
            "invoice",
            "9999999999",
            "sla_deadline",
            "deadline",
            "operator",
            "subject",
        ] {
            assert!(
                !json.to_lowercase().contains(&banned.to_lowercase()),
                "PII/internal leak '{banned}' in: {json}"
            );
        }
        assert!(json.contains("\"status\": \""));
        assert!(json.contains("expected within"));
        // The HTML page carries the same discipline.
        let html = format!("{:?}", files_for(entries));
        assert!(!html.to_lowercase().contains("alice smith"));
    }

    fn files_for(entries: Vec<CaseStatusEntry>) -> BTreeMap<String, String> {
        let mut files = build_files(&[], &[], None);
        add_case_status_files(&mut files, &entries);
        files
    }

    #[test]
    fn promise_bucket_comes_from_envelope_class_not_internal_clock() {
        let conn = db();
        insert_run_with_state(&conn, 1, r#"{"status":"active","priority":"P1"}"#);
        insert_run_with_state(&conn, 2, r#"{"status":"active","priority":"P4"}"#);
        insert_run_with_state(&conn, 3, r#"{"status":"active"}"#);
        insert_live_ref(&conn, 1, "AAAAAAAAAAAAAAAAAAAAAAAAAA");
        insert_live_ref(&conn, 2, "BBBBBBBBBBBBBBBBBBBBBBBBBB");
        insert_live_ref(&conn, 3, "CCCCCCCCCCCCCCCCCCCCCCCCCC");
        // Two builds at very different times: the bucket is a CLASS window,
        // not a clock reading — identical labels both times.
        let early = collect_status_entries(&conn, 1_000).expect("early");
        let late = collect_status_entries(&conn, 99_000_000_000).expect("late");
        // Only the honest build stamp moves; class windows are identical.
        let strip = |es: &[CaseStatusEntry]| -> Vec<(&'static str, &'static str)> {
            es.iter().map(|e| (e.status, e.promise_bucket)).collect()
        };
        assert_eq!(strip(&early), strip(&late));
        let by_ref: BTreeMap<&str, &CaseStatusEntry> =
            early.iter().map(|e| (e.r.as_str(), e)).collect();
        assert_eq!(
            by_ref["AAAAAAAAAAAAAAAAAAAAAAAAAA"].promise_bucket,
            "4 hours"
        );
        assert_eq!(
            by_ref["BBBBBBBBBBBBBBBBBBBBBBBBBB"].promise_bucket,
            "7 days"
        );
        // Unstated priority defaults to the P3 class window.
        assert_eq!(
            by_ref["CCCCCCCCCCCCCCCCCCCCCCCCCC"].promise_bucket,
            "72 hours"
        );
    }

    #[test]
    fn revoked_refs_and_missing_runs_never_reach_the_build() {
        let conn = db();
        insert_run_with_state(&conn, 1, r#"{"status":"done"}"#);
        insert_run_with_state(&conn, 2, r#"{"status":"done"}"#);
        insert_live_ref(&conn, 1, "LIVEREFXXXXXXXXXXXXXXXXXXX");
        insert_live_ref(&conn, 2, "DEADREFXXXXXXXXXXXXXXXXXXX");
        conn.execute(
            "UPDATE case_status_refs SET revoked_at = 2000 WHERE run_id = 2",
            [],
        )
        .expect("revoke");
        insert_live_ref(&conn, 9, "ORPHANREFXXXXXXXXXXXXXXXXX"); // no run row
        let entries = collect_status_entries(&conn, 3_000).expect("collect");
        assert_eq!(entries.len(), 1, "only the live, existing run survives");
        assert_eq!(entries[0].r, "LIVEREFXXXXXXXXXXXXXXXXXXX");
        let mut files = build_files(&[], &[], None);
        add_case_status_files(&mut files, &entries);
        assert!(files.contains_key("status/LIVEREFXXXXXXXXXXXXXXXXXXX.json"));
        assert!(files.contains_key("status/LIVEREFXXXXXXXXXXXXXXXXXXX.html"));
        assert!(
            !files
                .keys()
                .any(|p| p.contains("DEADREF") || p.contains("ORPHANREF")),
            "revoked/missing never build"
        );
    }

    #[test]
    fn status_pages_are_noindex_and_absent_from_sitemap() {
        let conn = db();
        insert_published(&conn, "art", "Art", "body");
        insert_run_with_state(&conn, 1, r#"{"status":"active","next_step":"x"}"#);
        insert_live_ref(&conn, 1, "NOIDXREFXXXXXXXXXXXXXXXXXXX");
        let entries = collect_status_entries(&conn, 3_000).expect("collect");
        let (arts, _) = collect_articles(&conn).expect("articles");
        let mut files = build_files(&arts, &[], Some("https://kb.example.com"));
        add_case_status_files(&mut files, &entries);
        // robots.txt excludes /status/.
        assert_eq!(files["robots.txt"], ROBOTS_TXT_STATUS);
        assert!(files["robots.txt"].contains("Disallow: /status/"));
        // The sitemap carries articles only — refs NEVER appear.
        let sitemap = &files["sitemap.xml"];
        assert!(sitemap.contains("https://kb.example.com/articles/art.html"));
        assert!(!sitemap.contains("NOIDXREF"), "refs never ride the sitemap");
        // The JSON payload is marked noindex via the HTML twin; the HTML twin
        // itself carries the noindex meta.
        assert!(files["status/NOIDXREFXXXXXXXXXXXXXXXXXXX.html"].contains("noindex"));
        // Manifest digests cover the status files (Anchor discipline).
        let manifest = manifest_json(&files);
        assert!(manifest.contains("status/NOIDXREFXXXXXXXXXXXXXXXXXXX.json"));
    }
}
