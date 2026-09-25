//! Secret-file mode enforcement: one owner for the reader-side
//! fail-closed posture (the `check_secret_permissions` seam). The writer-side
//! contract stays `install-service.sh`'s chmod; non-Unix platforms are
//! unchecked (no POSIX modes to read).

/// A provider bearer is deliberately small; the bound keeps a hostile or
/// mistakenly selected file from turning configuration into an unbounded read.
pub const MAX_PROVIDER_SECRET_BYTES: usize = 16 * 1024;

/// Closed failure vocabulary for the confined provider-secret reader. The
/// variants intentionally carry no path or OS error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSecretError {
    RootUnavailable,
    OutsideRoot,
    Symlink,
    NotRegular,
    Permission,
    Unreadable,
    TooLarge,
    Empty,
    Multiline,
    InvalidEncoding,
}

impl std::fmt::Display for ProviderSecretError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::RootUnavailable => "provider secret root is unavailable",
            Self::OutsideRoot => "provider secret is outside the configured root",
            Self::Symlink => "provider secret symlinks are refused",
            Self::NotRegular => "provider secret is not a regular file",
            Self::Permission => "provider secret permissions are invalid",
            Self::Unreadable => "provider secret is unreadable",
            Self::TooLarge => "provider secret exceeds its size bound",
            Self::Empty => "provider secret is empty",
            Self::Multiline => "provider secret contains unsafe line content",
            Self::InvalidEncoding => "provider secret encoding is invalid",
        };
        formatter.write_str(message)
    }
}

/// Read one server-configured provider secret beneath `root`.
///
/// The target is rejected if it is a symlink, resolves outside the canonical
/// root, is not a regular file, is group/world-readable, is empty, contains
/// line breaks/control/whitespace, or exceeds the fixed byte cap. Exactly one
/// trailing LF or CRLF is accepted as file framing and is not part of the
/// returned bearer value.
pub fn read_provider_secret(
    root: &std::path::Path,
    configured_file: &std::path::Path,
) -> Result<String, ProviderSecretError> {
    let root = std::fs::canonicalize(root).map_err(|_| ProviderSecretError::RootUnavailable)?;
    let candidate = if configured_file.is_absolute() {
        configured_file.to_path_buf()
    } else {
        root.join(configured_file)
    };
    let link_metadata =
        std::fs::symlink_metadata(&candidate).map_err(|_| ProviderSecretError::Unreadable)?;
    if link_metadata.file_type().is_symlink() {
        return Err(ProviderSecretError::Symlink);
    }
    if !link_metadata.is_file() {
        return Err(ProviderSecretError::NotRegular);
    }
    let target = std::fs::canonicalize(&candidate).map_err(|_| ProviderSecretError::Unreadable)?;
    if !target.starts_with(&root) {
        return Err(ProviderSecretError::OutsideRoot);
    }
    check_secret_permissions(&target).map_err(|_| ProviderSecretError::Permission)?;
    let metadata = std::fs::metadata(&target).map_err(|_| ProviderSecretError::Unreadable)?;
    if metadata.len() > MAX_PROVIDER_SECRET_BYTES as u64 {
        return Err(ProviderSecretError::TooLarge);
    }
    let bytes = std::fs::read(&target).map_err(|_| ProviderSecretError::Unreadable)?;
    let text = String::from_utf8(bytes).map_err(|_| ProviderSecretError::InvalidEncoding)?;
    let without_lf = text.strip_suffix('\n').unwrap_or(&text);
    let value = without_lf.strip_suffix('\r').unwrap_or(without_lf);
    if value.is_empty() {
        return Err(ProviderSecretError::Empty);
    }
    if value
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(ProviderSecretError::Multiline);
    }
    Ok(value.to_string())
}

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
    use std::path::Path;

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

    #[cfg(unix)]
    #[test]
    fn gdl_secret_file_rejects_path_outside_configured_root() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let path = outside.path().join("secret");
        std::fs::write(&path, "token\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_provider_secret(root.path(), &path),
            Err(ProviderSecretError::OutsideRoot)
        );
    }

    #[cfg(unix)]
    #[test]
    fn gdl_secret_file_rejects_symlink_to_outside_secret() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let outside_path = outside.path().join("secret");
        let linked = root.path().join("linked");
        std::fs::write(&outside_path, "token\n").unwrap();
        std::fs::set_permissions(&outside_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&outside_path, &linked).unwrap();
        assert_eq!(
            read_provider_secret(root.path(), &linked),
            Err(ProviderSecretError::Symlink)
        );
    }

    #[cfg(unix)]
    #[test]
    fn gdl_secret_file_rejects_empty_and_multiline_secret() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::TempDir::new().unwrap();
        let empty = root.path().join("empty");
        let multiline = root.path().join("multiline");
        std::fs::write(&empty, "\n").unwrap();
        std::fs::write(&multiline, "one\ntwo\n").unwrap();
        std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&multiline, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_provider_secret(root.path(), Path::new("empty")),
            Err(ProviderSecretError::Empty)
        );
        assert_eq!(
            read_provider_secret(root.path(), Path::new("multiline")),
            Err(ProviderSecretError::Multiline)
        );
    }
}
