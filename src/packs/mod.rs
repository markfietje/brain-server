//! The Tier C pack format, as VALIDATION (plan 28 §4) — the one new artifact
//! a vertical plugin needs.
//!
//! A pack is a minisign-signed tar.gz carrying ONLY data and declarations.
//! This module owns the format's law, pure and I/O-free so the installer
//! (`brain plugin install`) and every pin drive the same validator:
//!
//! * **The file-set allowlist IS the format.** `manifest.json` (required),
//!   `domain.json`, `roles.json`, `taxonomy.json`, `rules.json`,
//!   `procedures/<name>.json`, `i18n/{de,en,es,fr,nl}.json`,
//!   `ui/manifest.json` — anything else is a refusal naming the file. There
//!   is no "other files are ignored" arm: a silent pass over an unlisted file
//!   is how a payload rides in wearing a data costume.
//! * **Packs are DATA, enforced structurally.** Any executable content —
//!   wasm, ELF/Mach-O/PE objects, scripts, interpreted shebangs — is a
//!   refusal naming the file. The kernel does not execute caller input; the
//!   installer does not unpack code. K5 connectors (supply-chained binaries)
//!   and K6 model adapters (sealed binaries) are refused AS KINDS: they are
//!   provenance-sealed binaries, never pack data (plan 28 §1's own table).
//! * **Every declaration validates against the machinery that will consume
//!   it**: `roles.json` through [`crate::role`]'s own parser and validator,
//!   `procedures/` through the registered wizard-pack schema (the
//!   compile-time-embedded catalog's shape — the drift gate stays
//!   byte-green), `rules.json` through the `rules` tables' shape,
//!   `domain.json` through the domain-create path's requirements, `i18n/*`
//!   through the five-catalog parity law.
//!
//! The adopt/refuse line this format encodes: **packs are signed DATA
//! adopted by an operator through doors that audit; the market is a later
//! phase and is a data market, never a code market.**

use std::collections::BTreeMap;

use crate::role;

pub mod archive;
pub mod install;
pub mod signature;

// ── the allowlist ───────────────────────────────────────────────────────────

pub const MANIFEST: &str = "manifest.json";
const DOMAIN: &str = "domain.json";
const ROLES: &str = "roles.json";
const TAXONOMY: &str = "taxonomy.json";
const RULES: &str = "rules.json";
const UI_MANIFEST: &str = "ui/manifest.json";
const PROCEDURES_DIR: &str = "procedures/";
const I18N_DIR: &str = "i18n/";
/// The five Tier A catalogs (the parity law's locales, sorted).
pub const I18N_LOCALES: [&str; 5] = ["de", "en", "es", "fr", "nl"];

/// The format's file set, verbatim for refusal messages: an operator holding
/// a refused pack can read the law that refused it.
pub const FILE_SET: &str = "manifest.json, domain.json, roles.json, taxonomy.json, \
     rules.json, procedures/<name>.json, i18n/{de,en,es,fr,nl}.json, ui/manifest.json";

/// Whether `path` is in the allowlist. `procedures/` and `i18n/` admit
/// structured members; everything else must be an exact name.
fn allowed(path: &str) -> bool {
    if let Some(rest) = path.strip_prefix(PROCEDURES_DIR) {
        return procedure_name_ok(rest);
    }
    if let Some(rest) = path.strip_prefix(I18N_DIR) {
        let Some(locale) = rest.strip_suffix(".json") else {
            return false;
        };
        return I18N_LOCALES.contains(&locale);
    }
    matches!(
        path,
        MANIFEST | DOMAIN | ROLES | TAXONOMY | RULES | UI_MANIFEST
    )
}

/// A `procedures/` member name: `<name>.json`, the stem domain-charset-ish
/// (lowercase alnum + hyphen + underscore, 1..=63) — it becomes the stored
/// procedure's title suffix. No nesting, no dotfiles.
fn procedure_name_ok(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    !stem.is_empty()
        && stem.len() <= 63
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// ── the executable-content refusal ─────────────────────────────────────────

/// File extensions that name executable content, whatever the bytes say.
/// A data pack has no use for any of them; each is a refusal, not a warning.
const REFUSED_EXTENSIONS: [&str; 17] = [
    "wasm", "so", "dylib", "dll", "exe", "sh", "bash", "zsh", "js", "mjs", "cjs", "py", "rb", "pl",
    "php", "ps1", "class",
];

/// Why this file is executable content, if it is: the name's extension and
/// the bytes' magic both count, plus the shebang — the interpreter
/// invocation a script wears as its first two bytes.
fn executable_reason(path: &str, bytes: &[u8]) -> Option<&'static str> {
    if let Some((_, ext)) = path.rsplit_once('.') {
        if REFUSED_EXTENSIONS.contains(&ext) {
            return Some("executable extension");
        }
    }
    // wasm: `\0asm`
    if bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]) {
        return Some("wasm magic");
    }
    // ELF
    if bytes.starts_with(&[0x7F, b'E', b'L', b'F']) {
        return Some("ELF object");
    }
    // PE (the two bytes survive every content-sniffing detour)
    if bytes.starts_with(b"MZ") {
        return Some("PE object");
    }
    // Mach-O 32/64, either endianness
    if bytes.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
        || bytes.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
        || bytes.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
        || bytes.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
    {
        return Some("Mach-O object");
    }
    // fat binary / Java class (the same magic serves both — both refused)
    if bytes.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]) || bytes.starts_with(&[0xBE, 0xBA, 0xFE, 0xCA])
    {
        return Some("fat/JVM binary");
    }
    // an interpreted shebang, whatever the extension claims
    if bytes.starts_with(b"#!") {
        return Some("interpreted shebang");
    }
    None
}

