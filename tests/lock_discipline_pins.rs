//! Lock discipline — **the committed lock is the reviewed truth; nothing may
//! move it silently.**
//!
//! **The gap this closes.** Both `tools/` manifests were bumped in one commit
//! without re-locking, so the committed locks stopped satisfying them — and
//! every bare `cargo` invocation (a CI lane, a local clippy) then re-locks
//! *silently*, reporting green against dependency versions nobody committed.
//! Worse, signal-gateway's `presage` git dependencies rode a `branch = "main"`
//! pointer, so any re-lock could move the whole libsignal stack to whatever
//! upstream merged that morning — CI green over unreviewed code.
//!
//! **Three laws, three failure modes.**
//!
//! 1. [`the_tools_locks_satisfy_their_manifests`] — every DIRECT dependency
//!    requirement in each tools/ manifest is satisfied by the committed lock.
//!    This parser is hermetic on purpose: the full-graph probe is
//!    `cargo metadata --locked`, which the verification sweep's
//!    lock-freshness lane and both gate lanes' `--locked` flags run for real —
//!    resolving signal-gateway's git graph inside the root suite would add
//!    multi-repo clones to every cold CI run for a property those lanes
//!    already enforce behaviourally. This check is deliberately WEAKER than
//!    cargo's resolver (direct deps only, caret forms only) and fails closed
//!    on any requirement shape it does not recognise, so it can never silently
//!    skip a dependency it failed to parse.
//!
//! 2. [`the_tools_ci_lanes_pin_resolution_with_locked`] — the two gate lanes
//!    run clippy and test with `--locked`. `cargo fmt` cannot carry the flag
//!    (it rejects `--locked` as an unexpected argument) and resolves via
//!    `--no-deps` metadata, which is exactly why the sweep probe must use the
//!    full form: the short form passes vacuously on a stale lock.
//!
//! 3. [`git_dependencies_ride_a_pinned_rev`] — presage and
//!    presage-store-sqlite are pinned by `rev`, never `branch`. A branch
//!    pointer re-resolves to upstream's current head the moment ANY re-lock
//!    occurs; a rev keeps the resolution a deliberate, reviewed act.

use std::path::{Path, PathBuf};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// One direct dependency row from a `[dependencies]` / `[dev-dependencies]`
/// table. Anything this enum cannot represent is a parse FAILURE, never a
/// skip — a checker that silently ignores a dependency it does not recognise
/// is a checker that passes while proving nothing.
enum DepReq {
    /// A crates.io requirement in caret form (`"1.2.3"`, `"0.30"`, `"3"`).
    Version(String),
    /// A git dependency pinned to an exact commit.
    GitRev { rev: String },
}

fn direct_deps(manifest: &str) -> Vec<(String, DepReq)> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]" || trimmed == "[dev-dependencies]";
            continue;
        }
        if !in_deps || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = trimmed.split_once(" = ") else {
            panic!("unparseable dependency line (fail-closed, not skipped): {trimmed:?}");
        };
        let name = name.trim();
        let req = if let Some(v) = rest.strip_prefix('"') {
            // `name = "1.2.3"` — a bare version requirement.
            let version = v.split('"').next().unwrap_or("");
            DepReq::Version(version.to_string())
        } else if let Some(body) = rest.strip_prefix('{') {
            let field = |key: &str| {
                let needle = format!("{key} = \"");
                body.find(&needle).map(|i| {
                    body[i + needle.len()..]
                        .split('"')
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
            };
            match (field("version"), field("rev")) {
                (Some(v), None) => DepReq::Version(v),
                (None, Some(r)) => DepReq::GitRev { rev: r },
                (Some(_), Some(_)) => panic!(
                    "dependency {name} carries both version and rev — shape not recognised, \
                     fail-closed: {trimmed:?}"
                ),
                (None, None) => panic!(
                    "dependency {name} has neither version nor rev (workspace/path forms are \
                     not expected in the tools manifests) — fail-closed: {trimmed:?}"
                ),
            }
        } else {
            panic!("unparseable dependency value (fail-closed, not skipped): {trimmed:?}");
        };
        out.push((name.to_string(), req));
    }
    out
}

/// One `[[package]]` row of a Cargo.lock. The lock repeats a name when two
/// versions coexist (e.g. base64 0.22 under presage beside 0.23 here), so
/// satisfaction checks must ask whether ANY entry fits, never exactly one.
struct LockedPkg {
    name: String,
    version: String,
    source: Option<String>,
}

fn locked_packages(lock: &str) -> Vec<LockedPkg> {
    let mut out: Vec<LockedPkg> = Vec::new();
    let mut cur: Option<LockedPkg> = None;
    for line in lock.lines() {
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("name = \"") {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            cur = Some(LockedPkg {
                name: v.split('"').next().unwrap_or("").to_string(),
                version: String::new(),
                source: None,
            });
        } else if let Some(v) = trimmed.strip_prefix("version = \"") {
            if let Some(c) = cur.as_mut() {
                c.version = v.split('"').next().unwrap_or("").to_string();
            }
        } else if let Some(v) = trimmed.strip_prefix("source = \"")
            && let Some(c) = cur.as_mut()
        {
            c.source = Some(v.split('"').next().unwrap_or("").to_string());
        }
    }
    if let Some(done) = cur.take() {
        out.push(done);
    }
    out
}

