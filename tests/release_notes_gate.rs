//! The release-notes gate, run LOCALLY.
//!
//! The checks that decide whether a release can publish — section present, a
//! `### Release notes` block, a prose intro, only categorized bullets, at
//! least one bullet, no audit/finding IDs — lived exclusively inside
//! `.github/workflows/release.yml`. A gate that exists only in the publisher
//! can only fail in the publisher, which for this repo means a full public
//! matrix plus four cross-builds before anyone learns what a `grep` could
//! have said in a second. v1.29.13 was refused at that step twice, for two
//! different reasons:
//!
//!   1. the category headings were `### Security fixes` / `### Improvements`
//!      (h3), and the extractor ends the notes block at the first `### ` —
//!      so the published body would have carried the intro and ZERO bullets;
//!   2. the fork-lane IDs `K4-05` / `K4-12` appeared in the intro prose and
//!      in a bullet.
//!
//! So the checks now live in `scripts/release-notes-check.sh`, which BOTH
//! `release.yml` and this test run. One implementation, two callers: a check
//! that exists in only one of them fails only when it is most expensive.
//!
//! Three arms, because a gate that cannot fail is worse than no gate:
//!   - the gate passes for the version being cut (the real assertion);
//!   - it FAILS for both historical defects, driven against temp copies of
//!     the CHANGELOG so the committed file is never touched;
//!   - `release.yml` still invokes the script, read from the workflow
//!     itself, so deleting the call cannot quietly return this to a
//!     CI-only check.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Demote EVERY bold category heading to an h3, so the first one already
/// terminates the notes block no matter which category the section leads with.
///
/// This used to rewrite `**Security fixes**` alone, which silently depended on
/// that being the section's FIRST category: when the notes began with another
/// category, the remaining bold heading kept the block alive, the gate
/// correctly accepted it, and the test failed while testing nothing. A guard
/// that disarms itself when the changelog's heading order changes is worse
/// than no guard, because it still looks like one.
const CATEGORY_NAMES: [&str; 5] = [
    "Improvements",
    "New",
    "Changed",
    "Bug fixes",
    "Security fixes",
];

fn demote_every_category_heading(section: &str) -> String {
    let mut out = section.to_string();
    for name in CATEGORY_NAMES {
        out = out.replace(&format!("**{name}**"), &format!("### {name}"));
    }
    out
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn gate_script() -> PathBuf {
    repo_root().join("scripts/release-notes-check.sh")
}

fn run_gate(root: &Path, version: &str) -> std::process::Output {
    Command::new("bash")
        .arg(root.join("scripts/release-notes-check.sh"))
        .arg(version)
        .current_dir(root)
        .output()
        .expect("spawn the release-notes gate")
}

/// A scratch tree holding a copy of the script + a CHANGELOG variant, so a
/// red-proof can mutate the notes without touching the repository's.
fn scratch(tag: &str, changelog: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-relnotes-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("scripts")).expect("mkdir scripts");
    std::fs::copy(gate_script(), dir.join("scripts/release-notes-check.sh"))
        .expect("copy the gate script");
    std::fs::write(dir.join("CHANGELOG.md"), changelog).expect("write CHANGELOG");
    // Cargo.toml is only consulted when no version argument is passed; this
    // gate always passes one, but the script reads it for its default, so
    // give it a stub rather than letting the read fail confusingly.
    std::fs::write(dir.join("Cargo.toml"), "version = \"9.9.9\"\n").expect("write Cargo.toml");
    dir
}

/// The real assertion: the version being cut publishes.
#[test]
fn the_release_being_cut_has_publishable_notes() {
    let root = repo_root();
    let version = cargo_version(&root);
    let out = run_gate(&root, &version);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the release notes for {version} would be REFUSED by the publisher \
         (v{version} could not publish):\n{stdout}{stderr}\n\
         Fix CHANGELOG.md — the gate is scripts/release-notes-check.sh and \
         runs in under a second; this test exists so the finding costs \
         seconds instead of a 45-minute release cycle."
    );
    assert!(
        stdout.contains("categorized bullet(s)"),
        "the gate must report what it checked, or a silent early-exit reads \
         as a pass: {stdout}"
    );
}

