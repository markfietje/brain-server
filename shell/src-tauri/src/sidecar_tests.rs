//! III-3 — the kernel contract's pins: detect · start · refuse.
//!
//! Every pin drives the REAL function in `sidecar.rs`:
//!
//!   * the probe pins drive a loopback `TcpListener` stub (no kernel, no
//!     network beyond 127.0.0.1);
//!   * the consent/auth pins drive temp directories (no user data);
//!   * the shutdown pin spawns a sleeper this test owns (no keychain, no
//!     sidecar);
//!   * the manifest/command posture pins read the committed artifacts.
//!
//! No pin constructs the login keychain, starts the real sidecar, binds the
//! real port, or reads the operator's data. The started path's Tauri edge is
//! exercised by the UI pins against an IPC stub; the refusal path it shares
//! is pinned here.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::sidecar::{
    self, BIND, ContractError, Decision, KernelState, PORT, PROVENANCE_REL, Preflight, Probe,
    ProbeOutcome, WaitOutcome,
};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp directory removed on drop. No `tempfile` edge was added: the shell
/// crate stays dependency-free for this round.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let counter = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "brain-shell-iii3-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

/// One-shot loopback HTTP stub. It accepts exactly one connection, reads the
/// request (so the client never sees a reset mid-write), writes the canned
/// response and closes.
fn stub(response: &'static str) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stub binds loopback");
    let port = listener.local_addr().expect("stub address").port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (port, handle)
}

// ── P1 ─────────────────────────────────────────────────────────────────────

#[test]
fn a_version_mismatch_names_both_versions() {
    let probe = Probe {
        api_version: "9.9.9".to_string(),
        body_version: "9.9.9".to_string(),
    };
    let decision = sidecar::decide("1.29.23", &probe);
    assert_eq!(
        decision,
        Decision::Mismatch {
            expected: "1.29.23".to_string(),
            observed: "9.9.9".to_string(),
        }
    );
    let text = decision.to_string();
    assert!(
        text.contains("1.29.23"),
        "the expected version is absent: {text}"
    );
    assert!(
        text.contains("9.9.9"),
        "the observed version is absent: {text}"
    );

    // Anti-vacuity: the SAME probe under the matching expectation is Ready,
    // so the pin is not satisfied by a decide that refuses everything.
    assert_eq!(sidecar::decide("9.9.9", &probe), Decision::Ready);
}

// ── P2 ─────────────────────────────────────────────────────────────────────

#[test]
fn a_header_body_disagreement_refuses() {
    let (port, server) = stub(concat!(
        "HTTP/1.1 200 OK\r\n",
        "x-api-version: 1.0.0\r\n",
        "Connection: close\r\n\r\n",
        "{\"status\":\"ok\",\"version\":\"2.0.0\"}"
    ));
    let outcome = sidecar::probe(port, Duration::from_millis(500));
    server.join().expect("stub thread");
    assert_eq!(
        outcome,
        ProbeOutcome::Inconsistent {
            header: "1.0.0".to_string(),
            body: "2.0.0".to_string(),
        }
    );

    // The header name is matched case-insensitively, and an agreeing answer
    // is a Probe (anti-vacuity for the parser).
    let (port, server) = stub(concat!(
        "HTTP/1.1 200 OK\r\n",
        "X-Api-Version: 1.29.23\r\n",
        "Connection: close\r\n\r\n",
        "{\"status\":\"ok\",\"version\":\"1.29.23\"}"
    ));
    let outcome = sidecar::probe(port, Duration::from_millis(500));
    server.join().expect("stub thread");
    assert_eq!(
        outcome,
        ProbeOutcome::Probe(Probe {
            api_version: "1.29.23".to_string(),
            body_version: "1.29.23".to_string(),
        })
    );
}

// ── P3 ─────────────────────────────────────────────────────────────────────

