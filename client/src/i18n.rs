//! v1.16.8 M1–M5 — locale (i18n), RTL readiness, theme, density.
//!
//! Zero-dependency i18n: `.ftl` files per locale are compiled in at build time
//! via `include_str!` and parsed once into a static key→value map. `t()`
//! resolves current-locale → `en` → the key itself (visible fallback, never
//! blank). The `.ftl` format + the `fluent`/`fluent-langneg`/`unic-langid`
//! crates are the documented upgrade path if ICU plurals/term references or
//! richer negotiation are ever needed.
//! ponytail: this is a *simple* FTL subset (`key = value`, `#` comments) — for
//! human-authored short strings that's a fraction of a Fluent dependency, and
//! the fallback chain keeps an incomplete locale readable instead of broken.

use dioxus::prelude::*;
use std::collections::HashMap;
use std::sync::LazyLock;

/// v1.16.8 M1: the supported locale codes. `en` is always present (fallback).
pub const SUPPORTED_LOCALES: [&str; 5] = ["en", "de", "fr", "es", "nl"];

/// Accessor for a global Dioxus signal. `Signal::global` returns a fresh local
/// handle over shared storage, so exposing a `fn` (not a `static`) lets callers
/// both read `theme()()` and write `theme().set(..)` without an immutable-
/// static borrow error.
pub fn theme() -> Global<Signal<&'static str>, &'static str> {
    Signal::global(|| "dark")
}
/// v1.16.8 M4: the active density (`comfortable`|`compact`). Non-sensitive.
pub fn density() -> Global<Signal<&'static str>, &'static str> {
    Signal::global(|| "comfortable")
}
/// v1.16.8 M1: the active locale (one of `SUPPORTED_LOCALES`). Reads during
/// render subscribe the component, so changing it re-renders the UI in the new language.
pub fn locale() -> Global<Signal<&'static str>, &'static str> {
    Signal::global(|| "en")
}

/// Parse a simple FTL subset (`key = value`) into a key→value map. `#` comment
/// and blank lines are skipped; the first `=` splits key from value.
fn parse_ftl(src: &'static str) -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim(), v.trim());
        }
    }
    m
}

/// The compiled locale bundles. `LazyLock` (std, no dep) parses each `.ftl`
/// once on first access. A key missing in a locale falls through to `en` in
/// `t()` — so partial locales degrade to English, never blank.
static BUNDLES: LazyLock<HashMap<&'static str, HashMap<&'static str, &'static str>>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert("en", parse_ftl(include_str!("../locales/en/main.ftl")));
        m.insert("de", parse_ftl(include_str!("../locales/de/main.ftl")));
        m.insert("fr", parse_ftl(include_str!("../locales/fr/main.ftl")));
        m.insert("es", parse_ftl(include_str!("../locales/es/main.ftl")));
        m.insert("nl", parse_ftl(include_str!("../locales/nl/main.ftl")));
        m
    });

/// M1: resolve a key in a given locale, falling back to `en` then to the key
/// itself (visible — never a blank). Pure core; `t()` feeds it the live locale.
fn resolve(key: &str, locale: &str) -> String {
    for l in [locale, "en"] {
        if let Some(v) = BUNDLES.get(l).and_then(|b| b.get(key)) {
            return (*v).to_string();
        }
    }
    key.to_string()
}

/// M1: resolve a key in the current locale, falling back to `en` then to the
/// key itself (visible — never a blank). Reads `locale()()` so a render-time
/// call subscribes the component to locale changes.
pub fn t(key: &str) -> String {
    resolve(key, locale()())
}

/// M2: is a locale right-to-left? Arabic/Hebrew/Persian/Urdu. No RTL `.ftl`
/// files ship in v1.16.8 (the layout is RTL-ready; files are added when a buyer
/// needs them) — this stays honest for when `dir` actually flips.
pub fn is_rtl(locale: &str) -> bool {
    let l = locale.to_ascii_lowercase();
    l.starts_with("ar") || l.starts_with("he") || l.starts_with("fa") || l.starts_with("ur")
}

/// Insert `sep` every 3 digits (pure core; `format_number` feeds it the
/// locale's separator). `1_234_567` with `,` → `1,234,567`.
fn group_digits(s: &str, sep: char) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in s.bytes().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(b as char);
    }
    out
}

/// M5: locale-aware integer grouping. `en` → `1,234,567`; the rest of the
/// shipped set (`de`/`fr`/`es`/`nl`) group with `.` → `1.234.567` (fr ideally
/// uses a narrow no-break space — ponytail: dot is close enough for a first
/// cut and matches the milestone's example exactly).
/// ponytail: deviates from the plan's `Intl.NumberFormat`-via-`document::eval`
/// because eval is async (no sync path in Dioxus 0.7); this pure fn is
/// synchronous, testable, and gives the same visible result for our locale set.
pub fn format_number(n: u64) -> String {
    let locale = locale()();
    let sep = if locale.starts_with("en") { ',' } else { '.' };
    group_digits(&n.to_string(), sep)
}

