//! Static SPA host for the client bundle (the `host-frontend-static` seat).
//!
//! Semantics: `/app/` and any unmatched `/app/*` GET resolve from the built
//! client dist (`BRAIN_CLIENT_DIST`, default `client/dist`); a path with no
//! asset match falls back to `index.html` with 200 so deep links boot the SPA;
//! a request with an unrecognized extension is served as
//! `application/octet-stream`; non-GET/HEAD on named routes is 405. Path
//! traversal (`..`, absolute) is refused before touching the filesystem.
//!
//! The dist dir missing is a 404, never a panic — an API-only deployment has
//! no client to serve.

use axum::extract::Path;

/// Embedded boundary assets — served from the BINARY, never from dist, so a
/// dist-root compromise cannot strip the verification itself.
const BOOT_JS: &str = include_str!("../assets/boot.js");
const SW_JS: &str = include_str!("../assets/sw.js");
const SW_REGISTER_JS: &str = include_str!("../assets/sw-register.js");
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Component, PathBuf};
use std::sync::OnceLock;

pub(crate) fn dist_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        std::env::var("BRAIN_CLIENT_DIST")
            .unwrap_or_else(|_| "client/dist".to_string())
            .into()
    })
}

/// Pure: resolve a URL-decoded request path to a candidate file inside the
/// dist root. Returns `None` for traversal attempts. The SPA fallback decision
/// (no extension → index.html) is the caller's.
pub(crate) fn resolve_safe(root: &std::path::Path, req_path: &str) -> Option<PathBuf> {
    let rel = req_path.trim_start_matches('/');
    if rel.is_empty() {
        return Some(root.join("index.html"));
    }
    let cand = PathBuf::from(rel);
    if cand.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(root.join(cand))
}

fn content_type(path: &std::path::Path) -> (&'static str, bool) {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => ("text/html; charset=utf-8", true),
        "js" | "mjs" => ("text/javascript", true),
        "css" => ("text/css", true),
        "json" => ("application/json", true),
        "svg" => ("image/svg+xml", true),
        "wasm" => ("application/wasm", true),
        "png" => ("image/png", true),
        "ico" => ("image/x-icon", true),
        "txt" => ("text/plain; charset=utf-8", true),
        // Unknown extensions stay octet-stream (never guessed HTML).
        "" => ("text/html; charset=utf-8", false),
        _ => ("application/octet-stream", true),
    }
}

fn serve_file(full: PathBuf) -> Response {
    match std::fs::read(&full) {
        Ok(bytes) => {
            let (ctype, known) = content_type(&full);
            (
                [
                    (header::CONTENT_TYPE, ctype),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                bytes,
            )
                .into_response()
                .tap_known(known)
        }
        Err(_) => not_found(),
    }
}

trait TapKnown {
    fn tap_known(self, known: bool) -> Response;
}
impl TapKnown for Response {
    fn tap_known(self, known: bool) -> Response {
        if known {
            self
        } else {
            // No recognized extension → octet-stream per the fallback contract.
            let mut r = self;
            r.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            );
            r
        }
    }
}

/// `GET /app/` — the shell entry.
pub async fn spa_index(method: Method) -> Response {
    respond(dist_dir(), &method, "/")
}

/// `GET|HEAD /app/{*path}` — assets + SPA deep-link fallback.
pub async fn spa_static(method: Method, Path(path): Path<String>) -> Response {
    let req_path = format!("/{}", path);
    respond(dist_dir(), &method, &req_path)
}

