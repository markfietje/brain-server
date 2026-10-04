// The carried human verdict, and the ONE seam that reads it.
//
// ## Why this file exists separately from the module
//
// The claim below is about where the frozen verdict may be READ. A module
// cannot assert its own absence — a module that could see its own wiring could
// argue with it — so these pins live outside it and read the tree.
//
// ## What changed, and why the claim is stronger than it was
//
// This file used to assert an absence: the verdict is carried and never read
// in production. The agreement column made that false, and the design named the
// consequence in advance — adding the field turns a green pin into a weaker
// claim than it was, unless the pin's claim is revised with it.
//
// So the claim moved from "never read" to **"read at exactly one declared
// seam, and nowhere else"**. That is not a relaxation:
//
// 1. `the_census_reports_agreement_and_carries_no_verdict_column` replaces an
//    absence with a PRESENCE claim — the exact field list of `CellResult`, the
//    column's declared nullability, and a continued refusal of any raw verdict
//    column. A field this file has never heard of still fails it, because it
//    reads the type's field list rather than a list of names.
// 2. `the_verdict_is_read_only_at_the_one_declared_seam` no longer asks whether
//    each occurrence is a known-good string. It **cuts the seam out of the
//    production region** and requires every remaining occurrence to be a carry
//    form. A new reader anywhere else therefore fails by construction, and
//    cannot be absorbed by growing an allowlist.
// 3. `no_production_site_outside_the_census_reads_the_verdict` is unchanged and
//    verbatim: the whole-tree walk still holds.
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
// 2. The pins assert **allowed forms exhaustively** and pin the count, so a
//    rename cannot silently empty the scan and a new site cannot appear
//    unremarked.
// 3. The reader-set pin walks the whole `src/` tree. A second reader anywhere —
//    a service core, a handler, the CLI — fails immediately, so the claim is
//    about the tree and not about one file's good behaviour.

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

/// The declared seam: the census's one production reader of the verdict.
///
/// Located by its declaration and cut out **whole**, signature included, so the
/// text that remains is exactly what a second reader would have to hide in.
const SEAM: &str = "pub(crate) fn verdicts(";

/// The production region with the declared seam cut out of it.
///
/// Cutting rather than filtering is what keeps the claim honest: an allowlist
/// of "known-good forms" can be widened by whoever writes the next reader,
/// whereas a hole cut around exactly one function cannot.
fn production_outside_the_seam() -> String {
    let src = production_region(CENSUS);
    let start = src
        .find(SEAM)
        .unwrap_or_else(|| panic!("the census must declare `{SEAM}` — the one production reader"));
    let rest = &src[start..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("`verdicts` must close at column 0"));
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..start]);
    out.push_str(&rest[end..]);
    out
}

/// The three forms a carried verdict may legally take outside the seam:
/// declared on the decode struct, declared on the corpus struct, and copied
/// between them by the decoder. Everything else is a read.
fn is_carry_form(line: &str) -> bool {
    matches!(
        line,
        "human_pass: bool," | "pub(crate) human_pass: bool," | "human_pass: case.human_pass,"
    )
}

/// The verdict is READ, but only inside the declared seam — and only once.
///
/// The count is exhaustive in both directions: one occurrence inside the seam
/// (a second one is a second comparison), and exactly three outside it (a fourth
/// is a reader that has not declared itself).
#[test]
fn the_verdict_is_read_only_at_the_one_declared_seam() {
    let production = production_region(CENSUS);

    // Inside the seam: exactly one read, and it is a read.
    let seam_start = production.find(SEAM).expect("the declared seam must exist");
    let seam_end = production[seam_start..]
        .find("\n}")
        .map(|e| seam_start + e)
        .expect("the seam must close");
    let seam = &production[seam_start..seam_end];
    let in_seam = identifier_sites(seam, "human_pass");
    assert_eq!(
        in_seam.len(),
        1,
        "the declared seam must read the verdict exactly once, found {}: {in_seam:?}. Two reads \
         in one function means a second comparison the claim does not describe.",
        in_seam.len()
    );
    assert!(
        in_seam[0].1.contains(".human_pass"),
        "the seam must read the FIELD, not a local wearing its name: {}",
        in_seam[0].1
    );

    // Outside the seam: carries only.
    let outside = production_outside_the_seam();
    let hits = identifier_sites(&outside, "human_pass");
    assert!(
        !hits.is_empty(),
        "the corpus struct must still carry the verdict, or this pin is asserting nothing."
    );
    let mut offenders = Vec::new();
    for (_, line) in &hits {
        if !is_carry_form(line) {
            offenders.push(line.clone());
        }
    }
    assert!(
        offenders.is_empty(),
        "the verdict may be read at the declared seam and nowhere else. A production site outside \
         it is a comparison, threshold, or gate that the seam does not describe:\n{}",
        offenders.join("\n")
    );
    assert_eq!(
        hits.len(),
        3,
        "expected the verdict at exactly three carry sites outside the seam, found {}: {hits:?}",
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

/// The census reports agreement, and reports no verdict.
///
/// The former claim here was an absence — no agreement column at all — and it
/// was written so that *adding* one would break this pin rather than drift in
/// silently. That has happened, deliberately, so the claim becomes its
/// successor: the column exists, its shape is pinned exactly, and the thing it
/// must never become — a raw verdict carried onto a report type — is still
/// refused.
#[test]
fn the_census_reports_agreement_and_carries_no_verdict_column() {
    let src = production_region(CENSUS);

    // `CellResult` is pinned by its exact field list. A field this pin has never
    // heard of therefore still fails it.
    let results = struct_fields(&src, "CellResult");
    assert_eq!(
        results,
        vec![
            "id",
            "verdict",
            "observed_units",
            "delta_units",
            "agreement_units"
        ],
        "`CellResult`'s fields are {results:?}. The agreement column is the fifth; a sixth is \
         either a second vocabulary the reader has to learn or a per-cell band the tolerance law \
         refuses, and neither may arrive without this pin failing."
    );

    // The nullability is the control. `Option` is what lets an unmeasured cell
    // read as refused; a bare `i32` would have to invent a number for it, and
    // the smallest invented number is a zero that reads as agreement.
    assert!(
        src.contains("agreement_units: Option<i32>"),
        "`agreement_units` must stay `Option<i32>`. A defaulted value puts a number nobody measured \
         on a reviewer's screen, and it would read as the weakest possible agreement rather than \
         as an absence."
    );

    // And no report type carries the raw verdict. `Cell` and `Census` gain
    // nothing; `CellResult` reports the agreement, not the label behind it.
    for ty in ["Cell", "Census"] {
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
            "`struct {ty}` must carry no human-verdict column. The census reports agreement, not \
             the label it compared against, and a verdict column on a report type would invite a \
             reader to treat one case's label as a threshold. Offending fields: {leaked:?}"
        );
    }
}

/// Field names that would make a report type carry a human verdict as data.
///
/// Deliberately NOT a match on `agree`: `agreement_units` is the sanctioned
/// column and is checked on its own terms above. This predicate is the narrower
/// question of whether the *label* itself rode out.
fn is_human_verdict_field(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    f.contains("human") || f.contains("pass")
}
