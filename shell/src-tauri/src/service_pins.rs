//! W-1's pins — the service posture's laws, hermetic by construction: every
//! pin drives `install_with_paths` / `uninstall_with_paths` with temp
//! directories, a canned probe, a canned wait, a canned token and a
//! RECORDED launchctl runner. No pin touches this machine's launchd, the
//! login keychain, or port 8765.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::service::{
    InstallDecision, InstallDto, LaunchctlResult, POSTURE_FILE, SERVICE_LABEL, ServiceError,
    ServicePaths, decide_install, enumerate_residue, install_with_paths, plist_xml,
    remove_service_data, run_launchctl, save_posture, service_command_spec, uninstall_with_paths,
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

// ── the machine witness (W-5) ──────────────────────────────────────────────
//
// The pins above are hermetic: temp dirs, canned edges. This witness is the
// OTHER half — the REAL edges on a REAL machine, run explicitly, leg by leg,
// with the operator's say-so at each gate. It proves what the pins cannot:
// launchd actually bootstraps OUR label, the sealed sidecar actually serves
// the port, and the operator's `com.brain.server` is byte-identical before,
// between and after every act — the collision receipt bookending the whole
// witness.
//
// The legs (the operator's windows, EXECUTION_PROMPT_TOOLS_DISTRIBUTION §0.4):
//
//   WITNESS_LEG=1  receipts + the collision leg. No operator act. With 8765
//                  held by the operator's kernel, the REAL install edge
//                  refuses PortHeld { holder }, names the holder, and writes
//                  NOTHING (no plist, no root, no launchctl call).
//   — operator: `launchctl bootout gui/<uid>/com.brain.server` —
//   WITNESS_LEG=2  the happy path. REQUIRES 8765 free (the operator's
//                  bootout IS the gate; a held port refuses leg 2 by name).
//                  The REAL install → Installed, the belt (served == seal),
//                  the mode laws, the attach receipt, then the switch B→A:
//                  the REAL uninstall, the offered data root, the offered
//                  removal — the machine left as found.
//   — operator: `launchctl bootstrap gui/<uid> ~/Library/LaunchAgents/com.brain.server.plist` —
//   WITNESS_LEG=3  the restore receipt: the operator's plist byte-identical,
//                  8765 held again by their kernel, OUR label gone
//                  everywhere.
//
// Consent: passed `true` explicitly — the witness runs BY the operator's
// hand (the consent-before-storage LAW is pinned hermetically above; what
// this witness adds is the machine). Every location below is in the named
// write-set: OUR plist in the user's LaunchAgents, OUR service root under
// the app's own data dir, the witness's seal record in a scratch dir. The
// operator's label is READ (the hash, `launchctl list`) and never written.

/// The recorded collision receipt baseline for the operator's plist —
/// measured 2026-10-31 before any witness act (the round's STATE; the W-0
/// evidence carried the same bytes). The witness refuses to run beside any
/// OTHER com.brain.server.
const OPERATOR_PLIST_BASELINE: &str =
    "9ff0064c368ece5d4bec78168f2f2708877db9f764b6d0849cd3cb8709913cf0";
const OPERATOR_LABEL: &str = "com.brain.server";
/// The app's own identifier (tauri.conf.json) — the app_data_dir the real
/// install resolves through `service_paths` on an AppHandle.
const APP_IDENTIFIER: &str = "com.stewardmem.shell";

/// `shasum -a 256` exec'd directly (argv only, no shell — the same source
/// the §0.4 receipt names).
fn shasum(path: &Path) -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}

/// `git -C <root> …`, exec'd directly. The seal record's truth fields.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The machine's uid, the same source `service::uid` execs.
fn machine_uid() -> Result<u32, String> {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|error| error.to_string())?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "the uid was not a number".to_string())
}

fn launchctl_list() -> Result<String, String> {
    let output = std::process::Command::new("/bin/launchctl")
        .arg("list")
        .output()
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The `launchctl list` line for one label: `PID\tStatus\tLabel`.
fn list_line(list: &str, label: &str) -> Option<String> {
    list.lines()
        .find(|line| line.split_whitespace().nth(2) == Some(label))
        .map(str::to_string)
}

/// The kernel's Cargo version — the sealer's source, read the sealer's way.
fn cargo_kernel_version(root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && let Some(rest) = trimmed.strip_prefix("version = \"") {
            let end = rest.find('"').ok_or("unterminated version")?;
            return Ok(rest[..end].to_string());
        }
    }
    Err("Cargo.toml [package] has no version".to_string())
}