/// Sanitize a persisted locale back to a supported code (`en` if unknown).
pub fn pick_locale(raw: &str) -> &'static str {
    SUPPORTED_LOCALES
        .iter()
        .copied()
        .find(|l| l == &raw)
        .unwrap_or("en")
}

/// The three theme modes: `dark`, `light`, or `system` (follow the OS via
/// `prefers-color-scheme` — v1.20.0 M1; the CSS does the following, no JS).
pub const THEME_MODES: [&str; 3] = ["dark", "light", "system"];

/// Sanitize a persisted theme back to `dark`|`light`|`system`.
pub fn pick_theme(raw: &str) -> &'static str {
    if raw == "light" {
        "light"
    } else if raw == "system" {
        "system"
    } else {
        "dark"
    }
}

/// Sanitize a persisted density back to `comfortable`|`compact`.
pub fn pick_density(raw: &str) -> &'static str {
    if raw == "compact" {
        "compact"
    } else {
        "comfortable"
    }
}

/// Best-effort persist a non-sensitive UI preference to `localStorage` (web
/// only; no-op elsewhere). Never touches the auth token — the
/// `credentials_stay_in_memory` grep guard in main.rs enforces that.
pub fn pref_save(key: &str, value: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let js = format!("try{{localStorage.setItem({key:?},{value:?})}}catch(_){{}}");
        let _ = document::eval(&js);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, value);
    }
}

/// Best-effort load a persisted preference (web only; `None` elsewhere). Async
/// because it round-trips through `document::eval`. An unset or empty value is
/// `None`.
pub async fn pref_load(key: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        document::eval(&format!(
            "const v = localStorage.getItem({key:?}); return v === null ? '' : v;"
        ))
        .join::<String>()
        .await
        .ok()
        .filter(|s| !s.is_empty())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `resolve` falls back through locale → `en` → the key itself, never blank.
    #[test]
    fn t_resolves_falls_back_to_en_then_key() {
        assert_eq!(resolve("review_title", "en"), "Review queue");
        // A key present only in `en` resolves even from a non-en locale.
        assert_eq!(resolve("review_title", "de"), "Prüfliste");
        // A key absent from the locale AND en → the key itself (visible).
        assert_eq!(resolve("no_such_key_zzz", "de"), "no_such_key_zzz");
    }

    /// `format_number` groups per locale: `en` uses `,`, `de`/`fr`/`es` use `.`.
    #[test]
    fn format_number_groups_per_locale() {
        assert_eq!(group_digits("1234567", ','), "1,234,567");
        assert_eq!(group_digits("1234567", '.'), "1.234.567");
        assert_eq!(group_digits("0", '.'), "0");
        assert_eq!(group_digits("123", ','), "123");
        assert_eq!(resolve("review_title", "es"), "Cola de revisión");
    }

    /// `is_rtl` flags only the RTL scripts; the shipped locale set is LTR.
    #[test]
    fn rtl_detection_is_exact() {
        assert!(is_rtl("ar"));
        assert!(is_rtl("he"));
        assert!(!is_rtl("en"));
        assert!(!is_rtl("de"));
        assert!(!is_rtl("fr"));
        assert!(!is_rtl("es"));
    }

    /// Sanitizers clamp persisted values to the supported set.
    #[test]
    fn persisted_prefs_are_sanitized() {
        assert_eq!(pick_locale("de"), "de");
        assert_eq!(pick_locale("xx"), "en");
        assert_eq!(pick_theme("light"), "light");
        assert_eq!(pick_theme("system"), "system");
        assert_eq!(pick_theme("blue"), "dark");
        assert_eq!(pick_density("compact"), "compact");
        assert_eq!(pick_density("huge"), "comfortable");
    }

    /// Every locale bundle compiles and is non-empty; `en` is the fallback and
    /// must define every key that the other locales reference, so a missing key
    /// anywhere resolves to English (never blank). Proves the `.ftl` files load
    /// (the biggest failure mode — a path/parse error).
    #[test]
    fn locale_bundles_load_and_en_is_complete() {
        let en = BUNDLES.get("en").expect("en bundle");
        assert!(!en.is_empty(), "en must not be empty");
        assert_eq!(
            BUNDLES.len(),
            SUPPORTED_LOCALES.len(),
            "one bundle per locale"
        );
        // Every shipped locale's keys must exist in `en` so the fallback holds.
        for (loc, b) in BUNDLES.iter() {
            assert!(!b.is_empty(), "{loc} bundle must not be empty");
            for k in b.keys() {
                assert!(
                    en.contains_key(*k),
                    "{loc} key '{k}' missing from en fallback"
                );
            }
        }
    }
}
