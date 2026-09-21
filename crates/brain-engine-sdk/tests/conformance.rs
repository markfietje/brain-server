//! The spec-pinned conformance matrix for the SDK's plugin/events/tools
//! surface — the normative reference is `docs/private/CORDIS_CONTRACT.md`
//! (included at compile time below: doc and matrix move together).
//!
//! PASS-then-pin: every row drives an EXISTING law through the PUBLIC API
//! end-to-end. A row failing against current code is real drift — stop and
//! record it; never weaken the row to get green.
//!
//! Rows (the matrix):
//!   1. session/event — sanitized read seam + durable broadcast (per-listener
//!      clones, panic containment, later listeners never starve, every
//!      outcome recorded).
//!   2. agent — the checked lifecycle: phase gates, steering mid-turn,
//!      retained-token continuation (stale/cross-harness refuse live).
//!   3. tools — malformed input refuses BEFORE engine start; dispatch rides
//!      the trust waterfall.
//!   4. waterfall vs guard — first-deny-wins monotonic denial composed with
//!      the capability ladder (a denial cannot be overturned by a later
//!      allow).
//!   5. waterfall vs emit — the contrast pin: a waterfall denial runs NO
//!      later listener; an emit panic still runs EVERY listener.
//!   6. `conformance_all_hooks` — the generated-catalog drift gate: the
//!      catalog below must equal the surface derived through the SDK's own
//!      registration APIs, and the spec doc must carry the anchors.
//!   7. the `ctx.*` key contract — frozen ABI, duplicate refusal.
//!   8. `store_field_granularity` — prefix-stable windows, truncation flags
//!      exactly the dropped tail, stable notes digests, frozen decision keys.
//!   9. `hmr_swap_without_leak` — zero residual effects, never-parallel
//!      engine slot, a panicking unload still reverses.
//!
//! Overflow law note: turn-token overflow is pinned by the same-module test
//! `harness::tests::token_overflow_refuses_instead_of_wrapping` (private
//! generation access); the catalog cites it by reference — no duplication.

#![cfg(feature = "harness-kernel")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::{Arc, Mutex};

use brain_engine_sdk::capability::{Capability, OpClass, allows, checked_dispatch};
use brain_engine_sdk::env::ExecutionEnv;
use brain_engine_sdk::events::{Hooks, Outcome, Verdict};
use brain_engine_sdk::harness::{AgentHarness, HarnessError, Phase};
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, HostTx, WorkflowHost};
use brain_engine_sdk::plugin::{Context, EffectHandle, KernelError, Service};
use brain_engine_sdk::services::{
    EvidenceSvc, SandboxSvc, ScoringSvc, SystemPromptSvc, install as install_core_services,
};
use brain_engine_sdk::session::{SanitizedSession, SessionSanitizer, SessionSource};
use brain_engine_sdk::tools::create_workflow_tool;
use brain_engine_sdk::trust::{Decision, EngineOverride, ExtensionPolicy, HostCallKind};
use brain_engine_sdk::workflow::{
    AgentId, RunBuilder, StopReason, WorkflowEngine, WorkflowError, WorkflowMeta, WorkflowResult,
    WorkflowRun, WorkflowStartRequest,
};
use brain_engine_sdk::workflow_state as store;

// ---------------------------------------------------------------------------
// Test scaffolding (in-process only: tape hosts, inline engines, logs)
// ---------------------------------------------------------------------------

type Log = Arc<Mutex<Vec<String>>>;

fn push(log: &Mutex<Vec<String>>, line: &str) {
    log.lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(line.to_string());
}

fn log_of(log: &Mutex<Vec<String>>) -> Vec<String> {
    log.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Recording host: checked settlement confirmed in-memory, everything logged
/// in call order. The port any real backend implements.
#[derive(Default)]
struct TapeHost {
    log: Mutex<Vec<String>>,
    settled: Mutex<Vec<(i64, String, String)>>,
}

impl WorkflowHost for TapeHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        Err(HostError::Busy)
    }
    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        _payload: &str,
        key: &str,
    ) -> Result<bool, HostError> {
        push(&self.log, &format!("enqueue:{topic}:{key}:{run_id}"));
        Ok(true)
    }
    fn cas(&self, _run_id: i64, _expected_rev: i64, _state_json: &str) -> Result<(), CasError> {
        Ok(())
    }
    fn load_state(&self, _run_id: i64) -> Result<Option<(String, i64)>, HostError> {
        Ok(None)
    }
    fn audit(&self, k: AuditKind, actor: &str, target: &str, s: AuditStatus, d: &str) {
        push(
            &self.log,
            &format!(
                "audit:{}/{}/{}/{}:{}",
                k.as_str(),
                actor,
                target,
                s.as_str(),
                d
            ),
        );
    }
    fn enqueue_settlement(
        &self,
        run_id: i64,
        topic: &str,
        _payload: &str,
        key: &str,
    ) -> Result<bool, HostError> {
        push(&self.log, &format!("settle_enqueue:{topic}:{key}:{run_id}"));
        let mut settled = self.settled.lock().unwrap_or_else(|p| p.into_inner());
        let replay = settled
            .iter()
            .any(|(r, t, k)| *r == run_id && t == topic && k == key);
        if replay {
            return Ok(false);
        }
        settled.push((run_id, topic.to_string(), key.to_string()));
        Ok(true)
    }
    fn audit_settlement(
        &self,
        k: AuditKind,
        actor: &str,
        target: &str,
        s: AuditStatus,
        d: &str,
    ) -> Result<(), HostError> {
        push(
            &self.log,
            &format!(
                "settle_audit:{}/{}/{}/{}:{}",
                k.as_str(),
                actor,
                target,
                s.as_str(),
                d
            ),
        );
        Ok(())
    }
}

