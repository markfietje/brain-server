//! Plugin-sync baseline — **the drift guard ships armed in every checkout.**
//!
//! **The gap this closes.** `scripts/sync-plugin.sh` guards every sync with
//! a three-way drift check against `.plugin-sync-baseline`, the brain-server
//! commit whose tree the fork's deployed extension last matched — but the
//! baseline was GITIGNORED, so it existed on exactly one machine. In every
//! fresh checkout (CI included) the script printed "initializing without
//! drift guard" and then `rsync --delete` overwrote target-side committed
//! edits with no complaint — while the script's own header claimed "Both
//! enforced, both fail-closed". A fail-closed control that disarms itself
//! in every clone is a one-machine convention, not a gate.
//!
//! Two laws, one file:
//!
//! 1. [`the_plugin_sync_baseline_is_tracked`] — `git ls-files` must list
//!    `.plugin-sync-baseline` (tracked, so every checkout carries it), and
//!    the ignore rules must not cover it (an ignored-but-tracked file is a
//!    lie the next `git rm --cached` makes true). The hostile arm proves
//!    the emptiness check can actually fail: a path nothing tracks yields
//!    an EMPTY `ls-files` result, so a non-empty assertion over it is
//!    meaningful, never vacuous.
//!
//! 2. [`the_baseline_names_a_commit_the_repo_can_resolve`] — the recorded
//!    value must be a commit SHA the script's own validity probe accepts
//!    (`git cat-file -e <sha>^{commit}`): a baseline naming a commit that
//!    no longer exists in this repository disarms the guard exactly as
//!    effectively as an absent file, and with the same silent
//!    "initializing without" message.

use std::path::Path;
use std::process::Command;

const BASELINE_PATH: &str = ".plugin-sync-baseline";

fn repo(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn git_at_repo(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo("."))
        .output()
        .unwrap_or_else(|e| panic!("git {:?} must run: {e}", args));
    assert!(
        out.status.success(),
        "git {:?} failed — the pin cannot measure, so it must not report green: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Law 1 — the baseline is a TRACKED file, so the drift guard is armed in
/// every checkout, not only on the machine that last ran a sync.
#[test]
fn the_plugin_sync_baseline_is_tracked() {
    let listed = git_at_repo(&["ls-files", "--", BASELINE_PATH]);
    let listed = listed.trim();
    assert_eq!(
        listed, BASELINE_PATH,
        "the plugin-sync drift-guard baseline must be tracked — an untracked \
         baseline disarms the guard in every fresh checkout and \
         sync-plugin.sh falls back to 'initializing without drift guard' \
         before an rsync --delete"
    );

    // Tracked AND not ignored: an ignore rule over a tracked file is inert
    // today but re-arms the moment someone `git rm --cached`s it, so the
    // rule itself must not exist. The probe is `--no-index` on purpose:
    // plain `git check-ignore` defers to the index and NEVER reports a rule
    // over a tracked file (measured: with the rule re-added, plain
    // check-ignore exits 1 and this arm passed green over the rule's
    // presence — the vacuous pass this flag exists to kill; --no-index
    // asks the rule question directly and exits 0 on that same state).
    let ignore = Command::new("git")
        .args(["check-ignore", "--no-index", BASELINE_PATH])
        .current_dir(repo("."))
        .output()
        .expect("git check-ignore must run");
    assert!(
        !ignore.status.success(),
        "{BASELINE_PATH} is tracked and must NOT be covered by an ignore rule — \
         delete the rule (the hostile-tail class: an ignore line that \
         re-arms on the next untracked transition)"
    );

    // Hostile arm, anti-vacuity: the emptiness assertion above can fail —
    // a path nothing tracks yields an EMPTY ls-files result, never the
    // baseline path. A broken `ls-files` (or a matcher that accepts any
    // output) would pass both sides of this pair.
    let hostile = git_at_repo(&["ls-files", "--", "definitely-not-tracked-xyz"]);
    assert!(
        hostile.trim().is_empty(),
        "the control path must be able to produce an empty result — if an \
         untracked path lists, the tracked-path assertion above is vacuous"
    );
}

/// Law 2 — the recorded baseline resolves to a real commit in THIS
/// repository, which is exactly the validity probe the sync script runs
/// before it will use the guard (`git cat-file -e <sha>^{commit}`).
#[test]
fn the_baseline_names_a_commit_the_repo_can_resolve() {
    let raw = std::fs::read_to_string(repo(BASELINE_PATH))
        .unwrap_or_else(|e| panic!("{BASELINE_PATH} must exist and be readable: {e}"));
    let sha = raw.trim();
    assert!(
        !sha.is_empty(),
        "an empty baseline disarms the guard as effectively as an absent one"
    );
    assert_eq!(
        sha.len(),
        40,
        "the baseline records a full commit SHA (the sync script probes \
         `<sha>^{{commit}}`); got {sha_len} characters: {sha}",
        sha_len = sha.len()
    );
    let probe = format!("{sha}^{{commit}}");
    let out = Command::new("git")
        .args(["cat-file", "-e", &probe])
        .current_dir(repo("."))
        .output()
        .unwrap_or_else(|e| panic!("git cat-file must run: {e}"));
    assert!(
        out.status.success(),
        "the baseline names a commit this repository cannot resolve ({sha}) — \
         the sync script treats that as 'no usable baseline' and disarms the \
         drift guard with the same silent fallback"
    );
}
