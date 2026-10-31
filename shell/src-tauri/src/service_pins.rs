//! W-1's pins — the service posture's laws, hermetic by construction: every
//! pin drives `install_with_paths` / `uninstall_with_paths` with temp
//! directories, a canned probe, a canned wait, a canned token and a
//! RECORDED launchctl runner. No pin touches this machine's launchd, the
//! login keychain, or port 8765.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::service::{
    InstallDecision, InstallDto, LaunchctlResult, POSTURE_FILE, SERVICE_LABEL, ServiceError,
    ServicePaths, decide_install, enumerate_residue, install_with_paths, plist_xml,
    remove_service_data, save_posture, service_command_spec, uninstall_with_paths,
};
use crate::sidecar::{
    self, AUTH_FILE, ContractError, KernelState, LOG_FILE, Probe, ProbeOutcome, WaitOutcome,
};

// ── the harness ────────────────────────────────────────────────────────────

fn temp_root(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "service-pins-{}-{}-{name}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp root");
    dir
}

/// The paths a pin drives: every location under a fresh temp root, so the
/// machine's real LaunchAgents, data roots and keychain are unreachable.
fn pin_paths(root: &Path) -> ServicePaths {
    ServicePaths {
        service_root: root.join("service"),
        launch_agents_dir: root.join("LaunchAgents"),
        config_dir: root.join("config"),
        sidecar: root.join("StewardMem.app/Contents/MacOS/brain-server"),
        resource_dir: root.join("resources"),
    }
}

/// The consent marker, as the wizard's gate writes it (presence IS consent).
fn grant_consent(config_dir: &Path) {
    fs::create_dir_all(config_dir).expect("config dir");
    fs::write(
        config_dir.join("consent-v1.json"),
        b"{\"granted\":true,\"version\":1}\n",
    )
    .expect("consent marker");
}

/// A well-formed sealed provenance (the version gate reads it BEFORE any
/// write, so every happy-path pin needs one).
fn seal_provenance(resource_dir: &Path, x_api_version: &str) {
    let dir = resource_dir.join("binaries");
    fs::create_dir_all(&dir).expect("resource dir");
    fs::write(
        dir.join("SIDECAR_PROVENANCE.json"),
        format!("{{\"kernel_version\":\"1.2.3\",\"x_api_version\":\"{x_api_version}\"}}\n"),
    )
    .expect("provenance");
}

/// A launchctl runner that RECORDS every call and answers per verb: the
/// `print` probe (the idempotency check) answers `print_ok`; bootstrap and
/// bootout answer `ok`. The pins assert on the recorded argv — the only
/// thing the real edge is allowed to vary is the outcome, and these pins
/// pin the argv.
#[derive(Debug)]
struct Recorder {
    calls: Vec<Vec<String>>,
    ok: bool,
    print_ok: bool,
}

impl Recorder {
    fn always_ok() -> Self {
        Self {
            calls: Vec::new(),
            ok: true,
            print_ok: false,
        }
    }

    fn run(&mut self, args: &[String]) -> Result<LaunchctlResult, String> {
        self.calls.push(args.to_vec());
        let ok = if args.first().map(String::as_str) == Some("print") {
            self.print_ok
        } else {
            self.ok
        };
        Ok(LaunchctlResult {
            ok,
            stderr: String::new(),
        })
    }
}

const UID: u32 = 501;
const TOKEN: &str = "pin-token-not-a-secret";

// ── the plist shape ────────────────────────────────────────────────────────