/// The two historical defects, each proven to fire.
///
/// Driven against scratch copies, never the committed CHANGELOG: the point
/// is to show the gate REFUSES these shapes, and doing that to the real file
/// would be the defect all over again.
#[test]
fn the_gate_refuses_an_empty_notes_block_and_audit_ids() {
    let root = repo_root();
    let version = cargo_version(&root);
    let real = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    let section = section_body(&real, &version).expect("the version's section exists");
    // Both plants below rebuild a scratch section around the NOTES BLOCK
    // alone — the text from `### Release notes` to the end of the section.
    // The real section may carry engineering-record h3 headings BEFORE the
    // notes block (this release does), and the gate's extractor ends a notes
    // block at the first h3 it meets. A plant that re-embedded the whole
    // section after a fresh `### Release notes` line handed the gate that
    // earlier h3 first, and the gate refused for "no bullets" — the right
    // verdict for the wrong reason, and an arm testing the section's shape
    // instead of the rule it exists for.
    let block = notes_block(&section).expect("the section carries a `### Release notes` block");

    // Defect 1 — the category headings are h3, so the extractor stops at the
    // first one and the block publishes nothing.
    //
    // EVERY category heading is demoted, not just one. This used to rewrite
    // `**Security fixes**` alone, which silently depended on that being the
    // section's FIRST category: when the notes began with another category,
    // the remaining bold heading kept the block alive, the gate correctly
    // accepted it, and the test failed while testing nothing. A guard that
    // disarms itself when the changelog's heading order changes is worse than
    // no guard, because it still looks like one.
    let as_h3 = demote_every_category_heading(block);
    assert_ne!(
        as_h3, block,
        "the section no longer uses a bold category heading"
    );
    // The assertion is about CATEGORY headings, not about "**" anywhere — the
    // notes legitimately carry inline bold emphasis inside bullets, and a
    // blanket check would fail on correct notes.
    let surviving = CATEGORY_NAMES
        .iter()
        .find(|n| as_h3.contains(&format!("**{n}**")));
    assert!(
        surviving.is_none(),
        "the h3 plant left {} as a bold category heading, so the block would \
         still publish — the plant no longer builds the shape it claims to",
        surviving.unwrap_or(&"")
    );
    let dir = scratch(
        "h3",
        &format!(
            "# Changelog\n\n## [{version}] — test\n\nIntro prose line.\n\n{as_h3}\n\n- a bullet\n"
        ),
    );
    let out = run_gate(&dir, &version);
    assert!(
        !out.status.success(),
        "a notes block whose category heading is `### X` must be refused — \
         that is the shape that would have published an EMPTY body"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("produced no bullets") || stderr.contains("uncategorized"),
        "the refusal must name the empty-block cause, not fail obscurely: {stderr}"
    );

    // Defect 2 — an audit/finding ID in the published surface.
    //
    // The anchor is whichever bold category heading this release actually
    // carries, not a hardcoded `**Security fixes**`. It was hardcoded, and a
    // docs-and-gates release with no security fixes therefore had nothing for
    // `replace` to match: the plant became a no-op and the arm refused with
    // "could not plant the ID probe" — a gate that can only be satisfied by
    // releases of a particular shape. Same class as the defect this test
    // already documents twice above: an arm that depends on the content it is
    // inspecting rather than on the rule it is testing.
    let anchor = CATEGORY_NAMES
        .iter()
        .map(|n| format!("**{n}**"))
        .find(|h| block.contains(h))
        .expect(
            "the section carries no bold category heading at all — the h3 plant \
             above already asserts this, so the two disagree",
        );
    let with_id = block.replace(
        &anchor,
        &format!("{anchor}\n- the fork lane (K4-05) refuses upstream sources"),
    );
    assert_ne!(with_id, block, "could not plant the ID probe");
    let dir = scratch(
        "ids",
        &format!("# Changelog\n\n## [{version}] — test\n\nIntro prose line.\n\n{with_id}\n"),
    );
    let out = run_gate(&dir, &version);
    assert!(
        !out.status.success(),
        "an audit ID in the published notes must be refused"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("audit/finding IDs"),
        "the refusal must name the ID rule: {stderr}"
    );
}

/// The gate is only a real gate if BOTH callers still call it. The workflow
/// side is read from the workflow file, so removing the call fails here
/// rather than silently returning this to a CI-only check.
#[test]
fn the_publisher_still_runs_the_same_gate() {
    let workflow = std::fs::read_to_string(repo_root().join(".github/workflows/release.yml"))
        .expect("read release.yml");
    assert!(
        workflow.contains("scripts/release-notes-check.sh"),
        "release.yml must invoke scripts/release-notes-check.sh — otherwise \
         the local gate and the publisher are two implementations, and the \
         only one that can fail on a bad notes block is the expensive one"
    );

    // And the script must be executable-by-name, not merely present.
    let out = run_gate(&repo_root(), &cargo_version(&repo_root()));
    assert!(
        out.status.success(),
        "the gate must run from a test as well as from the workflow"
    );
}

fn cargo_version(root: &Path) -> String {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|l| l.split('"').next())
        .expect("a version in Cargo.toml")
        .to_string()
}

/// The text of one version's section, heading through to the next `## [`.
fn section_body(changelog: &str, version: &str) -> Option<String> {
    let start = format!("## [{version}]");
    let mut out = String::new();
    let mut inside = false;
    for line in changelog.lines() {
        if line.starts_with("## [") {
            if inside {
                break;
            }
            inside = line.starts_with(&start);
            if inside {
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// The notes block of one section: from its `### Release notes` line to the
/// section's end. Everything before it — the intro prose and any
/// engineering-record h3 the section carries — is not the block.
fn notes_block(section: &str) -> Option<&str> {
    let at = section.find("### Release notes")?;
    Some(&section[at..])
}
