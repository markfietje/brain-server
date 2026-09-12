//! Model-artifact hash pinning.
//!
//! Model artifacts (BYO-ONNX dirs, embed models) are otherwise
//! operator-trusted local files. When the operator sets `BRAIN_MODEL_MANIFEST`
//! to a JSON object mapping file path → SHA-256 hex, every listed artifact is
//! verified at boot: any missing file, hash mismatch, or malformed entry
//! REFUSES boot (fail-closed — a pinned artifact must never silently differ).
//! Absent env = unpinned posture (the documented ceiling).

#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Verify every artifact named by `BRAIN_MODEL_MANIFEST` against its pinned
/// SHA-256. Paths are resolved relative to the manifest's directory unless
/// absolute. Returns the number of verified files.
pub fn verify_configured_models() -> Result<usize, String> {
    let Some(manifest_path) = std::env::var_os("BRAIN_MODEL_MANIFEST") else {
        return Ok(0); // unpinned posture
    };
    verify_manifest_file(&PathBuf::from(manifest_path))
}

/// The env-free core: verify one manifest against its artifacts.
pub fn verify_manifest_file(manifest: &Path) -> Result<usize, String> {
    let raw = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read manifest {}: {e}", manifest.display()))?;
    let entries: BTreeMap<String, String> =
        serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))?;
    let base = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
    for (rel, want) in &entries {
        let want = want.trim().to_ascii_lowercase();
        if want.len() != 64 || !want.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "manifest entry '{rel}': expected 64 hex chars, got '{want}'"
            ));
        }
        // Relative entries stay under the manifest dir: `..` escapes the
        // pinned tree into files the manifest was never meant to cover.
        // Absolute paths remain operator-explicit (documented).
        if !Path::new(rel).is_absolute()
            && Path::new(rel)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("manifest entry '{rel}': escapes its directory"));
        }
        let path = if Path::new(rel).is_absolute() {
            PathBuf::from(rel)
        } else {
            base.join(rel)
        };
        // Symlink refusal (2026-09-11 fix): `fs::read` FOLLOWS symlinks, so a
        // relative entry naming a link pointing OUTSIDE the manifest tree
        // would verify a file the manifest never named. Pin the file ITSELF,
        // not its destination.
        #[cfg(unix)]
        if std::fs::symlink_metadata(&path)
            .map_err(|e| format!("artifact '{}': {}", path.display(), e))?
            .file_type()
            .is_symlink()
        {
            return Err(format!(
                "manifest entry '{rel}': symlinks are not pinnable (resolve the real file)"
            ));
        }
        let bytes =
            std::fs::read(&path).map_err(|e| format!("artifact '{}': {}", path.display(), e))?;
        let got = hex_encode(&Sha256::digest(&bytes));
        if got != want {
            return Err(format!(
                "artifact '{}' hash mismatch: pinned {want}, found {got}",
                path.display()
            ));
        }
    }
    Ok(entries.len())
}

fn hex_encode(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn manifest_with(base: &std::path::Path, entries: &[(&str, String)]) -> PathBuf {
        let map: BTreeMap<String, String> = entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let path = base.join("manifest.json");
        std::fs::write(&path, serde_json::to_string(&map).unwrap()).unwrap();
        path
    }

    #[test]
    fn matching_artifact_verifies() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("models/m.onnx"), b"weights");
        let pinned = hex_encode(&Sha256::digest(b"weights"));
        let m = manifest_with(dir.path(), &[("models/m.onnx", pinned)]);
        assert_eq!(verify_manifest_file(&m).unwrap(), 1);
    }

    #[test]
    fn tampered_artifact_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let art = dir.path().join("a.bin");
        write(&art, b"clean");
        let pinned = hex_encode(&Sha256::digest(b"clean"));
        let m = manifest_with(dir.path(), &[("a.bin", pinned)]);
        assert_eq!(verify_manifest_file(&m).unwrap(), 1);
        write(&art, b"tampered");
        assert!(
            verify_manifest_file(&m)
                .unwrap_err()
                .contains("hash mismatch")
        );
    }

    #[test]
    fn missing_artifact_and_bad_hash_shape_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest_with(dir.path(), &[("nope.bin", "0".repeat(64))]);
        assert!(verify_manifest_file(&m).is_err());
        let bad = manifest_with(dir.path(), &[("x", "zz".to_string())]);
        assert!(verify_manifest_file(&bad).unwrap_err().contains("64 hex"));
    }

    #[test]
    fn dotdot_entry_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest_with(dir.path(), &[("../escape.bin", "0".repeat(64))]);
        assert!(
            verify_manifest_file(&m)
                .unwrap_err()
                .contains("escapes its directory")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_entry_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.onnx");
        write(&target, b"weights");
        let link = dir.path().join("linked.onnx");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let pinned = hex_encode(&Sha256::digest(b"weights"));
        // The hash matches the DESTINATION's bytes — refusal is about the
        // file being a link at all, not a mismatch.
        let m = manifest_with(dir.path(), &[("linked.onnx", pinned)]);
        assert!(
            verify_manifest_file(&m)
                .unwrap_err()
                .contains("symlinks are not pinnable")
        );
    }
}