/// Parse `x`, `x.y`, or `x.y.z` into numeric components. Anything else
/// (wildcards, operators, pre-release suffixes) is a panic: this checker
/// recognises exactly the forms the tools manifests use, and a new form must
/// fail loudly so it gets taught, not silently skipped.
fn version_parts(v: &str) -> (u64, u64, u64) {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        panic!("version shape not recognised (fail-closed): {v:?}");
    }
    let mut nums = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        nums[i] = p.parse().unwrap_or_else(|e| {
            panic!("version component {p:?} of {v:?} is not numeric (fail-closed): {e}")
        });
    }
    (nums[0], nums[1], nums[2])
}

/// Does `have` satisfy the bare-caret requirement `req`?
///
/// A bare requirement is caret semantics over the components actually
/// written: `"3"` means `>=3.0.0, <4.0.0`; `"0.30"` means `>=0.30.0,
/// <0.31.0`; `"1.2.3"` means `>=1.2.3, <2.0.0` — the upper bound widens at
/// the leftmost NON-ZERO written component, which is cargo's rule.
///
/// Two semver forms are honoured: build metadata (`0.9.34+deprecated`) is
/// precedence-IRRELEVANT and stripped; a pre-release (`1.0.0-rc.1`) never
/// satisfies a bare caret requirement under cargo's comparator rules, so it
/// reads as unsatisfied — the fail-closed direction.
fn satisfies(req: &str, have: &str) -> bool {
    let req_parts: Vec<&str> = req.split('.').collect();
    // Build metadata (`+deprecated`) never affects precedence.
    let have_core = have.split('+').next().unwrap_or(have);
    let (have_triple, have_pre) = match have_core.split_once('-') {
        Some((numeric, pre)) => (version_parts(numeric), Some(pre)),
        None => (version_parts(have_core), None),
    };
    if have_pre.is_some() {
        return false;
    }
    let floor = version_parts(req);
    if have_triple < floor {
        return false;
    }
    let ceiling = match req_parts.len() {
        1 => (floor.0 + 1, 0, 0),
        2 if floor.0 == 0 => (0, floor.1 + 1, 0),
        2 => (floor.0 + 1, 0, 0),
        3 if floor.0 > 0 => (floor.0 + 1, 0, 0),
        3 if floor.1 > 0 => (0, floor.1 + 1, 0),
        3 => (0, 0, floor.2 + 1),
        _ => panic!("requirement shape not recognised (fail-closed): {req:?}"),
    };
    have_triple < ceiling
}

/// Law 1 — the committed lock satisfies the committed manifest, for both
/// standalone tools. This is the drift class that made every bare cargo
/// invocation a silent re-lock: the manifest moved, the lock did not, and the
/// runner papered over the gap by regenerating it.
#[test]
fn the_tools_locks_satisfy_their_manifests() {
    for (manifest_rel, lock_rel) in [
        (
            "tools/channel-bridge/Cargo.toml",
            "tools/channel-bridge/Cargo.lock",
        ),
        (
            "tools/signal-gateway/Cargo.toml",
            "tools/signal-gateway/Cargo.lock",
        ),
    ] {
        let deps = direct_deps(&read(manifest_rel));
        // ── anti-vacuity: the manifest parser must see the real tables.
        assert!(
            deps.len() >= 15,
            "the dependency scan of {manifest_rel} found only {} rows — the parser is broken, \
             and a freshness check that sees nothing passes while proving nothing",
            deps.len()
        );
        let pkgs = locked_packages(&read(lock_rel));
        // ── anti-vacuity: the lock parser must see a real dependency graph.
        assert!(
            pkgs.len() >= 100,
            "the package scan of {lock_rel} found only {} rows — the parser is broken",
            pkgs.len()
        );

        for (name, req) in &deps {
            match req {
                DepReq::Version(want) => {
                    let ok = pkgs
                        .iter()
                        .any(|p| &p.name == name && satisfies(want, &p.version));
                    assert!(
                        ok,
                        "{manifest_rel} requires {name} {want} but the committed lock has no \
                         satisfying entry — the lock is STALE against its own manifest, and \
                         every bare cargo invocation (CI lane, local clippy) will silently \
                         re-lock and report green over versions nobody committed. Re-lock and \
                         commit BOTH files together.",
                    );
                }
                DepReq::GitRev { rev } => {
                    let needle = format!("#{rev}");
                    let ok = pkgs.iter().any(|p| {
                        &p.name == name && p.source.as_deref().is_some_and(|s| s.ends_with(&needle))
                    });
                    assert!(
                        ok,
                        "{manifest_rel} pins git dependency {name} at rev {rev} but the \
                         committed lock does not resolve it there — the lock and the pin \
                         disagree, which is exactly the unreviewed movement this law exists \
                         to refuse. Re-lock against the pinned rev and commit both files.",
                    );
                }
            }
        }
    }
}