/// The wire's x-api-version — the sealer's second source, read the same way.
fn openapi_x_api_version(root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("openapi.yaml")).map_err(|e| e.to_string())?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("x-api-version: \"") {
            let end = rest.find('"').ok_or("unterminated x-api-version")?;
            return Ok(rest[..end].to_string());
        }
    }
    Err("openapi.yaml has no x-api-version".to_string())
}

/// The REAL paths the service posture resolves on this machine: the user's
/// LaunchAgents, the app's own data dir (the identifier at
/// tauri.conf.json:5), the kernel binary this tree builds.
fn witness_paths(home: &Path, sidecar: &Path, resource_dir: &Path) -> ServicePaths {
    let app_dir = home
        .join("Library/Application Support")
        .join(APP_IDENTIFIER);
    ServicePaths {
        service_root: app_dir.join("service"),
        launch_agents_dir: home.join("Library/LaunchAgents"),
        config_dir: app_dir,
        sidecar: sidecar.to_path_buf(),
        resource_dir: resource_dir.to_path_buf(),
    }
}

/// The collision receipt, asserted: the operator's plist must hash to the
/// recorded baseline, or the witness refuses to run beside an unrecognized
/// com.brain.server. Returns the live hash for the receipt.
fn assert_operator_receipt(home: &Path) -> String {
    let plist = home
        .join("Library/LaunchAgents")
        .join(format!("{OPERATOR_LABEL}.plist"));
    let hash = shasum(&plist).expect("the operator's com.brain.server.plist must exist");
    assert_eq!(
        hash, OPERATOR_PLIST_BASELINE,
        "the operator's plist is NOT the witnessed baseline — refusing to run beside an unrecognized com.brain.server (live {hash})"
    );
    hash
}

