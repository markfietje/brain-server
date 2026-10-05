//! v1.28.20 Cockpit M1: the one download seam. Web keeps the blob+`eval`
//! browser save; native targets (desktop/mobile) write the bytes to
//! `BRAIN_DOWNLOAD_DIR` directly — no new dependency, no browser API. The
//! filename is sanitized by one pure gate both platforms share: a download
//! can never escape its target directory (`..`, separators, control chars
//! refused), matching the session-learnings traversal rule.

/// The safe on-disk/anchor filename for a download. `None` = refused.
///
/// **Quotes are refused, not sanitised.** This value is interpolated into a
/// single-quoted JavaScript string literal on the web target (see
/// [`download_script`]), so a name carrying `'` would terminate that literal
/// early and let the remainder land in statement position. Measured before
/// this refusal existed: `safe_filename("x';alert(1)//.json")` returned
/// `Some("x';alert(1)__.json")` and the emitted `eval` contained
/// `a.download='x';alert(1)//.json';`. The gate is the right place to stop it —
/// a name the browser cannot accept as a download attribute is not a safe
/// filename, and refusing is not always-refusing because every legitimate
/// caller name (a literal, or `trace-<i64>.json`) is quote-free.
pub fn safe_filename(name: &str) -> Option<String> {
    // Traversal is refused on the ORIGINAL name, before any flattening.
    if name.split(['/', '\\']).any(|c| c == "..") {
        return None;
    }
    // A quote would break out of the JS string literal this value lands in.
    if name.contains(['\'', '"', '`']) {
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

/// The blob-and-`eval` script the web target runs to save a download.
///
/// **Built from a checked `name` and a JSON-encoded `body`, never from raw
/// interpolation.** Both values used to be spliced in verbatim: `name` into a
/// single-quoted JS literal (`a.download='{safe}'`) and `body` through
/// `{:?}`, which is Rust's `Debug` and is not a JavaScript encoding — `\u{2028}`
/// has no meaning to JS, and a raw NUL followed by a digit renders as `\05`,
/// which JS reads as a legacy octal escape. Neither was reachable from the
/// three current call sites (their bodies are `serde_json` re-serialisations,
/// which already escape both), so this is hardening a latent defect rather
/// than a live fix — but it is one call-site edit from live, and it sat
/// directly above an `eval`.
///
/// `serde_json::to_string` produces a valid JS string literal including the
/// surrounding quotes, so the body needs no further wrapping. It is the same
/// helper `client/src/panels/mod.rs` already uses for this exact job.
///
/// One caveat, stated because it looks like a defect and is not: `serde_json`
/// emits U+2028/U+2029 RAW (it does not escape them). They were invalid inside
/// a JS string literal until ES2019's JSON-superset proposal made them legal,
/// so on a current engine this is valid output — verified empirically in Node
/// v24, and recorded in this repo at `src/ump_integrity.rs:167`. The hazard
/// that does remain is the **legacy octal escape**: Rust's `Debug` writes NUL as
/// `\0`, so a following digit becomes `\05`, which JS reads as octal.
/// `cfg(any(wasm32, test))` because its only production caller is the web arm
/// of `save_file`: on a host build this exists solely so the pins below can
/// drive the real builder rather than re-implement its escaping. Without the
/// `test` half it would be dead code on the host bin target, and `-D warnings`
/// (the clippy gate every push runs) would fail the build.
#[cfg(any(target_arch = "wasm32", test))]
fn download_script(name: &str, body: &str) -> Result<String, String> {
    let safe = safe_filename(name).ok_or_else(|| "invalid download filename".to_string())?;
    let encoded = serde_json::to_string(body).map_err(|e| format!("encode body: {e}"))?;
    Ok(format!(
        "(function(){{var b=new Blob([{encoded}],{{type:'application/json'}});\
         var u=URL.createObjectURL(b);var a=document.createElement('a');\
         a.href=u;a.download='{safe}';a.click();URL.revokeObjectURL(u);}})();"
    ))
}

#[cfg(target_arch = "wasm32")]
pub fn save_file(name: &str, body: &str) -> Result<(), String> {
    let js = download_script(name, body)?;
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

    /// A quote in the filename must REFUSE, not survive into a JS string
    /// literal. Pre-fix this returned `Some("x';alert(1)__.json")` and the
    /// emitted `eval` contained `a.download='x';alert(1)//.json';` — the quote
    /// terminated the literal and the rest landed in statement position.
    ///
    /// The sibling gates (traversal, separators, control chars) were already
    /// refused and this one was not, which is what makes it worth pinning: a
    /// reader checking "is the filename gate sound?" would have seen green.
    #[test]
    fn quote_in_a_filename_is_refused() {
        for hostile in [
            "x';alert(1)//.json",
            "a'b.json",
            "';alert(document.cookie)//",
            "\u{27}",
            "double\"quote.json",
            "back`tick.json",
        ] {
            assert_eq!(
                safe_filename(hostile),
                None,
                "a quote in a filename must be refused — it is interpolated \
                 into a single-quoted JS literal. Got {:?}",
                safe_filename(hostile)
            );
        }
    }

    /// **Anti-always-refuse.** The quote refusal must not cost the legitimate
    /// callers their names: every real `save_file` call site passes a literal
    /// or `trace-<i64>.json`.
    #[test]
    fn quote_refusal_does_not_refuse_ordinary_names() {
        for ok in [
            "audit.json",
            "brain-export.json",
            "brain-export.ump.md",
            "trace-42.json",
            "trace--7.json",
        ] {
            assert_eq!(
                safe_filename(ok).as_deref(),
                Some(ok),
                "an ordinary name must still pass — a guard that refuses \
                 everything would satisfy every other pin here"
            );
        }
    }

    /// The emitted script must not carry a value out of the body
    /// interpolation that JS reinterprets. `{body:?}` was Rust `Debug`, which
    /// is not a JavaScript encoding; `serde_json::to_string` is.
    ///
    /// **The hazard is the LEGACY OCTAL escape, not the line separators.**
    /// `{:?}` renders NUL as `\0`, so a NUL followed by a digit becomes
    /// `\05` — and JS reads `\05` as octal, silently turning NUL into U+0005.
    /// `serde_json` emits the four-digit `\u0000`, which is self-delimiting and
    /// safe. Measured in Node v24: `"nul\05"` has length 4 (`nul` + U+0005),
    /// while `"nul\u00005"` has length 5.
    ///
    /// **U+2028/U+2029 are deliberately NOT asserted absent.** They used to be
    /// invalid inside a JS string literal, and `serde_json` still emits them
    /// raw — but ES2019's JSON-superset proposal made them legal, verified
    /// empirically (Node v24 / V8 13.6 parses `"a<U+2028>b"` to length 3 with
    /// no SyntaxError; `--harmony` offers no pre-ES2019 mode to compare). An
    /// earlier draft of this test asserted their absence and was **red against
    /// the fix** — an over-strict assertion about a hazard the web target does
    /// not have. The repo already records the same fact at
    /// `src/ump_integrity.rs:167`.
    #[test]
    fn emitted_script_encodes_the_body_as_json_not_rust_debug() {
        // A NUL followed by a digit: the one body shape where Debug and JSON
        // encodings genuinely differ in what JS will do with them.
        let body = "{\"b\":\"nul\u{0}5\"}";
        let js = download_script("export.json", body).expect("script builds");
        assert!(
            !js.contains("\\05"),
            "Rust Debug renders NUL as `\\0`, so a following digit becomes a JS \
             legacy octal escape that silently becomes U+0005. The body must be \
             JSON-encoded as `\\u0000`: {js}"
        );
        // …and the JSON form is what actually landed.
        assert!(
            js.contains("nul\\u00005"),
            "the body must carry serde_json's self-delimiting 4-digit escape: {js}"
        );
        // The body appears as exactly one JSON string literal, quotes included.
        let expected = serde_json::to_string(body).expect("encodes");
        assert!(
            js.contains(&expected),
            "the body must be spliced as a serde_json-encoded literal: {js}"
        );
        // The filename is inside its own single-quoted literal, unescaped but
        // quote-free — which is exactly what `safe_filename` now guarantees.
        assert!(
            js.contains("a.download='export.json'"),
            "the filename must land in its literal: {js}"
        );
    }

    /// A hostile filename must not produce a script at all — the refusal
    /// happens before any string is built, so there is nothing to escape.
    #[test]
    fn hostile_filename_produces_no_script() {
        let err = download_script("x';alert(1)//.json", "{}").expect_err("must refuse");
        assert_eq!(err, "invalid download filename");
        // And the body is never even read on that path.
        assert!(download_script("ok.json", "anything").is_ok());
    }
}