/// Slice one job block out of the CI workflow: from the `  <job-id>:` header
/// line to the next sibling job key (a key at the same two-space indent).
fn job_block(ci: &str, job: &str) -> Option<String> {
    let header = format!("  {job}:");
    let lines: Vec<&str> = ci.lines().collect();
    let start = lines.iter().position(|l| *l == header)?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            l.starts_with("  ")
                && !l.starts_with("   ")
                && l.ends_with(':')
                && !l.trim_start().starts_with('-')
        })
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some(lines[start..end].join("\n"))
}

/// Law 2 — the two standalone-tool gate lanes build with `--locked`, so a
/// runner can never regenerate the lockfile and go green over dependency
/// versions nobody committed.
#[test]
fn the_tools_ci_lanes_pin_resolution_with_locked() {
    let ci = read(".github/workflows/ci.yml");
    for job in ["channel-bridge-gate", "signal-gateway-gate"] {
        // ── anti-vacuity: a RENAMED lane must fail here, not read as covered
        // by nothing. The existence assertion IS the guard against the pin
        // going vacuous when someone reorganises the workflow.
        let block = job_block(&ci, job).unwrap_or_else(|| {
            panic!(
                "job {job} not found in .github/workflows/ci.yml — the lane was renamed \
                     or removed, and this pin refuses to pass while covering nothing"
            )
        });
        let resolving: Vec<&str> = block
            .lines()
            .filter(|l| l.contains("cargo clippy") || l.contains("cargo test"))
            .collect();
        assert!(
            resolving.len() >= 2,
            "job {job} shows only {} cargo clippy/test command lines — the block parser is \
             broken or a step was deleted; a lane that resolves nothing proves nothing",
            resolving.len()
        );
        for line in &resolving {
            assert!(
                line.contains("--locked"),
                "job {job} runs `{line}` without --locked: a bare invocation re-locks \
                 SILENTLY when the committed lock drifts from the manifest, and the lane \
                 reports green against dependency versions no one committed (for \
                 signal-gateway that can move the whole libsignal stack, because its git \
                 dependencies are what a re-lock re-resolves). fmt is exempt — cargo fmt \
                 rejects the flag — but clippy and test must pin resolution."
            );
        }
    }
}

/// Law 3 — the git dependencies ride a pinned `rev`, never a `branch`.
///
/// A branch pointer is not a pin: the moment any re-lock occurs (a manifest
/// bump elsewhere in the file, a runner without `--locked`), cargo
/// re-resolves it to whatever upstream merged that morning. The rev makes
/// moving the stack a deliberate act: pick the commit, read what it resolves,
/// re-lock, and bump the package version that tracks the stack — in one
/// reviewed commit.
#[test]
fn git_dependencies_ride_a_pinned_rev() {
    let manifest = read("tools/signal-gateway/Cargo.toml");
    for dep in ["presage", "presage-store-sqlite"] {
        let prefix = format!("{dep} = ");
        let line = manifest
            .lines()
            .find(|l| l.trim_start().starts_with(&prefix))
            .unwrap_or_else(|| {
                panic!(
                    "dependency {dep} not found in tools/signal-gateway/Cargo.toml — the \
                         manifest was reorganised and this law now covers nothing"
                )
            });
        assert!(
            line.contains("rev = \""),
            "git dependency {dep} must be pinned by rev: {line:?}. A branch pointer \
             re-resolves to upstream's current head on ANY re-lock, shipping unreviewed \
             movement of the whole libsignal stack under a green build."
        );
        assert!(
            !line.contains("branch = "),
            "git dependency {dep} must not ride a branch pointer: {line:?}"
        );
    }
}

/// Every `cargo install` in the RELEASE workflow names a version: a release
/// that resolves its toolchain from crates.io HEAD is a release whose SBOM
/// generator (cargo-cyclonedx) or cross-linker can change behaviour between
/// two tags with no diff in this repo. The siblings (cross, mdbook) always
/// pinned; cargo-cyclonedx was the bare one.
#[test]
fn r109_release_workflow_installs_are_version_pinned() {
    let yml = read(".github/workflows/release.yml");
    let installs: Vec<&str> = yml
        .lines()
        .filter(|l| l.contains("cargo install"))
        .collect();
    assert!(
        !installs.is_empty(),
        "the release workflow must still install its tools somewhere"
    );
    for line in &installs {
        assert!(
            line.contains("--version"),
            "release.yml runs an UNPINNED cargo install: {line}"
        );
    }
}