#[test]
fn a_foreign_listener_is_not_adopted() {
    let (port, server) = stub(concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: application/json\r\n",
        "Connection: close\r\n\r\n",
        "{\"status\":\"ok\",\"version\":\"1.0.0\"}"
    ));
    let outcome = sidecar::probe(port, Duration::from_millis(500));
    server.join().expect("stub thread");
    assert_eq!(outcome, ProbeOutcome::Foreign);

    let scratch = Scratch::new("foreign");
    let present = scratch.path().join("brain-server");
    fs::write(&present, b"stub").expect("write stub binary");

    // A foreign listener refuses BEFORE the sidecar's presence matters: the
    // decision cannot reach a spawn.
    match sidecar::preflight("1.29.23", outcome, &present) {
        Preflight::Refuse(ContractError::ForeignListener { port: seen }) => {
            assert_eq!(seen, PORT);
        }
        other => panic!("a foreign listener must refuse, got {other:?}"),
    }

    // NotRunning is the ONLY start signal, and a missing binary refuses.
    let missing = scratch.path().join("absent");
    assert!(matches!(
        sidecar::preflight("1.29.23", ProbeOutcome::NotRunning, &missing),
        Preflight::Refuse(ContractError::MissingSidecar { .. })
    ));
    assert_eq!(
        sidecar::preflight("1.29.23", ProbeOutcome::NotRunning, &present),
        Preflight::Start
    );

    // A port nobody holds is NotRunning — the anti-vacuity arm for the probe
    // (a probe that always reported Foreign would pass the arms above).
    let closed = TcpListener::bind(("127.0.0.1", 0)).expect("probe bind");
    let closed_port = closed.local_addr().expect("probe addr").port();
    drop(closed);
    assert_eq!(
        sidecar::probe(closed_port, Duration::from_millis(200)),
        ProbeOutcome::NotRunning
    );
}

