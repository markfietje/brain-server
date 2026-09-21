//! The compaction admission vocabulary: pure, total, clock-free — the pi
//! `compaction_worker` admission semantics ported (semantics only, no code
//! copied), so the kernel's compaction executor keeps executing while its
//! ADMISSION policy lives here as data and decision tables.
//!
//! The shape: the caller samples the world into signals (memory, provider,
//! queue — already-sampled facts, never live probes) and owns the worker
//! counters (pending, attempt count, remaining cooldown as an integer —
//! the executor owns time). [`admission_decision`] weighs them into one
//! named [`CompactionAdmissionReason`]; every decision is evidence-shaped
//! and serializable. Compaction is demanded by CONTEXT pressure: a
//! preparation exists only at/over the pinned [`ResolvedCompactionSettings`]
//! pressure floor, and admission below that floor is structurally
//! impossible — admit only at pressure, never for convenience.
//!
//! No provider calls, no threads, no clocks, no I/O: everything is a
//! function of the arguments.

use crate::prompt::{COMPACT_PRESSURE_TOKENS, KEEP_VERBATIM_TOKENS};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The evidence schema this module emits.
pub const COMPACTION_ADMISSION_SCHEMA: &str = "steward.compaction.admission.v1";

/// Quota controls bounding how often compaction may start. The pi defaults
/// map onto integers the executor counts down (the SDK owns no clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionQuota {
    /// Minimum elapsed time between compaction starts, in milliseconds.
    pub cooldown_ms: u64,
    /// Maximum compaction attempts allowed in a single session.
    pub max_attempts_per_session: u32,
}

impl Default for CompactionQuota {
    fn default() -> Self {
        Self {
            cooldown_ms: 60_000,
            max_attempts_per_session: 100,
        }
    }
}

/// Memory posture sampled by the caller. Unknown memory posture is
/// unavailable, not healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionMemorySignal {
    pub available_bytes: Option<u64>,
    pub required_headroom_bytes: u64,
    pub pressure: bool,
}

impl CompactionMemorySignal {
    pub fn is_pressure(self) -> bool {
        self.pressure
            || self
                .available_bytes
                .is_some_and(|available| available < self.required_headroom_bytes)
    }
}

/// Provider posture sampled by the caller: background compaction must not
/// amplify an already degraded path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionProviderSignal {
    pub p95_latency_ms: Option<u64>,
    pub max_p95_latency_ms: Option<u64>,
    pub error_rate_per_mille: Option<u16>,
    pub max_error_rate_per_mille: Option<u16>,
    pub stale: bool,
    pub degraded: bool,
}

impl CompactionProviderSignal {
    pub fn is_degraded(self) -> bool {
        self.degraded
            || self.stale
            || self
                .p95_latency_ms
                .zip(self.max_p95_latency_ms)
                .is_some_and(|(p95, max)| p95 > max)
            || self
                .error_rate_per_mille
                .zip(self.max_error_rate_per_mille)
                .is_some_and(|(rate, max)| rate > max)
    }
}

/// Queue posture sampled by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionQueueSignal {
    pub queued_requests: u32,
    pub max_queued_requests: u32,
    pub saturated: bool,
}

impl CompactionQueueSignal {
    pub const fn is_saturated(self) -> bool {
        self.saturated
            || (self.max_queued_requests > 0 && self.queued_requests >= self.max_queued_requests)
    }
}

/// The sampled admission signals. The worker never probes live state; the
/// caller passes already-sampled facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionAdmissionSignals {
    pub memory: Option<CompactionMemorySignal>,
    pub provider: Option<CompactionProviderSignal>,
    pub queue: Option<CompactionQueueSignal>,
}

/// Why a signals payload refused to parse. Fail-closed: an unknown signal
/// name is a refusal, never an ignore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalsError {
    NotAnObject,
    UnknownSignal(String),
    Malformed(&'static str),
}

impl fmt::Display for SignalsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "admission signals must be a JSON object"),
            Self::UnknownSignal(name) => write!(f, "unknown admission signal: {name}"),
            Self::Malformed(why) => write!(f, "malformed admission signal: {why}"),
        }
    }
}

/// The closed-surface check shared by the signals parser.
fn reject_unknown_keys(
    obj: &serde_json::Map<String, Value>,
    known: &[&str],
) -> Result<(), SignalsError> {
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            return Err(SignalsError::UnknownSignal(key.clone()));
        }
    }
    Ok(())
}

