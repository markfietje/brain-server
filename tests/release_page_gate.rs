//! The page-shape gate — the release's PUBLISHED assets are the ~9-asset set
//! EXACTLY (the register row 127's owed guard, made real).
//!
//! The endpoint is the LATEST release's manifest, the updater's artefact and
//! its signature must be fetchable, and the Tools round trimmed the page to
//! the set the signing lane actually finishes: the app DMG, the updater chain
//! (tarball + sig + latest.json), the stapled Tools DMG, the one Linux
//! tarball, the sums, the platforms list, and the SBOM. A page that carries
//! MORE is the bloat growing back; a page that carries LESS is a broken
//! promise the updater discovers at the user's machine. **An EXTRA asset = red.**
//!
//! `scripts/release-page-check.sh` is ONE implementation with TWO callers —
//! `scripts/release.sh` (which must not call a release green while the page
//! is the wrong shape) and THIS file (which drives it against fixtures). A
//! check that exists in only one of them fails only in the more expensive
//! one — and the expensive one is the moment a release is being cut.
//!
//! The fixtures are a LOCAL file of asset names, not the internet: the check
//! takes `--fixture <file>`, so the pin can make the page carry whatever it
//! must refuse — without `gh`, without a network, and without a real release.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The tag under test; the version-dependent SBOM name follows it.
const TAG: &str = "v1.29.39";

/// The tests run in PARALLEL and share a process: the fixture dir is
/// serialized by a counter, never by a timestamp (an `Instant` created
/// inside one test reads ~zero for all of them, and two tests writing one
/// fixture file is a torn fixture, not a test).
static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write a fixture file of asset names and run the check against it.
fn check(names: &[&str]) -> (Option<i32>, String, String) {
    check_with_tag(TAG, names)
}

fn check_with_tag(tag: &str, names: &[&str]) -> (Option<i32>, String, String) {
    let fixture_dir = std::env::temp_dir().join(format!(
        "brain-page-gate-{}-{}",
        std::process::id(),
        FIXTURE_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&fixture_dir).expect("the fixture dir is creatable");
    let fixture = fixture_dir.join("assets.txt");
    std::fs::write(&fixture, names.join("\n") + "\n").expect("the fixture is writable");

    let script = script_path();
    let output = Command::new("bash")
        .arg(&script)
        .arg(tag)
        .arg("--fixture")
        .arg(&fixture)
        .output()
        .expect("the page check script is runnable");
    let result = (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    );
    std::fs::remove_dir_all(&fixture_dir).ok();
    result
}

fn script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/release-page-check.sh")
}

/// The trimmed page: the ~9-asset set, exactly as the signing lane finishes it.
fn the_trimmed_page() -> Vec<&'static str> {
    vec![
        "brain-server-1.29.39.cdx.json",
        "latest.json",
        "PLATFORMS.txt",
        "SHA256SUMS",
        "StewardMem-CLI-linux.tar.gz",
        "StewardMem-Tools.dmg",
        "StewardMem.app.tar.gz",
        "StewardMem.app.tar.gz.sig",
        "StewardMem.dmg",
    ]
}

/// The trimmed page is GREEN.
#[test]
fn the_trimmed_page_is_green() {
    let (status, stdout, stderr) = check(&the_trimmed_page());
    assert_eq!(
        status,
        Some(0),
        "the trimmed page must be green, got stdout {stdout:?} stderr {stderr:?}"
    );
    assert!(
        stdout.contains("9"),
        "the check names the count, got {stdout:?}"
    );
}

/// A MISSING asset is refused, and the refusal NAMES it.
#[test]
fn a_missing_asset_is_refused_and_named() {
    // The Tools DMG is the one the signing lane adds last — the exact shape
    // of a release whose lane died mid-pass.
    let without_tools: Vec<&str> = the_trimmed_page()
        .into_iter()
        .filter(|name| *name != "StewardMem-Tools.dmg")
        .collect();
    let (status, _, stderr) = check(&without_tools);
    assert_eq!(
        status,
        Some(1),
        "a page short an asset must be refused, got {stderr:?}"
    );
    assert!(
        stderr.contains("StewardMem-Tools.dmg"),
        "the refusal must name the missing asset, got {stderr:?}"
    );
    assert!(
        stderr.contains("missing"),
        "the refusal must say the asset is missing, got {stderr:?}"
    );
}

