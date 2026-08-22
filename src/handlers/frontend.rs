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
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Component, PathBuf};
use std::sync::OnceLock;

fn dist_dir() -> &'static PathBuf {
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
    if candidate.is_file() {
        return serve_file(candidate);
    }
    // Unmatched route or absent dist entry → the shell entry (deep links boot
    // the SPA). A wholly absent dist degrades to 404, never a panic.
    match std::fs::read(root.join("index.html")) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => not_found(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "client bundle not installed").into_response()
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

    #[tokio::test]
    async fn shell_entry_serves_index() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::GET, "/");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }

    #[tokio::test]
    async fn deep_link_falls_back_to_shell_with_200() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::GET, "/review/42");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res.headers()[header::CONTENT_TYPE].to_str().unwrap();
        assert!(ct.starts_with("text/html"), "{ct}");
    }

    #[tokio::test]
    async fn asset_is_served_with_its_type() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::GET, "/pkg/app.js");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/javascript");
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
    async fn unknown_extension_serves_octet_stream() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::GET, "/blob.xyz");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn head_is_allowed() {
        let (dir, root) = fixture();
        let res = respond(&root, &Method::HEAD, "/");
        drop(dir);
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_dist_is_404_never_panic() {
        let missing = std::path::Path::new("/nonexistent/definitely-missing");
        let res = respond(missing, &Method::GET, "/whatever");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
