//! A3 singularity pins: exactly ONE of each security-critical singleton.
//!
//! Why pins, not reviews: the generic [`crate::dup_guard`] gate forbids
//! *unlisted* top-level duplicates, but these three singletons need a
//! stronger, self-documenting guarantee — a named test that fails the moment
//! a second definition appears, with the security rationale attached so a
//! future author understands why the second copy must not exist:
//!
//! 1. `PUBLIC_PATHS` — the one public-path allowlist. A second list is a
//!    forked auth decision: one middleware could admit what the other
//!    denies (or vice versa), and drift is silent.
//! 2. `fn authorize` — the single AuthZ decision. Layered additions
//!    (`authorize_role`, `cap_gate`, `can_read_domain`,
//!    `review_flags_allowed`) refine the verdict; none replaces it. A
//!    second `authorize` is a forked verdict.
//! 3. The read-seam site table (`stored_text_fields_pass_the_read_seam`)
//!    — the one machine-checked list of stored-text emission sites. A
//!    second table is forked coverage: sites added to one list but not
//!    the other rot into unsanitized reads.
//!
//! Method: source-scan over `CARGO_MANIFEST_DIR` (`src/` + `tests/`), in
//! the `dup_guard` idiom. New files only — nothing under `src/` changes.

use std::path::{Path, PathBuf};

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Count definition sites of `needle` as a top-level item: a line whose
/// first non-whitespace tokens are `pub const PUBLIC_PATHS`-shaped, i.e.
/// optional `pub`, then `const`/`fn`, then the exact name. The exact-name
/// match matters: `authorize_role` must NOT count as `authorize`.
fn definition_sites(root: &Path, kind: &str, name: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    let mut sites = Vec::new();
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (i, line) in text.lines().enumerate() {
            // Top-level only: indented lines are impl methods / nested items.
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            let mut rest = line.trim_start();
            if let Some(after_pub) = rest.strip_prefix("pub ") {
                rest = after_pub.trim_start();
                // Skip visibility qualifiers: `pub(crate)`, `pub(super)`, ….
                if let Some(after_paren) = rest.strip_prefix('(')
                    && let Some(close) = after_paren.find(')')
                {
                    rest = after_paren[close + 1..].trim_start();
                }
            }
            let after_kind = match kind {
                "const" => rest.strip_prefix("const "),
                "fn" => rest.strip_prefix("fn "),
                _ => None,
            };
            if let Some(after_kind) = after_kind {
                let ident: String = after_kind
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident == name {
                    let display = f
                        .strip_prefix(root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .to_string();
                    sites.push(format!("{display}:{}", i + 1));
                }
            }
        }
    }
    sites.sort();
    sites
}

/// Count `fn {name}(` at ANY indent (trimmed-line match). Used for the
/// read-seam site table, which lives nested inside `mod tests` — the
/// singularity is "one table anywhere", not "one top-level item".
fn fn_sites_any_indent(root: &Path, name: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    let mut sites = Vec::new();
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            // Skip comments — prose about the table is not a table.
            if t.starts_with("//") {
                continue;
            }
            let after_fn = t
                .strip_prefix("fn ")
                .or_else(|| t.strip_prefix("async fn "));
            if let Some(after_fn) = after_fn {
                let ident: String = after_fn
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident == name {
                    let display = f
                        .strip_prefix(root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .to_string();
                    sites.push(format!("{display}:{}", i + 1));
                }
            }
        }
    }
    sites.sort();
    sites
}

fn tree_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 1. Exactly one `PUBLIC_PATHS` definition, living beside the tables it
///
/// feeds (`src/server/router/route_guards.rs`). Both auth middlewares must
/// consume it through `is_public_path` — never a second inline list.
#[test]
fn single_public_paths_definition() {
    let root = tree_root();
    let sites = definition_sites(&root.join("src"), "const", "PUBLIC_PATHS");
    assert_eq!(
        sites.len(),
        1,
        "a second PUBLIC_PATHS allowlist is a forked auth decision — \
         extend the one list, never copy it. Sites: {sites:?}"
    );
    assert!(
        sites[0].starts_with("server/router/route_guards.rs:"),
        "PUBLIC_PATHS moved out of route_guards.rs — update this pin \
         deliberately: {sites:?}"
    );
}

/// 2. Exactly one `fn authorize` AuthZ decision (`src/handlers/mod.rs`).
///
/// `authorize_role` / `cap_gate` / `can_read_domain` /
/// `review_flags_allowed` are layered refinements with distinct names —
/// they must never be renamed onto the decision itself.
#[test]
fn single_authorize_decision() {
    let root = tree_root();
    let sites = definition_sites(&root.join("src"), "fn", "authorize");
    assert_eq!(
        sites.len(),
        1,
        "a second `fn authorize` is a forked AuthZ verdict — layer a \
         distinctly-named refinement instead. Sites: {sites:?}"
    );
    assert!(
        sites[0].starts_with("handlers/mod.rs:"),
        "the authorize decision moved out of handlers/mod.rs — update \
         this pin deliberately: {sites:?}"
    );
}

