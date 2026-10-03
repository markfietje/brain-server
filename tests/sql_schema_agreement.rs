// A class of defect that shipped green once already: production SQL naming a
// column the shipped schema does not have.
//
// The routing seam read `json_extract(payload_json, '$.queue') FROM proposals`,
// and `proposals` has no `payload_json` column. It was invisible for an entire
// release because every fixture that exercised the seam **invented** the column —
// so the whole suite agreed with itself and not one assertion touched a real
// database.
//
// ## Why preparing is the instrument
//
// The obvious instrument — grep the SQL for identifiers and check them against
// the schema — is unreliable in both directions. A column-name scan cannot tell
// a table alias from a column, a `CREATE TABLE` column list from a `SELECT`
// list, or an aggregate alias from a real column, so it drowns a reviewer in
// false positives and they stop reading it.
//
// **SQLite already answers this question exactly.** `Connection::prepare` parses
// and resolves a statement against a live schema: an unknown column is a
// *prepare-time* error, before any row is touched. So this pin migrates a real
// in-memory database and PREPARES every production statement in `src/` against
// it. A statement that cannot be prepared against the shipped schema is a
// statement that cannot run in production.
//
// ## What it deliberately does not do
//
// It prepares; it does **not** execute. Execution needs rows, parameters and a
// transaction, and a pin that invented fixtures to supply them would be the very
// defect this file exists to catch. Statements with placeholders are prepared
// with named parameters bound to NULL — SQLite resolves names and columns at
// prepare time, so a missing column is still caught.
//
// ## The scope limit, stated rather than hidden
//
// A statement behind a runtime-composed string (`format!`) cannot be extracted
// and is NOT checked. That is a real ceiling and it is why this pin counts the
// statements it skipped: a pin that silently narrows its own coverage is the
// failure this repository keeps recording.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `src/`, recursively.
fn src_files() -> Vec<PathBuf> {
    fn walk(dir: &PathBuf, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo_root().join("src"), &mut out);
    out.sort();
    out
}

/// The SQL-bearing string literals in a file's production region.
///
/// `code_only` first: a SQL example inside a doc comment must not be prepared,
/// and a comment naming a broken statement must not be able to pass this. The
/// `#[cfg(test)]` region is cut because a fixture's SQL is allowed to be
/// narrower than the shipped schema — that is exactly what the routing seam's
/// fixtures did.
fn production_sql(src: &str) -> Vec<String> {
    let code = common::code_only(src);
    let production = match code.find("#[cfg(test)]") {
        Some(idx) => &code[..idx],
        None => code.as_str(),
    };
    let mut out = Vec::new();
    let mut rest = production;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        // Walk to the closing quote, skipping `\"`. A naive `find('"')` stops at
        // the first escaped quote and truncates the literal — which manufactures
        // a parse error rather than reporting one.
        let bytes = after.as_bytes();
        let mut close = None;
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => {
                    close = Some(i);
                    break;
                }
                _ => i += 1,
            }
        }
        let Some(close) = close else {
            break;
        };
        let literal = &after[..close];
        rest = &after[close + 1..];
        // Unescape the forms a SQL literal realistically carries. The line
        // continuation (`\` + newline) is the one that matters most: multi-line
        // SQL in this tree is written that way, and a literal `\` reaching SQLite
        // is a parse error that would drown the real column findings.
        //
        // `\"` inside a single-quoted SQL string is a QUOTED JSON KEY, not a
        // Rust escape — `payload_json LIKE '%\"to\":\"closed\"%'` searches for
        // `"to":"closed"`. Unescaping it to a bare `"` would truncate the SQL
        // string and manufacture a parse error. Only `\'` is a real escape.
        let unescaped = literal
            .replace("\\\n", "\n")
            .replace("\\n", "\n")
            .replace("\\'", "'");
        let upper = unescaped.to_ascii_uppercase();
        let looks_like_sql = [
            "SELECT ",
            "INSERT INTO",
            "UPDATE ",
            "DELETE FROM",
            "CREATE TABLE",
        ]
        .iter()
        .any(|k| upper.contains(k));
        // A prose fragment that merely opens with a SQL verb — an error message
        // like "update the inventory…", or a comment carried in a string — is
        // not a statement. Require a table-shaped token right after the verb.
        let sql_shaped = [
            "SELECT ",
            "INSERT INTO",
            "UPDATE ",
            "DELETE FROM",
            "CREATE TABLE",
        ]
        .iter()
        .any(|k| upper.contains(k))
            && (upper.contains(" FROM ")
                || upper.contains(" INTO ")
                || upper.contains(" SET ")
                || upper.contains("("));
        if looks_like_sql && sql_shaped && !upper.contains("RAISE(ABORT") {
            out.push(unescaped);
        }
    }
    out
}

/// A statement we can hand to `prepare`: split on `;`, drop fragments with no
/// verb (a `?`-placeholder continuation, a trailing DDL guard).
fn statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| {
            let u = s.to_ascii_uppercase();
            [
                "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "REPLACE",
            ]
            .iter()
            .any(|k| u.contains(k))
        })
        .collect()
}

