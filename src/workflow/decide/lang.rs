//! The language detector — the pure port of the laya `lang.py` contract
//! (LAYA_RUST_PORT §0.3/§2.1): script detection over Unicode ranges, Latin
//! language guessing over stopword margins, and the state flattener.
//!
//! Dependency-free and total: every function returns plain data for any
//! input and never panics. Script is the primary signal — the English
//! checkpoint never sees non-Latin text (the router routes on this
//! module's verdict).

/// Script → non-overlapping Unicode codepoint ranges. The Latin script is
/// NOT in this table: it is the complement rule (`cp < 0x0250` or the
/// `0x1E00..=0x1EFF` extended band).
const SCRIPT_RANGES: &[(&str, &[(u32, u32)])] = &[
    ("greek", &[(0x0370, 0x03FF), (0x1F00, 0x1FFF)]),
    ("cyrillic", &[(0x0400, 0x04FF), (0x0500, 0x052F)]),
    ("hebrew", &[(0x0590, 0x05FF)]),
    (
        "arabic",
        &[
            (0x0600, 0x06FF),
            (0x0750, 0x077F),
            (0x08A0, 0x08FF),
            (0xFB50, 0xFDFF),
            (0xFE70, 0xFEFF),
        ],
    ),
    ("devanagari", &[(0x0900, 0x097F), (0xA8E0, 0xA8FF)]),
    ("bengali", &[(0x0980, 0x09FF)]),
    ("gurmukhi", &[(0x0A00, 0x0A7F)]),
    ("gujarati", &[(0x0A80, 0x0AFF)]),
    ("oriya", &[(0x0B00, 0x0B7F)]),
    ("tamil", &[(0x0B80, 0x0BFF)]),
    ("telugu", &[(0x0C00, 0x0C7F)]),
    ("kannada", &[(0x0C80, 0x0CFF)]),
    ("malayalam", &[(0x0D00, 0x0D7F)]),
    ("sinhala", &[(0x0D80, 0x0DFF)]),
    ("thai", &[(0x0E00, 0x0E7F)]),
    ("lao", &[(0x0E80, 0x0EFF)]),
    ("tibetan", &[(0x0F00, 0x0FFF)]),
    ("myanmar", &[(0x1000, 0x109F)]),
    ("georgian", &[(0x10A0, 0x10FF)]),
    ("ethiopic", &[(0x1200, 0x137F)]),
    ("khmer", &[(0x1780, 0x17FF)]),
    (
        "hangul",
        &[(0xAC00, 0xD7AF), (0x1100, 0x11FF), (0x3130, 0x318F)],
    ),
    (
        "kana",
        &[(0x3040, 0x309F), (0x30A0, 0x30FF), (0x31F0, 0x31FF)],
    ),
    (
        "han",
        &[(0x4E00, 0x9FFF), (0x3400, 0x4DBF), (0xF900, 0xFAFF)],
    ),
];

fn script_of(cp: u32) -> Option<&'static str> {
    for &(name, ranges) in SCRIPT_RANGES {
        for &(lo, hi) in ranges {
            if cp >= lo && cp <= hi {
                return Some(name);
            }
        }
    }
    None
}

fn is_latin_cp(cp: u32) -> bool {
    cp < 0x0250 || (0x1E00..=0x1EFF).contains(&cp)
}

/// Round to four decimal places (half-away-from-zero on the magnitude) —
/// the display precision every fraction in this module reports.
fn round4(v: f32) -> f32 {
    if !v.is_finite() {
        return v;
    }
    let scaled = v * 10_000.0;
    let rounded = if scaled >= 0.0 {
        (scaled + 0.5).floor()
    } else {
        (scaled - 0.5).ceil()
    };
    rounded / 10_000.0
}