/// Inline engine: validates meta through the shared admit gate, completes the
/// run before handing it back, records every started workflow by name.
struct InlineEngine {
    outcome: StopReason,
    started: Log,
    name: &'static str,
}

impl WorkflowEngine for InlineEngine {
    fn start(&self, req: WorkflowStartRequest) -> Result<WorkflowRun, WorkflowError> {
        let mut b = RunBuilder::default();
        let id = b.admit(&req)?;
        let (completer, run) = b.build_run(id);
        let result = match self.outcome {
            StopReason::Completed => {
                WorkflowResult::completed(format!("ran {}", req.script.trim()))
            }
            StopReason::Error => WorkflowResult::error(),
            // StopReason is non-exhaustive: a variant added later surfaces
            // here as a visible tool error, never as a fake completion.
            _ => WorkflowResult::error(),
        };
        completer.complete(result);
        push(&self.started, self.name);
        Ok(run)
    }
}

fn engine_named(name: &'static str, log: &Log) -> Arc<dyn WorkflowEngine> {
    Arc::new(InlineEngine {
        outcome: StopReason::Completed,
        started: Arc::clone(log),
        name,
    })
}

fn start_request(name: &str) -> WorkflowStartRequest {
    WorkflowStartRequest {
        script: "say hi".into(),
        meta: WorkflowMeta {
            name: name.to_string(),
            description: "conformance fixture".into(),
            when_to_use: None,
            phases: Vec::new(),
        },
        args: "{}".into(),
        parent: AgentId("conf".into()),
    }
}

struct PlainSvc {
    key: &'static str,
}
impl Service for PlainSvc {
    fn key(&self) -> &'static str {
        self.key
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

// ---------------------------------------------------------------------------
// C2 — the generated catalog (committed const; the drift gate compares the
// DERIVED surface against exactly this table)
// ---------------------------------------------------------------------------

struct CatalogRow {
    surface: &'static str,
    mode: &'static str,
    law: &'static str,
    ctx_keys: &'static [&'static str],
}

const CONFORMANCE_CATALOG: &[CatalogRow] = &[
    CatalogRow {
        surface: "hooks",
        mode: "emit",
        law: "broadcast observe: per-listener clone, panic contained and counted, later listeners always run, every outcome recorded",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "hooks",
        mode: "waterfall",
        law: "short-circuit policy: first deny wins, later listeners never run, the denial is monotonic final",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "hooks",
        mode: "serial",
        law: "ordered mutations applied in registration order over shared state",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "hooks",
        mode: "parallel",
        law: "fan-out jobs over independent clones, aggregated in registration order",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "trust",
        mode: "waterfall",
        law: "deny-wins precedence (per-engine deny > global deny > per-engine allow > global allow > mode fallback); Permissive honors explicit denies; the HostCallKind vocabulary is closed and unknown is an error",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "capability",
        mode: "ladder",
        law: "monotonic fail-closed posture ladder (Safe, Standard, Permissive — anything unlisted denied); checked_dispatch audits denials too",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "tools",
        mode: "workflow",
        law: "malformed input refuses BEFORE engine start; the dispose guard runs on every path; non-completed ends surface as tool errors; dispatch rides the trust waterfall at the caller",
        ctx_keys: &["ctx.workflowEngine"],
    },
    CatalogRow {
        surface: "agent",
        mode: "lifecycle",
        law: "phase gates refuse structural ops while running, steering stays legal mid-turn, retained/stale/cross-harness tokens refuse live; overflow refuses (unit-pinned: harness::tests::token_overflow_refuses_instead_of_wrapping)",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "session",
        mode: "read",
        law: "sanitized read seam: raw bytes have exactly one consumer, the injected sanitizer; source errors propagate untouched",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "services",
        mode: "provide",
        law: "typed provide/require; duplicate claims fail loud; services::install mounts evidence + scoring",
        ctx_keys: &["ctx.evidence", "ctx.scoring"],
    },
    CatalogRow {
        surface: "services",
        mode: "keys",
        law: "frozen ABI wire names: rename or removal is a breaking release and fails the catalog",
        ctx_keys: &[
            "ctx.evidence",
            "ctx.scoring",
            "ctx.systemPrompt",
            "ctx.sandbox",
        ],
    },
    CatalogRow {
        surface: "plugin",
        mode: "mount",
        law: "inject ordering enforced before any mount side effect; duplicate keys refuse; audited mounts write denial rows too",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "plugin",
        mode: "hmr",
        law: "reload is unload-then-remount with zero residual effects (undo count equals mount count); the engine slot replaces and never runs two engines in parallel; a panicking unload still reverses",
        ctx_keys: &["ctx.workflowEngine"],
    },
    CatalogRow {
        surface: "workflow_state",
        mode: "decide",
        law: "four frozen routing keys (status, pending_question, next_step, next_state) with precedence terminal > pending > step > advance",
        ctx_keys: &[],
    },
    CatalogRow {
        surface: "workflow_state",
        mode: "window",
        law: "prefix-stable window derivation; field-budgeted truncation drops the oldest delta first and flags exactly that; FNV-1a notes digests are stable; decision keys stay frozen",
        ctx_keys: &[],
    },
];

/// The compile-time doc anchor: the matrix does not compile without the spec
/// doc, and the runtime half asserts the anchors stay in it.
const CORDIS_DOC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/private/CORDIS_CONTRACT.md"
));

fn catalog_pairs() -> Vec<(&'static str, &'static str)> {
    CONFORMANCE_CATALOG
        .iter()
        .map(|r| (r.surface, r.mode))
        .collect()
}