// ── the manifest ────────────────────────────────────────────────────────────

/// One plugin KIND (plan 28 §1's table). The pack vocabulary is the DATA
/// kinds only: K5 connectors and K6 model adapters are binaries by nature —
/// refused as kinds, not merely absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackKind {
    /// K1 — the vertical's isolated corpus + lifecycle.
    Domain,
    /// K2 — the vertical's role postures.
    Roles,
    /// K3 — the vertical's structured workflows (the wizard-pack schema).
    Procedures,
    /// K4 — the vertical's case taxonomy (classifier DATA).
    Taxonomy,
    /// K7 — the vertical's governance rows (the rules engine's shape).
    Rules,
}

impl PackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PackKind::Domain => "domain",
            PackKind::Roles => "roles",
            PackKind::Procedures => "procedures",
            PackKind::Taxonomy => "taxonomy",
            PackKind::Rules => "rules",
        }
    }

    fn from_manifest_name(name: &str) -> Result<Self, String> {
        match name {
            "domain" => Ok(PackKind::Domain),
            "roles" => Ok(PackKind::Roles),
            "procedures" => Ok(PackKind::Procedures),
            "taxonomy" => Ok(PackKind::Taxonomy),
            "rules" => Ok(PackKind::Rules),
            // The two binary kinds get their OWN refusal, naming why: a pack
            // is data, and these are the kinds that would have to be code.
            "connector" => Err(
                "kind 'connector' (K5) is refused in packs: connectors are supply-chained \
                 binaries with their own principal, never pack data"
                    .to_string(),
            ),
            "model-adapter" | "model_adapter" => Err(
                "kind 'model-adapter' (K6) is refused in packs: model adapters are sealed, \
                 provenance-attested binaries, never pack data"
                    .to_string(),
            ),
            other => Err(format!(
                "unknown kind '{other}' (the pack vocabulary is domain, roles, procedures, \
                 taxonomy, rules)"
            )),
        }
    }
}

/// The parsed `manifest.json`: who the pack is, what it runs against, what
/// it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct PackManifest {
    /// Domain-charset id (it becomes the `installed_packs` key and a
    /// filename stem on the operator's disk).
    pub id: String,
    /// `X.Y.Z`.
    pub version: String,
    /// The kernel-engine range, e.g. `">=1.29.0 <2.0.0"`.
    pub engine: String,
    /// The kind list — the DATA kinds only.
    pub kinds: Vec<PackKind>,
}

// ── the declaration shapes ──────────────────────────────────────────────────

/// `domain.json` — the K1 domain-create spec, validated against the
/// domain-create path's requirements.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainSpec {
    pub name: String,
    pub label: String,
    pub retention_days: i64,
    /// Optional profile binding (`crate::profile::bind` verifies existence
    /// at import; the format verifies the name's charset).
    pub profile: Option<String>,
}

/// One `rules.json` row, shaped exactly as the `rules` table's columns
/// (`jurisdiction, subject, rule_key, body, source_ref, effective_at,
/// reviewed_at, expires_at, revision`).
#[derive(Debug, Clone, PartialEq)]
pub struct RuleRow {
    pub jurisdiction: String,
    pub subject: String,
    pub rule_key: String,
    pub body: String,
    pub source_ref: String,
    pub effective_at: i64,
    pub reviewed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revision: i64,
}

/// A validated pack: every arm parsed against its consuming machinery.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedPack {
    pub manifest: PackManifest,
    pub domain: Option<DomainSpec>,
    pub roles: Vec<role::Role>,
    /// `(stem, the wizard-pack JSON as parsed)` — the registered schema's
    /// own validation ran at [`validate`]; the installer lands each as
    /// procedure DATA (content), never as a catalog entry.
    pub procedures: Vec<(String, serde_json::Value)>,
    pub taxonomy: Option<serde_json::Value>,
    pub rules: Vec<RuleRow>,
    /// locale → (key → string); present iff the pack carries `i18n/`.
    pub i18n: Option<BTreeMap<String, BTreeMap<String, String>>>,
    pub ui: Option<serde_json::Value>,
}

// ── the error taxonomy ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackErrorKind {
    UnknownFile,
    ExecutableContent,
    BadManifest,
    BadRoles,
    BadProcedure,
    BadTaxonomy,
    BadRules,
    BadDomain,
    BadI18n,
    BadUi,
    BadEngine,
}

impl PackErrorKind {
    fn as_str(self) -> &'static str {
        match self {
            PackErrorKind::UnknownFile => "unknown_file",
            PackErrorKind::ExecutableContent => "executable_content",
            PackErrorKind::BadManifest => "bad_manifest",
            PackErrorKind::BadRoles => "bad_roles",
            PackErrorKind::BadProcedure => "bad_procedure",
            PackErrorKind::BadTaxonomy => "bad_taxonomy",
            PackErrorKind::BadRules => "bad_rules",
            PackErrorKind::BadDomain => "bad_domain",
            PackErrorKind::BadI18n => "bad_i18n",
            PackErrorKind::BadUi => "bad_ui",
            PackErrorKind::BadEngine => "bad_engine",
        }
    }
}

