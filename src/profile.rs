//! v1.21.0 "Profiles" — a Profile is a typed JSON bundle of *existing* knob
//! defaults (access_scope, PII posture, per-kind retention, audit level, kind
//! vocabulary), stored as one row per name and bound to a domain. No new
//! governance primitives: every field configures a v1.14/v1.15/v1.17.1 seam.
//!
//! Apply semantics (the invariant): **the profile sets defaults, the row
//! wins.** An explicit per-row value (access_scope, expires_at/ttl_days) is
//! never overridden. A domain with no bound profile is byte-identical to
//! pre-v1.21 behavior (the back-compat invariant, test-pinned).
//!
//! PII modes: `off` (no change), `standard` (the v1.14 read-time output
//! redaction — unchanged), `strict` (write-time masking via the existing
//! `screen_source_prompt` maskers: email/phone/card placeholders are stored,
//! raw values never land in the DB). Deliberately NOT a vault: v1.20.19
//! "Vault" established that a fetchable placeholder→raw map would increase
//! the personal-data surface, so strict masking is one-way (irreversible).
//! ponytail: ceiling — the write-time mask runs after auto-routing (the route
//! needs the embedding), so the quantized vec0 embedding and the
//! caller-declared entity names derive from the raw text; neither is
//! practically invertible, and caller-declared entities were always stored
//! verbatim. The HITL `/ingest/proposal` flow keeps its v1.14 posture (the
//! human reviews raw content; promotion lands in `global` with column
//! defaults) — binding the gate flow to profiles is v1.22 work.

use rusqlite::Connection;
use std::collections::BTreeMap;

/// Valid `access_scope` values (must match what `gate::scope_filter` serves
/// non-admin JWT principals: private/domain/team — anything else would be
/// unreadable by every non-admin principal).
pub const ACCESS_SCOPES: &[&str] = &["private", "domain", "team"];

/// Valid `pii_mode` values.
pub const PII_MODES: &[&str] = &["off", "standard", "strict"];

/// Valid `audit_level` values.
pub const AUDIT_LEVELS: &[&str] = &["minimal", "standard", "verbose"];

/// A profile name must be filename/URL-safe (it becomes a path param and an
/// FK key): lowercase alnum + hyphen, 1..=63 chars — the domain charset.
pub fn is_valid_profile_name(name: &str) -> bool {
    crate::storage_layout::is_valid_domain(name)
}

/// One preset bundle. Every field is optional: absent = "this profile does not
/// touch that knob" (the current server default applies).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub name: String,
    /// Display-only: the one-line "what is this preset" the wizard + Health
    /// panel render. Never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default `access_scope` for ingests that omit it (row value always wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_access_scope: Option<String>,
    /// `off` | `standard` (read-time output redaction, the v1.14 default) |
    /// `strict` (write-time one-way masking).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pii_mode: Option<String>,
    /// Per-`memory_kind` retention in days. `null` for a kind = **no decay**
    /// (removes even the server-wide default for bound domains). The profile's
    /// map REPLACES the server-wide policy for the bound domain; an absent
    /// `retention` block leaves the server-wide policy untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<BTreeMap<String, Option<i64>>>,
    /// `minimal` (read-events off) | `standard` (JWT posture default) |
    /// `verbose` (read-events on). Drives `/recall` read-event auditing when
    /// `BRAIN_AUDIT_READ_EVENTS` is unset (the env stays the deployer override).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_level: Option<String>,
    /// Allowed `memory_kind` vocabulary for the bound domain. An ingest whose
    /// effective kind is not in the list is rejected with 422.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// Connector kinds registration may advertise for this domain. Stored +
    /// surfaced only — the connector registry is not domain-scoped in v1.21
    /// (enforcement lands with the v1.24 connector work). ponytail: ceiling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connectors_allowed: Option<Vec<String>>,
    /// v1.22.0 "Regulated" will read this when legal-hold enforcement ships;
    /// here it is a stored flag only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legal_hold_default: Option<bool>,
}