// ── P4 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_start_wait_is_bounded() {
    let started = Instant::now();
    let outcome = sidecar::wait_bounded(|| ProbeOutcome::NotRunning, "1.29.23", 150, 200);
    let elapsed = started.elapsed();
    match outcome {
        WaitOutcome::Timeout { waited_ms, .. } => {
            assert!(
                (100..1000).contains(&waited_ms),
                "the wait reported {waited_ms} ms outside its 150 ms bound"
            );
        }
        other => panic!("a silent stub must time out, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(1),
        "the bounded wait took {elapsed:?}"
    );

    // Anti-vacuity: a matching probe leaves the loop immediately.
    let started = Instant::now();
    let outcome = sidecar::wait_bounded(
        || {
            ProbeOutcome::Probe(Probe {
                api_version: "1.29.23".to_string(),
                body_version: "1.29.23".to_string(),
            })
        },
        "1.29.23",
        5_000,
        200,
    );
    assert_eq!(
        outcome,
        WaitOutcome::Ready {
            version: "1.29.23".to_string()
        }
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

// ── P5 ─────────────────────────────────────────────────────────────────────

#[test]
fn consent_refused_creates_no_data_root() {
    let scratch = Scratch::new("consent-no");
    let config = scratch.path().join("config");
    let data_root = scratch.path().join("kernel");

    assert_eq!(
        sidecar::consent_gate(&config, &data_root, false),
        Err(ContractError::ConsentRefused)
    );
    assert!(
        !config.exists(),
        "a refusal wrote the consent config directory"
    );
    assert!(!data_root.exists(), "a refusal created the data root");
}

// ── P6 ─────────────────────────────────────────────────────────────────────

#[test]
fn consent_granted_creates_a_0700_data_root() {
    let scratch = Scratch::new("consent-yes");
    let config = scratch.path().join("config");
    let data_root = scratch.path().join("kernel");

    let marker = sidecar::consent_gate(&config, &data_root, true).expect("consent writes");
    assert_eq!(mode(&data_root), 0o700, "the data root is not owner-only");
    assert_eq!(
        mode(&config),
        0o700,
        "the config directory is not owner-only"
    );
    assert_eq!(mode(&marker), 0o600, "the consent marker is not 0600");
    assert!(sidecar::consent_granted(&config));
    let body = fs::read_to_string(&marker).expect("marker is readable");
    assert!(body.contains("\"granted\":true"), "marker body: {body}");

    // Idempotent, and an existing looser root is tightened back to 0700.
    fs::set_permissions(&data_root, fs::Permissions::from_mode(0o777)).expect("loosen");
    sidecar::consent_gate(&config, &data_root, true).expect("second consent");
    assert_eq!(mode(&data_root), 0o700);
}

// ── P7 ─────────────────────────────────────────────────────────────────────

#[test]
fn a_world_readable_auth_file_refuses() {
    let scratch = Scratch::new("auth-mode");
    let data_root = scratch.path().join("kernel");
    fs::create_dir_all(&data_root).expect("data root");
    fs::set_permissions(&data_root, fs::Permissions::from_mode(0o700)).expect("root mode");

    let path = data_root.join(sidecar::AUTH_FILE);
    fs::write(&path, "old-token\n").expect("pre-existing auth file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("loosen auth file");

    match sidecar::prepare_auth_file(&data_root, "new-token") {
        Err(ContractError::InsecureAuthFile {
            path: refused,
            mode: 0o644,
        }) => {
            assert_eq!(refused, path.display().to_string());
        }
        other => panic!("a 0644 auth file must refuse, got {other:?}"),
    }

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("tighten auth file");
    let written = sidecar::prepare_auth_file(&data_root, "new-token").expect("0600 passes");
    assert_eq!(written, path);
    assert_eq!(
        fs::read_to_string(&path).expect("auth file is readable"),
        "new-token\n"
    );
    assert_eq!(mode(&path), 0o600);

    // A fresh file is created at 0600, which is the normal first-run path.
    let fresh_root = scratch.path().join("fresh");
    let fresh = sidecar::prepare_auth_file(&fresh_root, "fresh-token").expect("fresh auth file");
    assert_eq!(mode(&fresh), 0o600);
}

// ── P8 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_sidecar_command_is_loopback_and_carries_the_paths() {
    let scratch = Scratch::new("command");
    let sidecar_path = scratch
        .path()
        .join("Contents")
        .join("MacOS")
        .join("brain-server");
    let data_root = scratch.path().join("kernel");
    let auth_file = data_root.join(sidecar::AUTH_FILE);

    let spec = sidecar::command_spec(&sidecar_path, &data_root, &auth_file);
    assert_eq!(spec.program, sidecar_path.display().to_string());
    assert!(
        spec.args.is_empty(),
        "the kernel takes no argv: {:?}",
        spec.args
    );
    assert!(
        !spec.program.ends_with("/sh"),
        "the command is the sidecar, not a shell: {}",
        spec.program
    );

    let env: std::collections::BTreeMap<String, String> = spec.env.into_iter().collect();
    assert_eq!(env.len(), 7, "the env allowlist is exactly seven keys");
    assert_eq!(env.get("BIND_HOST").map(String::as_str), Some(BIND));
    assert_eq!(env.get("BIND_PORT").map(String::as_str), Some("8765"));
    assert_eq!(
        env.get("BRAIN_DATA_ROOT").map(String::as_str),
        data_root.to_str()
    );
    assert_eq!(
        env.get("BRAIN_DB_PATH").map(String::as_str),
        data_root.join("brain.db").to_str()
    );
    assert_eq!(
        env.get("AUTH_TOKEN_FILE").map(String::as_str),
        auth_file.to_str()
    );
    assert_eq!(env.get("BRAIN_REQUIRE_AUTH").map(String::as_str), Some("1"));
    assert_eq!(env.get("RUST_LOG").map(String::as_str), Some("warn"));

    // No inherited loopback escape hatch, no operator credential, no PATH.
    assert!(!env.contains_key("BIND_PUBLIC"));
    assert!(!env.contains_key("AUTH_TOKEN"));
    assert!(!env.contains_key("BRAIN_TOKEN_FILE"));
    assert!(!env.contains_key("PATH"));
}

// ── P9 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_sidecar_path_is_the_bundle_macos_path() {
    let path = sidecar::resolve_sidecar_path(Path::new(
        "/Applications/Brain Shell.app/Contents/MacOS/brain-shell",
    ));
    assert_eq!(
        path,
        PathBuf::from("/Applications/Brain Shell.app/Contents/MacOS/brain-server")
    );
    assert!(path.ends_with("Contents/MacOS/brain-server"));
}

// ── P10 ────────────────────────────────────────────────────────────────────

#[test]
fn shutdown_kills_the_child_it_owns() {
    let state = KernelState::default();
    let child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .expect("spawn a sleeper this test owns");
    let pid = child.id();
    *state.child.lock().expect("child lock") = Some(child);

    // The kill is the subject: a shutdown that only waited would return after
    // the sleeper's own five seconds. The bound is what tells them apart.
    let started = Instant::now();
    let report = sidecar::shutdown(&state);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "shutdown waited {elapsed:?}; the child was not killed"
    );
    assert!(
        report.status.is_some(),
        "the child was not reaped: {report:?}"
    );
    assert!(report.kill_error.is_none(), "kill failed: {report:?}");
    assert!(state.child.lock().expect("child lock").is_none());

    let alive = std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .status()
        .expect("run kill -0");
    assert!(
        !alive.success(),
        "the child {pid} is still alive after shutdown"
    );

    // Idempotent: a second shutdown holds nothing and does nothing.
    let again = sidecar::shutdown(&state);
    assert!(again.status.is_none());
    assert!(again.kill_error.is_none());
}

