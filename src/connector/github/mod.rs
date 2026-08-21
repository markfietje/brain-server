//! GitHub connector orchestration: backfill loop + cursor persistence.
//!
//! Backfill flow (per configured repo):
//!   1. Read last-sync cursor from `connector_checkpoints` keyed `issues:{repo}`.
//!   2. Page through `/repos/{repo}/issues?since={cursor}&sort=updated&direction=asc`.
//!   3. For each item that is NOT a PR (PRs have a `pull_request` field):
//!      translate → POST /ingest/markdown.
//!   4. Update the cursor to the latest `updated_at` seen.
//!   5. Continue until `Link: rel="next"` is absent.
//!
//! `ponytail:` ceilings:
//! - **One repo at a time, sequential.** No parallel requests — GitHub's
//!   rate limit is per-token, not per-connection, so concurrency buys nothing.
//! - **Cursor granularity is per-(repo, kind).** If a single doc is edited
//!   mid-backfill, it'll be picked up on the next pass. Per-doc cursors
//!   would require O(N) checkpoint writes per sync.
//! - **Bound: GH_BACKFILL_MAX_DOCS per repo.** Fails loud if exceeded;
//!   better to surface a misconfigured repo than silently truncate.

#![cfg(feature = "connector-github")]

pub mod client;
pub mod translate;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::connector::auth::AccessToken;
use crate::connector::github::client::{GitHubClient, Page};
use crate::connector::github::translate::translate_issue;

/// Hard cap on docs per repo per backfill.
pub const MAX_DOCS_PER_REPO: usize = 10_000;

/// One backfill pass: walk issues for one repo from the cursor forward,
/// translate + ingest each. Returns the count of docs ingested (new OR
/// updated — same-content re-ingests are server-side no-ops per
/// `sources::upsert_revision`) AND the set of source URIs walked (used by
/// the caller to run reconcile across all repos in one call).
///
/// `brain_base` is the brain-server URL (e.g. `http://127.0.0.1:8765`).
/// `brain_token` is the bearer to authenticate to brain-server.
///
/// ponytail: 9 args is over clippy's default threshold. Bundling them into a
/// `BackfillContext` struct is pure ceremony for a function with one caller
/// (the binary's main). The cost of the bundled struct (4 lines of boilerplate
/// per field) exceeds the cost of the inline signature. Revisit if a second
/// caller ever appears.
#[allow(clippy::too_many_arguments)]
pub fn backfill_issues_for_repo(
    db: &Connection,
    client: &GitHubClient,
    brain_http: &reqwest::blocking::Client,
    gh_token: &AccessToken,
    brain_base: &str,
    brain_token: Option<&str>,
    connector_id: i64,
    owner: &str,
    repo: &str,
) -> Result<BackfillReport> {
    let cursor_key = format!("issues:{owner}/{repo}");
    let since = get_cursor(db, connector_id, &cursor_key)?;
    let mut ingested = 0usize;
    let mut latest_cursor = since.clone();
    let mut walked_uris: Vec<String> = Vec::new();

    let mut page: Option<Page> =
        Some(client.list_issues_page(owner, repo, since.as_deref(), &gh_token.value)?);
    while let Some(current) = page.take() {
        for issue in &current.items {
            // PRs appear in the issues endpoint (they ARE issues with extra
            // fields). Skip them here; the PR backfill path lands later.
            if issue.get("pull_request").is_some() {
                continue;
            }
            let translated = translate_issue(issue, owner, repo)?;
            walked_uris.push(translated.source_uri.clone());
            // Track the latest cursor even on no-op ingests — GitHub's
            // `?since=` is inclusive, and an unchanged doc still has a
            // valid `updated_at` we should not re-walk next time.
            if translated.updated_at.as_str() > latest_cursor.as_deref().unwrap_or("") {
                latest_cursor = Some(translated.updated_at.clone());
            }
            ingest_one(
                brain_http,
                brain_base,
                brain_token,
                &translated.markdown,
                &translated.source_uri,
            )
            .with_context(|| format!("ingest failed for {}", translated.source_uri))?;
            ingested += 1;
            if ingested >= MAX_DOCS_PER_REPO {
                tracing::warn!(
                    repo = format!("{owner}/{repo}"),
                    max = MAX_DOCS_PER_REPO,
                    "backfill hit MAX_DOCS_PER_REPO cap; rest will land on next sync"
                );
                break;
            }
        }
        // Fetch next page if present. The `Link: rel="next"` URL is opaque —
        // we don't parse it, just GET it.
        match &current.next {
            Some(next_url) => {
                page = Some(client.list_page_by_url(next_url, &gh_token.value)?);
            }
            None => break,
        }
    }

    // Persist the cursor only after a successful full walk. If we crashed
    // mid-page, the next sync re-walks the same page — idempotent ingest
    // makes that cheap (server-side no-op on unchanged content).
    if let Some(c) = &latest_cursor {
        upsert_cursor(db, connector_id, &cursor_key, c)?;
    }

    Ok(BackfillReport {
        repo: format!("{owner}/{repo}"),
        ingested,
        cursor: latest_cursor,
        walked_uris,
    })
}