/// R120 "Conduit": the release/supply-chain band, pinned structurally (the
/// CI-shape home — same class as the lanes above). Each arm names the row it
/// closes.
#[test]
fn the_release_and_ci_band_carries_its_gates() {
    let release = read(".github/workflows/release.yml");
    // F4-13: a published release carries a checksum manifest over EVERY
    // staged file, generated before the release step.
    assert!(
        release.contains("sha256sum --binary * > SHA256SUMS"),
        "release.yml must publish a SHA256SUMS manifest (F4-13)"
    );
    // F4-14: the green gate iterates the publication workflows, not ci.yml
    // alone (a shell/codeql red must not ship).
    assert!(
        release.contains("for WF in ci.yml shell.yml codeql.yml fuzz.yml docs.yml"),
        "the release gate must iterate every publication workflow (F4-14)"
    );
    // F4-16: the shell type-check runs AFTER the supply-chain gates.
    let shell = read(".github/workflows/shell.yml");
    let audit = shell.find("pnpm audit --prod").expect("shell audit step");
    let check = shell
        .find("Svelte check (strict)")
        .expect("svelte-check step");
    assert!(
        audit < check,
        "the supply-chain audits must precede the type-check (F4-16: one red \
         check used to skip 13 steps)"
    );
    // F4-22: the model-output-parsing TS ships through CodeQL.
    let codeql = read(".github/workflows/codeql.yml");
    assert!(
        codeql.contains("javascript-typescript"),
        "CodeQL must analyze the plugin/shell TypeScript (F4-22)"
    );
    // F4-17: the link checker is invoked by a lane (it existed, uninvoked,
    // and vacuumed from any cwd but the root).
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("scripts/check-doc-links.py"),
        "ci.yml must invoke the doc-links checker (F4-17)"
    );
    // F4-19: the Docker bases are digest-pinned — a floating tag on the
    // image that holds the IdP client secret (oauth2-proxy) or compiles the
    // shipped binary (rust) is the trust chain's mutable input.
    let dockerfile = read("Dockerfile");
    let compose = read("docker-compose.yml");
    for (image, marker) in [
        ("rust:1-bookworm", "rust:1-bookworm@sha256:"),
        ("debian:bookworm-slim", "debian:bookworm-slim@sha256:"),
        ("oauth2-proxy:latest", "oauth2-proxy:latest@sha256:"),
    ] {
        assert!(
            dockerfile.contains(marker) || compose.contains(marker),
            "the `{image}` base must be digest-pinned (@sha256:…) — Dependabot              opens the bump PRs (F4-19)"
        );
    }
}

/// The release script's pre-tag drift gate (the 2026-10-19 stale-client
/// repair). `shell.yml` — the lane that carries
/// `openapi_types_are_drift_free_vs_the_kernel_contract` — triggers on
/// main-push / pull_request / workflow_dispatch and NEVER on a tag push,
/// private-repo Actions are throttled to zero here, and the release verdict
/// release.yml fail-closes on IS the ci.yml matrix. So the gate was silent
/// across every event a release actually travels: `openapi.yaml` moved in
/// release commits while the shell tree sat untouched, and no trigger ever
/// ran the lane — the drift had been in the tree since `520a1dea`, and the
/// v1.29.22 tag shipped a client whose documented shapes were stale ones.
/// The repair was a regen by hand; the DISCIPLINE is release.sh's drill (1f),
/// regenerate locally + byte-compare + refuse on BOTH sides, before the tag
/// exists.
///
/// **Structural, with the reason recorded.** The drill's regen half needs the
/// shell toolchain (`pnpm exec openapi-typescript`), which the kernel suite
/// does not install — the same class as the R123 carry-in rows. So the
/// behavioral proof is the RUN at release time, recorded in the guard's
/// register row; what cargo holds is the wiring: that the drill is still in
/// the script, that it still runs BEFORE the tag exists, and that an inability
/// to check is a REFUSAL rather than a pass. The third is the one that makes
/// this a gate and not a decoration: a script that skips the comparison when
/// its tool is missing ships exactly the lie the drill exists for.
#[test]
fn the_release_script_runs_the_drift_regen_before_it_tags() {
    let script = read("scripts/release.sh");

    let regen = script
        .find("pnpm exec openapi-typescript ../openapi.yaml -o \"$DRIFT_TMP\"")
        .expect(
            "release.sh must run the schema-drift drill (1f, the 1.29.22 stale-client \
             shape) — regenerating the client and byte-comparing it against the \
             contract BEFORE tagging",
        );

    // The comparison itself, and a refusal that NAMES the defect it refuses.
    assert!(
        script.contains("cmp -s shell/src/lib/api/schema.d.ts \"$DRIFT_TMP\""),
        "the drift drill must BYTE-COMPARE the regenerated client against the committed one — a size check, a timestamp check or a `git diff --quiet` would pass for a client that content-drifted"
    );
    assert!(
        script.contains("STALE against openapi.yaml"),
        "the drill must refuse with a diagnostic that names the staleness — a silent skip is the CI-absence that shipped the v1.29.22 client"
    );

    // Fail-closed on tooling absence: an inability to CHECK must be a refusal,
    // never a pass. Without this arm the drill is only a gate on machines that
    // have already paid for one — exactly the machines that do not need it.
    assert!(
        script.contains("pnpm is unavailable"),
        "release.sh must REFUSE the drift drill it cannot run (missing toolchain), rather than skipping it — a gate that cannot run must be louder than a green one"
    );

    // Order: the drill must run before tag creation. This is the arm that
    // catches a reordering — the trivially plausible edit that moves the
    // sanity checks after the push for "fast feedback" — because a drill
    // placed after `git tag -a` reports a lie to a release that already
    // shipped.
    let tag = script
        .find("git tag -a \"$TAG\"")
        .expect("release.sh must still create the annotated tag");
    assert!(
        regen < tag,
        "the drift drill (at byte {regen}) must run BEFORE `git tag -a` (at byte {tag}) — a drill placed after the tag reports a lie to a release that already shipped"
    );
}

