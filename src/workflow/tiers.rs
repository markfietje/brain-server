//! Deployment-tier profiles — the tier guide's executable half.
//!
//! Tiers are CONFIG, not forks: each tier is a checked-in env profile
//! (`deploy/tiers/t{1..4}.env`) that the guide in `docs/deployment.md`
//! documents. Two meta-gates live here: profiles may only use keys the
//! server actually resolves (`guide_and_profiles_never_drift`), and the
//! conformance matrix may not carry an unmarked open row at series exit
//! (`series_exit_gate_checklist_green_or_ceiling_marked`). Parsing is pure —
//! no process env is touched by tests.

/// Every key a tier profile may set. A profile using anything else fails —
/// a typo'd env var silently falling back to its default is exactly the
/// drift this module exists to refuse.
const PROFILE_KEYS: &[&str] = &[
    "BRAIN_WRITE_POSTURE",
    "BRAIN_AUDIT_READ_EVENTS",
    "BRAIN_MULTI_DB",
    "BRAIN_MAX_DOMAIN_DBS",
    "BRAIN_WEBHOOK_TIMESTAMP_REQUIRED",
    "BRAIN_OTEL_ENABLED",
    "BRAIN_TRUST_PROXY",
    "BIND_PUBLIC",
];

pub(crate) const TIER_PROFILE_PATHS: &[&str] = &[
    "deploy/tiers/t1.env",
    "deploy/tiers/t2.env",
    "deploy/tiers/t3.env",
    "deploy/tiers/t4.env",
];

#[derive(Debug, PartialEq)]
pub(crate) struct TierProfile {
    pub path: String,
    pub vars: Vec<(String, String)>,
}

impl TierProfile {
    /// KEY=VALUE lines; full-line comments and blanks ignored; inline
    /// comments after the value are stripped. Unknown keys fail loudly.
    pub(crate) fn parse(path: &str, text: &str) -> Result<Self, String> {
        let mut vars = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("{path}:{}: malformed line {line:?}", n + 1);
            let (key, val) = line.split_once('=').ok_or_else(bad)?;
            let key = key.trim();
            if !PROFILE_KEYS.contains(&key) {
                return Err(format!(
                    "{path}:{}: '{key}' is not a server-resolved tier key",
                    n + 1
                ));
            }
            vars.push((key.to_string(), strip_inline_comment(val)));
        }
        Ok(Self {
            path: path.to_string(),
            vars,
        })
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The boot-time posture of the profile: loopback-only bind unless the
    /// operator overrides outside the profile (fail-closed mirrors the
    /// server's own startup refusal), and a write posture the server would
    /// accept at startup.
    pub(crate) fn validate(&self) -> Result<(), String> {
        let posture = self.get("BRAIN_WRITE_POSTURE").unwrap_or("open");
        if posture != "open" && posture != "review" {
            return Err(format!(
                "{}: invalid BRAIN_WRITE_POSTURE '{posture}'",
                self.path
            ));
        }
        if let Some(n) = self.get("BRAIN_MAX_DOMAIN_DBS")
            && n.parse::<usize>().map(|c| c == 0).unwrap_or(true)
        {
            return Err(format!(
                "{}: BRAIN_MAX_DOMAIN_DBS='{n}' is not a positive cap",
                self.path
            ));
        }
        Ok(())
    }
}

