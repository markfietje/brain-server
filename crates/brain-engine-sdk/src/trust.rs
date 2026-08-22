//! Capability & trust policy.
//!
//! Invariant: the decision is a three-way value (`Allowed`/`Prompt`/`Denied`)
//! computed by one pure function — deny always outranks allow, and anything
//! the vocabulary does not name is denied (fail-closed). Semantics ported
//! from Cordis/dsh policy shapes, hardened: `exec`/`env` are denied by
//! default in every profile (the container is the boundary, not a popup).
//! Honest ceiling: this gates the hostcall boundary only — it is not a
//! sandbox for hostile code running inside an engine.

use std::collections::HashMap;

/// Coarse trust posture a host boots with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyMode {
    /// Deny unless explicitly allowed.
    Strict,
    /// Explicitly allowed caps run; everything else needs interactive consent.
    Prompt,
    /// Allow all unlisted capabilities (local development only).
    Permissive,
}

impl PolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyMode::Strict => "strict",
            PolicyMode::Prompt => "prompt",
            PolicyMode::Permissive => "permissive",
        }
    }
}

/// The decision for one (engine, capability) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// Requires interactive consent before the call runs.
    Prompt,
    Denied,
}

/// Per-engine allow/deny lists layered over the global lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOverride {
    pub allow_caps: Vec<String>,
    pub deny_caps: Vec<String>,
}

/// Full extension policy: posture + memory ceiling + global/per-engine
/// capability lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPolicy {
    pub mode: PolicyMode,
    pub max_memory_mb: u32,
    pub default_caps: Vec<String>,
    pub deny_caps: Vec<String>,
    pub per_engine: HashMap<String, EngineOverride>,
}

impl Default for ExtensionPolicy {
    fn default() -> Self {
        // No plugin config == Standard: byte-compatible with the pre-trust
        // kernel's effective posture (reads/writes prompt-gated, exec/env off).
        ExtensionPolicy::standard()
    }
}

impl ExtensionPolicy {
    /// Safe profile: only pure read/write state operations; exec/env/http denied.
    pub fn safe() -> Self {
        ExtensionPolicy {
            mode: PolicyMode::Strict,
            max_memory_mb: 256,
            default_caps: vec!["read".into(), "write".into()],
            deny_caps: vec!["exec".into(), "env".into(), "http".into()],
            per_engine: HashMap::new(),
        }
    }

    /// Standard profile: reads/writes/http/events/session allowed, exec/env
    /// hard-denied, everything else prompts.
    pub fn standard() -> Self {
        ExtensionPolicy {
            mode: PolicyMode::Prompt,
            max_memory_mb: 256,
            default_caps: vec![
                "read".into(),
                "write".into(),
                "http".into(),
                "events".into(),
                "session".into(),
            ],
            deny_caps: vec!["exec".into(), "env".into()],
            per_engine: HashMap::new(),
        }
    }

    /// Permissive profile: nothing denied globally (local dev only).
    pub fn permissive() -> Self {
        ExtensionPolicy {
            mode: PolicyMode::Permissive,
            max_memory_mb: 256,
            default_caps: Vec::new(),
            deny_caps: Vec::new(),
            per_engine: HashMap::new(),
        }
    }

    /// Precedence: per-engine deny > global deny > per-engine allow >
    /// global allow > mode fallback. A `Permissive` mode fallback still
    /// honors every explicit deny.
    pub fn decide(&self, engine: &str, cap: &str) -> Decision {
        let override_for = self.per_engine.get(engine);
        let denies = |caps: &Vec<String>| caps.iter().any(|c| c == cap);
        let allows = |caps: &Vec<String>| caps.iter().any(|c| c == cap);
        if override_for.is_some_and(|o| denies(&o.deny_caps)) || denies(&self.deny_caps) {
            return Decision::Denied;
        }
        if override_for.is_some_and(|o| allows(&o.allow_caps)) || allows(&self.default_caps) {
            return Decision::Allowed;
        }
        match self.mode {
            PolicyMode::Strict => Decision::Denied,
            PolicyMode::Prompt => Decision::Prompt,
            PolicyMode::Permissive => Decision::Allowed,
        }
    }

    /// Whether an (engine, cap) pair may run without further consent.
    pub fn allows(&self, engine: &str, cap: &str) -> bool {
        self.decide(engine, cap) == Decision::Allowed
    }
}

/// Hostcall operation classes the dispatch boundary knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCallKind {
    Tool,
    Exec,
    Http,
    Session,
    Events,
    Ui,
    Log,
}