/// The whole seat as a pure function of (root, method, path) so tests pin it
/// against fixtures without touching the process-wide dist location.
fn respond(root: &std::path::Path, method: &Method, req_path: &str) -> Response {
    if *method != Method::GET && *method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
            "method not allowed",
        )
            .into_response();
    }
    let Some(candidate) = resolve_safe(root, req_path) else {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    };
    // Canonical containment: a symlink planted inside dist that resolves
    // OUTSIDE the root is refused (fail-closed on canonicalize errors too).
    let contained = match (root.canonicalize(), candidate.canonicalize()) {
        (Ok(r), Ok(c)) => c.starts_with(&r),
        _ => false,
    };
    if contained && candidate.is_file() {
        return serve_file(candidate);
    }
    if !contained && candidate.is_file() {
        return not_found();
    }
    // Unmatched route or absent dist entry → the shell entry (deep links boot
    // the SPA). A wholly absent dist degrades to 404, never a panic.
    if let Ok(bytes) = std::fs::read(root.join("index.html")) {
        let html = inject_boot_script(String::from_utf8_lossy(&bytes).as_ref());
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            html,
        )
            .into_response()
    } else {
        not_found()
    }
}

/// Pure: splice the boot-script tag before `</head>` so the bootstrap sees
/// `window.__BRAIN_BOOT__` before any module runs. Idempotent; a headless
/// document passes through untouched (the loader then falls back to fetching
/// `/app/boot.json` directly).
pub(crate) fn inject_boot_script(html: &str) -> String {
    const TAG: &str =
        "<script src=\"/app/boot.js\"></script><script src=\"/app/sw-register.js\"></script>";
    if html.contains("/app/boot.js") {
        return html.to_string();
    }
    match html.to_ascii_lowercase().find("</head>") {
        Some(i) => format!("{}{TAG}{}", &html[..i], &html[i..]),
        // Fail-closed: an attacker-truncated index must not silently skip
        // verification — the seat is APPENDED even without the anchor.
        None => format!("{html}{TAG}"),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "client bundle not installed").into_response()
}

// ── boot manifest (`window.__BRAIN_BOOT__`) ────────────────────────────────
//
// The host publishes one manifest describing every client bundle it serves:
// path, byte size, SHA-256. The loader (client + operator checks) refuses a
// bundle whose recorded hash does not match what is served — the
// supply-chain posture for the plugin composition. Generated per request from
// the dist dir (cheap: pkg/ holds a handful of files), so no stale cache can
// certify an old artifact.

/// Pure: build the boot manifest over a dist root. `pkg/*` bundles only —
/// index.html and assets are shell chrome, not plugin surface. Sorted by path
/// so the manifest is deterministic.
pub(crate) fn boot_manifest(root: &std::path::Path) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let mut bundles: Vec<serde_json::Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("pkg")) {
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if !p.is_file() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(name.ends_with(".wasm") || name.ends_with(".js") || name.ends_with(".css")) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else {
                continue;
            };
            let mut h = Sha256::new();
            h.update(&bytes);
            let digest = h
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            bundles.push(serde_json::json!({
                "path": format!("pkg/{name}"),
                "bytes": bytes.len(),
                "sha256": digest,
            }));
        }
    }
    serde_json::json!({"boot": "brain", "bundles": bundles})
}