/// The plist's EnvironmentVariables is the spawn allowlist VERBATIM — one
/// allowlist implementation, two consumers (the spawned child and the
/// launchd agent). Drift here would give the service kernel a different
/// contract from the app's own child.
#[test]
fn plist_env_is_the_spawn_allowlist_verbatim() {
    let root = temp_root("env-parity");
    let paths = pin_paths(&root);
    let spec = service_command_spec(&paths.sidecar, &paths.service_root);
    let xml = plist_xml(SERVICE_LABEL, &paths.sidecar, &spec.env, &paths.log_path());

    // The EIGHT documented keys, each with the value command_spec derives.
    let expected: Vec<(&str, String)> = vec![
        ("AUTH_TOKEN_FILE", paths.auth_file().display().to_string()),
        ("BIND_HOST", sidecar::BIND.to_string()),
        ("BIND_PORT", sidecar::PORT.to_string()),
        ("BRAIN_DATA_ROOT", paths.service_root.display().to_string()),
        (
            "BRAIN_DB_PATH",
            paths.service_root.join("brain.db").display().to_string(),
        ),
        ("BRAIN_REQUIRE_AUTH", "1".to_string()),
        ("RUST_LOG", "warn".to_string()),
        ("CORS_ORIGINS", sidecar::SPAWNED_CORS_ORIGINS.to_string()),
    ];
    assert_eq!(spec.env.len(), 8, "the allowlist is eight keys");
    for (name, value) in expected {
        let pair = format!("<key>{name}</key>\n        <string>{value}</string>");
        assert!(
            xml.contains(&pair),
            "the plist must carry {name} with the allowlist's value"
        );
    }
    fs::remove_dir_all(&root).ok();
}

/// The plist's mechanical shape: the label, the CURRENT sidecar path, the
/// KeepAlive/RunAtLoad the W-0.1 drill measured launchd honoring, the log
/// inside the service root — and no secret anywhere in the file.
#[test]
fn plist_shape_laws_hold() {
    let root = temp_root("plist-shape");
    let paths = pin_paths(&root);
    let spec = service_command_spec(&paths.sidecar, &paths.service_root);
    let xml = plist_xml(SERVICE_LABEL, &paths.sidecar, &spec.env, &paths.log_path());

    assert!(xml.contains(&format!(
        "<key>Label</key>\n    <string>{SERVICE_LABEL}</string>"
    )));
    assert!(xml.contains(&format!("<string>{}</string>", paths.sidecar.display())));
    assert!(xml.contains("<key>KeepAlive</key>\n    <true/>"));
    assert!(xml.contains("<key>RunAtLoad</key>\n    <true/>"));
    assert!(xml.contains(&format!(
        "<key>StandardOutPath</key>\n    <string>{}</string>",
        paths.log_path().display()
    )));
    assert!(xml.contains(&format!(
        "<key>StandardErrorPath</key>\n    <string>{}</string>",
        paths.log_path().display()
    )));
    assert!(xml.starts_with("<?xml"));
    assert!(xml.trim_end().ends_with("</plist>"));
    // No secret rides the file: the token travels by REFERENCE only.
    assert!(!xml.contains(TOKEN), "no token material in the plist");
    fs::remove_dir_all(&root).ok();
}

/// Paths carry XML metacharacters; a plist that interpolates them unescaped
/// is a launchd syntax error waiting for the first user whose home has an
/// ampersand in it.
#[test]
fn plist_escapes_xml_metacharacters() {
    let root = temp_root("plist-escape");
    let tricky = root.join("a&b<c>d\"e");
    let spec = service_command_spec(&tricky.join("brain-server"), &tricky);
    let xml = plist_xml(
        SERVICE_LABEL,
        &tricky.join("brain-server"),
        &spec.env,
        &tricky.join(LOG_FILE),
    );
    assert!(xml.contains("a&amp;b&lt;c&gt;d&quot;e"));
    fs::remove_dir_all(&root).ok();
}

// ── the collision law (the source-scan pin) ────────────────────────────────

/// The install core's source contains NO other service's label — not in a
/// write path, not in a comment, NOWHERE. The constant exists so no caller
/// ever has to name it, and this pin is what keeps it that way.
#[test]
fn the_install_core_never_names_the_operators_service() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/service.rs"));
    assert!(
        !source.contains("com.brain.server"),
        "the install core must never name another service's identity"
    );
    // And the only label it DOES name is the module's own constant.
    assert!(source.contains("com.stewardmem.shell.service"));
}

// ── the port decision ──────────────────────────────────────────────────────