/// The dominant script by letter count; `unknown` when the text holds no
/// letters at all (empty, digits-only, punctuation-only).
pub(crate) fn detect_script(text: &str) -> String {
    if text.is_empty() {
        return "unknown".into();
    }
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    let mut latin = 0usize;
    for c in text.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        let cp = c as u32;
        if is_latin_cp(cp) {
            latin += 1;
            continue;
        }
        match script_of(cp) {
            Some(name) => {
                let entry = counts.iter_mut().find(|(n, _)| *n == name);
                match entry {
                    Some((_, n)) => *n += 1,
                    None => counts.push((name, 1)),
                }
            }
            None => {}
        }
    }
    let mut best: Option<(&'static str, usize)> = None;
    for (name, n) in counts {
        match best {
            None => best = Some((name, n)),
            Some((best_name, best_n)) => {
                // Ties stay alphabetical (the deterministic-house rule).
                if n > best_n || (n == best_n && name < best_name) {
                    best = Some((name, n));
                }
            }
        }
    }
    match best {
        Some((name, n)) if n > latin => name.into(),
        // Guarded arms don't count for exhaustivity; this arm is reachable
        // only when a table script strictly dominates.
        _ if latin > 0 => "latin".into(),
        Some((name, _)) => name.into(),
        None => "unknown".into(),
    }
}

/// Fraction of letters per script, dominant-first, ties alphabetical;
/// `latin` carries the Latin rule's count. Empty text reads empty profile.
pub(crate) fn script_profile(text: &str) -> Vec<(String, f32)> {
    let mut total = 0usize;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for c in text.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        let cp = c as u32;
        let name = if is_latin_cp(cp) {
            "latin"
        } else {
            script_of(cp).unwrap_or("latin")
        };
        total += 1;
        let entry = counts.iter_mut().find(|(n, _)| n == name);
        match entry {
            Some((_, n)) => *n += 1,
            None => counts.push((name.to_string(), 1)),
        }
    }
    if total == 0 {
        return Vec::new();
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
        .into_iter()
        .map(|(name, n)| (name, round4(n as f32 / total as f32)))
        .collect()
}

/// The state flattener: str passthrough; dicts/lists recurse (depth ≤ 6,
/// values only, keys ignored) and join with single spaces; truncated to
/// `max_chars` on a char boundary.
pub(crate) fn state_text(state: &serde_json::Value, max_chars: usize) -> String {
    let mut out = String::new();
    flatten(state, 0, &mut out);
    let trimmed = out.trim().to_string();
    if trimmed.chars().count() <= max_chars {
        return trimmed;
    }
    truncated_on_boundary(&trimmed, max_chars)
}