/// A format refusal: code + the file that earned it + the reason. `Display`
/// is what the CLI prints and what the pins match — a refusal that cannot
/// name its file cannot be told from a broken archive.
#[derive(Debug, Clone, PartialEq)]
pub struct PackError {
    pub kind: PackErrorKind,
    pub file: Option<String>,
    pub message: String,
}

impl PackError {
    fn new(kind: PackErrorKind, file: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            file: Some(file.into()),
            message: message.into(),
        }
    }

    fn bare(kind: PackErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            file: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.file {
            Some(file) => write!(
                f,
                "pack_refused[{}: {file}]: {}",
                self.kind.as_str(),
                self.message
            ),
            None => write!(f, "pack_refused[{}]: {}", self.kind.as_str(), self.message),
        }
    }
}

// ── the validator ───────────────────────────────────────────────────────────

/// Validate one pack, arm by arm, from its in-memory files. Pure: the caller
/// (the installer) owns the archive's bytes and the signature check that
/// must have run BEFORE this — a pack that fails its signature is never
/// validated, let alone imported.
pub fn validate_pack(entries: &[(&str, Vec<u8>)]) -> Result<ValidatedPack, PackError> {
    // Structural law first — and the executable scan before the allowlist,
    // so a payload is named AS a payload, never merely as an unlisted file:
    for (path, bytes) in entries {
        if let Some(reason) = executable_reason(path, bytes) {
            return Err(PackError::new(
                PackErrorKind::ExecutableContent,
                *path,
                format!("{reason} — a pack is data; nothing in a pack may execute"),
            ));
        }
    }
    // the allowlist over EVERY entry — there is no "ignored files" arm, so a
    // payload cannot ride in wearing a data costume:
    for (path, _) in entries {
        if !allowed(path) {
            return Err(PackError::new(
                PackErrorKind::UnknownFile,
                *path,
                format!("not in the pack format's file set ({FILE_SET})"),
            ));
        }
    }

    // The manifest: required, parsed, kinds closed.
    let manifest_bytes = entries
        .iter()
        .find(|(p, _)| *p == MANIFEST)
        .map(|(_, b)| b)
        .ok_or_else(|| {
            PackError::bare(
                PackErrorKind::BadManifest,
                format!("every pack carries {MANIFEST}"),
            )
        })?;
    let manifest = parse_manifest(manifest_bytes)?;

    // Arm by arm, deterministic order: domain, roles, procedures, taxonomy,
    // rules, i18n, ui.
    let domain = match entries.iter().find(|(p, _)| *p == DOMAIN) {
        Some((_, bytes)) => Some(parse_domain(bytes)?),
        None => None,
    };
    let roles = match entries.iter().find(|(p, _)| *p == ROLES) {
        Some((_, bytes)) => parse_roles(bytes)?,
        None => Vec::new(),
    };
    let mut procedures = Vec::new();
    for (path, bytes) in entries
        .iter()
        .filter(|(p, _)| p.starts_with(PROCEDURES_DIR))
    {
        let stem = path
            .strip_prefix(PROCEDURES_DIR)
            .and_then(|n| n.strip_suffix(".json"))
            .expect("allowlist checked the shape")
            .to_string();
        let value = parse_procedure(path, bytes)?;
        procedures.push((stem, value));
    }
    let taxonomy = match entries.iter().find(|(p, _)| *p == TAXONOMY) {
        Some((_, bytes)) => Some(parse_taxonomy(bytes)?),
        None => None,
    };
    let rules = match entries.iter().find(|(p, _)| *p == RULES) {
        Some((_, bytes)) => parse_rules(bytes)?,
        None => Vec::new(),
    };
    let i18n = parse_i18n(entries)?;
    let ui = match entries.iter().find(|(p, _)| *p == UI_MANIFEST) {
        Some((_, bytes)) => Some(parse_ui(bytes)?),
        None => None,
    };
    // A Tier A manifest names label keys the shell renders — they resolve
    // through the five catalogs, so a manifest without them is half a
    // surface (the parity law, at the format).
    if ui.is_some() && i18n.is_none() {
        return Err(PackError::new(
            PackErrorKind::BadI18n,
            I18N_DIR,
            "a pack carrying ui/manifest.json must carry all five i18n catalogs — an \
             extension cannot ship half-translated",
        ));
    }

    Ok(ValidatedPack {
        manifest,
        domain,
        roles,
        procedures,
        taxonomy,
        rules,
        i18n,
        ui,
    })
}

