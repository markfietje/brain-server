//! The Tools round's asset-trim pin (W-T3) — the release page is the ~9-asset
//! set EXACTLY, and the bloat cannot silently return. The darwin binaries
//! ship inside the stapled StewardMem-Tools.dmg (the operator lane signs it;
//! D6); the Linux set ships as ONE tarball; the workflow's upload surface can
//! reach nothing else. A structural pin is a REGRESSION LOCK — the behavioral
//! proof is the release itself, and the drill below plants the bloat back.

use std::path::PathBuf;

fn release_yml() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join(".github/workflows/release.yml"))
        .expect("release.yml must exist and read")
}

/// The trimmed upload surface: the darwin binaries never stage into `dist/`
/// (they ride the stapled Tools DMG, signed by the operator lane — D6), the
/// Linux set ships as ONE tarball whose insides keep the historical asset
/// names, and the publish gates name the trimmed set. An EXTRA asset = red.
#[test]
fn the_release_page_is_the_trimmed_set_and_the_bloat_cannot_return() {
    let yml = release_yml();

    // The darwin artifacts stage BESIDE the page, never in it.
    assert!(
        yml.contains("path: dist-darwin-staging"),
        "the darwin artifacts must stage beside dist/ — the page carries them only inside the stapled Tools DMG"
    );
    let dist_blocks = yml.split("download-artifact").skip(1);
    for block in dist_blocks {
        if block.contains("dist-darwin-arm64") || block.contains("dist-darwin-x86_64") {
            assert!(
                block.contains("dist-darwin-staging"),
                "a darwin artifact downloads into the page's dist/ — the trim leaks; it must stage beside it"
            );
        }
    }

    // The Linux set: ONE tarball, built and then the bare binaries REMOVED
    // from dist/ — the page never carries both shapes.
    assert!(
        yml.contains("tar -czf StewardMem-CLI-linux.tar.gz"),
        "the Linux set must ship as the one tarball"
    );
    assert!(
        yml.contains("rm -f brain-server-linux-*"),
        "the bare linux binaries must leave dist/ after the tarball — the page never carries both shapes"
    );

    // PLATFORMS.txt describes the trimmed shape and no longer walks darwin
    // per-binary suffixes.
    assert!(
        !yml.contains("*-darwin-arm64)  p="),
        "PLATFORMS.txt still enumerates darwin binaries — they left the page"
    );
    assert!(
        yml.contains("StewardMem-CLI-linux.tar.gz) p="),
        "PLATFORMS.txt must describe the tarball"
    );

    // The verify gate: the tarball's INSIDES are the required set, and the
    // darwin builds must have succeeded (the Tools DMG is cut from exactly
    // those bytes).
    assert!(
        yml.contains("required binary missing from the Linux tarball"),
        "the integrity gate must look INSIDE the tarball"
    );
    assert!(
        yml.contains("the Tools DMG cannot be cut"),
        "the integrity gate must refuse a release the Tools DMG cannot be cut from"
    );

    // The publish gates name the trimmed set — the darwin primaries are gone
    // from both.
    assert!(
        !yml.contains("brain-server-darwin-arm64 brain-server-darwin-x86_64; do"),
        "the publish gates still demand the darwin primaries — the trim never landed"
    );
    for required in [
        "StewardMem-CLI-linux.tar.gz SHA256SUMS PLATFORMS.txt",
        "brain-server-$version.cdx.json",
    ] {
        assert!(
            yml.contains(required),
            "the publish gates must name {required}"
        );
    }

    // Anti-vacuity: the builds themselves still PRODUCE the darwin set — the
    // trim removes the page assets, never the artifacts the Tools DMG and
    // the app lane are cut from.
    assert!(
        yml.contains("dist-darwin-arm64") && yml.contains("dist-darwin-x86_64"),
        "the darwin artifacts must still be produced — the operator lane downloads them from the run"
    );
}

/// The DRILL, planted: a darwin artifact downloads back into `dist/` — the
/// pin must go red naming the leak. (Reverted from the round's own copy.)
#[test]
fn the_trim_pin_fires_when_the_bloat_is_planted_back() {
    // The drill mutates NOTHING on disk: the pin's law is a pure function of
    // the workflow text, so the plant is an in-memory mutation — the red is
    // the assertion firing on the mutated text, the revert is the original
    // text still passing (drilled and reverted in one run, honestly).
    let yml = release_yml();
    let planted = yml.replace(
        "          name: dist-darwin-arm64\n          path: dist-darwin-staging",
        "          name: dist-darwin-arm64\n          path: dist",
    );
    assert_ne!(planted, yml, "the drill's mutation must apply");
    let leaks = planted
        .split("download-artifact")
        .skip(1)
        .any(|block| block.contains("dist-darwin-arm64") && !block.contains("dist-darwin-staging"));
    assert!(
        leaks,
        "the drill plants the bloat back — the pin's own law must read it as a leak"
    );
}