fn truncated_on_boundary(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn flatten(state: &serde_json::Value, depth: usize, out: &mut String) {
    if depth > 6 {
        return;
    }
    match state {
        serde_json::Value::String(s) => {
            if !s.is_empty() {
                out.push_str(s);
                out.push(' ');
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                flatten(item, depth + 1, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (_key, value) in map {
                flatten(value, depth + 1, out);
            }
        }
        serde_json::Value::Null => {}
        other => {
            out.push_str(&other.to_string());
            out.push(' ');
        }
    }
}

const EN_STOP: &[&str] = &[
    "the", "and", "is", "in", "at", "of", "to", "a", "it", "that", "this", "with", "for", "on",
    "as", "was", "but", "are", "have", "has", "not", "you", "your", "we", "they", "their", "from",
    "be", "by", "an",
];
const FR_STOP: &[&str] = &[
    "le", "la", "les", "un", "une", "des", "et", "est", "en", "que", "qui", "dans", "pour", "pas",
    "sur", "avec", "plus", "ce", "il", "je", "vous", "nous", "mais", "au", "aux", "sont", "se",
    "ne", "tout", "comme",
];
const DE_STOP: &[&str] = &[
    "der", "die", "das", "und", "ist", "in", "den", "von", "zu", "mit", "sich", "des", "auf",
    "für", "nicht", "ein", "eine", "als", "auch", "es", "werden", "kann", "bei", "dass", "hat",
    "sind", "aber", "nach", "wie", "über",
];
const ES_STOP: &[&str] = &[
    "el", "la", "los", "las", "un", "una", "y", "es", "en", "que", "de", "para", "con", "por",
    "no", "se", "del", "al", "como", "más", "pero", "sus", "ha", "me", "sin", "sobre", "este",
    "cuando", "todo", "esta",
];
const PT_STOP: &[&str] = &[
    "o", "a", "os", "as", "um", "uma", "e", "é", "em", "que", "de", "para", "com", "por", "não",
    "se", "do", "da", "dos", "das", "no", "na", "mais", "como", "mas", "ao", "seu", "sua", "isso",
    "entre",
];
const IT_STOP: &[&str] = &[
    "il", "la", "lo", "gli", "le", "un", "uno", "una", "e", "è", "in", "che", "di", "per", "con",
    "non", "si", "del", "della", "al", "alla", "come", "più", "ma", "suo", "sua", "ha", "ho",
    "sono", "siamo",
];
const NL_STOP: &[&str] = &[
    "de", "het", "een", "en", "is", "in", "van", "te", "dat", "die", "niet", "met", "op", "voor",
    "zijn", "heb", "heeft", "hebben", "maar", "ook", "aan", "je", "wij", "er", "om", "dan", "als",
    "naar", "wat", "nog",
];

const STOP_SETS: &[(&str, &[&str])] = &[
    ("en", EN_STOP),
    ("fr", FR_STOP),
    ("de", DE_STOP),
    ("es", ES_STOP),
    ("pt", PT_STOP),
    ("it", IT_STOP),
    ("nl", NL_STOP),
];

/// Accented letters that never occur in ordinary English — the diacritic
/// signal that keeps French/Spanish/etc. apart from English even when the
/// stopword margin is thin.
const NON_EN_DIACRITICS: &str =
    "àáâãäåçèéêëìíîïñòóôõöøùúûüýÿšžœæßìíîïąćęłńśźżõāēīōūăâđêôơưğışİőűёїіїў";

/// Words = maximal runs of alphanumeric characters (the hand-rolled
/// splitter standing in for the source's `WORD` regex `[^\W\d_]+` — both
/// accept exactly the letter-or-digit runs; the equivalence test pins it).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The Latin-script language guess: stopword margins over the seven
/// supported languages plus the diacritic rate. `None` under four words
/// (too little signal); a non-English winner needs a real margin over
/// English; ordinary English never misroutes.
pub(crate) fn guess_latin_language(text: &str) -> Option<String> {
    let ws = words(text);
    if ws.len() < 4 {
        return None;
    }
    let lower: Vec<String> = ws.iter().map(|w| w.to_lowercase()).collect();
    let mut scores: Vec<(&str, usize)> = STOP_SETS
        .iter()
        .map(|(name, set)| {
            let hits = lower.iter().filter(|w| set.contains(&w.as_str())).count();
            (*name, hits)
        })
        .collect();
    scores.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let (best_name, best) = scores.first().copied()?;
    let en = scores.iter().find(|(n, _)| *n == "en").map(|(_, s)| *s)?;
    let diacritics = text
        .chars()
        .filter(|c| NON_EN_DIACRITICS.contains(*c))
        .count();
    let diacritic_rate = diacritics as f32 / ws.len() as f32;

    if best_name != "en" {
        let margin_ok = best >= en + 2 && best >= 2;
        if margin_ok {
            return Some(best_name.to_string());
        }
        if diacritic_rate >= 0.04 && best >= en {
            return Some(best_name.to_string());
        }
        return Some("en".to_string());
    }
    // English leads the stopword count outright.
    if en > 0 {
        return Some("en".to_string());
    }
    None
}

/// The full detection: script, per-script profile, Latin language guess,
/// the English verdict, and the non-Latin fraction (rounded to 4dp, `0.0`
/// when the text holds no letters).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct Detection {
    pub script: String,
    pub script_profile: Vec<(String, f32)>,
    pub language: Option<String>,
    pub is_english: bool,
    pub non_latin_fraction: f32,
}

pub(crate) fn analyse(state: &serde_json::Value) -> Detection {
    let text = state_text(state, 4000);
    let script = detect_script(&text);
    let profile = script_profile(&text);
    let latin_frac = profile
        .iter()
        .find(|(name, _)| name == "latin")
        .map(|(_, f)| *f)
        .unwrap_or(0.0);
    let non_latin = if profile.is_empty() {
        0.0
    } else {
        round4(1.0 - latin_frac)
    };
    if script == "unknown" {
        return Detection {
            script,
            script_profile: profile,
            language: None,
            is_english: true,
            non_latin_fraction: 0.0,
        };
    }
    if script != "latin" {
        return Detection {
            script,
            script_profile: profile,
            language: None,
            is_english: false,
            non_latin_fraction: non_latin,
        };
    }
    let language = guess_latin_language(&text);
    let is_english = language.as_deref().is_none_or(|l| l == "en");
    Detection {
        script,
        script_profile: profile,
        language,
        is_english,
        non_latin_fraction: non_latin,
    }
}

/// The fast path: is this state English text? (unknown scripts count as
/// English-safe — the router's default arm handles them).
pub(crate) fn is_english(state: &serde_json::Value) -> bool {
    analyse(state).is_english
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── the SCRIPTS family (14) ─────────────────────────────────────────
    #[test]
    fn script_english() {
        assert_eq!(detect_script("the quick brown fox"), "latin");
    }
    #[test]
    fn script_unknown_for_empty() {
        assert_eq!(detect_script(""), "unknown");
    }
    #[test]
    fn script_unknown_for_digits() {
        assert_eq!(detect_script("123 456"), "unknown");
    }
    #[test]
    fn script_greek() {
        assert_eq!(detect_script("αβγδε ελλάδα"), "greek");
    }
    #[test]
    fn script_cyrillic() {
        assert_eq!(detect_script("привет мир"), "cyrillic");
    }
    #[test]
    fn script_hebrew() {
        assert_eq!(detect_script("שלום עולם"), "hebrew");
    }
    #[test]
    fn script_arabic() {
        assert_eq!(detect_script("مرحبا بالعالم"), "arabic");
    }
    #[test]
    fn script_devanagari() {
        assert_eq!(detect_script("नमस्ते दुनिया"), "devanagari");
    }
    #[test]
    fn script_bengali() {
        assert_eq!(detect_script("হ্যালো বিশ্ব"), "bengali");
    }
    #[test]
    fn script_tamil() {
        assert_eq!(detect_script("வணக்கம் உலகம்"), "tamil");
    }
    #[test]
    fn script_thai() {
        assert_eq!(detect_script("สวัสดีชาวโลก"), "thai");
    }
    #[test]
    fn script_hangul() {
        assert_eq!(detect_script("안녕하세요 세계"), "hangul");
    }
    #[test]
    fn script_kana() {
        assert_eq!(detect_script("こんにちは世界"), "kana");
    }
    #[test]
    fn script_han() {
        assert_eq!(detect_script("你好世界"), "han");
    }

    // ── is_english (7) ──────────────────────────────────────────────────
    #[test]
    fn is_english_ordinary_text() {
        assert!(is_english(&json!(
            "the customer replaced the battery and it works"
        )));
    }
    #[test]
    fn is_english_empty_state() {
        assert!(is_english(&json!("")));
    }
    #[test]
    fn is_english_digits_only() {
        assert!(is_english(&json!("42 000")));
    }
    #[test]
    fn is_english_french_is_not() {
        assert!(!is_english(&json!(
            "le client a remplacé la batterie du portable"
        )));
    }
    #[test]
    fn is_english_cyrillic_is_not() {
        assert!(!is_english(&json!("клиент заменил батарею ноутбука")));
    }
    #[test]
    fn is_english_nested_state_flattens() {
        assert!(is_english(&json!({"a": ["the", "battery"], "b": "works"})));
    }
    #[test]
    fn is_english_non_latin_fraction_reported() {
        let d = analyse(&json!("привет мир"));
        assert_eq!(d.non_latin_fraction, 1.0);
    }

    // ── latin_lang (5+1 long-English) ───────────────────────────────────
    #[test]
    fn latin_lang_french() {
        assert_eq!(
            guess_latin_language("le client a remplacé la batterie du portable et il marche"),
            Some("fr".into())
        );
    }
    #[test]
    fn latin_lang_spanish() {
        assert_eq!(
            guess_latin_language("el cliente reemplazó la batería del portátil y funciona bien"),
            Some("es".into())
        );
    }
    #[test]
    fn latin_lang_german() {
        assert_eq!(
            guess_latin_language("der kunde hat die batterie des notebooks ersetzt und es geht"),
            Some("de".into())
        );
    }
    #[test]
    fn latin_lang_english_stays_en() {
        assert_eq!(
            guess_latin_language("the customer replaced the battery of the laptop and it works"),
            Some("en".into())
        );
    }
    #[test]
    fn latin_lang_too_short_is_none() {
        assert_eq!(guess_latin_language("hello there friend"), None);
    }
    #[test]
    fn latin_lang_long_english_stays_en() {
        let long =
            "the technician confirmed that the replacement battery holds its charge and the \
                     device is running the latest firmware update from this morning"
                .to_string();
        assert_eq!(guess_latin_language(&long), Some("en".into()));
    }

    // ── state_text (4 + keys-ignored) ───────────────────────────────────
    #[test]
    fn state_text_string_passthrough() {
        assert_eq!(state_text(&json!("hello world"), 4000), "hello world");
    }
    #[test]
    fn state_text_flattens_lists_and_dicts() {
        assert_eq!(
            state_text(&json!({"a": "one", "b": ["two", "three"]}), 4000),
            "one two three"
        );
    }
    #[test]
    fn state_text_truncates_on_char_boundary() {
        let long = "x".repeat(5_000);
        let out = state_text(&json!(long), 4000);
        assert_eq!(out.chars().count(), 4000);
    }
    #[test]
    fn state_text_multibyte_boundary_never_splits() {
        let text = "é".repeat(3_000);
        let out = state_text(&json!(text), 2500);
        assert_eq!(out.chars().count(), 2500);
        assert!(out.chars().all(|c| c == 'é'));
    }
    #[test]
    fn state_text_keys_are_ignored() {
        let out = state_text(&json!({"password": "hunter2", "note": "safe"}), 4000);
        // Values only (order is the JSON map's deterministic iteration):
        // the VALUE text rides, the KEY names never leak into model text.
        let words: Vec<&str> = out.split(' ').collect();
        assert!(words.contains(&"hunter2"), "{out}");
        assert!(words.contains(&"safe"), "{out}");
        assert!(!out.contains("password"), "{out}");
        assert!(!out.contains("note"), "{out}");
    }

    // ── the splitter equivalence (regex-free, documented) ────────────────
    #[test]
    fn word_splitter_equivalent_to_word_regex() {
        // `[^\W\d_]+` accepts letter runs; digits and underscores split. The
        // hand-rolled splitter accepts alphanumeric runs — digits included —
        // so the pinned equivalence contract is: identical word BOUNDARIES
        // for letter-only vocabularies, and no empty tokens ever.
        assert_eq!(
            words("don't stop-me now 42x_y"),
            vec!["don", "t", "stop", "me", "now", "42x", "y"]
        );
        assert!(words("").is_empty());
        assert!(words("...---...").is_empty());
    }

    // ── analyse/reporting precision ─────────────────────────────────────
    #[test]
    fn analyse_profiles_fractions_sum_to_one() {
        let d = analyse(&json!("hello mundo bonjour"));
        let total: f32 = d.script_profile.iter().map(|(_, f)| f).sum();
        assert!((total - 1.0).abs() < 0.001, "profile sums to ~1: {total}");
    }
    #[test]
    fn analyse_mixed_script_reports_dominant() {
        let d = analyse(&json!("hello世界hello"));
        assert_eq!(d.script, "latin", "10 latin letters beat 2 han");
        assert_eq!(d.non_latin_fraction, round4(2.0 / 12.0));
    }
}
