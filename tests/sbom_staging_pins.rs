//! Release-matched SBOM assets: a release stages and publishes exactly the
//! SBOM for its tag — never the historical set.
//!
//! The fixture checks drive the staging seam (`scripts/stage-sbom.sh`)
//! behaviorally: exact match stages one, a missing or mismatched current
//! manifest fails closed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn workflow() -> String {
    std::fs::read_to_string(repo().join(".github/workflows/release.yml"))
        .expect("release.yml readable")
}

fn write_sbom(tree: &Path, version: &str) {
    let dir = tree.join("sbom");
    std::fs::create_dir_all(&dir).expect("fixture sbom dir");
    let body = serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": "brain-server",
                "version": version,
            }
        },
        "components": [],
    });
    std::fs::write(
        dir.join(format!("brain-server-{version}.cdx.json")),
        serde_json::to_string(&body).expect("fixture serializes"),
    )
    .expect("fixture written");
}

/// A fixture tree with several historical SBOMs plus the target one.
fn fixture(history: &[&str], target: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("temp dir");
    for v in history {
        write_sbom(tmp.path(), v);
    }
    write_sbom(tmp.path(), target);
    tmp
}

fn stage(version: &str, tree: &Path, dist: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(repo().join("scripts/stage-sbom.sh"))
        .arg(version)
        .arg(tree)
        .arg(dist)
        .output()
        .expect("stage-sbom.sh executes")
}

/// A dist fixture file with a chosen filename and reported version.
fn write_dist_sbom(dist: &Path, filename: &str, internal: &str) {
    let body = serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": "brain-server",
                "version": internal,
            }
        },
        "components": [],
    });
    std::fs::write(
        dist.join(filename),
        serde_json::to_string(&body).expect("fixture serializes"),
    )
    .expect("fixture written");
}

fn check_dist(version: &str, dist: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(repo().join("scripts/check-dist-sbom.sh"))
        .arg(version)
        .arg(dist)
        .output()
        .expect("check-dist-sbom.sh executes")
}

/// Fixture with several historical SBOMs stages exactly the tag-matched
/// manifest — and the merged multi-platform dist then holds one SBOM asset
/// for the target version.
#[test]
fn historical_tree_stages_exactly_the_tag_matched_manifest() {
    let tree = fixture(&["1.28.84", "1.29.3", "1.29.4"], "1.29.5");
    let dist = tree.path().join("dist");
    let out = stage("1.29.5", tree.path(), &dist);
    assert!(
        out.status.success(),
        "exact match stages cleanly: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let staged: Vec<String> = std::fs::read_dir(&dist)
        .expect("dist exists")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        staged,
        vec!["brain-server-1.29.5.cdx.json".to_string()],
        "one SBOM asset, the tag-matched one — no historicals: {staged:?}"
    );
}

/// A missing current-version file fails the staging step (fail closed —
/// never a silent release without, or with substitute, SBOMs).
#[test]
fn missing_current_version_file_fails_staging() {
    let tree = fixture(&["1.28.84", "1.29.4"], "1.29.5");
    std::fs::remove_file(tree.path().join("sbom/brain-server-1.29.5.cdx.json"))
        .expect("remove target");
    let dist = tree.path().join("dist");
    let out = stage("1.29.5", tree.path(), &dist);
    assert!(
        !out.status.success(),
        "a missing current SBOM must fail, not stage historicals"
    );
    let staged: usize = std::fs::read_dir(&dist)
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert_eq!(
        staged, 0,
        "nothing stages on failure — no historical fallback"
    );
}

/// A wrong-version manifest fails even when it is the only file present.
#[test]
fn wrong_version_manifest_fails_even_when_sole_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    write_sbom(tmp.path(), "1.29.4");
    // Renamed to the wanted filename but reporting the old version inside.
    std::fs::rename(
        tmp.path().join("sbom/brain-server-1.29.4.cdx.json"),
        tmp.path().join("sbom/brain-server-1.29.5.cdx.json"),
    )
    .expect("rename");
    let dist = tmp.path().join("dist");
    let out = stage("1.29.5", tmp.path(), &dist);
    assert!(
        !out.status.success(),
        "a version-mismatched manifest must fail even as the sole file"
    );
}

/// Removing or broadening the version-specific selection fails this check:
/// no build stage may copy the SBOM directory by glob.
#[test]
fn workflow_stages_no_sbom_glob() {
    let yml = workflow();
    let glob_stages: Vec<&str> = yml
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && t.contains("sbom/*.cdx.json")
        })
        .collect();
    assert!(
        glob_stages.is_empty(),
        "no non-comment SBOM glob staging may remain: {glob_stages:?}"
    );
    assert!(
        yml.contains("scripts/stage-sbom.sh"),
        "build staging routes through the version-specific staging seam"
    );
}

/// The merged-dist gate lives in the release job's verify step and runs
/// through `scripts/check-dist-sbom.sh` (a file, not inline: run-block
/// quoting layers do not survive the transport intact). Removing the call
/// fails this pin; the behavior below proves what the call enforces.
#[test]
fn release_verify_routes_through_dist_check() {
    let yml = workflow();
    assert!(
        yml.contains("scripts/check-dist-sbom.sh"),
        "the release verify step must invoke the dist SBOM check seam"
    );
}

/// A dist holding exactly the tag-matched manifest passes the check.
#[test]
fn dist_check_passes_for_exact_match() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(&dist).expect("dist dir");
    write_dist_sbom(&dist, "brain-server-1.29.5.cdx.json", "1.29.5");
    let out = check_dist("1.29.5", &dist);
    assert!(
        out.status.success(),
        "exact match passes: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A dist without the manifest fails the check (fail closed).
#[test]
fn dist_check_refuses_missing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(&dist).expect("dist dir");
    let out = check_dist("1.29.5", &dist);
    assert!(!out.status.success(), "a missing dist SBOM must fail");
}

/// A manifest reporting the wrong version fails even as the sole file.
#[test]
fn dist_check_refuses_mismatch() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(&dist).expect("dist dir");
    write_dist_sbom(&dist, "brain-server-1.29.5.cdx.json", "1.29.4");
    let out = check_dist("1.29.5", &dist);
    assert!(
        !out.status.success(),
        "a version-mismatched dist SBOM must fail"
    );
}

/// A historical manifest beside the matched one fails the check.
#[test]
fn dist_check_refuses_historicals() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(&dist).expect("dist dir");
    write_dist_sbom(&dist, "brain-server-1.29.5.cdx.json", "1.29.5");
    write_dist_sbom(&dist, "brain-server-1.29.4.cdx.json", "1.29.4");
    let out = check_dist("1.29.5", &dist);
    assert!(
        !out.status.success(),
        "a historical SBOM beside the matched one must fail"
    );
}