impl Profile {
    /// Validate every set field. Called at every write seam (`POST /profiles`)
    /// so a stored profile is always well-formed; the DB copy is trusted after
    /// that (fail-closed on unreadable rows at read time).
    pub fn validate(&self) -> Result<(), String> {
        if !is_valid_profile_name(&self.name) {
            return Err(format!(
                "invalid profile name '{}' (lowercase alnum + hyphen, max 63)",
                self.name
            ));
        }
        if let Some(s) = &self.default_access_scope {
            if !ACCESS_SCOPES.contains(&s.as_str()) {
                return Err(format!(
                    "default_access_scope must be one of {ACCESS_SCOPES:?}"
                ));
            }
        }
        if let Some(m) = &self.pii_mode {
            if !PII_MODES.contains(&m.as_str()) {
                return Err(format!("pii_mode must be one of {PII_MODES:?}"));
            }
        }
        if let Some(a) = &self.audit_level {
            if !AUDIT_LEVELS.contains(&a.as_str()) {
                return Err(format!("audit_level must be one of {AUDIT_LEVELS:?}"));
            }
        }
        if let Some(days) = &self.retention {
            for (kind, d) in days {
                if kind.is_empty() || kind.len() > 31 {
                    return Err(format!("retention kind '{kind}' invalid"));
                }
                if let Some(d) = d {
                    // Same bound as POST /retention (govern): 1 day..=100 years.
                    if !(1..=36500).contains(d) {
                        return Err(format!(
                            "retention days for '{kind}' must be in [1, 36500] or null"
                        ));
                    }
                }
            }
        }
        for list in [&self.kinds, &self.connectors_allowed]
            .into_iter()
            .flatten()
        {
            // An EMPTY list is a real constraint ("allow nothing" — the
            // gov-fedramp air-gap `connectors_allowed: []`), distinct from an
            // absent field ("no constraint"). Only item SHAPE is validated.
            for k in list {
                if k.is_empty()
                    || k.len() > 31
                    || !k.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                    })
                {
                    return Err(format!(
                        "list item '{k}' invalid (lowercase alnum/-/_, max 31)"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The retention policy this profile imposes on a bound domain, with the
    /// `null` (no-decay) entries dropped — the shape `effective_expiry` and
    /// `SearchFilters::retention_days` consume. `None` when the profile has no
    /// retention block (server-wide policy applies unchanged).
    pub fn retention_map(&self) -> Option<BTreeMap<String, i64>> {
        self.retention.as_ref().map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.map(|d| (k.clone(), d)))
                .collect()
        })
    }

    pub fn pii_strict(&self) -> bool {
        self.pii_mode.as_deref() == Some("strict")
    }

    /// v1.24.0 "Connectors": does this profile permit the connector `kind` for
    /// its bound domain? The vertical-configuration lever — a profile's
    /// `connectors_allowed` (v1.21.0) gates registration. A family-prefixed
    /// sub-kind (`crm-salesforce`) is granted by its bare family entry (`crm`),
    /// so the sales-team preset's `crm` permits every `crm-*` connector.
    ///
    /// Semantics mirror the validate() contract:
    /// - `None` (field absent) = no constraint → everything allowed.
    /// - `Some([])` (explicit empty, e.g. gov-fedramp air-gap) = allow nothing.
    /// - otherwise: exact match, or the bare family matches a `a-b` sub-kind.
    pub fn connector_allowed(&self, kind: &str) -> bool {
        let Some(list) = &self.connectors_allowed else {
            return true;
        };
        if list.is_empty() {
            return false;
        }
        let fam = crate::connector::kind::family(kind);
        list.iter()
            .any(|a| a == kind || (a == fam && kind.contains('-')))
    }
}

// ── persistence (the `profiles` + `domain_profiles` tables, global DB) ─────

/// Load one profile by name.
pub fn load(conn: &Connection, name: &str) -> Result<Option<Profile>, String> {
    let json: Option<String> =
        match conn.query_row("SELECT json FROM profiles WHERE name = ?1", [name], |r| {
            r.get(0)
        }) {
            Ok(j) => Some(j),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.to_string()),
        };
    match json {
        Some(j) => serde_json::from_str(&j)
            .map(Some)
            .map_err(|e| format!("stored profile '{name}' is unreadable: {e}")),
        None => Ok(None),
    }
}