fn json_object<'a>(
    bytes: &'a [u8],
    kind: PackErrorKind,
    file: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, PackError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| PackError::new(kind, file, format!("invalid JSON: {e}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| PackError::new(kind, file, "must be a JSON object"))
}

fn parse_manifest(bytes: &[u8]) -> Result<PackManifest, PackError> {
    let obj = json_object(bytes, PackErrorKind::BadManifest, MANIFEST)?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PackError::bare(PackErrorKind::BadManifest, "id must be a string"))?;
    if !crate::storage_layout::is_valid_domain(id) {
        return Err(PackError::bare(
            PackErrorKind::BadManifest,
            format!("invalid pack id '{id}' (lowercase alnum + hyphen, max 63)"),
        ));
    }
    let version = obj
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PackError::bare(PackErrorKind::BadManifest, "version must be a string"))?;
    if !version_ok(version) {
        return Err(PackError::bare(
            PackErrorKind::BadManifest,
            format!("invalid version '{version}' (X.Y.Z)"),
        ));
    }
    let engine = obj
        .get("engine")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PackError::bare(PackErrorKind::BadManifest, "engine must be a string"))?;
    if !engine_range_ok(engine) {
        return Err(PackError::bare(
            PackErrorKind::BadManifest,
            format!(
                "invalid engine range '{engine}' (comparators over X.Y.Z, e.g. '>=1.29.0 <2.0.0')"
            ),
        ));
    }
    let kinds_raw = obj
        .get("kinds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| PackError::bare(PackErrorKind::BadManifest, "kinds must be an array"))?;
    if kinds_raw.is_empty() {
        return Err(PackError::bare(
            PackErrorKind::BadManifest,
            "kinds must be non-empty — a pack that declares nothing is not a pack",
        ));
    }
    let mut kinds = Vec::with_capacity(kinds_raw.len());
    for k in kinds_raw {
        let name = k.as_str().ok_or_else(|| {
            PackError::bare(PackErrorKind::BadManifest, "each kind must be a string")
        })?;
        kinds.push(
            PackKind::from_manifest_name(name)
                .map_err(|m| PackError::bare(PackErrorKind::BadManifest, m))?,
        );
    }
    kinds.sort();
    kinds.dedup();
    Ok(PackManifest {
        id: id.to_string(),
        version: version.to_string(),
        engine: engine.to_string(),
        kinds,
    })
}

fn version_ok(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
}

/// One comparator clause: optional op + `X.Y.Z`.
fn engine_clause_ok(clause: &str) -> bool {
    version_ok(strip_op(clause))
}

fn strip_op(clause: &str) -> &str {
    clause
        .strip_prefix(">=")
        .or_else(|| clause.strip_prefix("<="))
        .or_else(|| clause.strip_prefix('='))
        .or_else(|| clause.strip_prefix('<'))
        .or_else(|| clause.strip_prefix('>'))
        .unwrap_or(clause)
}

fn engine_range_ok(range: &str) -> bool {
    !range.is_empty() && range.len() <= 64 && range.split_whitespace().all(engine_clause_ok)
}

/// Whether a kernel version satisfies the manifest's engine range. Every
/// clause must hold; a bare `X.Y.Z` clause is an exact pin.
pub fn engine_matches(range: &str, version: &str) -> bool {
    let Some(kernel) = parse_version(version) else {
        return false;
    };
    range.split_whitespace().all(|clause| {
        let op = if clause.starts_with(">=") {
            ">="
        } else if clause.starts_with("<=") {
            "<="
        } else if clause.starts_with('<') {
            "<"
        } else if clause.starts_with('>') {
            ">"
        } else {
            "="
        };
        let Some(bound) = parse_version(strip_op(clause)) else {
            return false;
        };
        match op {
            ">=" => kernel >= bound,
            "<=" => kernel <= bound,
            ">" => kernel > bound,
            "<" => kernel < bound,
            _ => kernel == bound,
        }
    })
}

/// The version tuple behind [`engine_matches`] and the installer's
/// register law — X.Y.Z compared numerically, never lexically (0.10.0 is
/// NOT below 0.9.0).
pub(crate) fn parse_version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    parse_version(version)
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

fn parse_domain(bytes: &[u8]) -> Result<DomainSpec, PackError> {
    let obj = json_object(bytes, PackErrorKind::BadDomain, DOMAIN)?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PackError::new(PackErrorKind::BadDomain, DOMAIN, "name must be a string"))?;
    // The domain-create path's own requirement — the SAME charset law the
    // HTTP create route and every per-domain file name enforce.
    if !crate::storage_layout::is_valid_domain(name) {
        return Err(PackError::new(
            PackErrorKind::BadDomain,
            DOMAIN,
            format!("invalid domain name '{name}' (lowercase alnum + hyphen, max 63)"),
        ));
    }
    let label = obj.get("label").and_then(|v| v.as_str()).ok_or_else(|| {
        PackError::new(PackErrorKind::BadDomain, DOMAIN, "label must be a string")
    })?;
    if label.trim().is_empty() || label.len() > 128 {
        return Err(PackError::new(
            PackErrorKind::BadDomain,
            DOMAIN,
            "label must be a non-empty string of at most 128 chars",
        ));
    }
    let retention_days = obj
        .get("retention_days")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            PackError::new(
                PackErrorKind::BadDomain,
                DOMAIN,
                "retention_days must be an integer",
            )
        })?;
    if retention_days < 1 {
        return Err(PackError::new(
            PackErrorKind::BadDomain,
            DOMAIN,
            "retention_days must be >= 1",
        ));
    }
    let profile = match obj.get("profile") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let p = v.as_str().ok_or_else(|| {
                PackError::new(PackErrorKind::BadDomain, DOMAIN, "profile must be a string")
            })?;
            if !crate::profile::is_valid_profile_name(p) {
                return Err(PackError::new(
                    PackErrorKind::BadDomain,
                    DOMAIN,
                    format!("invalid profile name '{p}'"),
                ));
            }
            Some(p.to_string())
        }
    };
    Ok(DomainSpec {
        name: name.to_string(),
        label: label.to_string(),
        retention_days,
        profile,
    })
}

