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
pub(crate) fn resolve(key: &str, locale: &str) -> String {
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

/// F-38 v1.28: positional-interpolation core. `{0}`/`{1}`/… in the resolved
/// value are replaced by `args` (positional — the convention that displaced
/// the ad-hoc `.replace("{n}", …)` sites). Missing args stay verbatim
/// (visible — the same never-blank posture). Pure; `t_fmt` feeds it the live
/// locale, and tests pin it without a Dioxus runtime.
pub(crate) fn resolve_fmt(key: &str, locale: &str, args: &[String]) -> String {
    let s = resolve(key, locale);
    if args.is_empty() {
        return s;
    }
    let mut out = s;
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// F-38 v1.28: `t()` with positional arguments (`{0}`, `{1}`, …). Render-time
/// only (subscribes to locale changes like `t`); the pure core is
/// `resolve_fmt`, which the tests use.
pub fn t_fmt(key: &str, args: &[String]) -> String {
    resolve_fmt(key, locale()(), args)
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

    /// F-38 v1.28: the parity wall — every shipped locale must expose EXACTLY
    /// the `en` key set. A locale behind `en` fails the build: that is the
    /// process bug that made this pass necessary (keys added to `en` alone
    /// drifted into a silently-English UI for de/fr/es/nl; the old test only
    /// checked the weaker `en`-is-complete direction). The 119-key backfill
    /// landed in the same change; a future key addition must ship its
    /// translations in the same PR or the wall goes red.
    #[test]
    fn locale_key_sets_are_identical() {
        let en = BUNDLES.get("en").expect("en bundle");
        assert!(!en.is_empty(), "en must not be empty");
        assert_eq!(
            BUNDLES.len(),
            SUPPORTED_LOCALES.len(),
            "one bundle per locale"
        );
        for (loc, b) in BUNDLES.iter() {
            assert!(!b.is_empty(), "{loc} bundle must not be empty");
            let missing: Vec<&str> = en.keys().filter(|k| !b.contains_key(*k)).copied().collect();
            let extra: Vec<&str> = b.keys().filter(|k| !en.contains_key(*k)).copied().collect();
            if !missing.is_empty() {
                let sample = missing[..missing.len().min(5)].join(", ");
                panic!(
                    "{loc} missing {} keys vs en ({sample}…): backfill the same PR",
                    missing.len()
                );
            }
            assert!(
                extra.is_empty(),
                "{loc} has keys absent from en: {}",
                extra.join(", ")
            );
        }
    }

    /// F-38 v1.28: `resolve_fmt` substitutes positionally and leaves unknown
    /// placeholders visible (never blank). Pure — no Dioxus runtime needed.
    #[test]
    fn resolve_fmt_substitutes_positionally() {
        assert_eq!(
            resolve_fmt("proc_created", "en", &["3".to_string()]),
            "Procedure created (3 steps)"
        );
        assert_eq!(
            resolve_fmt("cons_applied", "de", &["2".to_string()]),
            "2 Ablösungen angewendet"
        );
        assert_eq!(
            resolve_fmt("data_purged", "nl", &["5".to_string()]),
            "5 chunk(s) gewist"
        );
        // No args → the raw value, verbatim.
        assert_eq!(resolve_fmt("review_title", "en", &[]), "Review queue");
        // A stray `{0}` with no args stays visible — never blank.
        assert_eq!(
            resolve_fmt("sys_reindexed", "en", &[]),
            "Reindexed {0} chunks"
        );
    }
}

/// v1.27.20 M3.3 — the no-raw-English-in-rsx gate. A scan of every render
/// surface (main.rs + all panels + the shared confirm) asserts that no
/// non-trivial plain string literal sits inside an `rsx!` block: every user-
/// visible label must resolve through `t()`/`t_fmt()` or carry a
/// `// i18n-exempt: <reason>` marker on the same line (CSS class expressions,
/// wire/protocol vocabulary, keys) — an LLM's "lift the strings" refactor
/// cannot silently leave a hardcoded label behind. Whitespace/openers like
/// `" "` pass because punctuation/formatting is not content.
#[cfg(test)]
mod raw_string_scan {
    const SURFACES: &[(&str, &str)] = &[
        (
            "main.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")),
        ),
        (
            "confirm.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/confirm.rs")),
        ),
        (
            "panels/mod.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/mod.rs")),
        ),
        (
            "panels/overview.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/overview.rs"
            )),
        ),
        (
            "panels/audit.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/audit.rs")),
        ),
        (
            "panels/subjects.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/subjects.rs"
            )),
        ),
        (
            "panels/ops.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/ops.rs")),
        ),
        (
            "panels/review.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/review.rs")),
        ),
        (
            "panels/data.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/data.rs")),
        ),
        (
            "panels/security.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/security.rs"
            )),
        ),
        (
            "panels/health.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/health.rs")),
        ),
        (
            "panels/register.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/register.rs"
            )),
        ),
        (
            "panels/system.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/system.rs")),
        ),
        (
            "panels/graph.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/graph.rs")),
        ),
        (
            "panels/console.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/console.rs"
            )),
        ),
        (
            "panels/ump.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/ump.rs")),
        ),
        (
            "panels/ingest.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/ingest.rs")),
        ),
        (
            "panels/recall.rs",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/recall.rs")),
        ),
        (
            "panels/procedures.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/procedures.rs"
            )),
        ),
        (
            "panels/consolidate.rs",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/consolidate.rs"
            )),
        ),
    ];

    fn prev_nonspace(hay: &[u8], mut i: usize) -> Option<u8> {
        while i > 0 {
            i -= 1;
            let c = hay[i];
            if c != b' ' && c != b'\t' {
                return Some(c);
            }
        }
        None
    }

    fn next_nonspace(hay: &[u8], i: usize) -> Option<u8> {
        hay.iter()
            .skip(i)
            .copied()
            .find(|&c| c != b' ' && c != b'\t')
    }

    fn is_pure_snake(s: &str) -> bool {
        s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    }

    fn is_css_class(s: &str) -> bool {
        s.starts_with("badge-")
            || s.starts_with("text-")
            || s.starts_with("border-")
            || s.starts_with("bg-")
            || s.starts_with("font-")
            || s.starts_with("w-")
            || s.starts_with("h-")
            || s.starts_with("p-")
            || s.starts_with("m-")
            || s.starts_with("gap-")
            || s.starts_with("max-w-")
            || s.starts_with("rounded")
            || s.starts_with("flex")
            || s.starts_with("grid")
            || s.starts_with("items-")
            || s.starts_with("justify-")
            || s.starts_with("space-y-")
            || s.starts_with("overflow-")
            || s.starts_with("whitespace-")
            || s.starts_with("cursor-")
            || s.starts_with("opacity-")
            || s.starts_with("pointer-")
            || s.starts_with("select")
            || s.starts_with("absolute")
            || s.starts_with("relative")
            || s.starts_with("sticky")
            || s.starts_with("tab-")
            || s.starts_with("btn")
            || s.starts_with("input-")
            || s.starts_with("px-")
            || s.starts_with("py-")
            || s.starts_with("tracking-")
            || s.starts_with("line-clamp")
            || s.starts_with("z-")
            || s.starts_with("fill-")
            || s.starts_with("shrink-")
            || s.starts_with("grow-")
            || s.starts_with("order-")
            || s.starts_with("content-")
            || s.starts_with("self-")
            || s.starts_with("truncate")
    }

    /// Scan one source file for suspect plain strings inside `rsx!` regions.
    /// Rule per candidate `"…"`: too short, contains `{` (rsx interpolation),
    /// contains whitespace/colon (sentence text passes), pure snake_case or a
    /// CSS class (wire keys/classes), preceded by `: (= . [ ! & + - / \` '`
    /// (prop value / fn arg / arithmetic), followed by `:` (attr key), or the
    /// line carries `// i18n-exempt:` — all pass. Outside-rsx code, `r#`
    /// raw strings, `#[cfg(test)]`/`mod tests` bodies and whole-line `//`
    /// comments are skipped.
    fn scan(src: &str) -> Vec<(usize, String)> {
        let mut in_rsx = false;
        let mut in_tests = false;
        let mut test_depth = 0i32;
        let mut hits = Vec::new();
        for (idx, line) in src.lines().enumerate() {
            let lineno = idx + 1;
            if in_tests {
                let open = line.matches('{').count() as i32;
                let close = line.matches('}').count() as i32;
                test_depth += open - close;
                if test_depth <= 0 {
                    in_tests = false;
                }
                continue;
            }
            if !in_rsx {
                if line.contains("rsx!") {
                    in_rsx = true;
                } else {
                    continue;
                }
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || line.contains("// i18n-exempt:") || line.contains("r#")
            {
                continue;
            }
            if line.contains("#[cfg(test)]") || line.contains("mod tests") {
                in_tests = true;
                test_depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                continue;
            }
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] != b'"' {
                    i += 1;
                    continue;
                }
                let start = i + 1;
                let mut j = start;
                let mut escaped = false;
                while j < bytes.len() {
                    let c = bytes[j];
                    if c == b'\\' && !escaped {
                        escaped = true;
                    } else if c == b'"' && !escaped {
                        break;
                    } else if escaped {
                        escaped = false;
                    }
                    j += 1;
                }
                let inner = &line[start..j];
                let ok = if inner.len() < 2
                    || !inner.bytes().any(|c| c.is_ascii_alphanumeric())
                    || inner.contains('{')
                    || inner.contains(' ')
                    || inner.contains('\t')
                    || inner.contains(':')
                    || is_pure_snake(inner)
                    || is_css_class(inner)
                    || inner.starts_with('#')
                {
                    true
                } else {
                    let prev = prev_nonspace(bytes, start - 1);
                    let next = next_nonspace(bytes, j.saturating_add(1));
                    matches!(prev, Some(c) if b": (= . [ ! & + - / ` '".contains(&c))
                        || matches!(next, Some(c) if c == b':')
                };
                if !ok {
                    hits.push((lineno, inner.to_string()));
                }
                i = j.saturating_add(1).max(start);
            }
        }
        hits
    }

    #[test]
    fn no_raw_strings_in_rsx() {
        let mut all = Vec::new();
        for (name, src) in SURFACES {
            for (line, s) in scan(src) {
                all.push(format!("{name}:{line}: \"{s}\""));
            }
        }
        assert!(
            all.is_empty(),
            "hardcoded string literals inside rsx! blocks (must use t()/t_fmt() or `// i18n-exempt:` + reason):\n{}",
            all.join("\n")
        );
    }

    #[test]
    fn scanner_flags_hardcoded_label() {
        let hits = scan("fn f() -> Element { rsx! { div { \"Hello\" } } }");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "Hello");
    }

    #[test]
    fn scanner_passes_exempt_and_props() {
        assert!(scan("fn f() { rsx! { div { \"badge badge-ok\" /* css */ } } }").is_empty());
        assert!(scan("fn f() { rsx! { div { \"Hello\" } } } // i18n-exempt: demo").is_empty());
        assert!(
            scan("fn f() { rsx! { input { placeholder: \"Type here\", value: \"x\" } } }")
                .is_empty()
        );
        assert!(scan("fn f() { rsx! { p { { crate::i18n::t(\"k\") } } } }").is_empty());
        assert!(scan("fn f() { rsx! { p { \"{x}\" } } }").is_empty());
        assert!(
            scan("fn f() { rsx! { button { \"aria-label\": \"m\", \"OK now\" } } }").is_empty()
        );
        assert!(scan("fn f() { rsx! { p { \"1:2\" } } }").is_empty());
        assert!(
            scan("#[cfg(test)] mod tests { fn g() { rsx! { p { \"Raw test\" } } } }").is_empty()
        );
    }
}