/// List every profile (seeded presets + operator-created), name-ordered.
pub fn list(conn: &Connection) -> Result<Vec<Profile>, String> {
    let mut stmt = conn
        .prepare("SELECT json FROM profiles ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for j in rows.flatten() {
        // A corrupt row fails closed (surfaced), never silently dropped.
        out.push(serde_json::from_str(&j).map_err(|e| format!("stored profile unreadable: {e}"))?);
    }
    Ok(out)
}

/// Upsert one profile row (validated by the caller before storing).
pub fn upsert(conn: &Connection, p: &Profile) -> Result<(), String> {
    let json = serde_json::to_string(p).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO profiles(name, json) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET json = excluded.json",
        rusqlite::params![p.name, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The profile bound to `domain`, if any (JOIN through `domain_profiles`).
/// Fails closed on an unreadable bound profile.
pub fn profile_for_domain(conn: &Connection, domain: &str) -> Result<Option<Profile>, String> {
    let json: Option<String> = match conn.query_row(
        "SELECT p.json FROM domain_profiles dp
         JOIN profiles p ON p.name = dp.profile
         WHERE dp.domain = ?1",
        [domain],
        |r| r.get(0),
    ) {
        Ok(j) => Some(j),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.to_string()),
    };
    match json {
        Some(j) => serde_json::from_str(&j)
            .map(Some)
            .map_err(|e| format!("profile bound to '{domain}' is unreadable: {e}")),
        None => Ok(None),
    }
}

/// Every bound domain → its profile (one query; the recall path reads this
/// once per request for per-domain retention + the audit level).
pub fn domain_profiles(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, Profile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT dp.domain, p.json FROM domain_profiles dp
             JOIN profiles p ON p.name = dp.profile",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = std::collections::HashMap::new();
    for (domain, j) in rows.flatten() {
        let p: Profile = serde_json::from_str(&j)
            .map_err(|e| format!("profile bound to '{domain}' is unreadable: {e}"))?;
        out.insert(domain, p);
    }
    Ok(out)
}

/// Bind `domain` to `profile` (which must already exist — the FK enforces it,
/// the friendly pre-check gives a 404), or unbind when `None`.
pub fn bind(conn: &Connection, domain: &str, profile: Option<&str>) -> Result<(), String> {
    match profile {
        Some(name) => {
            if load(conn, name)?.is_none() {
                return Err(format!("no profile named '{name}'"));
            }
            conn.execute(
                "INSERT INTO domain_profiles(domain, profile) VALUES (?1, ?2)
                 ON CONFLICT(domain) DO UPDATE SET profile = excluded.profile",
                rusqlite::params![domain, name],
            )
            .map_err(|e| e.to_string())?;
        }
        None => {
            conn.execute("DELETE FROM domain_profiles WHERE domain = ?1", [domain])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ── retrieval-time knobs ───────────────────────────────────────────────────

/// Whether `/recall` read-events fire for a domain. Layering: an explicit
/// `BRAIN_AUDIT_READ_EVENTS` (deployer kill-switch, resolved by the caller
/// via `config::audit_read_events_explicit`) always wins; else the bound
/// profile's `audit_level`; else the v1.15 default (JWT on, loopback off).
/// Unbound domain → byte-identical pre-v1.21 behavior.
pub fn audit_read_events_for(
    explicit_env: Option<bool>,
    profile: Option<&Profile>,
    principal_is_jwt: bool,
) -> bool {
    if let Some(explicit) = explicit_env {
        return explicit;
    }
    match profile.and_then(|p| p.audit_level.as_deref()) {
        Some("verbose") => true,
        Some("minimal") => false,
        _ => principal_is_jwt, // standard, unset field, or unbound domain
    }
}

// ── M2 — the 12 ship-with presets (curated from USE_CASES.md) ──────────────
//
// Starting points, not locked: every field is editable via POST /profiles.
// Migration seeds them with INSERT OR IGNORE, so operator edits survive a
// re-migration (only a missing preset is (re-)inserted).

pub fn presets() -> Vec<Profile> {
    PRESETS_RAW
        .iter()
        .map(|(name, json)| {
            let mut p: Profile = serde_json::from_str(json).expect("preset JSON parses");
            p.name = (*name).to_string();
            p
        })
        .collect()
}

/// The 12 ship-with presets as (name, json) — the exact bytes `migration`
/// seeds and the parse test validates. `presets()` is the parsed view.
pub const PRESETS_RAW: &[(&str, &str)] = &[
    (
        "gov-fedramp",
        r#"{"name":"gov-fedramp","description":"Government: long retention, strict audit, air-gap posture, legal-hold by default","default_access_scope":"private","pii_mode":"strict","retention":{"fact":null,"episodic":365,"procedure":null,"decision":null},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":[],"legal_hold_default":true}"#,
    ),
    (
        "health-hipaa",
        r#"{"name":"health-hipaa","description":"Health/care: PHI never stored raw (strict write-time masking), episodic 90-day decay, verbose audit","default_access_scope":"private","pii_mode":"strict","retention":{"fact":null,"episodic":90,"procedure":null},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["ehr-readonly"],"legal_hold_default":false}"#,
    ),
    (
        "call-center",
        r#"{"name":"call-center","description":"Call center: per-agent private scope, episodic 90-day TTL, QA trace via verbose read audit","default_access_scope":"private","pii_mode":"standard","retention":{"fact":730,"episodic":90,"procedure":730},"audit_level":"verbose","kinds":["fact","episodic","procedure"],"connectors_allowed":["crm","ticketing"],"legal_hold_default":false}"#,
    ),
    (
        "sales-team",
        r#"{"name":"sales-team","description":"Sales: team-shared memory, GDPR-light, opportunity procedures kept long","default_access_scope":"team","pii_mode":"standard","retention":{"fact":365,"episodic":180,"procedure":730,"decision":null},"audit_level":"standard","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["crm"],"legal_hold_default":false}"#,
    ),
    (
        "engineering",
        r#"{"name":"engineering","description":"Engineering: project domains, decision/procedural memory kept indefinitely, code-friendly kinds","default_access_scope":"team","pii_mode":"off","retention":{"fact":730,"episodic":90,"procedure":null,"decision":null,"step":null},"audit_level":"standard","kinds":["fact","episodic","procedure","decision","step"],"connectors_allowed":["github","jira","slack"],"legal_hold_default":false}"#,
    ),
    (
        "hr-people",
        r#"{"name":"hr-people","description":"HR: PII strict, retention schedule by record class, DSAR-heavy, verbose audit","default_access_scope":"private","pii_mode":"strict","retention":{"fact":2555,"episodic":90,"procedure":null},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["hris-readonly"],"legal_hold_default":true}"#,
    ),
    (
        "finance-sox",
        r#"{"name":"finance-sox","description":"Finance: SOX 7-year retention, legal-hold default, immutable audit trail","default_access_scope":"private","pii_mode":"standard","retention":{"fact":2555,"episodic":2555,"procedure":2555,"decision":2555},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["erp-gl-readonly"],"legal_hold_default":true}"#,
    ),
    (
        "smb-simple",
        r#"{"name":"smb-simple","description":"Small business: sane out-of-box defaults, minimal knobs, nothing decays, one person","default_access_scope":"private","pii_mode":"off","retention":{},"audit_level":"minimal","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":[],"legal_hold_default":false}"#,
    ),
    (
        "medium-team",
        r#"{"name":"medium-team","description":"Medium business: team scopes, light admin, review queue, GDPR-light","default_access_scope":"team","pii_mode":"standard","retention":{"fact":365,"episodic":90,"procedure":730},"audit_level":"standard","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["slack","docs"],"legal_hold_default":false}"#,
    ),
    (
        "bpo-multi",
        r#"{"name":"bpo-multi","description":"BPO/outsourcer: strict per-agent isolation today; per-client tenancy is v2.0 Cortex","default_access_scope":"private","pii_mode":"strict","retention":{"fact":365,"episodic":90,"procedure":730},"audit_level":"verbose","kinds":["fact","episodic","procedure"],"connectors_allowed":[],"legal_hold_default":false}"#,
    ),
    (
        "enterprise",
        r#"{"name":"enterprise","description":"Large enterprise: SOC 2/ISO 42001 posture, full connector set, distributed review","default_access_scope":"team","pii_mode":"standard","retention":{"fact":730,"episodic":180,"procedure":1825,"decision":1825},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["github","jira","slack","crm","docs"],"legal_hold_default":false}"#,
    ),
    (
        "global-multi-region",
        r#"{"name":"global-multi-region","description":"Global enterprise: strict PII, region-residency readiness (pinning is v2.x), verbose audit","default_access_scope":"private","pii_mode":"strict","retention":{"fact":730,"episodic":180,"procedure":1825},"audit_level":"verbose","kinds":["fact","episodic","procedure","decision"],"connectors_allowed":["github","jira","slack","crm"],"legal_hold_default":true}"#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(
            "CREATE TABLE profiles(name TEXT PRIMARY KEY, json TEXT NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE domain_profiles(domain TEXT PRIMARY KEY, profile TEXT NOT NULL REFERENCES profiles(name), bound_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);",
        ).expect("schema");
        conn
    }

    #[test]
    fn all_presets_parse_and_validate() {
        let ps = presets();
        assert_eq!(ps.len(), 12, "the 12 ship-with presets");
        for p in &ps {
            p.validate().unwrap_or_else(|e| panic!("{}: {e}", p.name));
        }
    }

    #[test]
    fn plan_example_health_hipaa_is_exact() {
        let db = db();
        for p in presets() {
            upsert(&db, &p).unwrap();
        }
        let p = load(&db, "health-hipaa").unwrap().expect("seeded");
        assert_eq!(p.default_access_scope.as_deref(), Some("private"));
        assert_eq!(p.pii_mode.as_deref(), Some("strict"));
        assert_eq!(
            p.retention.as_ref().unwrap().get("episodic"),
            Some(&Some(90))
        );
        assert_eq!(p.retention.as_ref().unwrap().get("fact"), Some(&None));
        assert_eq!(p.audit_level.as_deref(), Some("verbose"));
        let want_kinds: Vec<String> = ["fact", "episodic", "procedure", "decision"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(p.kinds.as_ref(), Some(&want_kinds));
        let want_conn: Vec<String> = ["ehr-readonly"].iter().map(|s| s.to_string()).collect();
        assert_eq!(p.connectors_allowed.as_ref(), Some(&want_conn));
        assert_eq!(p.legal_hold_default, Some(false));
    }

    #[test]
    fn connector_allowed_gates_by_family_and_exact() {
        let ps = super::presets();
        let by_name = |n: &str| ps.iter().find(|p| p.name == n).unwrap();

        // health-hipaa allows only ehr-readonly: slack refused, ehr granted.
        let hipaa = by_name("health-hipaa");
        assert!(
            !hipaa.connector_allowed("slack"),
            "slack must be refused by health-hipaa"
        );
        assert!(
            hipaa.connector_allowed("ehr-readonly"),
            "ehr-readonly must be allowed by health-hipaa"
        );

        // sales-team allows `crm` (bare family) → every crm-* sub-kind granted.
        let sales = by_name("sales-team");
        assert!(sales.connector_allowed("crm-salesforce"));
        assert!(sales.connector_allowed("crm-hubspot"));
        assert!(!sales.connector_allowed("slack"));

        // air-gap (empty list) allows nothing; absent field allows all.
        let airgap = by_name("gov-fedramp");
        assert!(!airgap.connector_allowed("github"));
        assert!(!airgap.connector_allowed("ehr-readonly"));
        let mut open: Profile = sales.clone();
        open.connectors_allowed = None;
        assert!(open.connector_allowed("slack"));
    }

    #[test]
    fn retention_map_drops_nulls_but_presence_is_authoritative() {
        let mut p = Profile {
            name: "x".into(),
            retention: Some(BTreeMap::from([
                ("fact".to_string(), None),
                ("episodic".to_string(), Some(90)),
            ])),
            ..Default::default()
        };
        // Some(empty-ish map) = a policy: fact has NO decay even though the
        // server-wide default would say 365.
        let m = p.retention_map().unwrap();
        assert_eq!(m.get("fact"), None);
        assert_eq!(m.get("episodic"), Some(&90));
        // Absent block = None = don't touch the server-wide policy.
        p.retention = Some(BTreeMap::new());
        assert!(p.retention_map().unwrap().is_empty());
        p.retention = None;
        assert!(p.retention_map().is_none());
    }

    #[test]
    fn validate_rejects_bad_vocab() {
        let mut p = Profile {
            name: "bad".into(),
            default_access_scope: Some("public".into()),
            ..Default::default()
        };
        assert!(p.validate().is_err());
        p.default_access_scope = None;
        p.pii_mode = Some("maximum".into());
        assert!(p.validate().is_err());
        p.pii_mode = Some("strict".into());
        p.audit_level = Some("loud".into());
        assert!(p.validate().is_err());
        p.audit_level = None;
        p.retention = Some(BTreeMap::from([("fact".to_string(), Some(0))]));
        assert!(p.validate().is_err());
        p.retention = None;
        p.name = "Bad Name".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn bind_load_and_unbind_round_trip() {
        let db = db();
        let p = Profile {
            name: "call-center".into(),
            retention: Some(BTreeMap::from([("episodic".to_string(), Some(90))])),
            ..Default::default()
        };
        upsert(&db, &p).unwrap();
        assert!(profile_for_domain(&db, "work").unwrap().is_none());
        bind(&db, "work", Some("call-center")).unwrap();
        let bound = profile_for_domain(&db, "work").unwrap().expect("bound");
        assert_eq!(bound.retention_map().unwrap().get("episodic"), Some(&90));
        // Unknown profile is refused before the FK ever fires.
        assert!(bind(&db, "work", Some("nope")).is_err());
        bind(&db, "work", None).unwrap();
        assert!(profile_for_domain(&db, "work").unwrap().is_none());
    }

    #[test]
    fn audit_level_layers_env_over_profile_over_default() {
        let verbose = Profile {
            name: "v".into(),
            audit_level: Some("verbose".into()),
            ..Default::default()
        };
        let minimal = Profile {
            name: "m".into(),
            audit_level: Some("minimal".into()),
            ..Default::default()
        };
        // The deployer env override wins over any profile — even a verbose
        // posture cannot audit past an explicit off.
        assert!(!audit_read_events_for(Some(false), Some(&verbose), true));
        assert!(audit_read_events_for(Some(true), Some(&minimal), false));
        // Profile level decides when the env is unset.
        assert!(audit_read_events_for(None, Some(&verbose), false));
        assert!(!audit_read_events_for(None, Some(&minimal), true));
        // Unbound / standard → the v1.15 posture default.
        assert!(audit_read_events_for(None, None, true));
        assert!(!audit_read_events_for(None, None, false));
    }
}