/// F4-21: the audit-ignore list's re-check triggers are now MACHINE-evaluated
/// for the one case a lockfile can decide — an advisory whose crate is GONE
/// from every tracked lockfile is fixed-or-removed, and suppressing it
/// indefinitely is a hole, not a tolerance. Each ignore entry carries its
/// crate inline (see .cargo/audit.toml); the pin parses the pair and walks
/// the tracked lockfiles. Red-proof: deleting a crate from every lock while
/// keeping its ignore fires this by construction (the crate-presence walk
/// IS the assertion).
#[test]
fn no_ignored_advisory_is_fixed_in_the_current_lock() {
    let audit_toml = read(".cargo/audit.toml");
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in audit_toml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("\"RUSTSEC-") {
            let id = format!("RUSTSEC-{}", rest.trim_end_matches(','));
            // the inline crate annotation: `# crate — reason` or `# crate —reason`
            let after = line.split('#').nth(1).unwrap_or("");
            let crate_name = after
                .trim()
                .split([' ', '—', ','])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            assert!(
                !crate_name.is_empty(),
                "{id} carries no inline crate annotation — the staleness pin \
                 cannot evaluate it; annotate `\"{id}\", # <crate> — reason`"
            );
            pairs.push((id, crate_name));
        }
    }
    assert!(
        pairs.len() >= 7,
        "the ignore list shrank without its register row: {} entries",
        pairs.len()
    );
    let locks = [
        "Cargo.lock",
        "shell/src-tauri/Cargo.lock",
        "tools/channel-bridge/Cargo.lock",
        "tools/signal-gateway/Cargo.lock",
        "crates/Cargo.lock",
    ];
    for (id, crate_name) in pairs {
        let present = locks
            .iter()
            .any(|l| read(l).contains(&format!("name = \"{crate_name}\"")));
        assert!(
            present,
            "{id} ignores {crate_name}, but that crate is in NO tracked \
             lockfile — the advisory is fixed-or-removed; prune the ignore \
             (and its register row) instead of suppressing it forever"
        );
    }
}

/// The path-dependency NAMES cargo's own resolver produced for one manifest.
///
/// R123 CONVERSION. This used to be `manifest.contains("crates/<member>")` — a
/// text read, which the register's law 2 calls a regression lock, not proof,
/// because a COMMENT naming the member satisfies it exactly as well as the
/// dependency does. The census now asks cargo instead: only a real path
/// dependency serialises as `"name":"…","source":null`, and a comment cannot
/// make the resolver emit anything.
///
/// `--offline` first because the committed lock is complete and the resolver
/// needs no network for it; the online retry exists so a cold CI cache is not a
/// false red. `cargo metadata` on both manifests costs ~0.1s each here.
///
/// RED-PROOF (run 2026-10-12, reverted). The defect planted was the one the
/// register's law 2 names: a member that LOOKS consumed. A comment —
/// `# crates/brain-care-core = { path = … }` in the root Cargo.toml — plus
/// `brain-care-core` moved from the island list to the consumed list. The OLD
/// text condition (`root_manifest.contains("crates/brain-care-core")`)
/// evaluated TRUE on the comment; this probe returned false and the test
/// failed. That is the directional proof the conversion bought teeth rather
/// than re-describing the same fact. (The other direction is not available as
/// a drill: deleting the real dependency does not compile — `src/workflow/
/// delivery.rs:167` names `brain_consensus_core` — so the compiler, not this
/// pin, is the guard for a member that is used.)
/// `cargo metadata` output per manifest, resolved ONCE.
///
/// The first draft ran `cargo metadata` inside `resolved_path_dep`, so the
/// census below spawned it once per member — fifteen times at ~0.3s each, for
/// two facts about two workspaces. The resolution cannot change within a test,
/// so it is memoised per manifest; the needle stays built from parts.
fn metadata_for(manifest_rel: &str) -> &'static str {
    use std::sync::OnceLock;
    static ROOT: OnceLock<String> = OnceLock::new();
    static HARNESS: OnceLock<String> = OnceLock::new();
    let cell = if manifest_rel == "Cargo.toml" {
        &ROOT
    } else {
        &HARNESS
    };
    cell.get_or_init(|| resolve_metadata(manifest_rel)).as_str()
}

fn resolve_metadata(manifest_rel: &str) -> String {
    for offline in [true, false] {
        let mut cmd = std::process::Command::new("cargo");
        cmd.args([
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            manifest_rel,
        ]);
        if offline {
            cmd.arg("--offline");
        }
        let Ok(out) = cmd.output() else {
            panic!("cargo must be on PATH to resolve {manifest_rel}");
        };
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).into_owned();
        }
    }
    panic!(
        "cargo metadata could not resolve {manifest_rel} offline OR online — \
         the islands census must fail closed, never pass on a resolver it did \
         not get to run"
    )
}

