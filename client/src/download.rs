//! v1.28.20 Cockpit M1: the one download seam. Web keeps the blob+`eval`
//! browser save; native targets (desktop/mobile) write the bytes to
//! `BRAIN_DOWNLOAD_DIR` directly — no new dependency, no browser API. The
//! filename is sanitized by one pure gate both platforms share: a download
//! can never escape its target directory (`..`, separators, control chars
//! refused), matching the session-learnings traversal rule.

/// The safe on-disk/anchor filename for a download. `None` = refused.
pub fn safe_filename(name: &str) -> Option<String> {
    // Traversal is refused on the ORIGINAL name, before any flattening.
    if name.split(['/', '\\']).any(|c| c == "..") {
        return None;
    }
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || c == '/' || c == '\\' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn save_file(name: &str, body: &str) -> Result<(), String> {
    let safe = safe_filename(name).ok_or_else(|| "invalid download filename".to_string())?;
    let js = format!(
        "(function(){{var b=new Blob([{body:?}],{{type:'application/json'}});\
         var u=URL.createObjectURL(b);var a=document.createElement('a');\
         a.href=u;a.download='{safe}';a.click();URL.revokeObjectURL(u);}})();"
    );
    dioxus::prelude::document::eval(&js);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_file(name: &str, body: &str) -> Result<(), String> {
    use std::path::PathBuf;
    let safe = safe_filename(name).ok_or_else(|| "invalid download filename".to_string())?;
    let dir = std::env::var_os("BRAIN_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .or_else(dirs_download)
        .ok_or_else(|| "no download directory (set BRAIN_DOWNLOAD_DIR)".to_string())?;
    let path = dir.join(&safe);
    // The parent must already exist — we never create directories implicitly
    // outside the operator's chosen location, and never follow a symlinked
    // final component out of it (the filename gate refuses separators).
    std::fs::write(&path, body.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn dirs_download() -> Option<std::path::PathBuf> {
    // XDG-style Downloads without a `dirs` dependency.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let dl = home.join("Downloads");
    dl.is_dir().then_some(dl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_path_is_native_save_on_desktop() {
        // The pure gate both platforms share: traversal and control chars
        // never survive, ordinary names do.
        assert_eq!(
            safe_filename("audit.json").as_deref(),
            Some("audit.json"),
            "ordinary name passes"
        );
        assert!(safe_filename("../etc/passwd").is_none(), "`..` refused");
        assert!(safe_filename("..").is_none());
        assert!(safe_filename(".").is_none());
        assert!(safe_filename("").is_none());
        assert!(safe_filename("   ").is_none(), "blank refused");
        assert_eq!(
            safe_filename("a/b.json").as_deref(),
            Some("a_b.json"),
            "separators flattened, never traversed"
        );
        assert_eq!(safe_filename("a\\b.json").as_deref(), Some("a_b.json"));
        assert_eq!(
            safe_filename("trailing\u{0}").as_deref(),
            Some("trailing_"),
            "control chars flattened"
        );
        // Nested `..` inside an otherwise-normal name still refused.
        assert_eq!(safe_filename("a/../b"), None);
    }
}