fn catalog_row(surface: &str, mode: &str) -> &'static CatalogRow {
    CONFORMANCE_CATALOG
        .iter()
        .find(|r| r.surface == surface && r.mode == mode)
        .unwrap_or_else(|| panic!("catalog row {surface}/{mode} vanished"))
}

fn assert_pairs_equal(mut derived: Vec<(&str, &str)>, mut catalog: Vec<(&str, &str)>) {
    derived.sort();
    catalog.sort();
    assert_eq!(
        derived.len(),
        catalog.len(),
        "surface/catalog drift: derived {derived:?} vs catalog {catalog:?}"
    );
    let mut d = derived.clone();
    d.dedup();
    assert_eq!(d.len(), derived.len(), "duplicate derivation observations");
    assert_eq!(derived, catalog, "surface/catalog drift");
}

// ---------------------------------------------------------------------------
// Row 1 — session/event: the sanitized seam + the durable broadcast law
// ---------------------------------------------------------------------------

struct FixedSource(String);
impl SessionSource for FixedSource {
    fn read_raw(&self, _key: &str) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

struct FailingSource;
impl SessionSource for FailingSource {
    fn read_raw(&self, _key: &str) -> Result<String, String> {
        Err("session gone".into())
    }
}

struct MaskEmails;
impl SessionSanitizer for MaskEmails {
    fn sanitize_view(&self, raw: &str) -> String {
        if raw.contains('@') {
            "[redacted:*]".to_string()
        } else {
            raw.to_string()
        }
    }
}

#[test]
fn session_event_durable_broadcast_and_sanitized_seam() {
    // The seam: the only session handle hands out the SANITIZED view; the raw
    // value never crosses; source errors propagate untouched.
    let session = SanitizedSession::new(
        FixedSource("contact jane@example.com about the export".into()),
        MaskEmails,
    );
    let view = session.read("state").unwrap();
    assert_eq!(view, "[redacted:*]");
    assert!(
        !view.contains("jane@example.com"),
        "raw bytes crossed the seam"
    );
    let failing = SanitizedSession::new(FailingSource, MaskEmails);
    assert_eq!(failing.read("x").unwrap_err(), "session gone");

    // The broadcast: per-listener clones, panic containment, later listeners
    // never starve, and EVERY outcome recorded (the durable per-listener
    // record), with provenance as sidecar metadata keyed by hook id.
    let h = Hooks::new();
    let seen: Log = Arc::default();
    let id_first = h
        .on::<String, _>("turn.start", "collector", {
            let seen = Arc::clone(&seen);
            move |e| {
                push(&seen, &format!("first:{e}"));
                Verdict::Allow
            }
        })
        .unwrap();
    let id_middle = h
        .on::<String, _>("turn.start", "chaos", |_e| {
            std::panic::panic_any("listener blew up");
        })
        .unwrap();
    let id_last = h
        .on::<String, _>("turn.start", "reporter", {
            let seen = Arc::clone(&seen);
            move |e| {
                push(&seen, &format!("last:{e}"));
                Verdict::Allow
            }
        })
        .unwrap();

    let report = h.emit("turn.start", &"payload".to_string());
    assert_eq!(report.outcomes.len(), 3, "every listener recorded");
    assert_eq!(report.outcomes[0], (id_first, Outcome::Ran));
    assert_eq!(report.outcomes[1], (id_middle, Outcome::Panicked));
    assert_eq!(report.outcomes[2], (id_last, Outcome::Ran));
    assert_eq!(report.panicked(), 1);
    assert_eq!(report.ran(), 2);
    // Per-listener clones: both live listeners saw the same original payload.
    assert_eq!(
        log_of(&seen),
        vec!["first:payload".to_string(), "last:payload".to_string()]
    );
    assert_eq!(h.provenance(id_first).as_deref(), Some("collector"));
    assert_eq!(h.provenance(id_last).as_deref(), Some("reporter"));
}

// ---------------------------------------------------------------------------
// Row 2 — agent: the checked lifecycle (live validation + continuation)
// ---------------------------------------------------------------------------

#[test]
fn agent_checked_lifecycle_live_validation_and_continuation() {
    let tape = Arc::new(TapeHost::default());
    let harness = AgentHarness::new(Arc::clone(&tape), "m1", "sys");
    assert_eq!(harness.phase(), Phase::Idle);

    // Start: snapshot captured, phase Running, token minted.
    let (snapshot, token) = harness.start_turn(1).unwrap();
    assert_eq!(snapshot.model(), "m1");
    assert_eq!(harness.phase(), Phase::Running);

    // Structural ops refuse mid-turn; steering and config stay legal.
    assert!(matches!(
        harness.compact(),
        Err(HarnessError::PhaseBusy {
            op: "compact",
            phase: Phase::Running
        })
    ));
    harness.steer("slow down, verify first").unwrap();
    harness.set_model("m2").unwrap();
    assert_eq!(
        snapshot.model(),
        "m1",
        "the in-flight snapshot is defensive"
    );

    // Owned delivery under the live token: message_end persists first, then
    // the drain (confirmed by the host's settlement record).
    harness
        .message_end_owned(&token, r#"{"final":true}"#, "msg-1")
        .unwrap();
    harness.save_point_owned(&token).unwrap();
    {
        let settled = tape.settled.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            settled
                .iter()
                .any(|(r, t, k)| *r == 1 && t == "message_end" && k == "msg-1"),
            "message_end was not settled: {settled:?}"
        );
    }

    // Retained-token continuation: a clone kept from turn 1 cannot mutate
    // turn 2, even with the same run id — live validation, not assumption.
    let retained = token.clone();
    harness.finish_turn(token).unwrap();
    assert_eq!(harness.phase(), Phase::Idle);
    harness.compact().unwrap();

    let (_snap2, token2) = harness.start_turn(1).unwrap();
    let err = harness
        .message_end_owned(&retained, r#"{"stale":true}"#, "msg-stale")
        .unwrap_err();
    assert!(matches!(err, HarnessError::Host(ref m) if m == "stale_turn_token"));
    assert!(
        !tape
            .settled
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|(_, _, k)| k == "msg-stale"),
        "a retained token wrote after its turn"
    );

    // Cross-harness tokens are foreign by construction.
    let other = AgentHarness::new(Arc::new(TapeHost::default()), "m", "s");
    let (_s, foreign) = other.start_turn(42).unwrap();
    let err = harness
        .message_end_owned(&foreign, "{}", "msg-foreign")
        .unwrap_err();
    assert!(matches!(err, HarnessError::Host(ref m) if m == "stale_turn_token"));

    // The current token still works; abort settles too and consumes it.
    harness
        .message_end_owned(&token2, r#"{"ok":true}"#, "msg-2")
        .unwrap();
    harness.abort_turn(token2).unwrap();
    assert_eq!(harness.phase(), Phase::Idle);
}

// ---------------------------------------------------------------------------
// Row 3 — tools: malformed input refuses before engine start; dispatch rides
// the trust waterfall
// ---------------------------------------------------------------------------

#[test]
fn tools_refuse_malformed_input_and_dispatch_rides_the_waterfall() {
    let started: Log = Arc::default();
    let engine = engine_named("workflow-starts", &started);
    let tool = create_workflow_tool(Arc::clone(&engine));

    // The caller-side gate is the trust waterfall: only an ALLOWED capability
    // reaches the tool invocation.
    let mut policy = ExtensionPolicy::standard();
    policy.default_caps.push("tools".into());
    assert_eq!(policy.decide("engine-a", "tools"), Decision::Allowed);

    // Malformed input refuses BEFORE the engine starts (both an empty input
    // and a missing script line).
    let env = ExecutionEnv::default();
    assert!(tool.execute(&env, "").is_err());
    assert!(tool.execute(&env, "only-a-name\n").is_err());
    assert!(
        log_of(&started).is_empty(),
        "a refused tool started the engine: {:?}",
        log_of(&started)
    );

    // A well-formed input starts the engine exactly once and surfaces the
    // completed output; a non-completed end is a tool error AFTER dispose.
    let out = tool.execute(&env, "demo\na demo workflow\nsay hi").unwrap();
    assert_eq!(out, "ran say hi");
    assert_eq!(log_of(&started), vec!["workflow-starts".to_string()]);

    let failing: Log = Arc::default();
    let failing_engine = Arc::new(InlineEngine {
        outcome: StopReason::Error,
        started: Arc::clone(&failing),
        name: "workflow-fails",
    });
    let failing_tool = create_workflow_tool(failing_engine);
    assert!(failing_tool.execute(&env, "bad\nbad\nboom").is_err());
    assert_eq!(log_of(&failing), vec!["workflow-fails".to_string()]);

    // The closed kind vocabulary: the tool wire class is "tool" mapping to
    // the "tools" capability; an unknown class is an error, never a default.
    assert_eq!(
        HostCallKind::parse("tool").unwrap().required_capability(),
        "tools"
    );
    assert!(HostCallKind::parse("workflow").is_err());
}

// ---------------------------------------------------------------------------
// Row 4 — waterfall vs guard: monotonic denial composed with the ladder
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ToolCall {
    engine: &'static str,
    cap: &'static str,
    posture: Capability,
    spawn: bool,
}

#[test]
fn waterfall_vs_guard_monotonic_deny_composition() {
    let h = Hooks::new();
    let ran: Log = Arc::default();
    let host = Arc::new(TapeHost::default());

    // The guard listener: deny when the POLICY denies (even against an
    // explicit per-engine allow) or when the CAPABILITY LADDER denies; allow
    // otherwise. The worker listener after it would execute the call.
    let policy = ExtensionPolicy {
        per_engine: [(
            "engine-a".to_string(),
            EngineOverride {
                allow_caps: vec!["exec".to_string()],
                deny_caps: vec![],
            },
        )]
        .into_iter()
        .collect(),
        deny_caps: vec!["exec".to_string()],
        ..ExtensionPolicy::standard()
    };
    let guard_host = Arc::clone(&host);
    h.on::<ToolCall, _>("tool.call", "guard", move |call| {
        if policy.decide(call.engine, call.cap) == Decision::Denied {
            return Verdict::Deny("policy denied: monotonic, later allows cannot overturn".into());
        }
        let class = if call.spawn {
            OpClass::ProcessSpawn
        } else {
            OpClass::ReadState
        };
        if !allows(call.posture, class) {
            checked_dispatch(&guard_host, call.posture, class, call.engine);
            return Verdict::Deny("capability ladder denied: fail-closed".into());
        }
        Verdict::Allow
    })
    .unwrap();
    h.on::<ToolCall, _>("tool.call", "worker", {
        let ran = Arc::clone(&ran);
        move |call| {
            push(&ran, call.engine);
            Verdict::Allow
        }
    })
    .unwrap();

    // 1) A global deny beats the explicit per-engine allow: the denial is
    //    final and the worker NEVER runs.
    let denied_call = ToolCall {
        engine: "engine-a",
        cap: "exec",
        posture: Capability::Standard,
        spawn: false,
    };
    let err = h.waterfall("tool.call", &denied_call).unwrap_err();
    assert!(err.contains("policy denied"), "unexpected denial: {err}");
    assert!(log_of(&ran).is_empty(), "a denied call reached the worker");

    // 2) The ladder denies what the policy allowed: also final, and the
    //    denial is audited by the dispatch itself.
    let spawn_call = ToolCall {
        engine: "engine-honest",
        cap: "tools",
        posture: Capability::Safe,
        spawn: true,
    };
    let err = h.waterfall("tool.call", &spawn_call).unwrap_err();
    assert!(
        err.contains("capability ladder denied"),
        "unexpected: {err}"
    );
    assert!(log_of(&ran).is_empty());
    assert!(
        log_of(&host.log)
            .iter()
            .any(|l| l.contains("process_spawn/denied")),
        "the ladder denial bypassed the audit chain"
    );

    // 3) The allowed call rides through both listeners.
    let ok_call = ToolCall {
        engine: "engine-honest",
        cap: "tools",
        posture: Capability::Standard,
        spawn: false,
    };
    let report = h.waterfall("tool.call", &ok_call).unwrap();
    assert_eq!(report.ran(), 2);
    assert_eq!(log_of(&ran), vec!["engine-honest".to_string()]);
}

// ---------------------------------------------------------------------------
// Row 5 — waterfall short-circuit vs emit broadcast (the contrast pin)
// ---------------------------------------------------------------------------

#[test]
fn waterfall_short_circuit_vs_emit_broadcast_contrast() {
    // Waterfall: the FIRST denial short-circuits — a later listener never
    // runs and cannot overturn the denial.
    let wf = Hooks::new();
    let later_ran: Log = Arc::default();
    wf.on::<String, _>("gate", "denier", |_e| Verdict::Deny("stopped".into()))
        .unwrap();
    wf.on::<String, _>("gate", "would-run", {
        let later_ran = Arc::clone(&later_ran);
        move |_e| {
            push(&later_ran, "ran after a denial");
            Verdict::Allow
        }
    })
    .unwrap();
    let err = wf.waterfall("gate", &"x".to_string()).unwrap_err();
    assert_eq!(err, "stopped");
    assert!(
        log_of(&later_ran).is_empty(),
        "a waterfall denial ran a later listener"
    );

    // Emit: broadcast observe — a panicking listener is contained and EVERY
    // listener still runs.
    let em = Hooks::new();
    let ran: Log = Arc::default();
    em.on::<String, _>("bus", "boomer", |_e| {
        std::panic::panic_any("listener blew up");
    })
    .unwrap();
    for who in ["a", "b"] {
        em.on::<String, _>("bus", who, {
            let ran = Arc::clone(&ran);
            move |e| {
                push(&ran, &format!("{who}:{e}"));
                Verdict::Allow
            }
        })
        .unwrap();
    }
    let report = em.emit("bus", &"payload".to_string());
    assert_eq!(report.panicked(), 1, "the panic was contained");
    assert_eq!(report.ran(), 2, "an emit panic ran EVERY listener");
    assert_eq!(report.outcomes.len(), 3, "every outcome recorded");
    assert_eq!(
        log_of(&ran),
        vec!["a:payload".to_string(), "b:payload".to_string()]
    );
}

// ---------------------------------------------------------------------------
// Rows 7-8 — the ctx.* key contract (C3)
// ---------------------------------------------------------------------------

#[test]
fn ctx_key_contract_is_frozen_abi() {
    // The four production wire names, read through the SDK's own Service
    // trait — the frozen ABI set. A renamed or removed key fails here before
    // it breaks an engine.
    let mut keys = vec![
        EvidenceSvc.key().to_string(),
        ScoringSvc.key().to_string(),
        SystemPromptSvc.key().to_string(),
        SandboxSvc.key().to_string(),
    ];
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "ctx.evidence".to_string(),
            "ctx.sandbox".to_string(),
            "ctx.scoring".to_string(),
            "ctx.systemPrompt".to_string(),
        ]
    );