impl CompactionAdmissionSignals {
    /// Parse the canonical JSON form. The key set is CLOSED at every level
    /// — an unknown signal (or unknown field inside one) refuses
    /// (fail-closed), it is never silently dropped.
    pub fn from_json(value: &Value) -> Result<Self, SignalsError> {
        let obj = value.as_object().ok_or(SignalsError::NotAnObject)?;
        for key in obj.keys() {
            if !matches!(key.as_str(), "memory" | "provider" | "queue") {
                return Err(SignalsError::UnknownSignal(key.clone()));
            }
        }
        let memory = match obj.get("memory") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let m = v.as_object().ok_or(SignalsError::Malformed("memory"))?;
                reject_unknown_keys(
                    m,
                    &["available_bytes", "required_headroom_bytes", "pressure"],
                )?;
                let required = m
                    .get("required_headroom_bytes")
                    .and_then(Value::as_u64)
                    .ok_or(SignalsError::Malformed("memory.required_headroom_bytes"))?;
                let available = match m.get("available_bytes") {
                    None | Some(Value::Null) => None,
                    Some(n) => Some(
                        n.as_u64()
                            .ok_or(SignalsError::Malformed("memory.available_bytes"))?,
                    ),
                };
                let pressure = m
                    .get("pressure")
                    .and_then(Value::as_bool)
                    .ok_or(SignalsError::Malformed("memory.pressure"))?;
                Some(CompactionMemorySignal {
                    available_bytes: available,
                    required_headroom_bytes: required,
                    pressure,
                })
            }
        };
        let provider = match obj.get("provider") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let p = v.as_object().ok_or(SignalsError::Malformed("provider"))?;
                reject_unknown_keys(
                    p,
                    &[
                        "p95_latency_ms",
                        "max_p95_latency_ms",
                        "error_rate_per_mille",
                        "max_error_rate_per_mille",
                        "stale",
                        "degraded",
                    ],
                )?;
                let opt_u64 = |key: &str| -> Result<Option<u64>, SignalsError> {
                    match p.get(key) {
                        None | Some(Value::Null) => Ok(None),
                        Some(n) => n
                            .as_u64()
                            .map(Some)
                            .ok_or(SignalsError::Malformed("provider integer field")),
                    }
                };
                let opt_u16 = |key: &str| -> Result<Option<u16>, SignalsError> {
                    match p.get(key) {
                        None | Some(Value::Null) => Ok(None),
                        Some(n) => n
                            .as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .map(Some)
                            .ok_or(SignalsError::Malformed("provider rate field")),
                    }
                };
                Some(CompactionProviderSignal {
                    p95_latency_ms: opt_u64("p95_latency_ms")?,
                    max_p95_latency_ms: opt_u64("max_p95_latency_ms")?,
                    error_rate_per_mille: opt_u16("error_rate_per_mille")?,
                    max_error_rate_per_mille: opt_u16("max_error_rate_per_mille")?,
                    stale: p
                        .get("stale")
                        .and_then(Value::as_bool)
                        .ok_or(SignalsError::Malformed("provider.stale"))?,
                    degraded: p
                        .get("degraded")
                        .and_then(Value::as_bool)
                        .ok_or(SignalsError::Malformed("provider.degraded"))?,
                })
            }
        };
        let queue = match obj.get("queue") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let q = v.as_object().ok_or(SignalsError::Malformed("queue"))?;
                reject_unknown_keys(q, &["queued_requests", "max_queued_requests", "saturated"])?;
                let field = |key: &str| -> Result<u32, SignalsError> {
                    q.get(key)
                        .and_then(Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or(SignalsError::Malformed("queue integer field"))
                };
                Some(CompactionQueueSignal {
                    queued_requests: field("queued_requests")?,
                    max_queued_requests: field("max_queued_requests")?,
                    saturated: q
                        .get("saturated")
                        .and_then(Value::as_bool)
                        .ok_or(SignalsError::Malformed("queue.saturated"))?,
                })
            }
        };
        Ok(Self {
            memory,
            provider,
            queue,
        })
    }

    fn to_json(self) -> Value {
        let memory = self.memory.map(|m| {
            serde_json::json!({
                "available_bytes": m.available_bytes,
                "required_headroom_bytes": m.required_headroom_bytes,
                "pressure": m.pressure,
            })
        });
        let provider = self.provider.map(|p| {
            serde_json::json!({
                "p95_latency_ms": p.p95_latency_ms,
                "max_p95_latency_ms": p.max_p95_latency_ms,
                "error_rate_per_mille": p.error_rate_per_mille,
                "max_error_rate_per_mille": p.max_error_rate_per_mille,
                "stale": p.stale,
                "degraded": p.degraded,
            })
        });
        let queue = self.queue.map(|q| {
            serde_json::json!({
                "queued_requests": q.queued_requests,
                "max_queued_requests": q.max_queued_requests,
                "saturated": q.saturated,
            })
        });
        serde_json::json!({"memory": memory, "provider": provider, "queue": queue})
    }
}