fn resolved_path_dep(manifest_rel: &str, member: &str) -> bool {
    // Built from parts so this file's own source cannot satisfy the needle.
    let needle = format!("\"na{}\":\"{}\",\"source\":null", "me", member);
    metadata_for(manifest_rel).contains(&needle)
}

/// F4-18: the crate-islands census, now resolved by cargo rather than read as
/// text (R123). A `crates/` member that no workspace consumes is either a live
/// island (documented, per-entry reason) or dead weight waiting to rot — this
/// pins the SET, so a member joining or leaving either list is a named act.
/// Consumed = cargo resolves a PATH dependency on it, from the root workspace
/// or from the steward-harness workspace (a separate workspace, so it needs
/// its own resolution; that is why one list is not enough).
#[test]
fn crates_consumed_set_is_pinned() {
    // The consumed census: the crates/ members cargo resolves a path edge to
    // (measured by cargo, not grep-verified — 2026-10-12).
    let consumed = [
        "brain-engine-sdk",        // root + steward-harness (harness-kernel feature)
        "brain-evidence-core",     // root
        "legal-rules-db",          // root
        "brain-delivery-core",     // root
        "brain-consensus-core",    // root
        "brain-executor-core",     // root
        "brain-evolve-core",       // root
        "brain-troubleshoot-core", // steward-harness
        "gold-sets",               // steward-harness
    ];
    for member in consumed {
        assert!(
            resolved_path_dep("Cargo.toml", member)
                || resolved_path_dep("tools/steward-harness/Cargo.toml", member),
            "`{member}` is listed as consumed but cargo resolves NO path \
             dependency on it from either workspace — update the census"
        );
    }

    // The ISLANDS: unconsumed, with the per-entry reason the register owes.
    // `brain-fuzz` is the nightly-only fuzz workspace (its own Cargo.lock is
    // gitignored by design); the three *-core members are domain engines
    // reserved by the product roadmap, unwired deliberately.
    let islands = [
        "brain-aftersales-core",
        "brain-care-core",
        "brain-interview-core",
        "brain-fuzz",
    ];
    // ANTI-VACUITY: the probe above can only fail if it can SEE. A needle that
    // never matches would make every island arm pass vacuously, so the census
    // names one member cargo is known to resolve and requires the probe to find
    // it. If cargo's metadata format ever stops serialising path deps this way,
    // this arm fails instead of the census quietly emptying.
    assert!(
        resolved_path_dep("Cargo.toml", "brain-engine-sdk")
            && resolved_path_dep("tools/steward-harness/Cargo.toml", "gold-sets"),
        "the path-dependency probe found nothing in either workspace — it is \
         reading the wrong shape of `cargo metadata`, not reporting an empty \
         dependency graph. Every island arm above would be passing vacuously."
    );

    for member in islands {
        assert!(
            !resolved_path_dep("Cargo.toml", member)
                && !resolved_path_dep("tools/steward-harness/Cargo.toml", member),
            "`{member}` is listed as an ISLAND but cargo resolves a path \
             dependency on it from a workspace — move it to the consumed census"
        );
    }
}

/// Every tracked `Cargo.toml`, excluding build output and vendored trees.
/// A walk rather than a list, so a manifest added tomorrow is covered today.
fn manifests() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), "target" | "node_modules" | ".git" | "dist") {
                continue;
            }
            let path = e.path();
            if path.is_dir() {
                walk(&path, out);
            } else if name == "Cargo.toml"
                && let Ok(rel) = path.strip_prefix(repo(""))
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo(""), &mut out);
    out.sort();
    out
}