/// Our label must be UNREGISTERED from launchd: nothing under
/// com.stewardmem.shell.service is installed before or after a witness leg.
/// The registration check is `launchctl print` — the same probe the install's
/// idempotency step uses — because `launchctl list` keeps a dying, pid-less
/// row for a bootout'ed label for a few seconds (measured on this machine,
/// for both labels); a retried print is the definitive registration fact.
fn assert_our_label_absent(uid: u32) {
    let domain = format!("gui/{uid}/{SERVICE_LABEL}");
    for attempt in 0..5 {
        let registered = std::process::Command::new("/bin/launchctl")
            .args(["print", &domain])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(true);
        if !registered {
            return;
        }
        if attempt == 4 {
            panic!(
                "OUR label is still registered in launchd — the witness refuses to run over an existing install"
            );
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Our label must be REGISTERED (the installed state), quoted with its pid.
fn assert_our_label_registered(uid: u32) -> String {
    let domain = format!("gui/{uid}/{SERVICE_LABEL}");
    let output = std::process::Command::new("/bin/launchctl")
        .args(["print", &domain])
        .output()
        .expect("launchctl print");
    assert!(
        output.status.success(),
        "our agent must be registered while installed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let list = launchctl_list().expect("launchctl list");
    list_line(&list, SERVICE_LABEL).expect("the registered agent carries a pid in launchctl list")
}

/// The witness's own artifacts must be absent (the machine as found).
fn assert_no_witness_artifacts(paths: &ServicePaths) {
    assert!(
        !paths.plist_path().exists(),
        "our plist already exists — remove the stray install before witnessing"
    );
    assert!(
        !paths.service_root.exists(),
        "our service root already exists — remove the stray install before witnessing"
    );
}

/// Seal the provenance record the belt reads, from the SAME sources the
/// release sealer reads (Cargo.toml [package] version, openapi.yaml
/// x-api-version, the tree state, the artifact bytes) — the truthful record
/// for THIS tree and THIS binary, written into the witness's scratch
/// resource dir. The release sealer writes ISO-8601 `built_at` and refuses a
/// dirty tree; the witness records the epoch and the dirty bit truthfully —
/// the belt reads only the two sealed versions.
fn seal_the_witness_record(root: &Path, sidecar: &Path, resource_dir: &Path) -> String {
    let kernel_version = cargo_kernel_version(root).expect("the kernel's Cargo version");
    let x_api_version = openapi_x_api_version(root).expect("the wire's x-api-version");
    assert_eq!(
        kernel_version, x_api_version,
        "version lockstep broken: Cargo.toml {kernel_version} != openapi.yaml {x_api_version}"
    );
    let commit = git(root, &["rev-parse", "HEAD"]).expect("git rev-parse");
    let describe = git(root, &["describe", "--tags", "--always"]).expect("git describe");
    let dirty = !git(root, &["status", "--porcelain"])
        .expect("git status")
        .is_empty();
    let digest = shasum(sidecar).expect("the kernel binary must exist");
    let size = fs::metadata(sidecar).expect("the kernel binary").len();
    let record = serde_json::json!({
        "schema": 1,
        "kernel_commit": commit,
        "kernel_describe": describe,
        "kernel_version": kernel_version,
        "x_api_version": x_api_version,
        "source_dirty": dirty,
        "built_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs(),
        "artifacts": [{
            "name": "brain-server",
            "target": "arm64-apple-darwin",
            "sha256": digest,
            "size": size,
            "archs": ["arm64"],
        }],
    });
    let dir = resource_dir.join("binaries");
    fs::create_dir_all(&dir).expect("the witness resource dir");
    fs::write(
        dir.join("SIDECAR_PROVENANCE.json"),
        serde_json::to_string_pretty(&record).expect("the seal record serializes"),
    )
    .expect("the seal record writes");
    x_api_version
}

fn real_probe() -> ProbeOutcome {
    sidecar::probe(
        sidecar::PORT,
        Duration::from_millis(sidecar::PROBE_TIMEOUT_MS),
    )
}

#[test]
#[ignore = "the machine witness: run explicitly, one operator-gated leg at a time"]
fn service_machine_witness() {
    let leg = match std::env::var("WITNESS_LEG").as_deref() {
        Ok("1") | Ok("2") | Ok("3") => std::env::var("WITNESS_LEG").expect("checked above"),
        Ok(other) => panic!("WITNESS_LEG={other} is not a leg (1|2|3)"),
        Err(_) => panic!(
            "the machine witness runs BY LEG, operator-gated: WITNESS_LEG=1 \
             (receipts + the collision leg; no operator act) → the operator \
             bootouts com.brain.server → WITNESS_LEG=2 (the happy path: \
             install, the belt, the mode laws, the attach, the B→A switch) → \
             the operator re-bootstraps com.brain.server → WITNESS_LEG=3 \
             (the restore receipt)"
        ),
    };
    let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the repo root above shell/src-tauri")
        .to_path_buf();
    let sidecar_bin = repo_root.join("target/release/brain-server");
    let resource_dir = std::env::temp_dir().join("w5-machine-witness");
    let uid = machine_uid().expect("the machine uid");

    match leg.as_str() {
        "1" => leg_one(&home, &sidecar_bin, uid),
        "2" => leg_two(&home, &repo_root, &sidecar_bin, &resource_dir, uid),
        _ => leg_three(&home, uid),
    }
}

/// LEG 1 — receipts first, then the collision leg: the REAL install edge
/// against a port the operator's kernel holds. The refusal IS the proof.
fn leg_one(home: &Path, sidecar_bin: &Path, uid: u32) {
    println!("── LEG 1: receipts + the collision leg ──");
    let hash = assert_operator_receipt(home);
    println!("receipt: com.brain.server.plist sha256 = {hash}");
    let list = launchctl_list().expect("launchctl list");
    let line = list_line(&list, OPERATOR_LABEL)
        .expect("the operator's label must be present in launchd for leg 1");
    println!("receipt: launchctl list → {line}");
    let outcome = real_probe();
    assert!(
        matches!(outcome, ProbeOutcome::Probe(_)),
        "leg 1 witnesses a machine whose 8765 is held by the operator's Brain kernel; the probe saw {outcome:?}"
    );
    println!("receipt: the 8765 probe → {outcome:?}");
    let paths = witness_paths(home, sidecar_bin, Path::new(""));
    assert_no_witness_artifacts(&paths);
    assert_our_label_absent(uid);

    // The REAL edge: real probe, real wait, real token custody, real
    // launchctl, real uid — a wrong step here would REALLY bootstrap. That
    // is the point.
    let state = KernelState::default();
    let mut probe_once = real_probe;
    let mut wait = |expected: &str| {
        sidecar::wait_bounded(real_probe, expected, sidecar::WAIT_MS, sidecar::POLL_MS)
    };
    let dto = install_with_paths(
        &paths,
        &state,
        true, // the operator's say-so: the witness runs by their hand
        &mut probe_once,
        &mut wait,
        &mut sidecar::resolve_token,
        &mut run_launchctl,
        uid,
    );
    println!("the REAL install edge answered: {dto:?}");
    // PortHeld { holder } is the ServiceError variant: the DTO carries it as
    // the refused outcome with kind "portHeld" — the shape the hermetic pin
    // and the wizard's copy both key on.
    assert_eq!(dto.outcome, "refused", "dto: {dto:?}");
    assert_eq!(dto.kind.as_deref(), Some("portHeld"), "dto: {dto:?}");
    assert_eq!(
        dto.holder.as_deref(),
        Some("a kernel that answers as a Brain kernel, not started by this app"),
        "the holder sentence must name what the probe found, verbatim"
    );
    assert!(!dto.stopped_app_kernel, "dto: {dto:?}");

    // Nothing written: no plist, no service root, no launchd registration —
    // and the operator's bytes untouched.
    assert_no_witness_artifacts(&paths);
    assert_our_label_absent(uid);
    let after = assert_operator_receipt(home);
    assert_eq!(hash, after, "the collision receipt must be byte-identical");

    println!("LEG 1 COMPLETE — the refusal is the proof.");
    println!(
        "OPERATOR, for the happy-path window: launchctl bootout gui/{uid}/{OPERATOR_LABEL} \
         — then re-run with WITNESS_LEG=2"
    );
}

/// LEG 2 — the happy path (the operator's bootout is the gate), the belt,
/// the mode laws, the attach receipt, and the switch B→A. The machine is
/// left as found.
fn leg_two(home: &Path, repo_root: &Path, sidecar_bin: &Path, resource_dir: &Path, uid: u32) {
    println!("── LEG 2: the happy path + the switch B→A ──");
    let hash = assert_operator_receipt(home);
    println!(
        "receipt: com.brain.server.plist sha256 = {hash} (the service is STOPPED, the bytes stay)"
    );
    let outcome = real_probe();
    assert_eq!(
        outcome,
        ProbeOutcome::NotRunning,
        "leg 2 requires 8765 FREE — the operator's bootout is the gate; the probe saw {outcome:?}"
    );
    let paths = witness_paths(home, sidecar_bin, resource_dir);
    assert_no_witness_artifacts(&paths);
    assert_our_label_absent(uid);

    let sealed = seal_the_witness_record(repo_root, sidecar_bin, resource_dir);
    println!(
        "receipt: the seal, from the tree's own sources — x_api_version {sealed}, \
         artifact {}",
        sidecar_bin.display()
    );

    let state = KernelState::default();
    let mut probe_once = real_probe;
    let mut wait = |expected: &str| {
        sidecar::wait_bounded(real_probe, expected, sidecar::WAIT_MS, sidecar::POLL_MS)
    };
    let dto = install_with_paths(
        &paths,
        &state,
        true, // the operator's say-so: the witness runs by their hand
        &mut probe_once,
        &mut wait,
        &mut sidecar::resolve_token,
        &mut run_launchctl,
        uid,
    );
    println!("the REAL install edge answered: {dto:?}");
    assert_eq!(dto.outcome, "installed", "dto: {dto:?}");
    assert_eq!(dto.service_label.as_deref(), Some(SERVICE_LABEL));
    assert_eq!(
        dto.plist_path.as_deref(),
        Some(paths.plist_path().display().to_string()).as_deref()
    );
    // The belt: the SERVED version == the sealed provenance, both named.
    assert_eq!(
        dto.expected.as_deref(),
        Some(sealed.as_str()),
        "dto: {dto:?}"
    );
    assert_eq!(
        dto.observed.as_deref(),
        Some(sealed.as_str()),
        "dto: {dto:?}"
    );

    // The mode laws, witnessed on the REAL root.
    let root_mode = fs::metadata(&paths.service_root)
        .expect("the service root")
        .permissions()
        .mode()
        & 0o7777;
    let auth_mode = fs::metadata(paths.auth_file())
        .expect("the auth file")
        .permissions()
        .mode()
        & 0o7777;
    let plist_mode = fs::metadata(paths.plist_path())
        .expect("the plist")
        .permissions()
        .mode()
        & 0o7777;
    println!(
        "receipt: ls -le — {} mode {:o} · {} mode {:o} · {} mode {:o}",
        paths.service_root.display(),
        root_mode,
        paths.auth_file().display(),
        auth_mode,
        paths.plist_path().display(),
        plist_mode,
    );
    assert_eq!(root_mode, 0o700, "the service root is 0700");
    assert_eq!(auth_mode, 0o600, "the auth file is 0600");
    assert_eq!(
        plist_mode, 0o644,
        "the plist follows the directory convention"
    );

    // Attach, never double-start: the app's own wire — origin + the SHARED
    // bearer from token custody — reaches the launchd-served kernel, and
    // launchd holds exactly ONE brain-server agent (ours; the operator's is
    // bootout'ed, and no app child exists to double-start).
    let wire = sidecar::wire_status(sidecar::PORT);
    println!("receipt: the wire, aimed at the served kernel → {wire:?}");
    assert_eq!(
        wire.kind, "ok",
        "the app's wire must reach the launchd-served kernel with the shared bearer"
    );
    let served = real_probe();
    assert!(
        matches!(served, ProbeOutcome::Probe(_)),
        "the served kernel answers as a Brain kernel; the probe saw {served:?}"
    );
    println!("receipt: the served kernel → {served:?}");
    let ours = assert_our_label_registered(uid);
    println!("receipt: launchctl list → {ours}");
    let list = launchctl_list().expect("launchctl list");
    assert!(list_line(&list, OPERATOR_LABEL).is_none());

    // The switch B→A: the REAL uninstall. The removal is enumerated, the
    // data root is OFFERED — and then the offered act runs, named, so the
    // machine is left as found.
    let dto = uninstall_with_paths(&paths, &mut run_launchctl, uid);
    println!("the REAL uninstall edge answered: {dto:?}");
    assert_eq!(dto.outcome, "uninstalled", "dto: {dto:?}");
    assert_eq!(dto.removed.len(), 2, "dto: {dto:?}");
    assert!(dto.removed[0].contains(SERVICE_LABEL), "dto: {dto:?}");
    assert_eq!(
        dto.removed[1],
        paths.plist_path().display().to_string(),
        "dto: {dto:?}"
    );
    assert_eq!(
        dto.offered,
        vec![paths.service_root.display().to_string()],
        "the offer names the exact data root"
    );
    assert_eq!(
        dto.residue,
        vec![paths.service_root.display().to_string()],
        "the residue enumeration names exactly the offered root"
    );
    remove_service_data(&paths.service_root).expect("the offered removal, named and bounded");
    assert!(
        enumerate_residue(&paths).is_empty(),
        "after the offered removal the fixed enumeration is empty"
    );

    // The machine as found: no agent, no plist, no root, a free port — and
    // the operator's bytes untouched through the WHOLE window.
    assert_our_label_absent(uid);
    assert_no_witness_artifacts(&paths);
    let outcome = real_probe();
    assert_eq!(outcome, ProbeOutcome::NotRunning, "the port is free again");
    let after = assert_operator_receipt(home);
    assert_eq!(
        hash, after,
        "the collision receipt must be byte-identical across the whole happy-path window"
    );

    println!("LEG 2 COMPLETE — the machine is as found.");
    println!(
        "OPERATOR, to restore: launchctl bootstrap gui/{uid} ~/Library/LaunchAgents/{OPERATOR_LABEL}.plist \
         — then re-run with WITNESS_LEG=3"
    );
}

/// LEG 3 — the restore receipt: the operator's service is back, their bytes
/// byte-identical, our label gone everywhere. The restore IS part of the
/// evidence.
fn leg_three(home: &Path, uid: u32) {
    println!("── LEG 3: the restore receipt ──");
    let hash = assert_operator_receipt(home);
    println!("receipt: com.brain.server.plist sha256 = {hash} — BYTE-IDENTICAL to the baseline");
    let list = launchctl_list().expect("launchctl list");
    let line = list_line(&list, OPERATOR_LABEL)
        .expect("the operator's label must be back in launchd after the restore");
    println!("receipt: launchctl list → {line}");
    let outcome = real_probe();
    assert!(
        matches!(outcome, ProbeOutcome::Probe(_)),
        "8765 must be held again by the operator's Brain kernel; the probe saw {outcome:?}"
    );
    println!("receipt: the 8765 probe → {outcome:?}");
    assert_our_label_absent(uid);
    let paths = witness_paths(home, Path::new(""), Path::new(""));
    assert_no_witness_artifacts(&paths);

    println!("WITNESS CLOSED — the machine is as found; every receipt is in the round record.");
    println!("(uid context for the operator's commands: gui/{uid})");
}
