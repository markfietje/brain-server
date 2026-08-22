//! The extension-session read seam: callers get a SANITIZED view of session
//! state — raw content stays host-private (LLM06 sensitive-info disclosure;
//! GDPR Art 5/6 minimization). The sanitizer is injected so each host applies
//! its own redaction posture; the SDK enforces only that the raw bytes have
//! exactly one consumer: the sanitizer.

/// Reads raw session state (host-private side).
pub trait SessionSource {
    fn read_raw(&self, key: &str) -> Result<String, String>;
}

/// Host-specific redaction (PII mask, invisible-strip, markdown-ref strip).
pub trait SessionSanitizer {
    fn sanitize_view(&self, raw: &str) -> String;
}

/// The only session handle extensions ever see. There is no method that can
/// return unsanitized content — misalignment is unrepresentable.
pub struct SanitizedSession<S: SessionSource, Z: SessionSanitizer> {
    source: S,
    sanitizer: Z,
}

impl<S: SessionSource, Z: SessionSanitizer> SanitizedSession<S, Z> {
    pub fn new(source: S, sanitizer: Z) -> Self {
        SanitizedSession { source, sanitizer }
    }

    /// The sanitized view; the raw value is dropped inside this call.
    pub fn read(&self, key: &str) -> Result<String, String> {
        let raw = self.source.read_raw(key)?;
        Ok(self.sanitizer.sanitize_view(&raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(String);
    impl SessionSource for Fixed {
        fn read_raw(&self, _: &str) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    struct MaskEmails;
    impl SessionSanitizer for MaskEmails {
        fn sanitize_view(&self, raw: &str) -> String {
            // Deliberately naive test stand-in for the host's real redactor.
            if raw.contains('@') {
                "[redacted:*]".repeat(1)
            } else {
                raw.to_string()
            }
        }
    }

    #[test]
    fn raw_content_never_crosses_the_seam() {
        let s = SanitizedSession::new(Fixed("contact jane@example.com now".into()), MaskEmails);
        let view = s.read("state").unwrap();
        assert_eq!(view, "[redacted:*]");
        assert!(!view.contains("jane@example.com"));
    }

    #[test]
    fn clean_state_passes_through_verbatim() {
        let s = SanitizedSession::new(Fixed("{\"step\":3}".into()), MaskEmails);
        assert_eq!(s.read("state").unwrap(), "{\"step\":3}");
    }

    #[test]
    fn source_errors_propagate_untouched() {
        struct Fail;
        impl SessionSource for Fail {
            fn read_raw(&self, _: &str) -> Result<String, String> {
                Err("gone".into())
            }
        }
        let s = SanitizedSession::new(Fail, MaskEmails);
        assert_eq!(s.read("x").unwrap_err(), "gone");
    }
}