/// The repo does not COMPILE a QUIC stack, and that fact is pinned rather
/// than left to the lockfile's appearance.
///
/// quinn-proto sits in five tracked lockfiles and appears in no compiled
/// artifact: it reaches them only as an OPTIONAL dependency of reqwest 0.13
/// behind reqwest's `http3` feature, which no workspace enables. Measured
/// 2026-10-13 — `cargo tree --all-targets` resolves zero quinn packages in
/// every workspace, and `target/{debug,release}/deps` hold no quinn rlib.
///
/// This pin was written for a different reason and outlived it by one merge,
/// which is worth recording because the sequence is the point. It first
/// justified three `quinn-proto` audit ignores (RUSTSEC-2026-0343/0338/0340,
/// three remotely-triggerable QUIC panics). The re-lock to quinn-proto
/// 0.11.19 FIXED all three, at which point the ignores were dead
/// suppressions and were pruned in the same breath — which is the rule the
/// staleness pin above already enforces for a crate leaving every lockfile,
/// applied here by hand for an advisory a re-lock fixed. What survived is
/// the invariant underneath: we do not build QUIC, so its CVE surface is out
/// of scope by construction and a future quinn advisory needs no ignore.
///
/// Two arms, split by what each can afford to prove:
///
///   1. TEXT — no tracked Cargo.toml may name the `http3` feature. This is the
///      only thing OUR side controls, it costs nothing, and it is the edit
///      that would actually be made. It WALKS the tree rather than listing
///      manifests, and that is not decoration: the first draft listed seven
///      workspace-root manifests, and its own drill planted `http3` in
///      `crates/brain-care-core/Cargo.toml` and the pin stayed GREEN. Fourteen
///      crate manifests and the Tauri client were invisible. A guard
///      defeatable by a manifest it never looked at is not a guard.
///   2. ACTIVATION — the root workspace's `cargo tree` must contain no quinn.
///      This is the only arm that reads what is actually COMPILED (0.18s).
///
///      WHY NOT `cargo metadata`, which the islands census already pays for:
///      measured 2026-10-13, `cargo metadata` lists quinn, quinn-proto and
///      quinn-udp in `packages[]` AND in `resolve.nodes` — even with
///      `--filter-platform`, and even when `cargo tree` reports zero. Its
///      package universe is a SUPERSET covering unactivated optional
///      dependencies; it answers "could this build", not "does this build". A
///      guard built on it would have been permanently red over a true fact.
///
/// RED-PROOF (run 2026-10-13, reverted by file copy): `http3` appended to
/// `crates/brain-care-core/Cargo.toml` fires arm 1 naming the file, and the
/// same token in `shell/src-tauri/Cargo.toml` — a manifest no hardcoded list
/// named — fires it too.
///
/// DRILL THAT CANNOT BE RUN, recorded so nobody retries it: enabling `http3`
/// on reqwest for real never reaches this pin, because reqwest refuses to
/// compile ("the `http3` feature is unstable, and requires
/// `RUSTFLAGS='--cfg reqwest_unstable'`"). Arm 2 is therefore unproven by
/// plant; its ground truth is stated and re-checkable — `cargo tree
/// --prefix none --all-targets` reports zero quinn, and no quinn rlib exists
/// under `target/`.
#[test]
fn no_workspace_enables_http3() {
    // WALK, do not enumerate. The first draft listed seven workspace-root
    // manifests and its own drill planted `http3` in
    // `crates/brain-care-core/Cargo.toml` — a manifest the list did not name —
    // and the pin stayed green over the very edit it exists to catch. Fourteen
    // crate manifests and the Tauri client were invisible. A guard that can be
    // defeated by a manifest it never looked at is not a guard.
    for rel in manifests() {
        let text = read(&rel);
        assert!(
            !text.contains("http3"),
            "{rel} names reqwest's `http3` feature. That COMPILES quinn-proto, \\
             and the three RUSTSEC-2026-03xx ignores in .cargo/audit.toml are \\
             justified by its being unreachable — so enabling it here makes \\
             those ignores false. Re-read them (and bump the crate) instead of \\
             letting the ignore outlive its reason."
        );
    }

    // Built from parts so this file's own source cannot satisfy the needle.
    let needle = format!("qu{}n", "pro");
    let out = std::process::Command::new("cargo")
        .args([
            "tree",
            "--offline",
            "--manifest-path",
            "Cargo.toml",
            "--prefix",
            "none",
            "--all-targets",
        ])
        .output();
    let Ok(out) = out else {
        panic!("cargo must be on PATH to resolve the activation graph");
    };
    let tree = String::from_utf8_lossy(&out.stdout);
    assert!(
        !tree.contains(&needle),
        "the root workspace's ACTIVATION graph now contains quinn-proto, so the \
         QUIC stack IS compiled into a shipped artifact. The RUSTSEC-2026-03xx \
         ignores in .cargo/audit.toml are no longer justified by reachability — \
         bump quinn-proto or drop the ignores."
    );
}

/// The Tauri IPC boundary is a cross-language contract, and nothing enforced
/// it.
///
/// Tauri generates the JavaScript side of the IPC contract from the RUST crate.
/// `@tauri-apps/api` and the `tauri` crate are published from one repository
/// and are expected to move together; when they drift, the JS side calls
/// commands the installed Rust side does not expose, and the failure appears at
/// runtime in the webview rather than at compile time.
///
/// This drifted to three different patch releases across the boundary (crate
/// 2.12.1, API 2.12.3, CLI 2.12.2) with no test, script or CI step looking at
/// either side. The invariant is major.MINOR, not patch: the crate's published
/// contract is a minor-version line, and a patch on either side is a
/// same-release bump by construction.
#[test]
fn the_tauri_rust_and_javascript_sides_agree_on_their_minor_version() {
    // The RESOLVED crate version, not the manifest floor: the floor is a
    // lower bound that a re-lock may legitimately raise, and comparing floors
    // would let a stale floor pass while the built artifact drifts.
    let lock = read("shell/src-tauri/Cargo.lock");
    let crate_version = lock
        .split("[[package]]")
        .find(|blk| blk.contains("name = \"tauri\"\n"))
        .and_then(|blk| {
            blk.lines()
                .find_map(|l| l.strip_prefix("version = \""))
                .and_then(|v| v.split('"').next())
        })
        .unwrap_or_else(|| panic!("tauri must be present in shell/src-tauri/Cargo.lock"));

    let pkg = read("shell/package.json");
    let api_floor = pkg
        .lines()
        .find(|l| l.contains("\"@tauri-apps/api\""))
        .and_then(|l| l.split_once('^').map(|(_, v)| v))
        .and_then(|v| v.split('"').next())
        .unwrap_or_else(|| {
            panic!(
                "@tauri-apps/api must be declared in shell/package.json; a floor of \
                    nothing means this pin is comparing against a vacuum"
            )
        });

    let minor = |v: &str| {
        let mut it = v.split('.');
        match (it.next(), it.next()) {
            (Some(maj), Some(min)) => format!("{maj}.{min}"),
            _ => panic!("version {v:?} has no major.minor to compare"),
        }
    };
    assert_eq!(
        minor(crate_version),
        minor(api_floor),
        "the Tauri IPC boundary is skewed: the Rust crate resolves to {crate_version} but \
         @tauri-apps/api declares a floor of {api_floor}. These are published together and \
         the JS side's command surface is generated from the crate, so a minor skew is a \
         runtime failure inside the webview that no compile step in this repo can see. \
         Move both to the same release."
    );
}