/// The resolved threshold set. Pinned to the crate's own prompt constants
/// BY REFERENCE — the numbers are declared exactly once in the SDK, and
/// the kernel's executor rides the same constants, so kernel thresholds
/// and SDK admission agree by construction (pinned on both sides).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedCompactionSettings {
    /// The window-token pressure floor: compaction is DEMANDED only at or
    /// above this (the pi posture: "16k from the window" — the shipped
    /// constant already carries that number).
    pub pressure_tokens: usize,
    /// The verbatim tail kept when compaction runs ("~20k verbatim").
    pub keep_verbatim_tokens: usize,
}

impl ResolvedCompactionSettings {
    pub const fn new() -> Self {
        Self {
            pressure_tokens: COMPACT_PRESSURE_TOKENS,
            keep_verbatim_tokens: KEEP_VERBATIM_TOKENS,
        }
    }
}

impl Default for ResolvedCompactionSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// The demand side: the caller's measurement that the window is AT/over
/// the pressure floor. `tokens_before` is the observed window weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPreparation {
    pub tokens_before: u64,
}

/// The executor-owned worker counters (the pi worker state minus its
/// runtime): the SDK never spawns, waits, or reads a clock — the executor
/// updates these and asks for a decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionAdmissionState {
    /// A compaction is already in flight (bounded concurrency: one).
    pub pending: bool,
    /// Attempts started this session (reset by the executor on success).
    pub attempt_count: u32,
    /// Remaining cooldown before the next start may admit, in
    /// milliseconds (executor-counted).
    pub cooldown_remaining_ms: u64,
}

/// The closed reason vocabulary. `Ord` follows declaration order (the pi
/// order): allowed sorts lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompactionAdmissionReason {
    Allowed,
    Pending,
    SessionAttemptLimit,
    Cooldown,
    NoPreparation,
    MemoryPressure,
    ProviderDegraded,
    QueueSaturated,
}

impl CompactionAdmissionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Pending => "pending",
            Self::SessionAttemptLimit => "session_attempt_limit",
            Self::Cooldown => "cooldown",
            Self::NoPreparation => "no_preparation",
            Self::MemoryPressure => "memory_pressure",
            Self::ProviderDegraded => "provider_degraded",
            Self::QueueSaturated => "queue_saturated",
        }
    }
}

/// One deterministic admission decision — the evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionAdmissionDecision {
    pub schema: &'static str,
    pub allowed: bool,
    pub reason: CompactionAdmissionReason,
    pub tokens_before: Option<u64>,
    pub attempt_count: u32,
    pub max_attempts_per_session: u32,
    pub cooldown_remaining_ms: u64,
    pub signals: CompactionAdmissionSignals,
}

impl CompactionAdmissionDecision {
    pub const fn is_allowed(&self) -> bool {
        self.allowed
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "allowed": self.allowed,
            "reason": self.reason.as_str(),
            "tokens_before": self.tokens_before,
            "attempt_count": self.attempt_count,
            "max_attempts_per_session": self.max_attempts_per_session,
            "cooldown_remaining_ms": self.cooldown_remaining_ms,
            "signals": self.signals.to_json(),
        })
    }
}