// ── S1 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_expected_version_is_the_sealed_provenance() {
    let scratch = Scratch::new("manifest");
    let manifest = scratch.path().join("SIDECAR_PROVENANCE.json");
    fs::write(
        &manifest,
        r#"{"schema":1,"kernel_version":"9.9.9","x_api_version":"9.9.9","kernel_commit":"deadbeef","source_dirty":false}"#,
    )
    .expect("write manifest fixture");
    let provenance = sidecar::load_provenance(&manifest).expect("the record parses");
    assert_eq!(provenance.x_api_version, "9.9.9");
    assert_eq!(provenance.kernel_version, "9.9.9");

    // A prerelease is a valid kernel version shape.
    fs::write(
        &manifest,
        r#"{"kernel_version":"1.29.23-rc.1","x_api_version":"1.29.23-rc.1"}"#,
    )
    .expect("write prerelease fixture");
    assert!(sidecar::load_provenance(&manifest).is_ok());

    // Every malformed shape refuses.
    for body in [
        r#"{"kernel_version":"1.29.23","x_api_version":""}"#,
        r#"{"kernel_version":"1.29","x_api_version":"1.29"}"#,
        r#"{"kernel_version":"  ","x_api_version":"1.29.23"}"#,
    ] {
        fs::write(&manifest, body).expect("write malformed fixture");
        assert!(
            matches!(
                sidecar::load_provenance(&manifest),
                Err(ContractError::MissingProvenance { .. })
            ),
            "the malformed record passed: {body}"
        );
    }
    assert!(matches!(
        sidecar::load_provenance(&scratch.path().join("absent.json")),
        Err(ContractError::MissingProvenance { .. })
    ));

    // A record whose two sealed fields DISAGREE is not a broken record. The
    // sealer checks `kernel_version` against the kernel's Cargo version and
    // `x_api_version` against the wire's openapi version, each against its own
    // source — it does not require the two to be equal. So the shell reads
    // the record, takes `x_api_version` as the expectation, and lets the
    // contract report the skew as the NAMED mismatch it is.
    fs::write(
        &manifest,
        r#"{"kernel_version":"1.29.23","x_api_version":"9.9.9"}"#,
    )
    .expect("write skewed fixture");
    let skewed = sidecar::load_provenance(&manifest).expect("a skewed record still parses");
    assert_eq!(skewed.kernel_version, "1.29.23");
    assert_eq!(skewed.x_api_version, "9.9.9");
    let decision = sidecar::decide(
        &skewed.x_api_version,
        &Probe {
            api_version: "1.29.23".to_string(),
            body_version: "1.29.23".to_string(),
        },
    );
    match &decision {
        Decision::Mismatch { expected, observed } => {
            assert_eq!(expected, "9.9.9");
            assert_eq!(observed, "1.29.23");
            let text = decision.to_string();
            assert!(text.contains("9.9.9") && text.contains("1.29.23"));
        }
        other => panic!("a skewed record must be a named mismatch, got {other:?}"),
    }

    // The expected version's source is the MANIFEST, never the shell's own
    // package.json. The needle is assembled so the scan cannot match itself.
    let source = include_str!("sidecar.rs");
    let package_json = ["package", ".json"].concat();
    assert!(
        !source.contains(&package_json),
        "sidecar.rs must not read {package_json}"
    );
    assert_eq!(PROVENANCE_REL, "binaries/SIDECAR_PROVENANCE.json");
    let macos_overlay = include_str!("../tauri.macos.conf.json");
    assert!(
        macos_overlay.contains(PROVENANCE_REL),
        "the bundle must ship the sealed provenance at {PROVENANCE_REL}"
    );
}

