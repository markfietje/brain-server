//! The boot-manifest loader contract: the host publishes `/app/boot.json` (+ the
//! `window.__BRAIN_BOOT__` script seat) describing every client bundle with its
//! SHA-256. The loader validates the manifest fail-closed and refuses any
//! bundle whose recorded digest does not match what it verifies — the
//! supply-chain posture for plugin composition.
//!
//! Honest ceiling: UI plugins are compile-time crates (no JS third-party
//! loading), so today the manifest is consumed by operator checks + the
//! bootstrap's integrity assertion; the runtime fetch-and-refuse driver lands
//! with the streaming surface.

#![allow(dead_code)]

use serde_json::Value;

const BUNDLE_EXTS: &[&str] = &["wasm", "js", "css"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootBundle {
    /// Always `pkg/<file>` — bounded, relative, no traversal.
    pub path: String,
    pub bytes: u64,
    /// 64 lowercase hex chars.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootManifest {
    pub bundles: Vec<BootBundle>,
}

/// Fail-closed parse. Every violation (wrong boot label, traversal path,
/// unknown extension, non-hex digest, oversized field) refuses the WHOLE
/// manifest — never a partial acceptance.
pub fn validate_boot_manifest(v: &Value) -> Result<BootManifest, String> {
    if v.get("boot").and_then(Value::as_str) != Some("brain") {
        return Err("not a brain boot manifest".into());
    }
    let arr = v
        .get("bundles")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing bundles array".to_string())?;
    if arr.len() > 256 {
        return Err("bundle count unbounded".into());
    }
    let mut bundles = Vec::with_capacity(arr.len());
    for b in arr {
        let path = b
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "bundle missing path".to_string())?;
        let bad = |why: &str| format!("bundle `{path}` refused: {why}");
        if !path.starts_with("pkg/")
            || path[4..].contains('/')
            || path.contains("..")
            || path.contains('\\')
        {
            return Err(bad("path escapes pkg/"));
        }
        let ext = path.rsplit('.').next().unwrap_or("");
        if !BUNDLE_EXTS.contains(&ext) {
            return Err(bad("unknown bundle extension"));
        }
        let bytes = b
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad("bytes not a u64"))?;
        let sha = b
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("sha256 missing"))?;
        if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad("sha256 not 64 hex"));
        }
        bundles.push(BootBundle {
            path: path.to_string(),
            bytes,
            sha256: sha.to_ascii_lowercase(),
        });
    }
    Ok(BootManifest { bundles })
}

/// The refusal predicate: does the manifest certify this bundle at exactly
/// `actual_sha256`? A mismatch (or an absent entry) is a refuse.
pub fn certifies(m: &BootManifest, path: &str, actual_sha256: &str) -> bool {
    m.bundles
        .iter()
        .any(|b| b.path == path && b.sha256 == actual_sha256.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sha() -> String {
        "a".repeat(64)
    }

    fn good() -> Value {
        json!({"boot":"brain","bundles":[{"path":"pkg/app.wasm","bytes":100,"sha256":sha()}]})
    }

    #[test]
    fn valid_manifest_parses() {
        let m = validate_boot_manifest(&good()).unwrap();
        assert_eq!(m.bundles.len(), 1);
        assert_eq!(m.bundles[0].path, "pkg/app.wasm");
        assert_eq!(m.bundles[0].sha256, sha());
    }

    #[test]
    fn wrong_boot_label_refused() {
        assert!(validate_boot_manifest(&json!({"boot":"other","bundles":[]})).is_err());
        assert!(validate_boot_manifest(&json!({})).is_err());
    }

    #[test]
    fn hostile_paths_refused() {
        for p in [
            "../etc/passwd",
            "pkg/../../x",
            "/abs/x.wasm",
            "pkg/a/b.wasm",
            "pkg/x.exe",
        ] {
            let v = json!({"boot":"brain","bundles":[{"path":p,"bytes":1,"sha256":sha()}]});
            assert!(validate_boot_manifest(&v).is_err(), "`{p}` must be refused");
        }
    }

    #[test]
    fn malformed_digests_refused() {
        for s in ["", "abc", "A".repeat(63).as_str(), "zz.."] {
            let v = json!({"boot":"brain","bundles":[
                {"path":"pkg/app.wasm","bytes":1,"sha256":s}]});
            assert!(validate_boot_manifest(&v).is_err(), "digest `{s}` refused");
        }
    }

    #[test]
    fn certification_refuses_mismatch_and_absence() {
        let m = validate_boot_manifest(&good()).unwrap();
        assert!(certifies(&m, "pkg/app.wasm", &sha()));
        assert!(
            !certifies(&m, "pkg/app.wasm", "b".repeat(64).as_str()),
            "hash mismatch"
        );
        assert!(
            !certifies(&m, "pkg/app.js", &sha()),
            "absent entry never certified"
        );
    }

    #[test]
    fn bundle_count_is_bounded() {
        let bundles: Vec<Value> = (0..300)
            .map(|i| json!({"path": format!("pkg/b{i}.js"), "bytes": 1, "sha256": sha()}))
            .collect();
        let v = json!({"boot":"brain","bundles":bundles});
        assert!(validate_boot_manifest(&v).is_err(), ">256 bundles refused");
    }
}
