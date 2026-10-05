//! ANTI-REGRESSION: the engagement's name must not re-enter this tree.
//!
//! WHY THIS FILE EXISTS. The published repository is vendor-agnostic by design
//! (see `.gitignore`, which records that the engagement's name is "deliberately
//! NOT named here or in AGENTS.md" because the file is public). That rule held
//! in the prose and FAILED in the code: the classifier carried a `dell_support`
//! category whose lexicon held the vendor name and its product lines, so the
//! name would have shipped with the next public sync while every document
//! claimed it was absent.
//!
//! A rule that lives only in a comment is not a control. This is the control.
//!
//! WHAT IT PINS. The engagement name may appear in exactly TWO places:
//! `docs/product-site/about.md`, which is the operator's own professional bio on
//! the product site — a deliberate, human-owned decision, not a code artefact —
//! and THIS file, which must hold the vocabulary in order to police it.
//!
//! WHY THIS FILE IS EXEMPT. The pin scans git-TRACKED files, and this file is
//! tracked. It therefore always flagged ITSELF, on the two `NAMES` literals,
//! which made the control permanently red and — worse — trained everyone to
//! read it as pre-existing noise rather than as a failure. An exempt list is
//! the honest fix, and it is named here for the same reason the bio is: so the
//! exception cannot go stale silently. If this file ever stops needing the
//! literal, the exemption must be removed deliberately, not left behind.
//!
//! HOW IT SCANS. Only git-TRACKED files, so `target/`, `node_modules/` and
//! local scratch are excluded by construction rather than by an ignore list that
//! could drift.

use std::path::Path;
use std::process::Command;

/// The files permitted to name the engagement: the operator's bio, and this
/// pin itself (see the module header — the vocabulary must live somewhere, and
/// scanning this file without exempting it makes the control red forever).
const ALLOWED_FILES: [&str; 2] = ["docs/product-site/about.md", "tests/no_engagement_name.rs"];

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files must run");
    assert!(
        out.status.success(),
        "git ls-files failed — the pin cannot scan, so it must not report green"
    );
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_engagement_name_does_not_appear_in_tracked_files() {
    // Case-insensitive, word-bounded: `embellish` must not trip it, and neither
    // must a legitimate `EMC`-shaped token elsewhere.
    const NAMES: [&str; 2] = ["dell", "emc"];

    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for rel in tracked_files() {
        if ALLOWED_FILES.contains(&rel.as_str()) {
            continue;
        }
        let path = Path::new(&rel);
        // Binary artefacts are not prose and cannot leak a readable name; skip
        // them rather than printing mojibake into the failure message.
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        scanned += 1;

        let lowered = text.to_lowercase();
        for (line_no, line) in lowered.lines().enumerate() {
            for name in NAMES {
                // Word-bounded on both sides.
                let mut from = 0usize;
                while let Some(offset) = line[from..].find(name) {
                    let start = from + offset;
                    let end = start + name.len();
                    let before_ok =
                        start == 0 || !line.as_bytes()[start - 1].is_ascii_alphanumeric();
                    let after_ok =
                        end >= line.len() || !line.as_bytes()[end].is_ascii_alphanumeric();
                    if before_ok && after_ok {
                        offenders.push(format!("{rel}:{}: {name}", line_no + 1));
                        break;
                    }
                    from = end;
                }
            }
        }
    }

    assert!(
        scanned > 200,
        "sanity: only {scanned} files scanned — a walk that reads almost nothing \
         would pass this pin without inspecting anything"
    );
    // ANTI-VACUITY for the exemption above. This file is exempt so it can hold
    // the vocabulary; if the vocabulary were ever moved out (or the literals
    // renamed), the exemption would silently become a blanket pass over a file
    // that no longer needs it — so assert the pin is still genuinely scanning
    // the file it exempts, by confirming the literals it exempts itself for are
    // actually present in its own source.
    let own = std::fs::read_to_string("tests/no_engagement_name.rs")
        .expect("this pin must be able to read its own source");
    assert!(
        NAMES.iter().all(|n| own.contains(n)),
        "this file is exempt from the scan so it can hold the vocabulary — but the \
         vocabulary is gone ({NAMES:?} absent from this source). The exemption has \
         become a blanket pass over a file that no longer needs it; remove it from \
         ALLOWED_FILES deliberately."
    );
    assert!(
        offenders.is_empty(),
        "the engagement's name is back in {} tracked file(s), and this repository \
         is published. Either the code re-introduced it, or one of {:?} must be \
         updated deliberately (the pin names them so the exceptions cannot go stale \
         silently):\n  {}",
        offenders.len(),
        ALLOWED_FILES,
        offenders.join("\n  ")
    );
}

#[test]
fn the_subject_matter_vocabulary_is_not_hard_coded() {
    /// The companion property: the pool must be EMPTY in the tree, so there is
    /// no second copy of the vocabulary for the name pin to have to catch. A
    /// deployment binds it with `BRAIN_SUBJECT_MATTER_TERMS`.
    const FORBIDDEN_IN_SOURCE: [&str; 4] = ["poweredge", "powerstore", "powerscale", "cachevault"];

    let src = std::fs::read_to_string("src/procedural.rs").expect("procedural.rs must be readable");
    // The literal pool is `&[]`; assert the deployment read is present so a
    // future refactor cannot quietly revert to a hard-coded list.
    assert!(
        src.contains("BRAIN_SUBJECT_MATTER_TERMS"),
        "the subject-matter vocabulary must stay deployment-supplied"
    );
    for term in FORBIDDEN_IN_SOURCE {
        assert!(
            !src.to_lowercase().contains(term),
            "{term:?} is a product line from the engagement's corpus and must not be \
             hard-coded in the published tree; bind it with BRAIN_SUBJECT_MATTER_TERMS"
        );
    }
}