/// Any listener that is not this app's own child refuses posture B — the
/// probe's every non-free outcome, tabled. The holder is NAMED (what the
/// probe found), never guessed at, and posture C is the offered answer.
#[test]
fn install_refuses_every_held_port_that_is_not_our_child() {
    let matching = ProbeOutcome::Probe(Probe {
        api_version: "1.0.0".to_string(),
        body_version: "1.0.0".to_string(),
    });
    let inconsistent = ProbeOutcome::Inconsistent {
        header: "1.0.0".to_string(),
        body: "9.9.9".to_string(),
    };
    for outcome in [
        matching,
        inconsistent,
        ProbeOutcome::Foreign,
        ProbeOutcome::Unanswered,
    ] {
        let decision = decide_install(&outcome, false);
        let InstallDecision::Refuse(ServiceError::PortHeld { holder }) = decision else {
            panic!("a held port must refuse: {outcome:?}");
        };
        assert!(
            holder.contains("not started by this app") || holder.contains("listener"),
            "the holder is named by what the probe found: {holder}"
        );
    }
    // A free port proceeds.
    assert_eq!(
        decide_install(&ProbeOutcome::NotRunning, false),
        InstallDecision::Proceed
    );
    // Our OWN child holding the port is stopped, not refused: that is the
    // A→B transition's first named fact.
    let matching = ProbeOutcome::Probe(Probe {
        api_version: "1.0.0".to_string(),
        body_version: "1.0.0".to_string(),
    });
    assert_eq!(
        decide_install(&matching, true),
        InstallDecision::StopOurChild
    );
}

// ── the install order ──────────────────────────────────────────────────────

/// Consent before storage: no marker, no service root, no plist, no
/// launchctl call — the wizard's gate runs first and stays first.
#[test]
fn install_refuses_before_storage_without_consent() {
    let root = temp_root("no-consent");
    let paths = pin_paths(&root);
    seal_provenance(&paths.resource_dir, "1.0.0");
    let state = KernelState::default();
    let mut probe = || ProbeOutcome::NotRunning;
    let mut wait = |_expected: &str| WaitOutcome::Ready {
        version: "1.0.0".to_string(),
    };
    let mut token = || Ok(TOKEN.to_string());
    let mut recorder = Recorder::always_ok();
    let dto = install_with_paths(
        &paths,
        &state,
        false,
        &mut probe,
        &mut wait,
        &mut token,
        &mut |args: &[String]| recorder.run(args),
        UID,
    );
    assert_eq!(dto.kind.as_deref(), Some("consentRefused"));
    assert!(!paths.service_root.exists(), "no root before consent");
    assert!(!paths.plist_path().exists(), "no plist before consent");
    assert!(
        recorder.calls.is_empty(),
        "launchd is never touched before consent"
    );
    fs::remove_dir_all(&root).ok();
}