/// An EXTRA asset is refused, and the refusal NAMES it — the bloat cannot
/// quietly return. The name used here is the exact one the trim removed.
#[test]
fn an_extra_asset_is_refused_and_named() {
    let mut bloated = the_trimmed_page();
    bloated.push("brain-server-darwin-arm64");
    let (status, _, stderr) = check(&bloated);
    assert_eq!(
        status,
        Some(1),
        "a page carrying an extra asset must be refused, got {stderr:?}"
    );
    assert!(
        stderr.contains("brain-server-darwin-arm64"),
        "the refusal must name the extra asset, got {stderr:?}"
    );
    assert!(
        stderr.contains("extra"),
        "the refusal must say the asset is extra, got {stderr:?}"
    );
}

/// BOTH wrong at once is still one red: a missing asset does not distract
/// from an extra one, and neither passes alone.
#[test]
fn a_page_wrong_in_both_directions_is_refused() {
    let mut wrong: Vec<&str> = the_trimmed_page()
        .into_iter()
        .filter(|name| *name != "latest.json")
        .collect();
    wrong.push("mcp-darwin-x86_64");
    let (status, _, stderr) = check(&wrong);
    assert_eq!(status, Some(1), "a wrong page is refused, got {stderr:?}");
    assert!(
        stderr.contains("latest.json") && stderr.contains("missing"),
        "the missing half is named, got {stderr:?}"
    );
    assert!(
        stderr.contains("mcp-darwin-x86_64") && stderr.contains("extra"),
        "the extra half is named, got {stderr:?}"
    );
}

/// The version-dependent SBOM name must be the TAG's version — a page whose
/// SBOM belongs to another release is a page lying about itself.
#[test]
fn the_sbom_name_must_be_the_tag_s_version() {
    let page_with_sbom = |version: &str| -> Vec<String> {
        the_trimmed_page()
            .into_iter()
            .map(|name| {
                if name == "brain-server-1.29.39.cdx.json" {
                    format!("brain-server-{version}.cdx.json")
                } else {
                    name.to_string()
                }
            })
            .collect()
    };
    let stale_owned = page_with_sbom("1.29.99");
    let stale: Vec<&str> = stale_owned.iter().map(String::as_str).collect();
    let (status, _, stderr) = check_with_tag(TAG, &stale);
    assert_eq!(
        status,
        Some(1),
        "a stale SBOM name must be refused, got {stderr:?}"
    );
    let own_owned = page_with_sbom("1.29.39");
    let own: Vec<&str> = own_owned.iter().map(String::as_str).collect();
    let (status, _, stderr) = check_with_tag(TAG, &own);
    assert_eq!(
        status,
        Some(0),
        "the tag's own SBOM name is green, got {stderr:?}"
    );
}

/// An EMPTY page is refused — the check says what is missing, all of it.
#[test]
fn an_empty_page_is_refused() {
    let (status, _, stderr) = check(&[]);
    assert_eq!(status, Some(1), "an empty page is refused, got {stderr:?}");
    assert!(
        stderr.contains("missing"),
        "the refusal says the page is missing its assets, got {stderr:?}"
    );
}

/// The SECOND CALLER is real: `scripts/release.sh` must invoke the page
/// check — AFTER the chain check, so a green release means the endpoint
/// answered AND the page is the right shape.
#[test]
fn the_release_script_carries_the_page_check_after_the_chain_check() {
    let release = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/release.sh"),
    )
    .expect("read scripts/release.sh");
    let chain_at = release
        .find("release-chain-check.sh")
        .expect("release.sh must call the chain check");
    let page_at = release.find("release-page-check.sh").expect(
        "release.sh must call the page check — the trim's behavioural gate is the release's own",
    );
    assert!(
        page_at > chain_at,
        "the page check must run AFTER the chain check — the chain's three assets are page assets too"
    );
}

/// The check refuses to run at all without `gh` in its NETWORK mode — and
/// says which tool is missing, so a missing tool is never read as a red page.
#[test]
fn the_network_mode_names_the_missing_tool() {
    let empty = std::env::temp_dir().join("brain-page-gate-no-gh");
    std::fs::create_dir_all(&empty).expect("the scratch dir is creatable");
    let bash = std::env::var("SHELL")
        .ok()
        .filter(|shell| Path::new(shell).exists())
        .unwrap_or_else(|| "/bin/bash".to_string());
    let output = Command::new(&bash)
        .arg(script_path())
        .arg(TAG)
        .env("PATH", &empty)
        .output()
        .expect("the page check script is runnable");
    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing gh must be reported as 'could not run', not as a red page"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("gh"),
        "the refusal must name the missing tool, got {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
