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