// ── S2 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_shell_has_no_shell_plugin() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("tauri-plugin-shell"),
        "the shell plugin must not be a dependency"
    );

    let capabilities: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/main.json"))
            .expect("capabilities/main.json parses");
    let permissions = capabilities
        .get("permissions")
        .and_then(|value| value.as_array())
        .expect("the capability declares permissions");
    assert_eq!(
        permissions.len(),
        1,
        "the main capability stays core:default only"
    );
    for permission in permissions {
        let name = permission.as_str().expect("a permission is a string");
        for family in ["shell:", "process:", "fs:", "http:"] {
            assert!(
                !name.starts_with(family),
                "the capability must not grant {family} ({name})"
            );
        }
    }
}

// ── S3 ─────────────────────────────────────────────────────────────────────

#[test]
fn the_shell_adds_no_http_client() {
    let manifest = include_str!("../Cargo.toml");
    for dependency in ["reqwest", "ureq", "hyper", "isahc", "attohttpc", "curl"] {
        assert!(
            !manifest.contains(dependency),
            "an HTTP client edge was added: {dependency}"
        );
    }

    // The probe is the raw stream; the source names no client library.
    let source = include_str!("sidecar.rs");
    assert!(source.contains("TcpStream"), "the probe is a raw stream");
    for dependency in ["reqwest", "ureq", "hyper"] {
        assert!(!source.contains(dependency));
    }
}

// ── the second pass: silence, attach, and the refusals that write nothing ──