/// 3. Exactly one read-seam site table
///
/// (`stored_text_fields_pass_the_read_seam`). Coverage for stored-text
/// emission sites must accumulate in the one table, never a forked copy.
#[test]
fn single_read_seam_site_table() {
    let root = tree_root();
    let mut sites = fn_sites_any_indent(&root.join("src"), "stored_text_fields_pass_the_read_seam");
    sites.extend(fn_sites_any_indent(
        &root.join("tests"),
        "stored_text_fields_pass_the_read_seam",
    ));
    sites.sort();
    assert_eq!(
        sites.len(),
        1,
        "a second read-seam site table is forked coverage — add the row \
         to the one table instead. Sites: {sites:?}"
    );
}

#[cfg(test)]
mod scanner_proofs {
    //! Red-proof: the scanner above really fires on a second definition.
    //! These fixtures never touch the real tree — if the detector missed a
    //! duplicate here, the three pins above would be theater.

    use super::*;

    fn fixture_tree(files: &[(&str, &str)]) -> tempfile_like::TempTree {
        tempfile_like::TempTree::new(files)
    }

    mod tempfile_like {
        //! Minimal temp-dir helper (std only — zero new dependencies).
        use std::path::{Path, PathBuf};

        /// Process-wide sequence so two trees built on parallel test threads
        /// never share a directory: nanos alone collide across threads (two
        /// `TempTree::new` calls in the same nanosecond overwrite each other's
        /// `a.rs`, and the scanner then reads a foreign fixture — a silent
        /// cross-test false pass/fail. The seq makes every tree unique even
        /// when pid + nanos + thread id all repeat.
        static TREE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        pub struct TempTree {
            dir: PathBuf,
        }

        impl TempTree {
            pub fn new(files: &[(&str, &str)]) -> Self {
                let seq = TREE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let dir = std::env::temp_dir().join(format!(
                    "singularity-pin-{}-{:?}-{}-{}",
                    std::process::id(),
                    std::thread::current().id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0),
                    seq
                ));
                for (rel, content) in files {
                    let p = dir.join(rel);
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(&p, content).unwrap();
                }
                Self { dir }
            }

            pub fn root(&self) -> &Path {
                &self.dir
            }
        }

        impl Drop for TempTree {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }

    #[test]
    fn detector_fires_on_a_second_public_paths() {
        let tree = fixture_tree(&[
            ("a.rs", "pub const PUBLIC_PATHS: &[&str] = &[];\n"),
            ("sub/b.rs", "pub const PUBLIC_PATHS: &[&str] = &[];\n"),
        ]);
        let sites = definition_sites(tree.root(), "const", "PUBLIC_PATHS");
        assert_eq!(
            sites.len(),
            2,
            "the detector must see both copies: {sites:?}"
        );
    }

    #[test]
    fn detector_ignores_similar_names_and_methods() {
        // NOTE: no `\`-at-EOL continuations below — a trailing `\` eats the
        // newline AND the next line's leading whitespace, which would silently
        // promote the indented method to column 0 and defeat the test. The
        // `\n    ` escapes keep real indentation in the fixture text.
        let tree = fixture_tree(&[(
            "a.rs",
            "pub const PUBLIC_PATHS: &[&str] = &[];\n\
             pub const PUBLIC_PATHS_EXTRA: &[&str] = &[];\n\
             impl Guard {\n    fn authorize(&self) {}\n}\npub fn authorize_role() {}\n",
        )]);
        // `PUBLIC_PATHS_EXTRA` is a different name — not a second list.
        assert_eq!(
            definition_sites(tree.root(), "const", "PUBLIC_PATHS").len(),
            1
        );
        // Indented methods and `authorize_role` are not the decision.
        assert!(definition_sites(tree.root(), "fn", "authorize").is_empty());
        assert_eq!(
            definition_sites(tree.root(), "fn", "authorize_role").len(),
            1
        );
    }

    #[test]
    fn detector_fires_on_a_second_nested_table() {
        let tree = fixture_tree(&[
            (
                "a.rs",
                "mod tests {\n    fn stored_text_fields_pass_the_read_seam() {}\n}\n",
            ),
            (
                "b.rs",
                "#[test]\nfn stored_text_fields_pass_the_read_seam() {}\n",
            ),
        ]);
        let sites = fn_sites_any_indent(tree.root(), "stored_text_fields_pass_the_read_seam");
        assert_eq!(
            sites.len(),
            2,
            "nested or top-level, a copy is a copy: {sites:?}"
        );
    }

    #[test]
    fn detector_fires_on_a_second_authorize() {
        let tree = fixture_tree(&[
            ("a.rs", "pub fn authorize() {}\n"),
            ("b.rs", "pub fn authorize() {}\n"),
        ]);
        let sites = definition_sites(tree.root(), "fn", "authorize");
        assert_eq!(
            sites.len(),
            2,
            "the detector must see both verdicts: {sites:?}"
        );
    }
}