impl HostCallKind {
    /// Parse from the wire string; an unknown class is an error, never a
    /// guessed default (the dispatcher turns any unknown into a denial).
    pub fn parse(s: &str) -> Result<Self, UnknownKind> {
        match s {
            "tool" => Ok(HostCallKind::Tool),
            "exec" => Ok(HostCallKind::Exec),
            "http" => Ok(HostCallKind::Http),
            "session" => Ok(HostCallKind::Session),
            "events" => Ok(HostCallKind::Events),
            "ui" => Ok(HostCallKind::Ui),
            "log" => Ok(HostCallKind::Log),
            other => Err(UnknownKind(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HostCallKind::Tool => "tool",
            HostCallKind::Exec => "exec",
            HostCallKind::Http => "http",
            HostCallKind::Session => "session",
            HostCallKind::Events => "events",
            HostCallKind::Ui => "ui",
            HostCallKind::Log => "log",
        }
    }

    /// The capability a caller must hold to make this call.
    pub const fn required_capability(self) -> &'static str {
        match self {
            HostCallKind::Tool => "tools",
            HostCallKind::Exec => "exec",
            HostCallKind::Http => "http",
            HostCallKind::Session => "session",
            HostCallKind::Events => "events",
            HostCallKind::Ui => "ui",
            HostCallKind::Log => "log",
        }
    }
}

/// An operation class outside the closed vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKind(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_match_the_spec() {
        let safe = ExtensionPolicy::safe();
        assert_eq!(safe.mode, PolicyMode::Strict);
        assert!(safe.allows("any", "read"));
        assert!(safe.allows("any", "write"));
        assert_eq!(safe.decide("any", "exec"), Decision::Denied);
        assert_eq!(safe.decide("any", "http"), Decision::Denied);

        let std = ExtensionPolicy::standard();
        assert_eq!(std.decide("any", "read"), Decision::Allowed);
        assert_eq!(std.decide("any", "http"), Decision::Allowed);
        assert_eq!(std.decide("any", "session"), Decision::Allowed);
        assert_eq!(std.decide("any", "exec"), Decision::Denied);
        assert_eq!(std.decide("any", "env"), Decision::Denied);
        assert_eq!(std.decide("any", "ui"), Decision::Prompt);

        let perm = ExtensionPolicy::permissive();
        assert_eq!(perm.decide("any", "anything"), Decision::Allowed);
    }

    #[test]
    fn default_policy_is_standard() {
        assert_eq!(ExtensionPolicy::default(), ExtensionPolicy::standard());
    }

    #[test]
    fn precedence_table_deny_outranks_allow_everywhere() {
        let mut policy = ExtensionPolicy::standard();
        policy.default_caps.push("ui".into());
        policy.per_engine.insert(
            "rogue".into(),
            EngineOverride {
                allow_caps: vec!["exec".into(), "ui".into()],
                deny_caps: vec![],
            },
        );
        // Global deny beats per-engine allow...
        assert_eq!(policy.decide("rogue", "exec"), Decision::Denied);
        // ...and per-engine deny beats global allow.
        policy
            .per_engine
            .get_mut("rogue")
            .unwrap()
            .deny_caps
            .push("ui".into());
        assert_eq!(policy.decide("rogue", "ui"), Decision::Denied);
        // Another engine still gets the global allow.
        assert_eq!(policy.decide("honest", "ui"), Decision::Allowed);

        // Even Permissive mode honors explicit denies.
        let mut perm = ExtensionPolicy::permissive();
        perm.deny_caps = vec!["exec".into()];
        assert_eq!(perm.decide("any", "exec"), Decision::Denied);
        assert_eq!(perm.decide("any", "other"), Decision::Allowed);
    }

    #[test]
    fn strict_mode_falls_back_to_denied() {
        let policy = ExtensionPolicy {
            mode: PolicyMode::Strict,
            ..ExtensionPolicy::safe()
        };
        assert_eq!(policy.decide("any", "unlisted-cap"), Decision::Denied);
    }

    #[test]
    fn kind_capability_map_is_closed() {
        let cases = [
            ("tool", "tools"),
            ("exec", "exec"),
            ("http", "http"),
            ("session", "session"),
            ("events", "events"),
            ("ui", "ui"),
            ("log", "log"),
        ];
        for (wire, cap) in cases {
            let kind = HostCallKind::parse(wire).unwrap();
            assert_eq!(kind.as_str(), wire);
            assert_eq!(kind.required_capability(), cap);
        }
        // Unknown classes are errors, not defaults.
        assert!(HostCallKind::parse("shell").is_err());
        assert!(HostCallKind::parse("").is_err());
        assert!(
            HostCallKind::parse("TOOL").is_err(),
            "exact wire match only"
        );
    }
}