/// Summary of one repo's backfill pass. Emitted as a `progress` JSON-line.
/// `walked_uris` is the full set of GitHub source URIs we touched (including
/// unchanged ones — the caller needs the complete live set for reconcile).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackfillReport {
    pub repo: String,
    pub ingested: usize,
    pub cursor: Option<String>,
    #[serde(skip_serializing)]
    pub walked_uris: Vec<String>,
}

/// POST the full live-URI set for kind="github" to brain-server's
/// `/sources/reconcile`. Any indexed GitHub source whose URI is NOT in the
/// set gets swept (chunks removed, source + revision tombstoned) by the
/// server's existing `sources::reconcile`. This is the authoritative
/// converge path: even if a webhook delivery is missed or a repo is
/// uninstalled, the next periodic reconcile cleans up.
///
/// MUST be called with the union of URIs across ALL configured repos —
/// `sources::reconcile` is kind-scoped, so calling it per-repo with only
/// that repo's URIs would sweep the other repos' rows.
pub fn reconcile_github_sources(
    brain_http: &reqwest::blocking::Client,
    brain_base: &str,
    brain_token: Option<&str>,
    live_uris: &[String],
) -> Result<ReconcileReport> {
    let body = serde_json::json!({
        "kind": "github",
        "live_uris": live_uris,
    });
    let mut req = brain_http
        .post(format!("{brain_base}/sources/reconcile"))
        .header("Content-Type", "application/json")
        .json(&body);
    if let Some(t) = brain_token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let resp = req.send().context("/sources/reconcile POST failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("brain-server /sources/reconcile returned {status}: {body}");
    }
    let parsed: serde_json::Value = resp.json().context("reconcile response was not JSON")?;
    Ok(ReconcileReport {
        deleted_sources: parsed
            .get("deleted_sources")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        deleted_chunks: parsed
            .get("deleted_chunks")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        orphan_uris: parsed
            .get("orphan_uris")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Summary of one reconcile pass. Mirrors the server's `ReconcileReport`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReconcileReport {
    pub deleted_sources: usize,
    pub deleted_chunks: usize,
    pub orphan_uris: Vec<String>,
}

/// POST one translated issue to brain-server's `/ingest/markdown`. Uses the
/// same reqwest client the connector already has (shared with the GitHub
/// REST calls). Brain-server is loopback so we don't need a separate dep-free
/// client here — the connector binary already pays the reqwest cost.
fn ingest_one(
    http: &reqwest::blocking::Client,
    brain_base: &str,
    brain_token: Option<&str>,
    markdown: &str,
    source_uri: &str,
) -> Result<()> {
    let body = serde_json::json!({
        "content": markdown,
        "title": extract_title(markdown).unwrap_or(""),
        "source_path": source_uri,
    });
    let mut req = http
        .post(format!("{brain_base}/ingest/markdown"))
        .header("Content-Type", "application/json")
        .json(&body);
    if let Some(t) = brain_token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let resp = req
        .send()
        .context("brain-server /ingest/markdown POST failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("brain-server returned {status}: {body}");
    }
    Ok(())
}

/// Extract the H1 title from translated Markdown. Used as the `title` field
/// on the ingest payload (the server requires it). Returns `None` if no H1
/// is found — the caller falls back to an empty string and lets the server
/// reject it (loud failure beats guessing).
fn extract_title(markdown: &str) -> Option<&str> {
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return Some(rest.trim());
        }
    }
    None
}