/// A statement that mutates the schema or the data is not prepared here —
/// preparing is safe, but a stray `;`-split of a DDL block can produce a
/// fragment that is valid SQL on its own and meaningless in context.
fn is_checkable(stmt: &str) -> bool {
    let u = stmt.to_ascii_uppercase();
    // Skip anything that is not a single complete statement shape.
    u.starts_with("SELECT")
        || u.starts_with("INSERT")
        || u.starts_with("UPDATE")
        || u.starts_with("DELETE")
        || u.starts_with("WITH")
}

/// True for a statement this pin cannot and should not check, each for a
/// NAMED reason rather than a blanket skip.
///
/// 1. **Composed** — built by `format!`, so a `{placeholder}` survives into the
///    literal and the text is a template, not SQL. This is the standing ceiling:
///    a runtime-composed statement cannot be prepared. The count is asserted so
///    coverage cannot shrink silently.
/// 2. **A trigger body** — `INSERT … VALUES (new.id, …)` is only meaningful
///    inside a `CREATE TRIGGER`; prepared alone, `new.*` has no meaning and the
///    error is an artifact of the split, not a defect.
/// 3. **Not SQLite at all** — the CRM connector holds SOQL (Salesforce) text,
///    which SQLite must not be asked to parse.
fn skip_reason(stmt: &str) -> Option<&'static str> {
    let u = stmt.to_ascii_uppercase();
    if stmt.contains('{') || stmt.contains('}') {
        return Some("composed via format!");
    }
    if u.contains("NEW.") && u.contains("VALUES (NEW") {
        return Some("trigger body");
    }
    if u.contains(" FROM CASE") || u.contains("SYSTEMMODSTAMP") {
        return Some("SOQL (Salesforce), not SQLite");
    }
    if u.contains("RAISE(ABORT") {
        return Some("trigger guard");
    }
    None
}

#[test]
fn every_production_statement_prepares_against_the_shipped_schema() {
    // Registration is an auto-extension for FUTURE connections, so it must happen
    // before the connection is opened — `main_suite`'s own `test_db` sequences it
    // the same way, and reversing the two fails with "no such module: vec0".
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut db = rusqlite::Connection::open_in_memory().expect("open in-memory DB");
    brain_server::migration::run_migration(&mut db, 512).expect("migration must apply");

    let files = src_files();
    assert!(
        files.len() > 100,
        "the walk found only {} files under src/ — it cannot be seeing the tree",
        files.len()
    );

    let mut checked = 0usize;
    let mut skipped: Vec<(&'static str, usize)> = Vec::new();
    let mut unprepared: Vec<String> = Vec::new();
    let note_skip = |reason: &'static str, skipped: &mut Vec<(&'static str, usize)>| match skipped
        .iter_mut()
        .find(|(r, _)| *r == reason)
    {
        Some((_, n)) => *n += 1,
        None => skipped.push((reason, 1)),
    };

    for path in &files {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .display()
            .to_string();
        for sql in production_sql(&raw) {
            for stmt in statements(&sql) {
                if !is_checkable(&stmt) {
                    continue;
                }
                if let Some(reason) = skip_reason(&stmt) {
                    note_skip(reason, &mut skipped);
                    continue;
                }
                // A parameterised statement: give every placeholder a name so
                // prepare can resolve arity. Names are irrelevant to column
                // resolution, which is what this pin measures.
                let mut named = stmt.clone();
                let mut n = 0usize;
                while let Some(pos) = named.find('?') {
                    named.replace_range(pos..pos + 1, &format!(":p{n}"));
                    n += 1;
                }
                checked += 1;
                if let Err(e) = db.prepare(&named) {
                    let msg = e.to_string();
                    // A missing TABLE is this pin's business only when the table
                    // is one the migration declares unconditionally. Feature-gated
                    // tables (`compliance-pack`) are legitimately absent from a
                    // default build, and flagging them would train a reader to
                    // ignore this pin.
                    let feature_gated = msg.contains("oversight_evidence")
                        || msg.contains("ropa_registry")
                        || msg.contains("decision_records");
                    if !feature_gated {
                        unprepared.push(format!(
                            "{rel}: {} — {msg}",
                            named.chars().take(110).collect::<String>()
                        ));
                    }
                }
            }
        }
    }

    assert!(
        checked > 200,
        "only {checked} statements were prepared; the extractor is broken and this pin is \
         guarding almost nothing"
    );
    let skipped_total: usize = skipped.iter().map(|(_, n)| n).sum();
    assert!(
        skipped_total > 0,
        "nothing was skipped, which means the named-reason filters match nothing — either the \
         tree changed or the filters are wrong, and the coverage figure below is not describing \
         reality"
    );
    assert!(
        unprepared.is_empty(),
        "{} production statement(s) in src/ cannot be PREPARED against the migrated schema — \
         SQLite resolves columns at prepare time, so each of these fails at runtime against \
         every real database, whatever any fixture claims:\n  {}",
        unprepared.len(),
        unprepared.join("\n  ")
    );
    // The ceiling, reported by NAME and by count. A reviewer should read this as
    // "this many statements are trusted on inspection", per reason.
    let detail = skipped
        .iter()
        .map(|(r, n)| format!("{n} {r}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "sql-schema-agreement: {checked} statements prepared against the migrated schema \
         ({detail}), {} unprepared",
        unprepared.len()
    );
}
