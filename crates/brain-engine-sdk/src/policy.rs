//! Law/compliance vocabulary as pure data: the single owner of the P-class
//! SLA clock and the default per-kind retention table. The server facades
//! these verbatim (env overrides layer on top there); engines read the same
//! truth — policy is never duplicated across the ABI.

/// Priority class for an inbound request; the SLA clock it buys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
}

impl Priority {
    /// Time-to-live seconds per class: P1 4h, P2 24h, P3 72h, P4 7d.
    pub fn ttl_secs(&self) -> i64 {
        match self {
            Priority::P1 => 4 * 3600,
            Priority::P2 => 24 * 3600,
            Priority::P3 => 72 * 3600,
            Priority::P4 => 168 * 3600,
        }
    }
}

/// An SLA-stamped envelope: priority + derived deadline.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Envelope {
    pub p_class: Priority,
    pub sla_deadline: i64,
    pub created_at: i64,
}

/// Stamp an envelope with its SLA deadline from the class TTL table.
pub fn stamp_envelope(created_at: i64, p_class: Priority) -> Envelope {
    Envelope {
        sla_deadline: created_at + p_class.ttl_secs(),
        p_class,
        created_at,
    }
}

/// Default retention (days) per `memory_kind` for chunks with no explicit
/// `expires_at`. Per-chunk `expires_at` always wins; this table governs whole
/// classes. Server config layers env overrides on top — these numbers live
/// here and nowhere else.
pub const DEFAULT_RETENTION_KIND_DAYS: &[(&str, i64)] = &[
    ("fact", 365),
    ("episodic", 30),
    ("procedure", 730),
    ("step", 730),
    ("decision", 730),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p_class_ttl_table() {
        assert_eq!(
            stamp_envelope(1000, Priority::P2).sla_deadline,
            1000 + 24 * 3600
        );
        let e = stamp_envelope(0, Priority::P1);
        assert_eq!(e.sla_deadline, 4 * 3600);
        let e2 = stamp_envelope(0, Priority::P4);
        assert!(e2.sla_deadline > e.sla_deadline);
    }

    #[test]
    fn retention_table_shape() {
        // Every kind maps to a positive day count; the five governed kinds are present.
        assert_eq!(DEFAULT_RETENTION_KIND_DAYS.len(), 5);
        assert!(DEFAULT_RETENTION_KIND_DAYS.iter().all(|(_, d)| *d > 0));
        assert!(
            DEFAULT_RETENTION_KIND_DAYS
                .iter()
                .any(|(k, _)| *k == "episodic")
        );
    }
}