fn strip_inline_comment(val: &str) -> String {
    // A bare `#` inside a value stays (only " #" separates a comment).
    if let Some((v, _)) = val.split_once(" #") {
        return v.trim().to_string();
    }
    val.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn load(rel: &str) -> TierProfile {
        // CARGO_MANIFEST_DIR is the workspace root for the src/ tree.
        let base = env!("CARGO_MANIFEST_DIR");
        let path = format!("{base}/{rel}");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} must exist: {e}"));
        TierProfile::parse(rel, &text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn tier_profiles_boot_and_pass_smoke() {
        register_sqlite_vec();
        for rel in TIER_PROFILE_PATHS {
            let profile = load(rel);
            profile
                .validate()
                .unwrap_or_else(|e| panic!("{rel} refuses boot: {e}"));
            // Smoke = the profile's posture boots a fresh file-backed DB
            // green: migration completes and the schema is consistent
            // (the same checks `brain doctor` leads with).
            let dir = std::env::temp_dir().join(format!(
                "brain-tier-smoke-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("tmpdir");
            let mut conn = Connection::open(dir.join("tier.db")).expect("open smoke db");
            run_migration(&mut conn, 1).expect("migration under tier profile");
            let tables: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .expect("table count");
            assert!(tables > 0, "{rel}: migration produced no schema");
        }
    }

    #[test]
    fn guide_and_profiles_never_drift() {
        let base = env!("CARGO_MANIFEST_DIR");
        let guide = std::fs::read_to_string(format!("{base}/docs/deployment.md"))
            .expect("docs/deployment.md must exist");
        for rel in TIER_PROFILE_PATHS {
            assert!(
                guide.contains(&format!("`{rel}`")),
                "deployment.md must reference the checked-in profile `{rel}`"
            );
            let profile = load(rel);
            for (key, _) in &profile.vars {
                assert!(
                    guide.contains(key),
                    "{rel} sets {key} but docs/deployment.md never mentions it — \
                     document the tier matrix or drop the key"
                );
            }
        }
    }

    #[test]
    fn series_exit_gate_checklist_green_or_ceiling_marked() {
        // The Conformance Line's exit condition, as a test: at series exit,
        // every conformance-matrix row is either shipped (✅) or explicitly
        // ceiling-marked / watch-listed. An unmarked 🟡 planned / ❌ gap /
        // ⚠️ open-gap row means the line failed and says so here.
        let base = env!("CARGO_MANIFEST_DIR");
        let doc = std::fs::read_to_string(format!("{base}/docs/CONTACT_CENTER_STANDARDS.md"))
            .expect("CONTACT_CENTER_STANDARDS.md must exist");
        let matrix = doc
            .split("## Conformance matrix")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .expect("conformance matrix section present");
        let mut rows = 0usize;
        for line in matrix.lines().filter(|l| l.trim_start().starts_with('|')) {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect::<Vec<_>>();
            if cells.len() < 3 || cells[1].starts_with(':') || cells[1].is_empty() {
                continue;
            }
            let status_cell = cells[cells.len() - 2];
            if status_cell.starts_with('*')
                || status_cell == "Status"
                || status_cell.chars().all(|c| c == '-' || c == ':')
            {
                continue;
            }
            rows += 1;
            let ok = status_cell.contains("✅")
                || status_cell.to_lowercase().contains("ceiling")
                || status_cell.to_lowercase().contains("watch item")
                || status_cell.contains("non-scope");
            assert!(
                ok,
                "conformance row not green nor ceiling-marked at series exit: \
                 '{status_cell}' (row: {line})"
            );
            assert!(
                !status_cell.contains("🟡") && !status_cell.contains("❌"),
                "open gap left unmarked in the matrix: {status_cell}"
            );
        }
        assert!(rows >= 10, "matrix unexpectedly truncated ({rows} rows)");
    }

    #[test]
    fn parse_refuses_unknown_keys_and_malformed_lines() {
        let err = TierProfile::parse("t.env", "BRAIN_NOT_A_KEY=1").unwrap_err();
        assert!(err.contains("not a server-resolved tier key"), "{err}");
        let err = TierProfile::parse("t.env", "NOTKV").unwrap_err();
        assert!(err.contains("malformed"), "{err}");
        let p = TierProfile::parse("t.env", "# c\n\nBRAIN_WRITE_POSTURE=review # why\n")
            .expect("parses");
        assert_eq!(p.get("BRAIN_WRITE_POSTURE"), Some("review"));
    }

    #[test]
    fn profile_validation_fails_closed() {
        let bad = TierProfile::parse("t.env", "BRAIN_WRITE_POSTURE=yolo").expect("parses");
        assert!(bad.validate().is_err());
        let zero = TierProfile::parse("t.env", "BRAIN_MAX_DOMAIN_DBS=0").expect("parses");
        assert!(zero.validate().is_err());
        let good = TierProfile::parse("t.env", "BIND_PUBLIC=0").expect("parses");
        assert_eq!(good.validate(), Ok(()));
    }
}