    // Discoverability: mount through the SDK's own installers, require back
    // by type; a missing service is a loud NotMounted, never a default.
    let mut ctx = Context::new();
    install_core_services(&mut ctx).unwrap();
    ctx.provide(SystemPromptSvc).unwrap();
    ctx.provide(SandboxSvc).unwrap();
    let evidence = ctx.require::<EvidenceSvc>().unwrap();
    assert_eq!(evidence.key(), "ctx.evidence");
    let sandbox = ctx.require::<SandboxSvc>().unwrap();
    assert_eq!(sandbox.key(), "ctx.sandbox");
    let err = ctx.require::<String>().unwrap_err();
    assert!(matches!(err, KernelError::NotMounted { .. }));
}

#[test]
fn ctx_duplicate_registration_fails_loud() {
    // Duplicate SERVICE installs refuse (the kernel's normal posture).
    let mut ctx = Context::new();
    install_core_services(&mut ctx).unwrap();
    assert!(install_core_services(&mut ctx).is_err());
    assert!(ctx.provide(SystemPromptSvc).is_ok());
    assert!(ctx.provide(SystemPromptSvc).is_err());

    // Duplicate KEY installs refuse with the named error.
    ctx.install(Box::new(PlainSvc { key: "ctx.dup" })).unwrap();
    let err = ctx
        .install(Box::new(PlainSvc { key: "ctx.dup" }))
        .unwrap_err();
    assert_eq!(
        err,
        KernelError::Duplicate {
            key: "ctx.dup".to_string()
        }
    );

    // The engine slot is a replacement, not a duplicate refusal: mounting
    // again REPLACES (config-driven), which the HMR row pins end-to-end.
    let log: Log = Arc::default();
    ctx.mount_workflow_engine(engine_named("first", &log))
        .unwrap();
    ctx.mount_workflow_engine(engine_named("second", &log))
        .unwrap();
    assert!(ctx.workflow_engine().is_some(), "the slot holds one engine");
}

