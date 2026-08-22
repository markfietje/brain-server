//! Secret-file mode enforcement: one owner for the reader-side
//! fail-closed posture (the `check_secret_permissions` seam). The writer-side
//! contract stays `install-service.sh`'s chmod; non-Unix platforms are
//! unchecked (no POSIX modes to read).

/// Refuse a secret file with group/world bits (mode & 0o077 != 0). A missing
/// file errors too — it cannot be validated.
pub fn check_secret_permissions(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("cannot stat secret file {}: {e}", path.display()))?;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "secret file {} is group/world-accessible (mode {:o}) — expected owner-only \
                 (0600/0400). chmod 600 {} and restart.",
                path.display(),
                mode & 0o777,
                path.display()
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn enforces_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("brain-secret-perm-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "tok\n").unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(check_secret_permissions(&path), Ok(()));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(check_secret_permissions(&path), Ok(()));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = check_secret_permissions(&path).unwrap_err();
        assert!(err.contains("644"), "error names the offending mode: {err}");

        let missing = dir.join("brain-test-no-such-secret-file");
        let _ = std::fs::remove_file(&missing);
        assert!(
            check_secret_permissions(&missing).is_err(),
            "unstatable file cannot be validated"
        );
        let _ = std::fs::remove_file(&path);
    }
}