/// P13 — a kernel that binds the port BEFORE it can answer is still a kernel.
/// The probe's 300 ms timeout sees an empty socket, which is silence, not a
/// wrong answer: the bounded wait must keep waiting for it instead of
/// refusing it as a foreign listener and killing the child it just spawned.
#[test]
fn a_kernel_that_answers_after_the_probe_timeout_is_waited_for() {
    const ANSWER: &str = concat!(
        "HTTP/1.1 200 OK\r\n",
        "x-api-version: 1.29.23\r\n",
        "Connection: close\r\n\r\n",
        "{\"status\":\"ok\",\"version\":\"1.29.23\"}"
    );

    // The stub accepts the first connection and holds it silent past the
    // probe's timeout (the boot), then answers every later request.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stub binds loopback");
    listener
        .set_nonblocking(true)
        .expect("the stub listens without blocking");
    let port = listener.local_addr().expect("stub address").port();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        let stop = server_stop;
        let mut first = true;
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0u8; 1024];
                    let _ = stream.read(&mut request);
                    if first {
                        first = false;
                        std::thread::sleep(Duration::from_millis(600));
                        continue;
                    }
                    let _ = stream.write_all(ANSWER.as_bytes());
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    let outcome = sidecar::wait_bounded(
        || sidecar::probe(port, Duration::from_millis(300)),
        "1.29.23",
        5_000,
        200,
    );
    stop.store(true, Ordering::Relaxed);
    server.join().ok();
    match outcome {
        WaitOutcome::Ready { version } => assert_eq!(version, "1.29.23"),
        other => panic!("a late answer must be waited for, got {other:?}"),
    }

    // Anti-vacuity: a WRONG answer still ends the wait at once. Silence is
    // the only thing that is waited through.
    let (port, server) = stub(concat!(
        "HTTP/1.1 200 OK\r\n",
        "x-api-version: 9.9.9\r\n",
        "Connection: close\r\n\r\n",
        "{\"status\":\"ok\",\"version\":\"9.9.9\"}"
    ));
    let started = Instant::now();
    let outcome = sidecar::wait_bounded(
        || sidecar::probe(port, Duration::from_millis(300)),
        "1.29.23",
        5_000,
        200,
    );
    server.join().expect("stub thread");
    assert!(
        matches!(
            outcome,
            WaitOutcome::Refused(ContractError::VersionMismatch { .. })
        ),
        "a wrong answer must refuse, got {outcome:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

/// P14 — a port that is held but SILENT is never spawned over. The refusal is
/// the same shape as a foreign listener's, because from outside the contract
/// a port that does not answer as a Brain kernel is not ours to touch.
#[test]
fn a_silent_listener_is_refused_before_any_start() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stub binds loopback");
    listener
        .set_nonblocking(true)
        .expect("the stub listens without blocking");
    let port = listener.local_addr().expect("stub address").port();
    let stop = Arc::new(AtomicBool::new(false));
    let silent_stop = stop.clone();
    let silent = std::thread::spawn(move || {
        let stop = silent_stop;
        let mut held = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => held.push(stream),
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    let outcome = sidecar::probe(port, Duration::from_millis(200));
    assert_eq!(outcome, ProbeOutcome::Unanswered);
    stop.store(true, Ordering::Relaxed);
    silent.join().ok();

    let scratch = Scratch::new("silent");
    let present = scratch.path().join("brain-server");
    fs::write(&present, b"stub").expect("write stub binary");
    match sidecar::preflight("1.29.23", outcome, &present) {
        Preflight::Refuse(ContractError::ForeignListener { port: seen }) => {
            assert_eq!(seen, PORT);
        }
        other => panic!("a held port must refuse, got {other:?}"),
    }
}

/// P15 — attach, never double-start. A matching running kernel is adopted
/// even when the bundled binary is absent (the shell does not need the bundle
/// to attach), and a DIFFERENT version is refused rather than adopted or
/// spawned over.
#[test]
fn a_matching_kernel_is_attached_and_never_respawned() {
    let scratch = Scratch::new("attach");
    let absent = scratch.path().join("absent");

    match sidecar::preflight(
        "1.29.23",
        ProbeOutcome::Probe(Probe {
            api_version: "1.29.23".to_string(),
            body_version: "1.29.23".to_string(),
        }),
        &absent,
    ) {
        Preflight::Attach { version } => assert_eq!(version, "1.29.23"),
        other => panic!("a matching kernel must be attached, got {other:?}"),
    }

    match sidecar::preflight(
        "1.29.23",
        ProbeOutcome::Probe(Probe {
            api_version: "9.9.9".to_string(),
            body_version: "9.9.9".to_string(),
        }),
        &absent,
    ) {
        Preflight::Refuse(ContractError::VersionMismatch { expected, observed }) => {
            assert_eq!(expected, "1.29.23");
            assert_eq!(observed, "9.9.9");
        }
        other => panic!("a differing kernel must refuse, got {other:?}"),
    }
}

/// P16 — a refusal writes NOTHING. The start path reads the sealed manifest
/// before it creates anything, so a missing record leaves no data root, no
/// consent marker and no child.
#[test]
fn a_refused_start_creates_no_data_root_and_spawns_nothing() {
    let scratch = Scratch::new("start-refusal");
    let paths = sidecar::AppPaths {
        // No manifest inside: the resource dir is empty.
        resource_dir: scratch.path().join("resources"),
        data_root: scratch.path().join("kernel"),
        config_dir: scratch.path().join("config"),
        sidecar: scratch.path().join("brain-server"),
    };
    let state = KernelState::default();

    let dto = sidecar::start_with_paths(&paths, &state, true, 200);
    assert_eq!(dto.state, "refused");
    assert_eq!(dto.kind.as_deref(), Some("missingProvenance"));
    assert!(!paths.data_root.exists(), "a refusal created the data root");
    assert!(
        !paths.config_dir.exists(),
        "a refusal wrote the consent config directory"
    );
    assert!(
        state.child.lock().expect("child lock").is_none(),
        "a refusal spawned a child"
    );
}

// ── D5's token law ─────────────────────────────────────────────────────────

#[test]
fn the_generated_token_is_hex_and_the_module_has_no_log_sink() {
    assert_eq!(sidecar::hex_32(&[0x00; 32]), "0".repeat(64));
    assert_eq!(sidecar::hex_32(&[0xab; 32]), "ab".repeat(32));

    // There is no sink in which a token (or anything else) could be logged,
    // and the token file write is the only place the value is emitted.
    let source = include_str!("sidecar.rs");
    for sink in ["println!", "eprintln!", "dbg!", "log::", "tracing::"] {
        assert!(
            !source.contains(sink),
            "the contract module must carry no log sink: {sink}"
        );
    }
}
