//! The release WITNESS's verdict — the signal the operator reads to learn
//! whether the public matrix for a tag is green.
//!
//! ## WHY THIS EXISTS
//!
//! `scripts/release.sh` watches the public runs after it pushes the tag and
//! then tells the operator what happened. That watcher had a hole, and it was
//! not a rare race: **an EMPTY run list read as "all green"**.
//!
//! The old loop decided the verdict with an inner `while` that could only
//! clear its "done" flag for a run it SAW:
//!
//! ```text
//! DONE=1
//! while IFS=$'\t' read -r ST CO; do
//!   [[ "$ST" != "completed" ]] && DONE=0
//!   [[ -n "$CO" && "$CO" != "success" ]] && VERDICT="$VERDICT $CO"
//! done < <(gh run list ... 2>/dev/null)
//! [[ "$DONE" == 1 ]] && break
//! ```
//!
//! So when `gh run list` returned nothing — a transient API failure, a rate
//! limit, a query that matched no rows — the loop body never ran, `DONE`
//! stayed 1, and the script printed "all public runs are green".
//!
//! **It did exactly that on the v1.29.26 release**: the witness reported
//! green while BOTH public runs were still `in_progress`, about twenty
//! minutes before the matrix landed. Nothing was mis-published — the
//! ENFORCEMENT is `release.yml`'s own fail-closed check on the ci.yml
//! verdict, not the helper's — but this helper is the operator's WITNESS, and
//! a witness that cries wolf on success teaches you to ignore the one signal
//! the release line has. The `release.sh` header already records that lesson
//! for the loop BESIDE this one (it queried the tag OBJECT rather than the
//! COMMIT and waited its full ten minutes on releases that had published);
//! this is the same class of defect, in the other loop.
//!
//! ## THE LAW THIS FILE HOLDS
//!
//! **No runs observed is UNKNOWN, and UNKNOWN is never GREEN.**
//!
//! The verdict now lives in ONE implementation — `scripts/release-watch.sh` —
//! with two callers: `scripts/release.sh` (watching a real release) and this
//! test (driving it against fixtures). A check that exists in only one of
//! them fails only in the more expensive one, and the expensive one is the
//! moment a release is being cut.
//!
//! The arms below drive the witness with a FAKE `gh` on `PATH`, so the law is
//! proved by RUNNING it, not by reading the script.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn witness_script() -> PathBuf {
    repo_root().join("scripts/release-watch.sh")
}

/// A fake `gh` that answers every query with the same lines, and RECORDS that
/// it was asked.
///
/// The marker is the anti-vacuity: a witness that never called `gh` cannot
/// have been told anything, so a test that passed because the query never ran
/// would be passing for the wrong reason.
struct FakeGh {
    dir: PathBuf,
    marker: PathBuf,
}

impl FakeGh {
    fn new(name: &str, lines: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("brain-witness-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir the fake gh's directory");
        let marker = dir.join("called");
        let body = format!(
            "#!/usr/bin/env bash\nprintf 'called\\n' >> '{}'\ncat <<'BRAIN_WITNESS_EOF'\n{}BRAIN_WITNESS_EOF\n",
            marker.display(),
            lines
                .iter()
                .map(|l| format!("{l}\n"))
                .collect::<Vec<String>>()
                .join("")
        );
        let gh = dir.join("gh");
        fs::write(&gh, body).expect("write the fake gh");
        let mut perms = fs::metadata(&gh).expect("stat the fake gh").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&gh, perms).expect("chmod the fake gh");
        FakeGh { dir, marker }
    }

    fn was_called(&self) -> bool {
        fs::read_to_string(&self.marker)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }
}