/// The same contract names the CLI, which builds and signs the binary. Its
/// floor is allowed to trail the API by a patch (the CLI is a build-time tool
/// and the two are versioned on different cadences), but it must never lead it
/// and must never sit on a different minor.
#[test]
fn the_tauri_cli_never_leads_the_api_it_builds() {
    let pkg = read("shell/package.json");
    let declared = |name: &str| {
        pkg.lines()
            .find(|l| l.contains(&format!("\"{name}\"")))
            .and_then(|l| l.split_once('^').map(|(_, v)| v))
            .and_then(|v| v.split('"').next())
            .map(|s| s.to_string())
            .unwrap_or_else(|| panic!("{name} must be declared in shell/package.json"))
    };
    let api = declared("@tauri-apps/api");
    let cli = declared("@tauri-apps/cli");
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|x| x.parse().unwrap_or(0)).collect() };
    let (a, c) = (parts(&api), parts(&cli));
    assert!(
        c[0] == a[0] && c[1] == a[1],
        "the Tauri CLI ({cli}) is on a different minor from the API it builds against \
         ({api})"
    );
    assert!(
        parts(&cli) <= parts(&api),
        "the Tauri CLI ({cli}) LEADS the API ({api}). A CLI newer than the runtime contract \
         it was built against is the one skew that cannot work."
    );
}

/// The two-compiler arrangement is deliberate, and it is the single most
/// breakable line in the shell's dependency set.
///
/// The shell typechecks with TypeScript 7's native compiler and lints with
/// TypeScript 6. `svelte-check --tsgo` is the authority for types;
/// `@typescript-eslint/parser` requires the previous compiler's API. Running
/// the two as one version means choosing: on 7, the typecheck is right and the
/// linter dies; on 6, the reverse.
///
/// The incompatibility is HARD, not advisory. Verified 2026-10-15 by planting
/// it rather than reading it: raising `typescript` to 7 makes `pnpm lint` exit
/// 2 with "typescript-eslint does not support TS 7.0." thrown from the parser
/// at import time, before a single file is read. The published note about
/// TS7 dropping the server-plugin API is scoped to editor language servers and
/// understates this — the CLI refuses too.
///
/// This is a pin because `pnpm update --latest` resolves both specifiers to the
/// same version. It did exactly that during the 1.29.16 sweep, and the only
/// thing that caught it was noticing the typecheck result afterwards. A
/// dependency bump is not a reviewer of intent.
#[test]
fn the_two_compiler_arrangement_is_held_apart_and_stays_held() {
    let pkg = read("shell/package.json");
    let declared = |name: &str| {
        pkg.lines()
            .find(|l| l.contains(&format!("\"{name}\"")))
            .map(|l| l.trim().trim_end_matches(',').to_string())
            .unwrap_or_else(|| panic!("{name} must be declared in shell/package.json"))
    };
    let tooling = declared("typescript");
    let native = declared("@typescript/native");

    // The native compiler is an ALIAS of the typescript package at the 7.x
    // line. If that stops being true the arrangement has changed shape and
    // this pin's reasoning no longer applies — so assert the shape first.
    assert!(
        native.contains("npm:typescript@"),
        "@typescript/native must remain an alias of the typescript package, found {native:?}. \
         If this changed, re-derive the arrangement rather than trusting this test."
    );
    let native_major = native
        .split("npm:typescript@")
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|m| m.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("could not read a version out of {native:?}"));
    let tooling_floor = tooling
        .split_once('^')
        .map(|(_, v)| v.trim_matches(|c| c == '"' || c == ',').to_string())
        .unwrap_or_else(|| panic!("typescript must carry a caret floor, found {tooling:?}"));
    let tooling_major = tooling_floor
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("could not read a major from {tooling_floor:?}"));

    assert_eq!(
        tooling_major,
        native_major - 1,
        "the tooling compiler must sit exactly one major behind the native one \
         ({tooling_floor} vs {native}). Equal majors is the shape \
         `pnpm update --latest` produces, and it breaks `pnpm lint` outright."
    );
    assert!(
        pkg.contains("--tsgo"),
        "the check script must invoke the native compiler, or the 7.x dependency is \
         installed and never run"
    );
}