// ---------------------------------------------------------------------------
// Row 9 — C4 store field granularity
// ---------------------------------------------------------------------------

fn ev(id: i64, topic: &str, payload: &str) -> store::EventRow {
    store::EventRow {
        id,
        topic: topic.to_string(),
        payload_json: payload.to_string(),
    }
}

fn conf_chain() -> Vec<store::EventRow> {
    vec![
        ev(1, "workflow/start", r#"{"note":"open"}"#),
        ev(2, "workflow/log", r#"{"line":"s1"}"#),
        ev(
            3,
            "workflow/checkpoint",
            r#"{"steps":[1],"findings":["f1"],"pending_question":"ship?"}"#,
        ),
        ev(4, "workflow/log", r#"{"line":"s2"}"#),
        ev(5, "workflow/log", r#"{"line":"s3"}"#),
    ]
}

#[test]
fn store_field_granularity() {
    // Appending event N leaves every window anchored at-or-before N
    // BYTE-IDENTICAL (prefix stability = the dirty set is exactly the
    // windows at-or-after N); only the new latest window moves.
    let events = conf_chain();
    let before = store::derive_context_at(&events, Some(4), 10_000);
    let mut grown = events.clone();
    grown.push(ev(6, "workflow/log", r#"{"line":"s4"}"#));
    grown.push(ev(7, "workflow/checkpoint", r#"{"steps":[2]}"#));
    let after = store::derive_context_at(&grown, Some(4), 10_000);
    assert_eq!(before, after, "an append dirties an earlier window");

    let latest = store::derive_context(&grown, 10_000);
    assert_eq!(
        latest.checkpoint.as_ref().map(|c| c.id),
        Some(7),
        "the new latest window rides the new checkpoint"
    );

    // Field-budgeted truncation: the flag marks EXACTLY the dropped tail —
    // the oldest delta drops first; checkpoint, notes, open question never
    // drop. Budget 0 means unbounded (documented), never truncated.
    let tight = store::derive_context(&events, 2);
    assert_eq!(
        tight.delta.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![5],
        "the dropped tail is exactly event 4"
    );
    assert!(tight.truncated);
    assert_eq!(tight.checkpoint.as_ref().map(|c| c.id), Some(3));
    assert_eq!(tight.open_question.as_deref(), Some("ship?"));
    assert_eq!(tight.findings_digests.len(), 1);
    let unbounded = store::derive_context(&events, 0);
    assert_eq!(unbounded.delta.len(), 2);
    assert!(!unbounded.truncated);

    // The notes hash is stable across renders: identical chains derive
    // identical digests, and re-deriving never moves a digest.
    let again = store::derive_context(&events.clone(), 10_000);
    let once_more = store::derive_context(&events, 10_000);
    assert_eq!(again.findings_digests, once_more.findings_digests);
    assert_eq!(tight.findings_digests, again.findings_digests);

    // Decision keys are frozen ABI: the four routing keys route exactly one
    // variant each, precedence included, byte-stable through a round-trip.
    let fixtures: Vec<(serde_json::Value, store::Decision)> = vec![
        (serde_json::json!({"status": "done"}), store::Decision::Done),
        (
            serde_json::json!({"status": "complete"}),
            store::Decision::Done,
        ),
        (
            serde_json::json!({"pending_question": "which disk?"}),
            store::Decision::AskHuman {
                question: "which disk?".into(),
            },
        ),
        (
            serde_json::json!({"next_step": "inventory"}),
            store::Decision::RunStep {
                step: "inventory".into(),
            },
        ),
        (
            serde_json::json!({"next_state": "{\"status\":\"active\"}"}),
            store::Decision::Advance {
                next_state: "{\"status\":\"active\"}".into(),
            },
        ),
        (serde_json::json!({}), store::Decision::Done),
    ];
    for (v, want) in fixtures {
        let round =
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(store::decide(&round), want, "routing drifted for {round}");
    }
    assert_eq!(
        store::decide(&serde_json::json!({"status": "done", "pending_question": "x"})),
        store::Decision::Done,
        "terminal beats pending"
    );
    assert!(matches!(
        store::decide(&serde_json::json!({"pending_question": "x", "next_step": "y"})),
        store::Decision::AskHuman { .. }
    ));
}

// ---------------------------------------------------------------------------
// Row 10 — C5 HMR swap without leak
// ---------------------------------------------------------------------------

struct SwapSvc {
    log: Log,
    undo: Log,
    handle: Option<EffectHandle>,
}
impl SwapSvc {
    fn new() -> (Self, Log, Log) {
        let mount_log: Log = Arc::default();
        let undo_log: Log = Arc::default();
        (
            Self {
                log: Arc::clone(&mount_log),
                undo: Arc::clone(&undo_log),
                handle: None,
            },
            mount_log,
            undo_log,
        )
    }
}
impl Service for SwapSvc {
    fn key(&self) -> &'static str {
        "ctx.swap"
    }
    fn mount(&mut self, ctx: &mut Context) {
        push(&self.log, "mount");
        let undo = Arc::clone(&self.undo);
        self.handle = Some(ctx.effect("swap-watch", move |_ctx| {
            move |_ctx| {
                push(&undo, "undo");
            }
        }));
    }
    fn unmount(&self) {
        push(&self.log, "unmount");
    }
}

struct PanicUnload {
    undo: Log,
    handle: Option<EffectHandle>,
}
impl Service for PanicUnload {
    fn key(&self) -> &'static str {
        "ctx.panic"
    }
    fn mount(&mut self, ctx: &mut Context) {
        let undo = Arc::clone(&self.undo);
        self.handle = Some(ctx.effect("panic-watch", move |_ctx| {
            move |_ctx| {
                push(&undo, "undo");
            }
        }));
    }
    fn unmount(&self) {
        std::panic::panic_any("unload blew up");
    }
}

#[test]
fn hmr_swap_without_leak() {
    // Reload = unload-then-remount with ZERO residual effects: the undo count
    // equals the mount count at every step, and the mounted set is exact.
    let (svc, mount_log, undo_log) = SwapSvc::new();
    let mut ctx = Context::new();
    ctx.install(Box::new(svc)).unwrap();
    assert_eq!(log_of(&mount_log), vec!["mount".to_string()]);
    assert!(log_of(&undo_log).is_empty());
    assert_eq!(ctx.mounted_keys(), vec!["ctx.swap"]);

    ctx.reload("ctx.swap").unwrap();
    assert_eq!(
        log_of(&mount_log),
        vec![
            "mount".to_string(),
            "unmount".to_string(),
            "mount".to_string()
        ]
    );
    assert_eq!(
        log_of(&undo_log),
        vec!["undo".to_string()],
        "the old effect reversed"
    );
    assert_eq!(ctx.mounted_keys(), vec!["ctx.swap"]);

    ctx.uninstall("ctx.swap").unwrap();
    assert_eq!(log_of(&undo_log).len(), 2, "undo count == mount count");
    assert!(ctx.mounted_keys().is_empty(), "zero residual mounts");

    // The engine slot: ONE engine per context — replacement, never parallel.
    let starts: Log = Arc::default();
    let first = engine_named("first", &starts);
    let second = engine_named("second", &starts);
    ctx.mount_workflow_engine(Arc::clone(&first)).unwrap();
    let mounted = ctx.workflow_engine().expect("slot holds the first engine");
    assert!(Arc::ptr_eq(&mounted, &first));
    ctx.mount_workflow_engine(Arc::clone(&second)).unwrap();
    let mounted = ctx.workflow_engine().expect("slot holds the second engine");
    assert!(
        !Arc::ptr_eq(&mounted, &first),
        "the slot still ran the old engine"
    );
    assert!(Arc::ptr_eq(&mounted, &second));
    mounted.start(start_request("demo")).unwrap();
    assert_eq!(
        log_of(&starts),
        vec!["second".to_string()],
        "never parallel"
    );

    // The effect stack reverses in strict reverse order (public handles).
    let order: Log = Arc::default();
    let h1 = {
        let order = Arc::clone(&order);
        ctx.effect("one", move |_| {
            push(&order, "setup-one");
            let order = Arc::clone(&order);
            move |_| {
                push(&order, "undo-one");
            }
        })
    };
    let h2 = {
        let order = Arc::clone(&order);
        ctx.effect("two", move |_| {
            push(&order, "setup-two");
            let order = Arc::clone(&order);
            move |_| {
                push(&order, "undo-two");
            }
        })
    };
    h2.dispose();
    drop(h1);
    assert_eq!(
        log_of(&order),
        vec![
            "setup-one".to_string(),
            "setup-two".to_string(),
            "undo-two".to_string(),
            "undo-one".to_string(),
        ]
    );

    // A panicking unload STILL reverses: the registry removed the service
    // before the hook ran, and the service's effects reverse during the
    // unwind (caught here — the gate stays green on a hostile unmount).
    let undo: Log = Arc::default();
    ctx.install(Box::new(PanicUnload {
        undo: Arc::clone(&undo),
        handle: None,
    }))
    .unwrap();
    assert_eq!(ctx.mounted_keys(), vec!["ctx.panic"]);
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.uninstall("ctx.panic")));
    assert!(outcome.is_err(), "the panic propagates to the caller");
    assert!(
        ctx.mounted_keys().is_empty(),
        "a panicking unload left the key mounted: {:?}",
        ctx.mounted_keys()
    );
    assert_eq!(
        log_of(&undo),
        vec!["undo".to_string()],
        "the effect reversed anyway"
    );
}

// ---------------------------------------------------------------------------
// Row 6 — C2 the generated-catalog drift gate (+ the C6 doc anchor)
// ---------------------------------------------------------------------------

#[test]
fn conformance_all_hooks() {
    let mut observed: Vec<(&str, &str)> = Vec::new();

    // Derive the @mode surface through the registration APIs: which dispatch
    // modes actually run each registered hook kind.
    let h = Hooks::new();
    h.on::<String, _>("conf.ob", "observer", |_e| Verdict::Allow)
        .unwrap();
    let mut state = String::new();
    h.on_mutate::<String, _>("conf.se", "mutator", |s: &mut String| s.push('m'))
        .unwrap();
    h.on_parallel::<String, _>("conf.pa", "job", |_e| {})
        .unwrap();

    assert_eq!(h.emit("conf.ob", &"p".to_string()).ran(), 1);
    assert!(h.waterfall("conf.ob", &"p".to_string()).is_ok());
    observed.push(("hooks", "emit"));
    observed.push(("hooks", "waterfall"));

    assert_eq!(
        h.emit("conf.se", &"p".to_string()).ran(),
        0,
        "mode isolation"
    );
    assert!(h.waterfall("conf.se", &"p".to_string()).is_ok());
    assert_eq!(h.parallel("conf.se", &"p".to_string()).ran(), 0);
    let report = h.serial("conf.se", &mut state);
    assert_eq!(report.ran(), 1);
    assert_eq!(state, "m");
    observed.push(("hooks", "serial"));

    assert_eq!(
        h.emit("conf.pa", &"p".to_string()).ran(),
        0,
        "mode isolation"
    );
    assert_eq!(h.parallel("conf.pa", &"p".to_string()).ran(), 1);
    observed.push(("hooks", "parallel"));

    // Derive the trust + capability faces.
    let mut policy = ExtensionPolicy::permissive();
    policy.deny_caps = vec!["exec".to_string()];
    assert_eq!(policy.decide("any", "exec"), Decision::Denied);
    assert!(HostCallKind::parse("nope").is_err());
    observed.push(("trust", "waterfall"));

    assert!(allows(Capability::Safe, OpClass::ReadState));
    assert!(!allows(Capability::Standard, OpClass::ProcessSpawn));
    assert!(allows(Capability::Permissive, OpClass::ProcessSpawn));
    let host = Arc::new(TapeHost::default());
    assert!(!checked_dispatch(
        &host,
        Capability::Safe,
        OpClass::ProcessSpawn,
        "e"
    ));
    observed.push(("capability", "ladder"));

    // Derive the tools face: the factory names its tool.
    let starts: Log = Arc::default();
    let tool = create_workflow_tool(engine_named("x", &starts));
    assert_eq!(tool.name, "workflow");
    observed.push(("tools", "workflow"));

    // Derive the agent face: a live lifecycle cycle through the public API.
    let harness = AgentHarness::new(Arc::new(TapeHost::default()), "m", "s");
    let (_snap, token) = harness.start_turn(7).unwrap();
    assert_eq!(harness.phase(), Phase::Running);
    harness.abort_turn(token).unwrap();
    assert_eq!(harness.phase(), Phase::Idle);
    observed.push(("agent", "lifecycle"));

    // Derive the session face: the seam sanitizes.
    let session = SanitizedSession::new(FixedSource("a@b.c".into()), MaskEmails);
    assert_eq!(session.read("k").unwrap(), "[redacted:*]");
    observed.push(("session", "read"));

    // Derive the services faces: install + the frozen keys.
    let mut ctx = Context::new();
    install_core_services(&mut ctx).unwrap();
    assert!(ctx.require::<ScoringSvc>().is_ok());
    observed.push(("services", "provide"));
    let mut keys = vec![
        EvidenceSvc.key().to_string(),
        ScoringSvc.key().to_string(),
        SystemPromptSvc.key().to_string(),
        SandboxSvc.key().to_string(),
    ];
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "ctx.evidence".to_string(),
            "ctx.sandbox".to_string(),
            "ctx.scoring".to_string(),
            "ctx.systemPrompt".to_string(),
        ]
    );
    observed.push(("services", "keys"));

    // Derive the plugin faces: install ordering + duplicate refusal, and the
    // engine slot's replace-never-parallel semantics.
    ctx.install(Box::new(PlainSvc { key: "ctx.derive" }))
        .unwrap();
    assert_eq!(ctx.mounted_keys(), vec!["ctx.derive"]);
    assert!(
        ctx.install(Box::new(PlainSvc { key: "ctx.derive" }))
            .is_err()
    );
    observed.push(("plugin", "mount"));

    let slot_log: Log = Arc::default();
    ctx.mount_workflow_engine(engine_named("d1", &slot_log))
        .unwrap();
    ctx.mount_workflow_engine(engine_named("d2", &slot_log))
        .unwrap();
    let mounted = ctx.workflow_engine().expect("the slot holds one engine");
    mounted.start(start_request("derive")).unwrap();
    assert_eq!(log_of(&slot_log), vec!["d2".to_string()]);
    observed.push(("plugin", "hmr"));

    // Derive the workflow_state faces.
    let chain = conf_chain();
    let window = store::derive_context(&chain, 10_000);
    assert!(window.checkpoint.is_some());
    assert_eq!(
        store::decide(&serde_json::json!({"status": "done"})),
        store::Decision::Done
    );
    observed.push(("workflow_state", "decide"));
    observed.push(("workflow_state", "window"));

    // CATALOG ↔ SURFACE EQUALITY: a mode/surface added or removed without the
    // matrix fails here.
    assert_pairs_equal(observed, catalog_pairs());

    // Every ctx key named anywhere in the catalog is one of the frozen ABI
    // names (the engine slot's wire name included).
    let frozen = [
        "ctx.evidence",
        "ctx.scoring",
        "ctx.systemPrompt",
        "ctx.sandbox",
        "ctx.workflowEngine",
    ];
    for row in CONFORMANCE_CATALOG {
        for key in row.ctx_keys {
            assert!(
                frozen.contains(key),
                "catalog names a non-frozen ctx key: {key}"
            );
        }
        assert!(!row.law.is_empty());
    }
    let keys_row = catalog_row("services", "keys");
    assert_eq!(keys_row.ctx_keys.len(), 4, "the frozen set is exactly four");
    assert!(
        catalog_row("tools", "workflow")
            .ctx_keys
            .contains(&"ctx.workflowEngine")
    );
    assert!(
        catalog_row("plugin", "hmr")
            .ctx_keys
            .contains(&"ctx.workflowEngine")
    );

    // The C6 doc anchor: the spec doc exists (compile-time include), and it
    // still names every mode, every frozen key, and the honest ceilings.
    for marker in [
        "emit",
        "waterfall",
        "serial",
        "parallel",
        "ctx.evidence",
        "ctx.scoring",
        "ctx.systemPrompt",
        "ctx.sandbox",
        "ctx.workflowEngine",
        "single-process",
        "No CBOR anywhere in 1.32.x",
        "experimental",
        "2.0 Cortex",
        "deferred by choice",
        "minimal, hardened reimplementation",
    ] {
        assert!(
            CORDIS_DOC.contains(marker),
            "the spec doc lost its anchor: {marker}"
        );
    }
}