fn parse_roles(bytes: &[u8]) -> Result<Vec<role::Role>, PackError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        PackError::new(PackErrorKind::BadRoles, ROLES, format!("invalid JSON: {e}"))
    })?;
    let arr = value.as_array().ok_or_else(|| {
        PackError::new(
            PackErrorKind::BadRoles,
            ROLES,
            "must be a JSON array of roles",
        )
    })?;
    if arr.is_empty() {
        return Err(PackError::new(
            PackErrorKind::BadRoles,
            ROLES,
            "a roles.json that declares nothing is not a pack section",
        ));
    }
    let mut roles = Vec::with_capacity(arr.len());
    for raw in arr {
        // The role store's own type and the write seam's own validator — the
        // parser's error IS the format's error, verbatim.
        let r: role::Role = serde_json::from_value(raw.clone())
            .map_err(|e| PackError::new(PackErrorKind::BadRoles, ROLES, e.to_string()))?;
        role::validate(&r).map_err(|e| PackError::new(PackErrorKind::BadRoles, ROLES, e))?;
        roles.push(r);
    }
    Ok(roles)
}

fn parse_procedure(path: &str, bytes: &[u8]) -> Result<serde_json::Value, PackError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        PackError::new(
            PackErrorKind::BadProcedure,
            path,
            format!("invalid JSON: {e}"),
        )
    })?;
    // The registered wizard-pack schema — the closed question types, the
    // branch totality, the termination law. Its refusal text is the
    // format's, verbatim (`wizard_pack_invalid: ...`).
    crate::workflow::wizard::validate_wizard_pack(&value)
        .map_err(|e| PackError::new(PackErrorKind::BadProcedure, path, e))?;
    Ok(value)
}

fn parse_taxonomy(bytes: &[u8]) -> Result<serde_json::Value, PackError> {
    let obj = json_object(bytes, PackErrorKind::BadTaxonomy, TAXONOMY)?;
    // Classifier DATA, content not code: label → array of terms. Keys and
    // terms are strings; nothing here is interpreted, ever.
    for (label, terms) in &obj {
        if label.is_empty() || label.len() > 63 {
            return Err(PackError::new(
                PackErrorKind::BadTaxonomy,
                TAXONOMY,
                format!("label '{label}' invalid (1..=63 chars)"),
            ));
        }
        let Some(arr) = terms.as_array() else {
            return Err(PackError::new(
                PackErrorKind::BadTaxonomy,
                TAXONOMY,
                format!("label '{label}' must map to an array of terms"),
            ));
        };
        if arr.iter().any(|t| !t.is_string()) {
            return Err(PackError::new(
                PackErrorKind::BadTaxonomy,
                TAXONOMY,
                format!("label '{label}' carries a non-string term"),
            ));
        }
    }
    Ok(serde_json::Value::Object(obj))
}

fn parse_rules(bytes: &[u8]) -> Result<Vec<RuleRow>, PackError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        PackError::new(PackErrorKind::BadRules, RULES, format!("invalid JSON: {e}"))
    })?;
    let arr = value.as_array().ok_or_else(|| {
        PackError::new(
            PackErrorKind::BadRules,
            RULES,
            "must be a JSON array of rule rows",
        )
    })?;
    let mut rows = Vec::with_capacity(arr.len());
    let mut keys = std::collections::BTreeSet::new();
    for raw in arr {
        let obj = raw.as_object().ok_or_else(|| {
            PackError::new(
                PackErrorKind::BadRules,
                RULES,
                "each rule must be a JSON object",
            )
        })?;
        let text = |field: &str| -> Result<String, PackError> {
            obj.get(field)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| {
                    PackError::new(
                        PackErrorKind::BadRules,
                        RULES,
                        format!("{field} must be a non-empty string"),
                    )
                })
        };
        let jurisdiction = text("jurisdiction")?;
        let subject = text("subject")?;
        let rule_key = text("rule_key")?;
        let body = text("body")?;
        let source_ref = text("source_ref")?;
        for (name, v) in [
            ("jurisdiction", &jurisdiction),
            ("subject", &subject),
            ("rule_key", &rule_key),
        ] {
            if v.is_empty() || v.len() > 63 {
                return Err(PackError::new(
                    PackErrorKind::BadRules,
                    RULES,
                    format!("{name} must be 1..=63 chars"),
                ));
            }
        }
        if body.trim().is_empty() || body.len() > 8192 {
            return Err(PackError::new(
                PackErrorKind::BadRules,
                RULES,
                "body must be a non-empty string of at most 8192 bytes",
            ));
        }
        if source_ref.len() > 255 {
            return Err(PackError::new(
                PackErrorKind::BadRules,
                RULES,
                "source_ref must be at most 255 chars",
            ));
        }
        let int = |field: &str| -> Result<i64, PackError> {
            obj.get(field).and_then(|v| v.as_i64()).ok_or_else(|| {
                PackError::new(
                    PackErrorKind::BadRules,
                    RULES,
                    format!("{field} must be an integer"),
                )
            })
        };
        let effective_at = int("effective_at")?;
        if effective_at < 0 {
            return Err(PackError::new(
                PackErrorKind::BadRules,
                RULES,
                "effective_at must be >= 0",
            ));
        }
        let reviewed_at = match obj.get("reviewed_at") {
            None | Some(serde_json::Value::Null) => None,
            Some(_) => Some(int("reviewed_at")?),
        };
        let expires_at = match obj.get("expires_at") {
            None | Some(serde_json::Value::Null) => None,
            Some(_) => Some(int("expires_at")?),
        };
        let revision = int("revision")?;
        if revision < 1 {
            return Err(PackError::new(
                PackErrorKind::BadRules,
                RULES,
                "revision must be >= 1",
            ));
        }
        if !keys.insert(rule_key.clone()) {
            return Err(PackError::new(
                PackErrorKind::BadRules,
                RULES,
                format!("duplicate rule_key '{rule_key}' within the pack"),
            ));
        }
        rows.push(RuleRow {
            jurisdiction,
            subject,
            rule_key,
            body,
            source_ref,
            effective_at,
            reviewed_at,
            expires_at,
            revision,
        });
    }
    if rows.is_empty() {
        return Err(PackError::new(
            PackErrorKind::BadRules,
            RULES,
            "a rules.json that declares nothing is not a pack section",
        ));
    }
    Ok(rows)
}