/// The admission function: total, pure, and fail-closed. Quota blocks
/// first (pending, then attempt limit, then cooldown); then demand (a
/// preparation must exist AND clear the pinned pressure floor); then the
/// sampled signals (memory pressure, provider degradation, queue
/// saturation each block — background work must not amplify a struggling
/// host).
pub fn admission_decision(
    state: &CompactionAdmissionState,
    quota: &CompactionQuota,
    settings: &ResolvedCompactionSettings,
    preparation: Option<&CompactionPreparation>,
    signals: &CompactionAdmissionSignals,
) -> CompactionAdmissionDecision {
    let reason = quota_block_reason(state, quota)
        .or_else(|| demand_block_reason(settings, preparation))
        .or_else(|| signal_block_reason(signals))
        .unwrap_or(CompactionAdmissionReason::Allowed);
    CompactionAdmissionDecision {
        schema: COMPACTION_ADMISSION_SCHEMA,
        allowed: reason == CompactionAdmissionReason::Allowed,
        reason,
        tokens_before: preparation.map(|p| p.tokens_before),
        attempt_count: state.attempt_count,
        max_attempts_per_session: quota.max_attempts_per_session,
        cooldown_remaining_ms: state.cooldown_remaining_ms,
        signals: *signals,
    }
}

fn quota_block_reason(
    state: &CompactionAdmissionState,
    quota: &CompactionQuota,
) -> Option<CompactionAdmissionReason> {
    if state.pending {
        Some(CompactionAdmissionReason::Pending)
    } else if state.attempt_count >= quota.max_attempts_per_session {
        Some(CompactionAdmissionReason::SessionAttemptLimit)
    } else if state.cooldown_remaining_ms > 0 {
        Some(CompactionAdmissionReason::Cooldown)
    } else {
        None
    }
}

fn demand_block_reason(
    settings: &ResolvedCompactionSettings,
    preparation: Option<&CompactionPreparation>,
) -> Option<CompactionAdmissionReason> {
    match preparation {
        // No demand, or demand below the floor — compaction is never a
        // convenience: admit only at pressure.
        None => Some(CompactionAdmissionReason::NoPreparation),
        Some(prep) if (prep.tokens_before as usize) < settings.pressure_tokens => {
            Some(CompactionAdmissionReason::NoPreparation)
        }
        Some(_) => None,
    }
}

fn signal_block_reason(signals: &CompactionAdmissionSignals) -> Option<CompactionAdmissionReason> {
    if signals
        .memory
        .is_some_and(CompactionMemorySignal::is_pressure)
    {
        Some(CompactionAdmissionReason::MemoryPressure)
    } else if signals
        .provider
        .is_some_and(CompactionProviderSignal::is_degraded)
    {
        Some(CompactionAdmissionReason::ProviderDegraded)
    } else if signals
        .queue
        .is_some_and(CompactionQueueSignal::is_saturated)
    {
        Some(CompactionAdmissionReason::QueueSaturated)
    } else {
        None
    }
}