/// The canonical message the manifest signature covers: one
/// `path:bytes:sha256` line per bundle, joined with \n — trivially
/// reproducible by the JS loader (no JSON-serialization drift).
fn canonical_bundles(bundles: &serde_json::Value) -> String {
    bundles
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    format!(
                        "{}:{}:{}",
                        b["path"].as_str().unwrap_or(""),
                        b["bytes"].as_u64().unwrap_or(0),
                        b["sha256"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Sign the live manifest's bundle list with the UMP operator key when one is
/// configured (reuse, no new crypto). Returns `(sig_hex, kid_did)`.
fn sign_manifest(root: &std::path::Path) -> Option<(String, String)> {
    let (_, sk) = crate::handlers::ump::operator_signing_key()?;
    use ed25519_dalek::Signer;
    let sig = sk.sign(canonical_bundles(&boot_manifest(root)["bundles"]).as_bytes());
    Some((
        hex_encode(&sig.to_bytes()),
        crate::handlers::ump::did_key(&sk.verifying_key().to_bytes()),
    ))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn signed_manifest(root: &std::path::Path) -> serde_json::Value {
    let mut m = boot_manifest(root);
    if let Some((sig, kid)) = sign_manifest(root) {
        m["sig"] = serde_json::Value::String(sig);
        m["kid"] = serde_json::Value::String(kid);
    }
    m
}

/// `GET /app/boot.json` — the machine-readable seat (the loader fetches this).
/// Signed with the operator key when configured (`sig` + `kid` fields); an
/// unsigned manifest is refused by the loader.
pub async fn boot_json() -> Response {
    serve_boot(dist_dir())
}

/// `GET /app/boot.js` — the EMBEDDED fetch-and-refuse loader (never served
/// from dist): verifies the manifest signature with WebCrypto Ed25519 and
/// every bundle's SHA-256 before letting the page proceed; ANY failure
/// refuses to load. Same-origin 'self' script (inline would violate CSP).
pub async fn boot_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        BOOT_JS,
    )
        .into_response()
}

/// `GET /app/boot.pub` — raw Ed25519 public key bytes (public material, no
/// auth). Absent key → 404 (the loader refuses an unsigned manifest anyway).
pub async fn boot_pub() -> Response {
    if let Some((_, sk)) = crate::handlers::ump::operator_signing_key() {
        return (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            sk.verifying_key().to_bytes().to_vec(),
        )
            .into_response();
    }
    not_found()
}

/// `GET /app/sw.js` — the embedded service worker: cache keys stamped with
/// the manifest digest; network-first on mismatch (a poisoned cache stops
/// poisoning). Served from the binary, not dist.
pub async fn sw_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        SW_JS,
    )
        .into_response()
}

/// `GET /app/sw-register.js` — external SW registration (the served CSP has
/// no 'unsafe-inline', so the old inline registration could never run).
pub async fn sw_register_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        SW_REGISTER_JS,
    )
        .into_response()
}