fn parse_i18n(
    entries: &[(&str, Vec<u8>)],
) -> Result<Option<BTreeMap<String, BTreeMap<String, String>>>, PackError> {
    let present: Vec<_> = entries
        .iter()
        .filter(|(p, _)| p.starts_with(I18N_DIR))
        .collect();
    if present.is_empty() {
        return Ok(None);
    }
    // The parity law, at the format: all five, each a flat string→string
    // catalog, identical key sets.
    if present.len() != I18N_LOCALES.len() {
        return Err(PackError::new(
            PackErrorKind::BadI18n,
            I18N_DIR,
            format!(
                "all five catalogs are required ({:?}); found {}",
                I18N_LOCALES,
                present.len()
            ),
        ));
    }
    let mut catalogs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut reference: Option<Vec<String>> = None;
    for (path, bytes) in present {
        let locale = path
            .strip_prefix(I18N_DIR)
            .and_then(|r| r.strip_suffix(".json"))
            .expect("allowlist checked the shape");
        let obj = json_object(bytes, PackErrorKind::BadI18n, path)?;
        let mut catalog = BTreeMap::new();
        for (key, value) in &obj {
            let text = value.as_str().ok_or_else(|| {
                PackError::new(
                    PackErrorKind::BadI18n,
                    *path,
                    format!("'{key}' must map to a string"),
                )
            })?;
            if text.trim().is_empty() {
                return Err(PackError::new(
                    PackErrorKind::BadI18n,
                    *path,
                    format!("'{key}' maps to an empty string"),
                ));
            }
            catalog.insert(key.clone(), text.to_string());
        }
        let keys: Vec<String> = catalog.keys().cloned().collect();
        match &reference {
            None => {
                reference = Some(keys);
            }
            Some(expected) => {
                if keys != *expected {
                    return Err(PackError::new(
                        PackErrorKind::BadI18n,
                        *path,
                        "catalog key set differs from the other locales — the parity law \
                         forbids a half-translated surface",
                    ));
                }
            }
        }
        catalogs.insert(locale.to_string(), catalog);
    }
    Ok(Some(catalogs))
}