/// Aggregate a decision series into the evidence shape the compliance
/// surfaces consume: counts, the rejection breakdown by named reason, and
/// the caller-supplied foreground impact fixture.
pub fn compaction_admission_evidence(
    decisions: &[CompactionAdmissionDecision],
    foreground_p95_ms: u64,
    foreground_p99_ms: u64,
) -> Value {
    let mut rejected_by_reason: BTreeMap<&'static str, usize> = BTreeMap::new();
    for decision in decisions {
        if !decision.allowed {
            *rejected_by_reason
                .entry(decision.reason.as_str())
                .or_default() += 1;
        }
    }
    let admitted = decisions.iter().filter(|d| d.allowed).count();
    serde_json::json!({
        "schema": COMPACTION_ADMISSION_SCHEMA,
        "decisionCount": decisions.len(),
        "admittedCount": admitted,
        "rejectedCount": decisions.len().saturating_sub(admitted),
        "rejectedByReason": rejected_by_reason,
        "foregroundImpact": {
            "source": "deterministic_fixture",
            "p95Ms": foreground_p95_ms,
            "p99Ms": foreground_p99_ms,
        },
        "decisions": decisions.iter().map(CompactionAdmissionDecision::to_json).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt;
    use serde_json::json;

    fn clean() -> CompactionAdmissionSignals {
        CompactionAdmissionSignals::default()
    }

    fn at_pressure(tokens: u64) -> Option<CompactionPreparation> {
        Some(CompactionPreparation {
            tokens_before: tokens,
        })
    }

    fn decide(
        state: &CompactionAdmissionState,
        preparation: Option<&CompactionPreparation>,
        signals: &CompactionAdmissionSignals,
    ) -> CompactionAdmissionDecision {
        admission_decision(
            state,
            &CompactionQuota::default(),
            &ResolvedCompactionSettings::new(),
            preparation,
            signals,
        )
    }

    /// THE LAW: without context pressure there is no compaction — for
    /// EVERY quota state and EVERY signal posture, a missing (or
    /// under-floor) preparation never admits.
    #[test]
    fn compaction_admits_only_at_pressure() {
        let states = [
            CompactionAdmissionState::default(),
            CompactionAdmissionState {
                pending: true,
                ..Default::default()
            },
            CompactionAdmissionState {
                attempt_count: 100,
                ..Default::default()
            },
            CompactionAdmissionState {
                cooldown_remaining_ms: 5_000,
                ..Default::default()
            },
        ];
        let pressured = CompactionAdmissionSignals {
            memory: Some(CompactionMemorySignal {
                available_bytes: Some(0),
                required_headroom_bytes: 1,
                pressure: true,
            }),
            ..Default::default()
        };
        for state in &states {
            for signals in [&clean(), &pressured] {
                let decision = decide(state, None, signals);
                assert!(
                    !decision.is_allowed(),
                    "admitted with no preparation and state {state:?}"
                );
                // and the under-floor preparation is the same refusal
                let under = at_pressure(15_999);
                let decision = decide(state, under.as_ref(), signals);
                assert!(
                    !decision.is_allowed(),
                    "admitted under the pressure floor and state {state:?}"
                );
            }
        }
        // on a clean worker the missing/under-floor demand IS the named
        // reason
        let fresh = CompactionAdmissionState::default();
        let decision = decide(&fresh, None, &clean());
        assert_eq!(decision.reason, CompactionAdmissionReason::NoPreparation);
        let under = at_pressure(15_999);
        let decision = decide(&fresh, under.as_ref(), &clean());
        assert_eq!(decision.reason, CompactionAdmissionReason::NoPreparation);
        // exactly at the floor the demand exists
        let fresh = CompactionAdmissionState::default();
        let decision = decide(&fresh, at_pressure(16_000).as_ref(), &clean());
        assert!(decision.is_allowed(), "16k is the pressure floor");
    }

    /// The decision table: quota blocks first, then signals; each blocked
    /// posture lands on exactly its named reason. Every row drives a
    /// at-pressure preparation — the no-demand rows live in
    /// [`compaction_admits_only_at_pressure`].
    #[test]
    fn decision_table_quota_blocks_first_then_signals() {
        let settings = ResolvedCompactionSettings::new();
        let prep = at_pressure(20_000);
        let quota = CompactionQuota::default();

        struct Row {
            name: &'static str,
            state: CompactionAdmissionState,
            signals: CompactionAdmissionSignals,
            expected: CompactionAdmissionReason,
        }
        let rows = [
            Row {
                name: "clean admits",
                state: CompactionAdmissionState::default(),
                signals: clean(),
                expected: CompactionAdmissionReason::Allowed,
            },
            Row {
                name: "pending blocks",
                state: CompactionAdmissionState {
                    pending: true,
                    ..Default::default()
                },
                signals: clean(),
                expected: CompactionAdmissionReason::Pending,
            },
            Row {
                name: "attempt limit blocks",
                state: CompactionAdmissionState {
                    attempt_count: 100,
                    ..Default::default()
                },
                signals: clean(),
                expected: CompactionAdmissionReason::SessionAttemptLimit,
            },
            Row {
                name: "cooldown blocks",
                state: CompactionAdmissionState {
                    cooldown_remaining_ms: 1_234,
                    ..Default::default()
                },
                signals: clean(),
                expected: CompactionAdmissionReason::Cooldown,
            },
            Row {
                name: "memory headroom shortfall blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    memory: Some(CompactionMemorySignal {
                        available_bytes: Some(1),
                        required_headroom_bytes: 2,
                        pressure: false,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::MemoryPressure,
            },
            Row {
                name: "memory pressure flag blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    memory: Some(CompactionMemorySignal {
                        available_bytes: None,
                        required_headroom_bytes: 1,
                        pressure: true,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::MemoryPressure,
            },
            Row {
                name: "provider p95 breach blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    provider: Some(CompactionProviderSignal {
                        p95_latency_ms: Some(900),
                        max_p95_latency_ms: Some(800),
                        error_rate_per_mille: None,
                        max_error_rate_per_mille: None,
                        stale: false,
                        degraded: false,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::ProviderDegraded,
            },
            Row {
                name: "stale provider blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    provider: Some(CompactionProviderSignal {
                        p95_latency_ms: None,
                        max_p95_latency_ms: None,
                        error_rate_per_mille: None,
                        max_error_rate_per_mille: None,
                        stale: true,
                        degraded: false,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::ProviderDegraded,
            },
            Row {
                name: "error-rate breach blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    provider: Some(CompactionProviderSignal {
                        p95_latency_ms: None,
                        max_p95_latency_ms: None,
                        error_rate_per_mille: Some(50),
                        max_error_rate_per_mille: Some(20),
                        stale: false,
                        degraded: false,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::ProviderDegraded,
            },
            Row {
                name: "explicit degraded flag blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    provider: Some(CompactionProviderSignal {
                        p95_latency_ms: None,
                        max_p95_latency_ms: None,
                        error_rate_per_mille: None,
                        max_error_rate_per_mille: None,
                        stale: false,
                        degraded: true,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::ProviderDegraded,
            },
            Row {
                name: "queue ceiling blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    queue: Some(CompactionQueueSignal {
                        queued_requests: 8,
                        max_queued_requests: 8,
                        saturated: false,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::QueueSaturated,
            },
            Row {
                name: "explicit saturated flag blocks",
                state: CompactionAdmissionState::default(),
                signals: CompactionAdmissionSignals {
                    queue: Some(CompactionQueueSignal {
                        queued_requests: 0,
                        max_queued_requests: 0,
                        saturated: true,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::QueueSaturated,
            },
            Row {
                name: "quota outranks signals (pending beats memory)",
                state: CompactionAdmissionState {
                    pending: true,
                    ..Default::default()
                },
                signals: CompactionAdmissionSignals {
                    memory: Some(CompactionMemorySignal {
                        available_bytes: None,
                        required_headroom_bytes: 1,
                        pressure: true,
                    }),
                    ..Default::default()
                },
                expected: CompactionAdmissionReason::Pending,
            },
        ];

        for row in &rows {
            let decision =
                admission_decision(&row.state, &quota, &settings, prep.as_ref(), &row.signals);
            assert_eq!(
                decision.reason,
                row.expected,
                "row `{}` landed on {}",
                row.name,
                decision.reason.as_str()
            );
            assert_eq!(decision.is_allowed(), decision.allowed);
        }

        // no demand: NoPreparation is reported even when a signal would
        // also block (demand is evaluated before signals)
        let decision = admission_decision(
            &CompactionAdmissionState::default(),
            &quota,
            &settings,
            None,
            &CompactionAdmissionSignals {
                queue: Some(CompactionQueueSignal {
                    queued_requests: 1,
                    max_queued_requests: 1,
                    saturated: false,
                }),
                ..Default::default()
            },
        );
        assert_eq!(decision.reason, CompactionAdmissionReason::NoPreparation);
    }

    /// The thresholds are THE crate's constants by reference — the pi
    /// literals (16k window / 20k verbatim) coincide with the shipped
    /// numbers; re-declaring them here would be duplication, so the test
    /// pins reference equality instead.
    #[test]
    fn resolved_settings_are_the_prompt_constants() {
        let settings = ResolvedCompactionSettings::new();
        assert_eq!(settings.pressure_tokens, prompt::COMPACT_PRESSURE_TOKENS);
        assert_eq!(settings.keep_verbatim_tokens, prompt::KEEP_VERBATIM_TOKENS);
        assert_eq!(settings.pressure_tokens, 16_000);
        assert_eq!(settings.keep_verbatim_tokens, 20_000);
        assert!(prompt::should_compact(settings.pressure_tokens));
        assert!(!prompt::should_compact(settings.pressure_tokens - 1));
    }

    /// The signals vocabulary: closed keys, named refusals for unknown
    /// signals, and no defaulting an ill-typed field.
    #[test]
    fn unknown_signal_json_fails_closed() {
        assert_eq!(
            CompactionAdmissionSignals::from_json(&json!([])),
            Err(SignalsError::NotAnObject)
        );
        let unknown = CompactionAdmissionSignals::from_json(&json!({
            "memory": null,
            "host_load": 0.75,
        }));
        assert_eq!(
            unknown,
            Err(SignalsError::UnknownSignal("host_load".to_string())),
            "an unknown signal is a refusal, never an ignore"
        );
        let malformed = CompactionAdmissionSignals::from_json(&json!({
            "memory": {"pressure": "yes"},
        }));
        assert!(matches!(malformed, Err(SignalsError::Malformed(_))));
        let parsed = CompactionAdmissionSignals::from_json(&json!({
            "memory": {
                "available_bytes": 1024,
                "required_headroom_bytes": 2048,
                "pressure": false,
            },
            "queue": {
                "queued_requests": 1,
                "max_queued_requests": 4,
                "saturated": false,
            },
        }))
        .unwrap();
        assert!(parsed.memory.unwrap().is_pressure(), "1 < 2 is pressure");
        assert!(!parsed.queue.unwrap().is_saturated());
        // unknown NESTED keys are outside the closed surface too
        let nested = CompactionAdmissionSignals::from_json(&json!({
            "queue": {
                "queued_requests": 1,
                "max_queued_requests": 4,
                "saturated": false,
                "surprise": true,
            },
        }));
        assert_eq!(
            nested,
            Err(SignalsError::UnknownSignal("surprise".to_string()))
        );
    }

    /// The evidence shape: counts add up, the rejection breakdown names
    /// every refused reason, decisions ride along verbatim.
    #[test]
    fn evidence_counts_and_breaks_down_rejections() {
        let quota = CompactionQuota::default();
        let settings = ResolvedCompactionSettings::new();
        let prep = at_pressure(20_000);
        let pressured = CompactionAdmissionSignals {
            memory: Some(CompactionMemorySignal {
                available_bytes: Some(0),
                required_headroom_bytes: 1,
                pressure: false,
            }),
            ..Default::default()
        };
        let decisions = vec![
            admission_decision(
                &CompactionAdmissionState::default(),
                &quota,
                &settings,
                prep.as_ref(),
                &clean(),
            ),
            admission_decision(
                &CompactionAdmissionState::default(),
                &quota,
                &settings,
                None,
                &clean(),
            ),
            admission_decision(
                &CompactionAdmissionState::default(),
                &quota,
                &settings,
                prep.as_ref(),
                &pressured,
            ),
        ];
        let evidence = compaction_admission_evidence(&decisions, 120, 240);
        assert_eq!(evidence["schema"], json!(COMPACTION_ADMISSION_SCHEMA));
        assert_eq!(evidence["decisionCount"], json!(3));
        assert_eq!(evidence["admittedCount"], json!(1));
        assert_eq!(evidence["rejectedCount"], json!(2));
        assert_eq!(evidence["rejectedByReason"]["no_preparation"], json!(1));
        assert_eq!(evidence["rejectedByReason"]["memory_pressure"], json!(1));
        assert_eq!(evidence["foregroundImpact"]["p95Ms"], json!(120));
        assert_eq!(evidence["foregroundImpact"]["p99Ms"], json!(240));
        let rows = evidence["decisions"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["reason"], json!("allowed"));
        assert_eq!(rows[1]["reason"], json!("no_preparation"));
        assert_eq!(rows[2]["reason"], json!("memory_pressure"));
    }

    /// The reason vocabulary is closed and its strings are ABI.
    #[test]
    fn as_str_vocabulary_is_closed() {
        let all = [
            (CompactionAdmissionReason::Allowed, "allowed"),
            (CompactionAdmissionReason::Pending, "pending"),
            (
                CompactionAdmissionReason::SessionAttemptLimit,
                "session_attempt_limit",
            ),
            (CompactionAdmissionReason::Cooldown, "cooldown"),
            (CompactionAdmissionReason::NoPreparation, "no_preparation"),
            (CompactionAdmissionReason::MemoryPressure, "memory_pressure"),
            (
                CompactionAdmissionReason::ProviderDegraded,
                "provider_degraded",
            ),
            (CompactionAdmissionReason::QueueSaturated, "queue_saturated"),
        ];
        for (reason, s) in all {
            assert_eq!(reason.as_str(), s);
        }
        let allowed = CompactionAdmissionDecision {
            schema: COMPACTION_ADMISSION_SCHEMA,
            allowed: true,
            reason: CompactionAdmissionReason::Allowed,
            tokens_before: None,
            attempt_count: 0,
            max_attempts_per_session: 0,
            cooldown_remaining_ms: 0,
            signals: clean(),
        };
        assert!(allowed.is_allowed());
        assert!(
            !CompactionAdmissionDecision {
                allowed: false,
                ..allowed.clone()
            }
            .is_allowed()
        );
        // the Ord derive keeps allowed lowest (evidence ordering stability)
        assert!(CompactionAdmissionReason::Allowed < CompactionAdmissionReason::Pending);
    }
}