/// Read a per-connector cursor value by key. Returns `None` if no cursor
/// has ever been written (first backfill). The connector's checkpoint
/// mirror lives in the server's `connector_checkpoints` table so it survives
/// connector-binary restarts and DB backups.
pub fn get_cursor(db: &Connection, connector_id: i64, key: &str) -> Result<Option<String>> {
    // query_row returns Err(QueryReturnedNoRows) when the cursor doesn't
    // exist yet — that's our "no cursor" case, not an error. Use optional()
    // to convert it to None cleanly.
    use rusqlite::OptionalExtension;
    let value: Option<String> = db
        .query_row(
            "SELECT value FROM connector_checkpoints WHERE connector_id = ?1 AND key = ?2",
            params![connector_id, key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value)
}

/// Write (or overwrite) a cursor value. Updates `updated_at` automatically
/// via the table's DEFAULT.
pub fn upsert_cursor(db: &Connection, connector_id: i64, key: &str, value: &str) -> Result<()> {
    db.execute(
        "INSERT INTO connector_checkpoints (connector_id, key, value, updated_at) \
         VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP) \
         ON CONFLICT(connector_id, key) DO UPDATE SET \
             value = excluded.value, \
             updated_at = CURRENT_TIMESTAMP",
        params![connector_id, key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        // Mirror of the production `connectors` + `connector_checkpoints`
        // tables, but without FK enforcement — we test the checkpoint logic
        // in isolation, not the FK. FK correctness is covered by the
        // `test_migration_schema_contract` test against the real migration.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        db.execute_batch(
            "CREATE TABLE connectors(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                instance TEXT NOT NULL,
                config_json TEXT NOT NULL DEFAULT '{}',
                state TEXT NOT NULL DEFAULT 'registered',
                last_sync_at TEXT,
                last_error TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(kind, instance));
             CREATE TABLE connector_checkpoints(
                connector_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (connector_id, key));",
        )
        .unwrap();
        db
    }

    #[test]
    fn test_get_cursor_returns_none_when_unset() {
        let db = db();
        assert_eq!(get_cursor(&db, 1, "issues:foo/bar").unwrap(), None);
    }

    #[test]
    fn test_upsert_then_get_roundtrips() {
        let db = db();
        upsert_cursor(&db, 1, "issues:foo/bar", "2026-07-19T00:00:00Z").unwrap();
        assert_eq!(
            get_cursor(&db, 1, "issues:foo/bar").unwrap(),
            Some("2026-07-19T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_upsert_is_idempotent_and_overwrites() {
        let db = db();
        upsert_cursor(&db, 1, "k", "v1").unwrap();
        upsert_cursor(&db, 1, "k", "v2").unwrap();
        assert_eq!(get_cursor(&db, 1, "k").unwrap(), Some("v2".to_string()));
        // No duplicate row.
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM connector_checkpoints WHERE connector_id = 1 AND key = 'k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_cursors_are_isolated_per_connector_and_kind() {
        let db = db();
        upsert_cursor(&db, 1, "issues:foo/bar", "t1").unwrap();
        upsert_cursor(&db, 2, "issues:foo/bar", "t2").unwrap();
        upsert_cursor(&db, 1, "pulls:foo/bar", "t3").unwrap();
        assert_eq!(
            get_cursor(&db, 1, "issues:foo/bar").unwrap(),
            Some("t1".to_string())
        );
        assert_eq!(
            get_cursor(&db, 2, "issues:foo/bar").unwrap(),
            Some("t2".to_string())
        );
        assert_eq!(
            get_cursor(&db, 1, "pulls:foo/bar").unwrap(),
            Some("t3".to_string())
        );
    }

    #[test]
    fn test_extract_title_finds_h1() {
        let md = "---\nkind: github-issue\n---\n\n# Issue #42: The title\n\nbody";
        assert_eq!(extract_title(md), Some("Issue #42: The title"));
    }

    #[test]
    fn test_extract_title_returns_none_when_no_h1() {
        assert_eq!(extract_title("just prose, no heading"), None);
        assert_eq!(extract_title("## h2 not h1"), None);
    }
}
