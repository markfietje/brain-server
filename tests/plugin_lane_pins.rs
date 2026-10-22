//! The plugin's lane, pinned from OUTSIDE the lane it pins (III-8 · D1).
//!
//! **The gap this closes.** The plugin carried 1,768 lines of tests across
//! seven files and NO lane executed any of them: no workflow ran
//! `plugin/test/**`, the plugin's `package.json` had no `test` script, and
//! `npm install` in `plugin/` failed outright (`workspace:*` in
//! devDependencies is an unresolvable URL outside the host's monorepo). Five
//! of the seven files failed to LOAD; the two that loaded ran 70 tests, in a
//! developer's terminal and nowhere else. A pin no lane executes is a comment
//! with assertions in it.
//!
//! **Why this pin lives in the KERNEL's suite.** A pin that lives inside the
//! lane it pins dies with it: delete `.github/workflows/plugin.yml` and a
//! vitest pin asserting the lane's steps would simply never run again — the
//! failure nobody sees. The kernel's suite runs in `ci.yml` on every push to
//! main, unfiltered, so a deleted or maimed plugin lane reds HERE.
//!
//! The pins are STRUCTURAL (they read the workflow and the config). Their
//! behavioural half is the plugin lane itself: the same commit that lands
//! this file lands `.github/workflows/plugin.yml`, which runs the suite on
//! every push, and the lane's own green is the behaviour. The register row
//! carries the class and the red-proofs.

use std::path::Path;

fn repo(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel} must exist: {e}"))
}

/// The plugin's suite files, as III-8 measured them. The baseline was seven
/// files; the round adds the two pin files. A file REMOVED from this list
/// must be a deliberate act, because the list is what "the whole suite" means.
const SUITE_FILES: [&str; 9] = [
    "test/plugin.test.ts",
    "test/team-bridge.test.ts",
    "test/fence-parity.test.ts",
    "test/lane-pins.test.ts",
    "test/integration-drill-pins.test.ts",
    "src/format.test.ts",
    "src/config.test.ts",
    "src/brain-client.contract.spec.ts",
    "src/gating.contract.spec.ts",
];

/// D1 — the lane exists, is unfiltered on push, and runs install →
/// type-check → suite in that order. Every step's failure is the lane's
/// failure (D6): no `|| true`, no `continue-on-error`.
#[test]
fn the_plugin_suite_runs_in_a_lane_that_fires_on_every_push() {
    let workflow = repo(".github/workflows/plugin.yml");

    // Unfiltered on push: `branches: [main]` and NO `paths:` filter. A paths
    // filter would make "every push" false, and D1 is the whole law.
    assert!(
        workflow.contains("on:\n  push:\n    branches: [main]"),
        "the plugin lane must fire on every push to main"
    );
    let push_block = workflow
        .split("on:\n  push:\n")
        .nth(1)
        .expect("the workflow must have a push trigger");
    let push_block = &push_block[..push_block
        .find("\n  pull_request:")
        .unwrap_or(push_block.len())];
    assert!(
        !push_block.contains("paths:"),
        "the plugin lane must not be paths-filtered — D1 says every push"
    );

    // The three steps, in order, each named by its command.
    let install = workflow
        .find("npm ci")
        .expect("the lane must install from the lockfile");
    let typecheck = workflow
        .find("npm run typecheck")
        .expect("the lane must type-check the plugin before it runs the suite");
    let suite = workflow
        .find("npm test")
        .expect("the lane must run the plugin's suite");
    assert!(
        install < typecheck && typecheck < suite,
        "the lane's steps must run install → type-check → suite, in that order"
    );

    // Fail closed: a lane that swallows a step's failure proves nothing.
    assert!(
        !workflow.contains("|| true") && !workflow.contains("continue-on-error"),
        "the plugin lane must not swallow a step's failure (D6)"
    );

    // The lane works ON the plugin tree.
    assert!(
        workflow.contains("working-directory: plugin"),
        "the lane must run in plugin/"
    );
    assert!(
        workflow.contains("cache-dependency-path: plugin/package-lock.json"),
        "the lane's npm cache must key on the plugin's lockfile"
    );
    // Every `uses:` is pinned to a commit SHA, as the repo's lanes are.
    for line in workflow.lines() {
        if let Some(reference) = line.trim().strip_prefix("uses:") {
            let reference = reference.trim().split('#').next().unwrap_or("").trim();
            assert!(
                reference.contains('@') && !reference.starts_with("./"),
                "the plugin lane must not use an unpinned action: {reference}"
            );
            let sha = reference.split('@').nth(1).unwrap_or("");
            assert!(
                sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
                "the plugin lane's actions must be pinned to a 40-char SHA: {reference}"
            );
        }
    }
}