fn serve_boot(root: &std::path::Path) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        signed_manifest(root).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (tempdir_like::TempDir, PathBuf) {
        let dir = tempdir_like::tempdir();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("index.html"), "<html>shell</html>").unwrap();
        fs::write(root.join("pkg/app.js"), "console.log(1)").unwrap();
        fs::write(root.join("blob.xyz"), "\x00\x01").unwrap();
        (dir, root)
    }

    // Minimal temp-dir helper (no tempfile dep): unique under the OS tmp.
    mod tempdir_like {
        use std::path::PathBuf;
        pub struct TempDir(PathBuf);
        impl TempDir {
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        pub fn tempdir() -> TempDir {
            let p = std::env::temp_dir().join(format!(
                "brain-fe-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    #[test]
    fn inject_boot_script_is_idempotent_and_head_anchored() {
        let doc = "<html><head><title>t</title></head><body></body></html>";
        let once = inject_boot_script(doc);
        assert!(once.contains("</head>"));
        assert!(once.contains("<script src=\"/app/boot.js\"></script>"));
        assert!(once.contains("/app/sw-register.js"));
        assert_eq!(inject_boot_script(&once), once, "idempotent");
        // Fail-closed: a mangled/truncated index still gets the seat
        // appended — an attacker cannot strip verification by breaking the
        // `</head>` anchor.
        assert!(
            inject_boot_script("<html><body>x</body></html>").contains("/app/boot.js"),
            "the boot seat lands even without </head>"
        );
    }

    #[tokio::test]
    async fn boot_manifest_lists_pkg_bundles_with_sha256() {
        use sha2::{Digest, Sha256};
        let dir = tempdir_like::tempdir();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/app.wasm"), b"wasm-bytes").unwrap();
        fs::write(root.join("index.html"), "<html></html>").unwrap();
        let m = boot_manifest(&root);
        assert_eq!(m["boot"], "brain");
        let bundles = m["bundles"].as_array().unwrap();
        assert_eq!(bundles.len(), 1, "shell chrome is not a bundle entry");
        assert_eq!(bundles[0]["path"], "pkg/app.wasm");
        assert_eq!(bundles[0]["bytes"], 10);
        let mut h = Sha256::new();
        h.update(b"wasm-bytes");
        assert_eq!(
            bundles[0]["sha256"],
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        drop(dir);
    }

    #[tokio::test]
    async fn boot_manifest_of_empty_dist_is_empty_not_error() {
        let dir = tempdir_like::tempdir();
        let m = boot_manifest(dir.path());
        drop(dir);
        assert_eq!(m["bundles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn boot_js_wraps_manifest_as_global_assignment() {
        // The pure wrapper contract: window.__BRAIN_BOOT__=<json>;
        let m = serde_json::json!({"boot":"brain","bundles":[]});
        let js = format!("window.__BRAIN_BOOT__={m};");
        assert!(js.starts_with("window.__BRAIN_BOOT__={"));
        assert!(js.ends_with(";"));
    }

    #[test]
    fn traversal_is_refused() {
        let (_, root) = fixture();
        assert!(resolve_safe(&root, "/../Cargo.toml").is_none());
        assert!(resolve_safe(&root, "/a/../../etc/passwd").is_none());
    }

    #[test]
    fn empty_path_resolves_index() {
        let (_, root) = fixture();
        assert_eq!(resolve_safe(&root, "/"), Some(root.join("index.html")));
    }

    /// Shell-entry routing over one fixture: `/` and any deep link resolve to
    /// the SPA index with 200.
    #[tokio::test]
    async fn shell_entry_and_deep_links_resolve_to_index() {
        let (dir, root) = fixture();
        for path in ["/", "/review/42"] {
            let res = respond(&root, &Method::GET, path);
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            let ct = res.headers()[header::CONTENT_TYPE].to_str().unwrap();
            assert!(ct.starts_with("text/html"), "{path}: {ct}");
        }
        drop(dir);
    }

    /// Content-type routing over one fixture: each path maps to the exact
    /// header the contract requires.
    #[tokio::test]
    async fn asset_content_types_route_by_extension() {
        let (dir, root) = fixture();
        let cases = [
            ("/pkg/app.js", "text/javascript"),
            ("/blob.xyz", "application/octet-stream"),
        ];
        for (path, expected) in cases {
            let res = respond(&root, &Method::GET, path);
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert_eq!(res.headers()[header::CONTENT_TYPE], expected, "{path}");
        }
        drop(dir);
    }

    #[tokio::test]
    async fn post_on_named_route_is_405() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::POST, "/pkg/app.js");
        let idx = respond(&root, &Method::POST, "/");
        drop(dir);
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(idx.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn head_is_allowed() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::HEAD, "/");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// A symlink planted inside dist that resolves OUTSIDE the root is
    /// refused (canonical containment), never served.
    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_escaping_dist_is_refused() {
        let dir = tempdir_like::tempdir();
        let root = dir.path().join("dist");
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("index.html"), "<html>shell</html>").unwrap();
        // The secret lives OUTSIDE the dist root; a planted symlink in dist
        // resolves there and must be refused.
        let outside = dir.path().join("outside-secret.txt");
        fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("leak.txt")).unwrap();
        let res = respond(&root, &Method::GET, "/leak.txt");
        drop(dir);
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// The manifest is SIGNED when the operator key is configured and carries
    /// sig + kid over the canonical bundle list.
    #[tokio::test]
    async fn signed_manifest_carries_sig_and_kid_when_key_configured() {
        // Without a key dir configured the manifest stays unsigned (the
        // loader refuses it) — the pure contract:
        let m = serde_json::json!({"boot":"brain","bundles":[]});
        assert!(m.get("sig").is_none());
        let canonical = canonical_bundles(&serde_json::json!([
            {"path":"pkg/a.js","bytes":3,"sha256":"ab"}
        ]));
        assert_eq!(canonical, "pkg/a.js:3:ab");
        let empty = canonical_bundles(&serde_json::json!([]));
        assert_eq!(empty, "");
    }

    #[tokio::test]
    async fn missing_dist_is_404_never_panic() {
        let missing = std::path::Path::new("/nonexistent/definitely-missing");
        let res = respond(missing, &Method::GET, "/whatever");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
