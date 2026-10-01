// R66 increment A — the carried human verdict, and the census that never reads it.
//
// ## Why this file exists separately from the module
//
// The claim below is an ABSENCE: that nothing in production scores, thresholds,
// or gates on the frozen human verdict. A module cannot assert its own absence —
// a module that could see its own wiring could argue with it — so these pins
// live outside it and read the tree.
//
// ## The read-as-text idiom, and why it is not the weak version
//
// `drift_census` is a `pub(crate)` module: an integration test cannot call it,
// so reading source is the only way in (the `r57_census_pins.rs` precedent).
//
// The vacuous-pass hazard for a source-reading pin is scanning a region that
// contains the scanner's own literals. Three properties close it here:
//
// 1. Every scan is cut to the **production region** — the source before the
//    file's single `#[cfg(test)]` boundary. The census's test fixtures carry
//    the identifier in JSON raw strings, so an unscoped scan would be reading
//    the fixtures rather than the claim.
// 2. The pins assert the **allowed forms exhaustively**, not the presence of a
//    string. A scan that merely checked "the identifier appears" would pass on
//    the field declaration alone and stay green through a comparison.
// 3. The reader-set pin walks the whole `src/` tree and asserts the identifier
//    occurs in **exactly one** file. A second reader anywhere — a service core,
//    a handler, the CLI — fails immediately, so the claim is about the tree and
//    not about one file's good behaviour.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} must be readable: {e}. A read-as-text pin that cannot read its subject passes \
             vacuously, which is worse than no pin.",
            path.display()
        )
    })
}

/// Everything before the file's first test module.
///
/// **Returns an owned `String`** so a caller can bind it without holding a
/// temporary across the assertion that uses it.
fn production_region(rel: &str) -> String {
    let src = read(rel);
    match src.split_once("#[cfg(test)]") {
        Some((head, _)) => head.to_string(),
        None => src,
    }
}

/// The lines `name` occurs on, as word-bounded tokens, one entry per line.
///
/// Bounded on both sides so `human_pass` does not match inside a longer
/// identifier — an unbounded `contains` here is the same vacuous-pass class one
/// level down.
///
/// One entry per **line**, not per token: the copy form names the field twice
/// (`field: case.field`), so a token count would report a site the reader-set
/// never sees. Counting tokens is the "a fixture emits a second signal" trap in
/// scanner form — the extra signal came from the scanner's own line.
fn identifier_sites(hay: &str, name: &str) -> Vec<(usize, String)> {
    let bytes = hay.as_bytes();
    let needle = name.as_bytes();
    let boundary = |b: u8| !(b.is_ascii_alphanumeric() || b == b'_');
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let before_ok = i == 0 || boundary(bytes[i - 1]);
            let after = i + needle.len();
            let after_ok = after >= bytes.len() || boundary(bytes[after]);
            if before_ok && after_ok {
                let line_no = hay[..i].bytes().filter(|b| *b == b'\n').count() + 1;
                let line = hay
                    .lines()
                    .nth(line_no - 1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !out.iter().any(|(n, _)| *n == line_no) {
                    out.push((line_no, line));
                }
                i = after;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The declared field names of `struct <name>`, read from its declaration.
///
/// Driven off the type's own field list rather than a hand-chosen string list,
/// so a field this pin has never heard of is still examined.
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    let open = format!("struct {name} {{");
    let start = src
        .find(&open)
        .unwrap_or_else(|| panic!("the census must declare `struct {name}`"));
    let body = &src[start + open.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("`struct {name}` must close at column 0"));
    body[..end]
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let rest = t
                .strip_prefix("pub(crate) ")
                .or_else(|| t.strip_prefix("pub "))?;
            let field = rest.split(':').next()?.trim();
            (!field.is_empty()).then(|| field.to_string())
        })
        .collect()
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

const CENSUS: &str = "src/workflow/drift_census.rs";

/// The three forms a carried verdict may legally take in production:
/// declared on the decode struct, declared on the corpus struct, and copied
/// between them by the decoder. Everything else is a read.
fn is_carry_form(line: &str) -> bool {
    matches!(
        line,
        "human_pass: bool," | "pub(crate) human_pass: bool," | "human_pass: case.human_pass,"
    )
}

/// Field names that would make the census *carry a human verdict as data*.
///
/// Deliberately not a match on `verdict`: the census's `CellResult::verdict` is
/// its own drift outcome, a different thing wearing a similar name.
fn is_human_verdict_field(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    f.contains("human") || f.contains("pass") || f.contains("agree")
}

#[test]
fn the_verdict_is_carried_and_never_read_in_production() {
    let production = production_region(CENSUS);
    let hits = identifier_sites(&production, "human_pass");
    assert!(
        !hits.is_empty(),
        "the corpus struct must still carry the verdict, or this pin is asserting nothing."
    );
    let mut offenders = Vec::new();
    for (line_no, line) in &hits {
        if !is_carry_form(line) {
            offenders.push(format!("{CENSUS}:{line_no}: {line}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "the census must carry the frozen human verdict without reading it. A production site \
         outside the declaration/copy forms is a comparison, threshold, or gate — and the comment \
         on the field would then be claiming a read that exists:\n{}",
        offenders.join("\n")
    );
    // Exhaustiveness: carrying is three forms today, and a fourth would be a
    // reader. Pinning the count stops a rename from silently emptying the scan.
    assert_eq!(
        hits.len(),
        3,
        "expected the verdict at exactly three carry sites in the production region, found {}: \
         {hits:?}",
        hits.len()
    );
}

#[test]
fn no_production_site_outside_the_census_reads_the_verdict() {
    let mut files = Vec::new();
    walk_rs(&repo_root().join("src"), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "src tree not found under {}",
        repo_root().display()
    );
    let mut readers = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .display()
            .to_string();
        if rel == CENSUS {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        if !identifier_sites(&src, "human_pass").is_empty() {
            readers.push(rel);
        }
    }
    assert!(
        readers.is_empty(),
        "the frozen human verdict must be read by the census module and nothing else. A reader \
         outside it is a gate, a threshold, or a report the census does not claim to produce: \
         {readers:?}"
    );
}

/// The census reports no agreement figure.
///
/// This is the measured answer, pinned so that *adding* the column becomes a
/// deliberate act that breaks this test rather than a silent drift — at which
/// point the field's doc comment has to be rewritten in the same commit.
#[test]
fn the_census_report_types_carry_no_verdict_column() {
    let src = production_region(CENSUS);
    for ty in ["Cell", "CellResult", "Census"] {
        let fields = struct_fields(&src, ty);
        assert!(
            !fields.is_empty(),
            "`struct {ty}` must declare at least one field, or the field-list scan is vacuous."
        );
        let leaked: Vec<&String> = fields
            .iter()
            .filter(|f| is_human_verdict_field(f))
            .collect();
        assert!(
            leaked.is_empty(),
            "`struct {ty}` must carry no human-verdict column. The census reports drift, not \
             scorer/verdict agreement; a column here would make an uncalibrated single-rater \
             measure look like a control. Offending fields: {leaked:?}"
        );
    }
}
