//! III-7 — the release's update CHAIN, and the check that can see it missing.
//!
//! The endpoint the desktop application fetches is the LATEST release's copy
//! of the update manifest, so **a release that ships without its chain leaves
//! the updater's path dead for everyone** — and nothing else in the release
//! pipeline can see it, because the chain can only be produced where the key
//! is (the signing lane), and the chain necessarily lands AFTER the release
//! is published.
//!
//! `scripts/release-chain-check.sh` is the check that can see it: it asks the
//! PUBLIC INTERNET whether the address answers. It is ONE implementation with
//! TWO callers — the release script (which must not report success while the
//! endpoint is dead) and THIS file (which drives it against fixtures). A check
//! that exists in only one of them fails only in the more expensive one — and
//! the expensive one is the moment a release is being cut.
//!
//! The fixtures are a LOCAL server, not the internet: the check takes its URL
//! as an argument, so the pin can make it answer or not answer on demand.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Run the chain check against `url`, with `attempts` tries and no waiting.
fn check(url: &str, attempts: u32) -> (Option<i32>, String, String) {
    let script = format!(
        "{}/scripts/release-chain-check.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new("bash")
        .arg(&script)
        .arg(url)
        .arg(attempts.to_string())
        .arg("0")
        .output()
        .expect("the chain check script is runnable");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    )
}

/// A local server that answers `path` with `status`, once.
///
/// **THE HANDLE IS DELIBERATELY NOT JOINED.** A mutant that stops connecting
/// would leave the server thread blocked in `accept()` forever, and a
/// `join()` would then hang the PIN rather than fail it — which the drill
/// found the hard way (the test run exceeded its timeout instead of going
/// red). The thread is detached and the process exit reaps it.
fn serve_once(status: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("the bound address").port();
    let status = status.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        // One connection is enough: the check asks once per attempt.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://127.0.0.1:{port}/latest.json")
}

/// The endpoint answers — the check says LIVE and exits 0.
#[test]
fn an_answering_endpoint_is_live() {
    let url = serve_once("200 OK", r#"{"version":"0.1.1"}"#);
    let (status, stdout, _) = check(&url, 2);
    assert_eq!(
        status,
        Some(0),
        "an endpoint that answers must be LIVE, got {stdout:?}"
    );
    assert_eq!(stdout, "LIVE");
}

/// The endpoint does not answer — the check says DEAD and exits 1.
///
/// **This is the whole point of the check**: a release that published without
/// its chain leaves exactly this shape, and the check has to say so rather
/// than pass quietly.
#[test]
fn a_dead_endpoint_is_refused_and_named() {
    let url = serve_once("404 Not Found", "Not Found");
    let (status, stdout, _) = check(&url, 2);
    assert_eq!(
        status,
        Some(1),
        "an endpoint that does not answer must be refused, got {stdout:?}"
    );
    assert_eq!(stdout, "DEAD");
}

/// A server error is not an answer either — the check must not treat a 500
/// as a live endpoint.
#[test]
fn a_server_error_is_not_a_live_endpoint() {
    let url = serve_once("500 Internal Server Error", "boom");
    let (status, stdout, _) = check(&url, 2);
    assert_eq!(status, Some(1), "a 500 is not an answer, got {stdout:?}");
    assert_eq!(stdout, "DEAD");
}

/// The check is BOUNDED: a host that accepts the connection and then says
/// nothing must not hang the release line forever.
#[test]
fn the_check_is_bounded_against_a_silent_host() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("the bound address").port();
    // Accept the connection and never answer.
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            // Hold the socket open, saying nothing.
            std::thread::sleep(Duration::from_secs(5));
            drop(stream);
        }
    });
    let url = format!("http://127.0.0.1:{port}/latest.json");
    let started = std::time::Instant::now();
    let (status, stdout, _) = check(&url, 1);
    let elapsed = started.elapsed();
    assert_eq!(status, Some(1), "a silent host is DEAD, got {stdout:?}");
    assert!(
        elapsed < Duration::from_secs(40),
        "the check took {elapsed:?} — a check that waits forever hangs the release line"
    );
}

/// The check refuses to run at all without `curl` — and says which tool is
/// missing, so a missing tool is never read as a dead endpoint.
#[test]
fn the_check_names_the_missing_tool() {
    // A PATH with no curl at all: the script must say it could not run
    // (exit 2), NOT report DEAD — "the check could not run" and "the endpoint
    // is dead" are different facts.
    //
    // **THE SHELL ITSELF IS RESOLVED BEFORE THE PATH IS EMPTIED** — the
    // empty PATH is what the script must find no `curl` in, and it would
    // otherwise take `bash` down with it.
    let empty = std::env::temp_dir().join("brain-chain-check-no-curl");
    std::fs::create_dir_all(&empty).expect("the scratch dir is creatable");
    let script = format!(
        "{}/scripts/release-chain-check.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    let bash = std::env::var("SHELL")
        .ok()
        .filter(|shell| Path::new(shell).exists())
        .unwrap_or_else(|| "/bin/bash".to_string());
    let output = Command::new(&bash)
        .arg(&script)
        .arg("http://127.0.0.1:1/latest.json")
        .arg("1")
        .arg("0")
        .env("PATH", &empty)
        .output()
        .expect("the chain check script is runnable");
    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing curl must be reported as 'could not run', not as a dead endpoint"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("curl"),
        "the refusal must name the missing tool, got {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
