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

/// Read an owner-only bearer-token file, refusing the shapes the provider
/// reader refuses.
///
/// ## WHY THIS EXISTS SEPARATELY FROM `read_provider_secret`
///
/// That one is scoped beneath a configured root, which a token file is not.
/// But the token readers did the two things that matter and this fixes for
/// both:
///
/// * **Symlinks were followed.** `check_secret_permissions` uses
///   `std::fs::metadata`, which FOLLOWS a link, so `AUTH_TOKEN_FILE` /
///   `AGENT_TOKEN_FILE` / the CLI token files accepted a symlink to any 0600
///   file on the host. `symlink_metadata` is what distinguishes the link from
///   its target, and it is checked first.
/// * **The check and the read were two syscalls.** The mode was validated, then
///   the path was re-opened for reading — a window in which the path can be
///   swapped for a different file. `TokenStore::reload_if_changed` re-reads on
///   every mtime advance, so a swap is picked up within one
///   `TOKEN_ROTATION_POLL_SECS`. This reader opens ONCE and checks the
///   opened handle (mode, length, and — on unix — the inode against the
///   pre-open stat), so the bytes it validates are the bytes it returns.
///
/// There is no `O_NOFOLLOW` bind without a new dependency, so this is defence
/// in depth rather than a single atomic open; the symlink refusal is what
/// closes the substitution, and the mode is re-checked on the opened file.
pub fn read_owner_only_token(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    // 1. symlink refusal — `symlink_metadata`, never `metadata`.
    let link = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    if link.file_type().is_symlink() {
        return Err(format!(
            "token file {} is a symlink — refusing (the link target is not the file you              named)",
            path.display()
        ));
    }
    if !link.is_file() {
        return Err(format!(
            "token file {} is not a regular file",
            path.display()
        ));
    }
    // 2. ONE open. Every check below reads the OPENED handle, never the path
    //    again: the old code validated the path, re-opened it, and re-stat'ed
    //    the path — a writer of the directory could swap the file between
    //    those syscalls and the bytes returned were never the bytes checked
    //    (audit: "fd re-check" was documented but not implemented).
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot stat opened {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The pre-open stat and the opened inode must be the same file. A
        // path swapped to a symlink/other file between the two shows up here.
        if meta.ino() != link.ino() || meta.dev() != link.dev() {
            return Err(format!(
                "token file {} changed between stat and open — refusing",
                path.display()
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
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
    }
    if meta.len() > MAX_PROVIDER_SECRET_BYTES as u64 {
        return Err(format!(
            "token file {} is {} bytes, over the {} byte cap",
            path.display(),
            meta.len(),
            MAX_PROVIDER_SECRET_BYTES
        ));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("token file {} is not valid UTF-8", path.display()))?;
    Ok(text.trim().to_string())
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

/// Write one owner-only secret file — the writer-side twin of
/// [`read_owner_only_token`], and the writer `brain principal grant` uses
/// (M3-g): the 0600 law held at CREATION rather than by a chmod after the
/// bytes land, so no window exists in which a minted bearer sits on disk
/// group/world-readable.
///
/// * **Create-new only.** An existing file is REFUSED: a re-grant must never
///   silently clobber a live credential — rotate by revoking and re-granting
///   (a new name, a new file).
/// * **Parent dirs 0700**, best-effort at creation, so the default roster
///   directory (`~/.config/brain-server/principals/`) does not list its
///   tenants to every local user.
/// * Non-Unix: the mode is unchecked (no POSIX modes to read), the same
///   posture as the readers.
pub fn write_owner_only_secret(path: &std::path::Path, secret: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "secret file {} already exists — refusing to overwrite a live \
                     credential (revoke, then re-grant under a new name and file)",
                    path.display()
                )
            } else {
                format!("cannot create {}: {e}", path.display())
            }
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot chmod {}: {e}", path.display()))?;
    }
    file.write_all(secret.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(())
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
    /// THE TOKEN READER REFUSES A SYMLINK. `check_secret_permissions` uses
    /// `std::fs::metadata`, which FOLLOWS one, so `AUTH_TOKEN_FILE` /
    /// `AGENT_TOKEN_FILE` / the CLI token files accepted a link to any 0600
    /// file on the host — and the mode check and the read were two syscalls,
    /// a window `TokenStore::reload_if_changed` re-opens on every mtime
    /// advance.
    ///
    /// Red-first: dropping the `symlink_metadata` arm reds this.
    #[cfg(unix)]
    #[test]
    fn a_token_file_that_is_a_symlink_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().expect("tmp");
        let real = dir.path().join("real-token");
        std::fs::write(&real, "the-real-token").expect("write");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        let link = dir.path().join("link-token");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let err = read_owner_only_token(&link).expect_err("a symlink must refuse");
        assert!(
            err.contains("symlink"),
            "the refusal names the shape, not just the failure: {err}"
        );
        // and the real file still reads
        assert_eq!(
            read_owner_only_token(&real).expect("the real file reads"),
            "the-real-token"
        );
    }

    /// A group/world-readable token file is still refused — the 0600 law the
    /// runtime readers enforce.
    #[cfg(unix)]
    #[test]
    fn a_world_readable_token_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().expect("tmp");
        let p = dir.path().join("wide-token");
        std::fs::write(&p, "leaked").expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let err = read_owner_only_token(&p).expect_err("a wide token file must refuse");
        assert!(err.contains("group/world-accessible"), "{err}");
    }

    /// THE WRITER HOLDS THE 0600 LAW AT CREATION AND NEVER CLOBBERS (M3-g).
    /// The file lands owner-only BEFORE the first byte is written, the mode
    /// reads back clean through the reader's own law, and a second write to
    /// the same path is REFUSED — a re-grant must never silently overwrite a
    /// live credential. Red-first: dropping the `create_new` arm or moving
    /// the chmod after the write reopens a world-readable window or a
    /// clobber; both refuse here by name.
    #[cfg(unix)]
    #[test]
    fn the_writer_creates_owner_only_and_refuses_a_clobber() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().expect("tmp");
        let p = dir.path().join("principals").join("edu-plugin.token");

        write_owner_only_secret(&p, "minted-bearer").expect("first write");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the minted file is owner-only at creation"
        );
        assert_eq!(
            check_secret_permissions(&p),
            Ok(()),
            "the reader's law agrees"
        );
        assert_eq!(
            read_owner_only_token(&p).expect("round trip"),
            "minted-bearer"
        );

        let err = write_owner_only_secret(&p, "second-bearer").expect_err("no clobber");
        assert!(
            err.contains("refusing to overwrite"),
            "the refusal names the act: {err}"
        );
        // and the live credential survived the refused write
        assert_eq!(read_owner_only_token(&p).unwrap(), "minted-bearer");
        let _ = std::fs::remove_file(&p);
    }
}