/// The happy path, in order: root 0700, auth file 0600, plist 0644 under
/// the user's LaunchAgents, ONE bootstrap call with the exact argv, and
/// the served version in the outcome. The A→B child stop rides the DTO.
#[test]
fn install_creates_the_mode_laws_and_bootstraps_our_label_only() {
    let root = temp_root("install-happy");
    let paths = pin_paths(&root);
    grant_consent(&paths.config_dir);
    seal_provenance(&paths.resource_dir, "1.0.0");
    let state = KernelState::default();
    let mut probe = || ProbeOutcome::NotRunning;
    let mut wait = |_expected: &str| WaitOutcome::Ready {
        version: "1.2.3".to_string(),
    };
    let mut token = || Ok(TOKEN.to_string());
    let mut recorder = Recorder::always_ok();
    let dto = install_with_paths(
        &paths,
        &state,
        true,
        &mut probe,
        &mut wait,
        &mut token,
        &mut |args: &[String]| recorder.run(args),
        UID,
    );
    assert_eq!(dto.outcome, "installed", "dto: {dto:?}");
    assert_eq!(dto.expected.as_deref(), Some("1.2.3"));
    assert_eq!(dto.service_label.as_deref(), Some(SERVICE_LABEL));

    // The mode laws inside the service root.
    let root_mode = fs::metadata(&paths.service_root)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(root_mode & 0o777, 0o700, "the service root is 0700");
    let auth_mode = fs::metadata(paths.auth_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(auth_mode & 0o777, 0o600, "the auth file is 0600");
    let plist_mode = fs::metadata(paths.plist_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        plist_mode & 0o777,
        0o644,
        "the plist follows LaunchAgents convention"
    );

    // TWO launchctl calls: the print probe (idempotency), then bootstrap
    // with the gui domain and OUR plist.
    assert_eq!(recorder.calls.len(), 2, "print + bootstrap: {recorder:?}");
    assert_eq!(recorder.calls[0][0], "print");
    assert_eq!(
        recorder.calls[1],
        vec![
            "bootstrap".to_string(),
            format!("gui/{UID}"),
            paths.plist_path().display().to_string(),
        ]
    );
    fs::remove_dir_all(&root).ok();
}

/// The version gate is the belt: the served version against the sealed
/// provenance, BOTH versions named, and the failed agent booted out — a
/// half-install never flaps.
#[test]
fn version_mismatch_names_both_versions_and_boots_the_agent_out() {
    let root = temp_root("version-mismatch");
    let paths = pin_paths(&root);
    grant_consent(&paths.config_dir);
    seal_provenance(&paths.resource_dir, "1.2.3");
    let state = KernelState::default();
    let mut probe = || ProbeOutcome::NotRunning;
    let mut wait = |_expected: &str| {
        WaitOutcome::Refused(ContractError::VersionMismatch {
            expected: "1.2.3".to_string(),
            observed: "9.9.9".to_string(),
        })
    };
    let mut token = || Ok(TOKEN.to_string());
    let mut recorder = Recorder::always_ok();
    let dto = install_with_paths(
        &paths,
        &state,
        true,
        &mut probe,
        &mut wait,
        &mut token,
        &mut |args: &[String]| recorder.run(args),
        UID,
    );
    assert_eq!(dto.kind.as_deref(), Some("versionMismatch"));
    assert_eq!(dto.expected.as_deref(), Some("1.2.3"));
    assert_eq!(dto.observed.as_deref(), Some("9.9.9"));
    // The bootout: OUR label only, after the print + bootstrap.
    assert_eq!(recorder.calls.len(), 3);
    assert_eq!(recorder.calls[2][0], "bootout");
    assert_eq!(recorder.calls[2][1], format!("gui/{UID}/{SERVICE_LABEL}"));
    fs::remove_dir_all(&root).ok();
}

/// A service that never answers is refused and booted out — the machine is
/// never left with a launchd agent flapping on a program that cannot run.
#[test]
fn a_silent_service_is_refused_and_booted_out() {
    let root = temp_root("silent-service");
    let paths = pin_paths(&root);
    grant_consent(&paths.config_dir);
    seal_provenance(&paths.resource_dir, "1.0.0");
    let state = KernelState::default();
    let mut probe = || ProbeOutcome::NotRunning;
    let mut wait = |_expected: &str| WaitOutcome::Timeout {
        waited_ms: 1234,
        last: "no response".to_string(),
    };
    let mut token = || Ok(TOKEN.to_string());
    let mut recorder = Recorder::always_ok();
    let dto = install_with_paths(
        &paths,
        &state,
        true,
        &mut probe,
        &mut wait,
        &mut token,
        &mut |args: &[String]| recorder.run(args),
        UID,
    );
    assert_eq!(dto.kind.as_deref(), Some("serviceDidNotAnswer"));
    assert_eq!(recorder.calls.len(), 3, "print, bootstrap, then bootout");
    assert_eq!(recorder.calls[2][0], "bootout");
    fs::remove_dir_all(&root).ok();
}

// ── uninstall + residue ────────────────────────────────────────────────────

/// Uninstall removes OURS (the agent, the plist), OFFERS the data with its
/// exact path, and never touches a sibling in LaunchAgents.
#[test]
fn uninstall_removes_only_ours_and_offers_the_data() {
    let root = temp_root("uninstall-ours");
    let paths = pin_paths(&root);
    fs::create_dir_all(&paths.launch_agents_dir).expect("launch agents");
    fs::create_dir_all(&paths.service_root).expect("service root");
    fs::write(paths.plist_path(), b"<plist/>").expect("plist");
    fs::write(paths.service_root.join(AUTH_FILE), b"token\n").expect("auth file");
    let decoy = paths.launch_agents_dir.join("com.other.service.plist");
    fs::write(&decoy, b"<plist/>").expect("decoy");

    let mut recorder = Recorder::always_ok();
    let dto = uninstall_with_paths(&paths, &mut |args: &[String]| recorder.run(args), UID);
    assert_eq!(dto.outcome, "uninstalled", "dto: {dto:?}");
    assert!(
        dto.removed
            .iter()
            .any(|p| p == &paths.plist_path().display().to_string()),
        "the plist is named as removed"
    );
    assert!(
        dto.removed.iter().any(|p| p.contains(SERVICE_LABEL)),
        "the agent is named as removed"
    );
    assert_eq!(
        dto.offered,
        vec![paths.service_root.display().to_string()],
        "the data root is OFFERED with its exact path"
    );
    assert!(!paths.plist_path().exists(), "our plist is gone");
    assert!(decoy.exists(), "a sibling plist is NEVER touched");
    assert!(
        paths.service_root.exists() && paths.service_root.join(AUTH_FILE).exists(),
        "the data is offered, not silently deleted"
    );
    // The bootout named OUR label and nothing else.
    assert_eq!(recorder.calls.len(), 1);
    assert_eq!(
        recorder.calls[0],
        vec!["bootout".to_string(), format!("gui/{UID}/{SERVICE_LABEL}")]
    );
    fs::remove_dir_all(&root).ok();
}

/// Uninstalling a ghost (nothing installed) is a named no-op, not an error.
#[test]
fn uninstalling_a_ghost_is_a_named_no_op() {
    let root = temp_root("uninstall-ghost");
    let paths = pin_paths(&root);
    fs::create_dir_all(&paths.launch_agents_dir).expect("launch agents");
    let mut recorder = Recorder {
        calls: Vec::new(),
        ok: false,
        print_ok: false,
    };
    let dto = uninstall_with_paths(&paths, &mut |args: &[String]| recorder.run(args), UID);
    assert_eq!(dto.outcome, "notInstalled");
    assert!(dto.removed.is_empty());
    fs::remove_dir_all(&root).ok();
}

/// The offered data removal is a named act on EXACTLY the service root —
/// a path that is not the service root is refused by shape.
#[test]
fn remove_service_data_is_bound_to_the_service_root() {
    let root = temp_root("remove-data");
    let paths = pin_paths(&root);
    fs::create_dir_all(&paths.service_root).expect("service root");
    fs::write(paths.service_root.join(AUTH_FILE), b"token\n").expect("auth file");

    assert!(remove_service_data(&paths.service_root).is_ok());
    assert!(!paths.service_root.exists(), "the offered removal removes");

    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    let refused = remove_service_data(&elsewhere);
    assert!(
        matches!(refused, Err(ServiceError::Filesystem { .. })),
        "a path that is not the service root is refused by shape"
    );
    assert!(elsewhere.exists(), "the refused path was not touched");
    fs::remove_dir_all(&root).ok();
}

/// The residue enumeration is EXACTLY the fixed set: the plist and the
/// service root. No scan, no discovery — nothing that could ever name
/// another service's paths.
#[test]
fn residue_enumeration_is_the_fixed_set() {
    let root = temp_root("residue");
    let paths = pin_paths(&root);
    fs::create_dir_all(&paths.launch_agents_dir).expect("launch agents");
    assert!(enumerate_residue(&paths).is_empty(), "nothing installed");
    fs::write(paths.plist_path(), b"<plist/>").expect("plist");
    assert_eq!(
        enumerate_residue(&paths),
        vec![paths.plist_path().display().to_string()]
    );
    fs::create_dir_all(&paths.service_root).expect("service root");
    assert_eq!(
        enumerate_residue(&paths),
        vec![
            paths.plist_path().display().to_string(),
            paths.service_root.display().to_string(),
        ]
    );
    fs::remove_dir_all(&root).ok();
}

// ── the posture marker ─────────────────────────────────────────────────────

/// The posture marker: defaults to AppManaged when absent, round-trips,
/// rides 0600 outside the data root, and the external posture carries a URL
/// and has NOWHERE to put a token (the enum has no token field — structural).
#[test]
fn posture_marker_laws_hold() {
    let root = temp_root("posture");
    let config = root.join("config");

    // Absent is the default posture — the app is fully usable without
    // answering anything.
    let dto = crate::service::load_posture(&config);
    assert_eq!(dto.posture, "app");
    assert!(!config.exists(), "reading the posture creates nothing");

    save_posture(&config, &crate::service::Posture::BackgroundService).expect("save service");
    let dto = crate::service::load_posture(&config);
    assert_eq!(dto.posture, "service");

    let url = "https://kernel.example.com:8765";
    save_posture(
        &config,
        &crate::service::Posture::external(url).expect("valid url"),
    )
    .expect("save external");
    let dto = crate::service::load_posture(&config);
    assert_eq!(dto.posture, "external");
    assert_eq!(dto.url.as_deref(), Some(url));

    // 0600, and the temp file never survives.
    let mode = fs::metadata(config.join(POSTURE_FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the posture marker is 0600");
    assert!(!config.join(format!(".{POSTURE_FILE}.tmp")).exists());

    // Garbage URLs are refused, never stored.
    assert!(crate::service::Posture::external("not a url").is_err());
    assert!(crate::service::Posture::external("ftp://host").is_err());
    assert!(crate::service::Posture::external("https://").is_err());
    let long = format!("https://{}", "a".repeat(3000));
    assert!(crate::service::Posture::external(&long).is_err());

    // An unknown posture name is refused at parse.
    assert!(crate::service::Posture::parse("daemon", None).is_err());
    fs::remove_dir_all(&root).ok();
}

/// Re-installing over an ALREADY-bootstrapped agent is named as such: the
/// plist is still repaired (rewritten), but no second bootstrap is attempted
/// and no refusal is invented.
#[test]
fn reinstall_over_a_live_agent_is_already_installed() {
    let root = temp_root("already-installed");
    let paths = pin_paths(&root);
    grant_consent(&paths.config_dir);
    seal_provenance(&paths.resource_dir, "1.0.0");
    let state = KernelState::default();
    let mut probe = || ProbeOutcome::NotRunning;
    let mut wait = |_expected: &str| WaitOutcome::Ready {
        version: "1.0.0".to_string(),
    };
    let mut token = || Ok(TOKEN.to_string());
    let mut recorder = Recorder {
        calls: Vec::new(),
        ok: true,
        print_ok: true,
    };
    let dto = install_with_paths(
        &paths,
        &state,
        true,
        &mut probe,
        &mut wait,
        &mut token,
        &mut |args: &[String]| recorder.run(args),
        UID,
    );
    assert_eq!(dto.outcome, "alreadyInstalled", "dto: {dto:?}");
    // The plist was still repaired (the repair semantics), and the ONLY
    // launchctl call was the print probe — never a second bootstrap.
    assert!(paths.plist_path().exists());
    assert_eq!(recorder.calls.len(), 1);
    assert_eq!(recorder.calls[0][0], "print");
    fs::remove_dir_all(&root).ok();
}

// ── silence keepers (compile-shape witnesses) ──────────────────────────────

/// The DTO surfaces used above are the ONLY surfaces: a compile-time witness
/// that the named-outcome law holds shape (InstallDto carries the stop fact
/// beside every outcome).
#[test]
fn install_dto_names_the_child_stop_fact() {
    let dto = InstallDto::refused(ServiceError::ConsentRefused);
    assert!(!dto.stopped_app_kernel);
    assert_eq!(dto.outcome, "refused");
}