/// The lane and the suite cannot drift apart: the `package.json` scripts the
/// lane invokes exist, and the vitest config's include list covers every suite
/// file — so a suite file that stops being collected reds the KERNEL's suite
/// (which runs on every push) rather than nothing at all.
#[test]
fn the_lane_names_the_whole_suite_and_the_suite_declares_itself() {
    let manifest = repo("plugin/package.json");
    assert!(
        manifest.contains("\"test\": \"vitest run\""),
        "the lane runs `npm test`; package.json must declare it"
    );
    assert!(
        manifest.contains("\"typecheck\": \"tsc --noEmit -p tsconfig.json\""),
        "the lane runs `npm run typecheck`; package.json must declare it"
    );

    let config = repo("plugin/vitest.config.ts");
    let include_start = config
        .find("include: [")
        .expect("plugin/vitest.config.ts must declare its include list");
    let rest = &config[include_start..];
    let include = &rest[..rest.find(']').expect("the include list must be closed")];
    let globs = include_globs(include);
    assert!(
        !globs.is_empty(),
        "plugin/vitest.config.ts's include list yielded no usable globs — a lane that \
         collects nothing is the defect this round exists to close"
    );

    for file in SUITE_FILES {
        let present = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugin")
            .join(file);
        assert!(
            present.is_file(),
            "the suite file {file} must exist — the lane's contract names it"
        );
        let (dir, name) = file
            .split_once('/')
            .unwrap_or_else(|| panic!("suite files live under test/ or src/: {file}"));
        let covered = globs
            .iter()
            .any(|(glob_dir, ext)| glob_dir == dir && name.ends_with(&format!(".{ext}")));
        assert!(
            covered,
            "{file} is no longer covered by plugin/vitest.config.ts's include list — \
             it would be a suite file the lane never collects, which is the defect this \
             round exists to close"
        );
    }
}

/// The `<dir>/**/*.<ext>` globs an include list declares. Anything the parser
/// cannot read is NOT a glob, and a list that yields none fails the caller —
/// a config this pin cannot parse must not look like coverage.
fn include_globs(include: &str) -> Vec<(String, String)> {
    // The caller hands over the slice from `include: [` onwards, so the first
    // token still carries that prefix — drop everything before the `[`.
    let list = include
        .split_once('[')
        .map(|(_, rest)| rest)
        .unwrap_or(include);
    list.split(',')
        .filter_map(|token| {
            let pattern = token.trim().trim_matches('\'').trim_matches('"');
            let (dir, tail) = pattern.split_once("/**/")?;
            let ext = tail.strip_prefix("*.")?;
            Some((dir.to_string(), ext.to_string()))
        })
        .collect()
}

/// D3's structural half: the manifest's INSTALL-TIME dependency fields carry
/// no `workspace:` specifier — the exact shape npm refuses with
/// EUNSUPPORTEDPROTOCOL, which is what made the plugin un-installable
/// standalone. The behavioural half is the plugin lane's `npm ci`, which runs
/// on a clean checkout on every push.
#[test]
fn the_plugin_installs_standalone_no_workspace_specifier_in_an_install_field() {
    let manifest = repo("plugin/package.json");

    let install_fields = [
        "\"dependencies\"",
        "\"devDependencies\"",
        "\"optionalDependencies\"",
    ];
    for field in install_fields {
        let Some(start) = manifest.find(field) else {
            continue;
        };
        let rest = &manifest[start..];
        let table = &rest[..rest.find('}').unwrap_or(rest.len())];
        assert!(
            !table.contains("workspace:"),
            "{field} carries a workspace: specifier — npm refuses it outside the host's \
             monorepo (EUNSUPPORTEDPROTOCOL), which is what left the plugin unable to \
             install its own dependencies"
        );
    }

    // The SDK stays declared — as the HOST's peer, optional, because the host
    // provides it and a standalone tree must not need it.
    assert!(
        manifest.contains("\"peerDependencies\""),
        "the SDK must stay declared as the host's peer"
    );
    assert!(
        manifest.contains("\"optional\": true"),
        "the SDK peer must be optional"
    );
}