fn parse_ui(bytes: &[u8]) -> Result<serde_json::Value, PackError> {
    let obj = json_object(bytes, PackErrorKind::BadUi, UI_MANIFEST)?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PackError::new(PackErrorKind::BadUi, UI_MANIFEST, "id must be a string"))?;
    if id.is_empty() || id.len() > 63 {
        return Err(PackError::new(
            PackErrorKind::BadUi,
            UI_MANIFEST,
            "id must be 1..=63 chars",
        ));
    }
    let keys = obj
        .get("labelKeys")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            PackError::new(
                PackErrorKind::BadUi,
                UI_MANIFEST,
                "labelKeys must be an array of message keys",
            )
        })?;
    for k in keys {
        let key = k.as_str().ok_or_else(|| {
            PackError::new(
                PackErrorKind::BadUi,
                UI_MANIFEST,
                "each labelKey must be a string",
            )
        })?;
        if key.is_empty() || key.len() > 128 || key.trim() != key {
            return Err(PackError::new(
                PackErrorKind::BadUi,
                UI_MANIFEST,
                format!("labelKey '{key}' invalid (non-empty, trimmed, max 128)"),
            ));
        }
    }
    Ok(serde_json::Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json(kinds: &[&str]) -> Vec<u8> {
        serde_json::json!({
            "id": "support-pack",
            "version": "0.1.0",
            "engine": ">=1.29.0 <2.0.0",
            "kinds": kinds
        })
        .to_string()
        .into_bytes()
    }

    fn rules_json() -> Vec<u8> {
        serde_json::json!([{
            "jurisdiction": "ph",
            "subject": "support-tickets",
            "rule_key": "ticket_retention",
            "body": "Resolved ticket corpus is retained 90 days post-resolution.",
            "source_ref": "pack#retention",
            "effective_at": 1_700_000_000,
            "reviewed_at": 1_700_000_000,
            "revision": 1
        }])
        .to_string()
        .into_bytes()
    }

    fn roles_json() -> Vec<u8> {
        serde_json::json!([{
            "name": "support-agent",
            "scopes": ["domain"],
            "owner_filter": "self",
            "can": ["read", "write"]
        }])
        .to_string()
        .into_bytes()
    }

    fn wizard_pack_json() -> Vec<u8> {
        serde_json::json!({
            "pack": "support-escalation",
            "first": "q_triage",
            "questions": [
                {
                    "id": "q_triage",
                    "qtype": "choice",
                    "instructions": "Which class does this ticket belong to?",
                    "criteria": { "outage": "down, outage", "routine": "how to, question" },
                    "next": { "outage": "end", "routine": "end" }
                }
            ]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_clean_pack_validates_arm_by_arm() {
        let mut all: Vec<(&str, Vec<u8>)> = vec![
            (
                MANIFEST,
                manifest_json(&["domain", "rules", "roles", "procedures"]),
            ),
            (
                DOMAIN,
                serde_json::json!({
                    "name": "support", "label": "Support desk", "retention_days": 90
                })
                .to_string()
                .into_bytes(),
            ),
            (ROLES, roles_json()),
            ("procedures/escalation.json", wizard_pack_json()),
            (RULES, rules_json()),
            (
                UI_MANIFEST,
                serde_json::json!({
                    "id": "supportPack", "version": "0.1.0", "labelKeys": ["nav.support"]
                })
                .to_string()
                .into_bytes(),
            ),
        ];
        for locale in I18N_LOCALES {
            let path: &'static str = Box::leak(format!("i18n/{locale}.json").into_boxed_str());
            all.push((
                path,
                serde_json::json!({ "nav.support": "Support desk" })
                    .to_string()
                    .into_bytes(),
            ));
        }
        let pack = validate_pack(&all).expect("a clean pack passes");
        assert_eq!(pack.manifest.id, "support-pack");
        assert_eq!(pack.manifest.kinds.len(), 4);
        assert_eq!(pack.domain.as_ref().unwrap().name, "support");
        assert_eq!(pack.roles.len(), 1);
        assert_eq!(pack.procedures.len(), 1);
        assert_eq!(pack.rules.len(), 1);
        assert_eq!(pack.i18n.as_ref().unwrap().len(), 5);
        assert!(pack.ui.is_some());
    }

    #[test]
    fn an_unknown_file_reds_naming_it() {
        // an unlisted, non-executable payload: the allowlist refusal
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("assets/blob.bin", b"harmless".to_vec()),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::UnknownFile);
        assert_eq!(err.file.as_deref(), Some("assets/blob.bin"));
        assert!(err.message.contains("file set"));

        // an unlisted EXECUTABLE payload: named as a payload, not merely
        // as an unlisted file
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("hooks/notify.sh", b"echo hi".to_vec()),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::ExecutableContent);
        assert_eq!(err.file.as_deref(), Some("hooks/notify.sh"));
    }

    #[test]
    fn executable_content_reds_naming_the_file() {
        // wasm magic inside a plausibly-named data file
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            (RULES, rules_json()),
            ("taxonomy.json", {
                let mut v = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
                v.extend_from_slice(b"payload");
                v
            }),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::ExecutableContent);
        assert_eq!(err.file.as_deref(), Some("taxonomy.json"));
        assert!(err.message.contains("wasm"));

        // a shebang wearing an allowed name
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("rules.json", b"#!/bin/sh\necho pwned".to_vec()),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::ExecutableContent);
        assert!(err.message.contains("shebang"));

        // an ELF under a .json skirt — the extension law catches the name
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("payload.so", {
                let mut v = vec![0x7F, b'E', b'L', b'F'];
                v.extend_from_slice(&[0u8; 16]);
                v
            }),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::ExecutableContent);
        assert_eq!(err.file.as_deref(), Some("payload.so"));

        // a script extension, harmless bytes
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("i18n/payload.py", b"print('hi')".to_vec()),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::ExecutableContent);

        // Mach-O, the arm a macOS payload actually arrives in
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            ("blob.bin", {
                let mut v = vec![0xCF, 0xFA, 0xED, 0xFE];
                v.extend_from_slice(&[0u8; 8]);
                v
            }),
        ];
        assert_eq!(
            validate_pack(&entries).unwrap_err().kind,
            PackErrorKind::ExecutableContent
        );
    }

    #[test]
    fn k5_and_k6_kinds_are_refused_as_kinds() {
        for kind in ["connector", "model-adapter"] {
            let entries = vec![(MANIFEST, manifest_json(&["rules", kind]))];
            let err = validate_pack(&entries).unwrap_err();
            assert_eq!(err.kind, PackErrorKind::BadManifest);
            assert!(
                err.message.contains("refused in packs"),
                "'{kind}' must be refused naming the reason, got: {err}"
            );
        }
    }

    #[test]
    fn a_malformed_roles_json_reds_with_the_parsers_own_error() {
        // structurally valid JSON, invalid per role.rs's own validator
        let entries = vec![
            (MANIFEST, manifest_json(&["roles"])),
            (
                ROLES,
                serde_json::json!([{
                    "name": "Bad Name",
                    "scopes": ["domain"],
                    "owner_filter": "self",
                    "can": ["read"]
                }])
                .to_string()
                .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadRoles);
        assert!(
            err.message.contains("invalid role name"),
            "the role parser's own error must surface verbatim, got: {err}"
        );

        // an unknown `can` action — the same law, the other arm
        let entries = vec![
            (MANIFEST, manifest_json(&["roles"])),
            (
                ROLES,
                serde_json::json!([{
                    "name": "ok-name",
                    "scopes": ["domain"],
                    "owner_filter": "self",
                    "can": ["launch-missiles"]
                }])
                .to_string()
                .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert!(err.message.contains("unknown"));
    }

    #[test]
    fn a_malformed_procedure_reds_with_the_registered_schemas_error() {
        // branch map not total: an answer key with no `next` target
        let entries = vec![
            (MANIFEST, manifest_json(&["procedures"])),
            (
                "procedures/broken.json",
                serde_json::json!({
                    "pack": "broken",
                    "first": "q1",
                    "questions": [
                        {
                            "id": "q1",
                            "qtype": "choice",
                            "instructions": "pick",
                            "criteria": { "a": "alpha", "b": "beta" },
                            "next": { "a": "end" }
                        }
                    ]
                })
                .to_string()
                .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadProcedure);
        assert_eq!(err.file.as_deref(), Some("procedures/broken.json"));
        assert!(
            err.message.contains("wizard_pack_invalid"),
            "the registered schema's refusal must surface verbatim, got: {err}"
        );
    }

    #[test]
    fn a_malformed_rules_json_reds_with_the_tables_shape() {
        // revision below 1 — the rules table's optimistic-concurrency floor
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            (
                RULES,
                serde_json::json!([{
                    "jurisdiction": "ph", "subject": "s", "rule_key": "k",
                    "body": "b", "source_ref": "r", "effective_at": 0, "revision": 0
                }])
                .to_string()
                .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadRules);
        assert!(err.message.contains("revision"));

        // duplicate rule_key within the pack
        let row = serde_json::json!({
            "jurisdiction": "ph", "subject": "s", "rule_key": "k",
            "body": "b", "source_ref": "r", "effective_at": 0, "revision": 1
        });
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            (
                RULES,
                serde_json::json!([row.clone(), row])
                    .to_string()
                    .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert!(err.message.contains("duplicate rule_key"));
    }

    #[test]
    fn a_bad_domain_reds_with_the_create_paths_requirement() {
        let entries = vec![
            (MANIFEST, manifest_json(&["domain"])),
            (
                DOMAIN,
                serde_json::json!({
                    "name": "Support Desk", "label": "x", "retention_days": 90
                })
                .to_string()
                .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadDomain);
        assert!(err.message.contains("invalid domain name"));
    }

    #[test]
    fn i18n_parity_holds_at_the_format() {
        let base = |keys: &[&str]| {
            let m: serde_json::Map<String, serde_json::Value> = keys
                .iter()
                .map(|k| (k.to_string(), serde_json::json!("value")))
                .collect();
            serde_json::Value::Object(m).to_string().into_bytes()
        };
        // four of five catalogs: a refusal
        let mut entries = vec![(MANIFEST, manifest_json(&["rules"]))];
        for locale in ["de", "en", "es", "fr"] {
            entries.push((
                Box::leak(format!("i18n/{locale}.json").into_boxed_str()),
                base(&["nav.support"]),
            ));
        }
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadI18n);
        assert!(err.message.contains("all five"));

        // five catalogs, one key short in nl: the parity law names it
        let mut entries = vec![(MANIFEST, manifest_json(&["rules"]))];
        for locale in I18N_LOCALES {
            let keys: &[&str] = if locale == "nl" {
                &["other.key"]
            } else {
                &["nav.support"]
            };
            entries.push((
                Box::leak(format!("i18n/{locale}.json").into_boxed_str()),
                base(keys),
            ));
        }
        let err = validate_pack(&entries).unwrap_err();
        assert!(err.message.contains("parity law"), "{err}");

        // a ui manifest without i18n at all: half a surface
        let entries = vec![
            (MANIFEST, manifest_json(&["rules"])),
            (
                UI_MANIFEST,
                serde_json::json!({ "id": "supportPack", "labelKeys": ["nav.support"] })
                    .to_string()
                    .into_bytes(),
            ),
        ];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadI18n);
    }

    #[test]
    fn the_engine_range_gates_the_kernel_version() {
        assert!(engine_matches(">=1.29.0 <2.0.0", "1.29.34"));
        assert!(!engine_matches(">=1.29.0 <2.0.0", "2.0.0"));
        assert!(!engine_matches(">=1.30.0", "1.29.34"));
        assert!(engine_matches("=1.29.34", "1.29.34"));
        assert!(!engine_matches("=1.29.34", "1.29.35"));
        assert!(
            engine_matches("1.29.34", "1.29.34"),
            "a bare version is an exact pin"
        );
    }

    #[test]
    fn a_missing_manifest_is_a_refusal_not_a_panic() {
        let entries = vec![(RULES, rules_json())];
        let err = validate_pack(&entries).unwrap_err();
        assert_eq!(err.kind, PackErrorKind::BadManifest);
    }
}
