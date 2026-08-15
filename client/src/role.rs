//! v1.23.0 "Roles" — client-side role capability + panel visibility helpers.
//!
//! Defense-in-depth ONLY: brain-server verifies + enforces the data filter
//! (`access_scope`/`owner_in`) and the action gate (`can`) — the client reads
//! the JWT `roles` claim purely to hide panels and disable buttons the actor's
//! role cannot use (the plan's M3). A hidden/disabled control is never the
//! security boundary; a roles-less (opaque/loopback) token sees everything
//! here and still passes through the server's scope gate unchanged (back-compat).
//!
//! The role→capability map mirrors the server's seeded presets in `role.rs`
//! (same names, same `can` sets). Drift here is a UX nuisance, not a security
//! hole — the narrowest risk a duplicated list can hold.
//!
//! Honest ceiling: the `reports` agent-tree (owner_filter) is resolved fully
//! server-side; the client has no agent-tree and only gates panels/actions.

/// Each ship-with role -> the actions its `can` allowlist carries (mirrors the
/// server presets; the `admin`-equivalent roles short-circuit to "all").
pub static ROLE_ACTION: &[(&str, &[&str])] = &[
    // Full-control roles: every action.
    (
        "admin",
        &[
            "read",
            "write",
            "approve",
            "reject",
            "calibrate",
            "release_quarantine",
            "dsar_export",
            "purge",
        ],
    ),
    (
        "solo",
        &[
            "read",
            "write",
            "approve",
            "reject",
            "calibrate",
            "release_quarantine",
            "dsar_export",
            "purge",
        ],
    ),
    (
        "controller",
        &[
            "read",
            "write",
            "approve",
            "reject",
            "calibrate",
            "release_quarantine",
            "dsar_export",
            "purge",
        ],
    ),
    // Analysts / leads.
    (
        "supervisor",
        &[
            "read",
            "write",
            "approve",
            "reject",
            "calibrate",
            "release_quarantine",
            "dsar_export",
        ],
    ),
    ("agent", &["read", "write", "reject"]),
    ("recruiter", &["read", "write", "reject"]),
    ("qa-specialist", &["read", "calibrate"]),
    ("clinician", &["read", "write"]),
    ("dpo", &["read", "dsar_export", "calibrate"]),
    ("exec", &["read"]),
];

/// Roles listed in `panels_hidden` — panel names a role never shows. A role
/// omitted here leaves every panel at its default (visible).
pub static ROLE_HIDDEN_PANELS: &[(&str, &[&str])] = &[
    ("agent", &["audit", "subjects"]),
    ("qa-specialist", &["audit", "subjects"]),
    ("clinician", &["audit", "subjects", "data"]),
    ("recruiter", &["audit", "subjects"]),
    ("controller", &["security"]),
    ("exec", &["audit", "data", "subjects"]),
];

/// Whether any held role's `can` allowlist names `action`. Empty roles
/// (no role claim / loopback) → true (nothing hidden at the client).
/// A role holding `approve` gates nothing here; only its `can` set matters.
pub fn role_allows(roles: &[String], action: &str) -> bool {
    if roles.is_empty() {
        return true;
    }
    roles.iter().any(|r| {
        ROLE_ACTION
            .iter()
            .find(|(name, _)| name == r)
            .is_some_and(|(_, can)| can.contains(&action))
    })
}

/// Whether the panel is visible for the held roles: hidden only if some held
/// role lists it as hidden AND no role is a full-control role (admin/solo/
/// controller see every panel). Empty roles (loopback) → visible.
pub fn role_can_see(roles: &[String], panel: &str) -> bool {
    if roles.is_empty() {
        return true;
    }
    if role_allows(roles, "purge") {
        // Full-control roles see every panel.
        return true;
    }
    !roles.iter().any(|r| {
        ROLE_HIDDEN_PANELS
            .iter()
            .find(|(name, _)| name == r)
            .is_some_and(|(_, hidden)| hidden.contains(&panel))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn qa_specialist_cannot_approve_or_purge() {
        let qa = r(&["qa-specialist"]);
        assert!(role_allows(&qa, "read"));
        assert!(role_allows(&qa, "calibrate"));
        assert!(!role_allows(&qa, "approve"), "qa cannot approve");
        assert!(!role_allows(&qa, "purge"), "qa cannot purge");
        assert!(!role_allows(&qa, "dsar_export"), "qa cannot run DSAR");
    }

    #[test]
    fn supervisor_approves_but_does_not_purge() {
        let sup = r(&["supervisor"]);
        assert!(role_allows(&sup, "approve"));
        assert!(role_allows(&sup, "reject"));
        assert!(!role_allows(&sup, "purge"));
    }

    #[test]
    fn solo_sees_all_actions_and_panels() {
        let solo = r(&["solo"]);
        for a in ["approve", "reject", "purge", "dsar_export", "calibrate"] {
            assert!(role_allows(&solo, a), "solo can {a}");
        }
        assert!(role_can_see(&solo, "audit"));
        assert!(role_can_see(&solo, "subjects"));
    }

    #[test]
    fn no_roles_is_unrestricted_client_ui() {
        let none: Vec<String> = vec![];
        assert!(role_allows(&none, "approve"));
        assert!(role_can_see(&none, "subjects"));
    }

    #[test]
    fn exec_hides_sensitive_panels_but_sees_dashboard() {
        let exec = r(&["exec"]);
        assert!(role_can_see(&exec, "overview"));
        assert!(!role_can_see(&exec, "subjects"));
        assert!(!role_can_see(&exec, "audit"));
        assert!(!role_allows(&exec, "purge"));
        assert!(!role_allows(&exec, "approve"));
    }

    #[test]
    fn agent_hides_audit_and_subjects() {
        let agent = r(&["agent"]);
        assert!(!role_can_see(&agent, "audit"));
        assert!(!role_can_see(&agent, "subjects"));
        assert!(role_can_see(&agent, "recall"));
    }
}