/// Run the witness against the fake `gh`: two attempts, no sleep between
/// them (the interval is an argument, not a seam — the real caller uses the
/// 30-second default).
fn watch(fake: &FakeGh) -> Output {
    let path = std::env::var("PATH").unwrap_or_default();
    Command::new("bash")
        .arg(witness_script())
        .arg("markfietje/brain-server")
        .arg("3b6bb8f025b9ca7f86aceb22c828e98dbb28bfc2")
        .arg("v1.29.26")
        .arg("2")
        .arg("0")
        .env("PATH", format!("{}:{path}", fake.dir.display()))
        .output()
        .expect("spawn the release witness")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// THE ARM. The vacuous pass, closed.
///
/// This is the defect as it was observed on v1.29.26: the query returned
/// nothing, and the witness said green.
#[test]
fn the_witness_reports_unknown_when_it_observes_no_runs() {
    let fake = FakeGh::new("empty", &[]);
    let out = watch(&fake);

    assert!(
        fake.was_called(),
        "the witness must have ASKED — a witness that never queried cannot \
         have been told anything, and a test that passes because the query \
         never ran is passing for the wrong reason"
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "an EMPTY run list is UNKNOWN, never GREEN. The witness returned \
         {:?} with stdout {:?} — the old witness read \"no runs observed\" as \
         \"all green\", and it did so on the v1.29.26 release while both public \
         runs were still in progress.",
        out.status.code(),
        stdout_of(&out)
    );
    assert_eq!(
        stdout_of(&out),
        "UNKNOWN",
        "the witness must SAY that it does not know, in a word the caller \
         cannot mistake for a verdict"
    );
    assert!(
        !stdout_of(&out).contains("GREEN"),
        "the witness must never print GREEN for a run list it did not read"
    );
}

/// A run that has not finished is not a verdict either.
#[test]
fn the_witness_reports_unknown_while_a_run_is_still_in_progress() {
    let fake = FakeGh::new("pending", &["in_progress\t", "completed\tsuccess"]);
    let out = watch(&fake);

    assert!(fake.was_called(), "the witness must have asked");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a run still in progress is UNKNOWN, not GREEN: {:?} / {:?}",
        out.status.code(),
        stdout_of(&out)
    );
    assert_eq!(stdout_of(&out), "UNKNOWN", "the witness must say UNKNOWN");
}

/// The only GREEN: every observed run completed, with a success conclusion.
#[test]
fn the_witness_reports_green_when_every_run_completed_successfully() {
    let fake = FakeGh::new("green", &["completed\tsuccess", "completed\tsuccess"]);
    let out = watch(&fake);

    assert!(fake.was_called(), "the witness must have asked");
    assert_eq!(
        out.status.code(),
        Some(0),
        "every run completed successfully, so the verdict is GREEN: {:?} / {:?}",
        out.status.code(),
        stdout_of(&out)
    );
    assert_eq!(stdout_of(&out), "GREEN", "the witness must say GREEN");
}

/// A run that concluded badly decides the release, and the witness names it.
#[test]
fn the_witness_reports_red_when_a_run_did_not_succeed() {
    let fake = FakeGh::new("failure", &["completed\tsuccess", "completed\tfailure"]);
    let out = watch(&fake);

    assert!(fake.was_called(), "the witness must have asked");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failed run is RED, not green: {:?} / {:?}",
        out.status.code(),
        stdout_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.starts_with("RED"),
        "the witness must report RED: {stdout}"
    );
    assert!(
        stdout.contains("failure"),
        "the witness must NAME the conclusion that failed — a RED that does \
         not say why teaches the operator nothing: {stdout}"
    );
}

/// The two conclusions that are not success and are easy to wave through: a
/// CANCELLED run, and a completed run with NO conclusion at all.
#[test]
fn the_witness_reports_red_for_a_cancelled_run_and_for_a_missing_conclusion() {
    let cancelled = FakeGh::new("cancelled", &["completed\tcancelled"]);
    let out = watch(&cancelled);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a cancelled run is not a success: {:?} / {:?}",
        out.status.code(),
        stdout_of(&out)
    );
    assert!(
        stdout_of(&out).contains("cancelled"),
        "the witness must name the conclusion: {}",
        stdout_of(&out)
    );

    let none = FakeGh::new("noconclusion", &["completed\t"]);
    let out = watch(&none);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a completed run with NO conclusion is not a success — fail closed: \
         {:?} / {:?}",
        out.status.code(),
        stdout_of(&out)
    );
}

/// The helper must DELEGATE, so the pin and the real release cannot disagree.
///
/// Without this arm the fix could sit in `scripts/release-watch.sh` while
/// `scripts/release.sh` kept deciding the verdict for itself — which is the
/// defect, unchanged, behind a green test. This is the same law the
/// release-notes gate holds on `release.yml`: the caller must still invoke
/// the implementation, read from the caller itself.
#[test]
fn the_release_helper_defers_to_the_witness_for_its_verdict() {
    let helper = fs::read_to_string(repo_root().join("scripts/release.sh"))
        .expect("read scripts/release.sh");

    assert!(
        helper.contains("release-watch.sh"),
        "scripts/release.sh must call scripts/release-watch.sh — the verdict \
         lives in ONE implementation so the pin that drives it with fixtures \
         and the helper that watches a real release cannot drift apart"
    );
    assert!(
        !helper.contains("@tsv"),
        "scripts/release.sh must no longer query the runs' verdicts itself \
         (the `.[] | select(...) | [.status, (.conclusion // \"\")] | @tsv` \
         query) — that query is where the empty list read as green, and it \
         belongs to the witness now"
    );
}
