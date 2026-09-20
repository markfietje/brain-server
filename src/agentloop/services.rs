//! The `ctx.*` service tree: thin adapters over the loop driver's own
//! owners, mounted through the SDK plugin kernel's audited lifecycle.
//!
//! Nothing here duplicates logic: each adapter holds the SAME `Arc`
//! instances the driver holds and delegates each call to the owning module
//! (`session_log`, `compaction`, `subagents`, `run_loop`, the SDK tool
//! registry, the provider seam). State lives in the injected Arcs, whose
//! lifecycle is the driver's own — `mount`/`unmount` are side-effect-free,
//! so reload can never double-claim and reverse-order uninstall is
//! trivially honest. The assembler keeps the kernel's two faces separate:
//! `provide` fills the typed `require` map, `install` fills the ordered,
//! audited, reversible mounted set; neither face re-claims the other, so a
//! reload of the installed face never `Duplicate`s.
//!
//! Separation of concerns (pinned by tests, mapping lives here by
//! contract): a tool DEFINITION — name, schema bytes, runner — is
//! registered once and executes identically under any PROVIDER-side
//! environment (the caller-narrowed `ExecutionEnv` differing only in
//! capability); the driver-side CONSUMER swaps environments without
//! touching the definition. The adapter is the consumer face; the
//! registry is the provider face; `ToolDef` is the definition.
//!
//! Inspectability law: the renderer in this module is pure — it reads the
//! plan and constructor-injected values only, never the process env, and
//! renders provider identity by NAME only. The sandbox service is an
//! honest denial: no sandbox backend exists in this tree, and nothing
//! here may present a local runner as one.

use std::sync::Arc;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef, ToolRegistry};
use brain_engine_sdk::plugin::{Context, KernelError, Service, install_audited, uninstall_audited};
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::compaction::{self, CompactionPolicy, CompactionSplit};
use crate::agentloop::context::{ContextError, ContextEvent};
use crate::agentloop::provider::LlmProvider;
use crate::agentloop::run_loop::{LoopConfig, LoopDriver, LoopError, RunOutcome};
use crate::agentloop::subagents::{
    ExchangeBudget, SubagentOutcome, SubagentSpec, delegate_owned_budgeted,
};
use crate::workflow::host::SqliteWorkflowHost;
use crate::workflow::session_log::{self, SessionEventRow};
use crate::workflow::tx::WorkflowTx;

pub(crate) const KEY_TOOLS: &str = "ctx.tools";
pub(crate) const KEY_LLM: &str = "ctx.llm";
pub(crate) const KEY_SESSIONS: &str = "ctx.sessions";
pub(crate) const KEY_SYSTEM_PROMPT: &str = "ctx.systemPrompt";
pub(crate) const KEY_COMPACTION: &str = "ctx.compaction";
pub(crate) const KEY_SANDBOX: &str = "ctx.sandbox";
pub(crate) const KEY_AGENTS: &str = "ctx.agents";
pub(crate) const KEY_AGENT_LOOP: &str = "ctx.agentLoop";
pub(crate) const KEY_EVIDENCE: &str = "ctx.evidence";
pub(crate) const KEY_SCORING: &str = "ctx.scoring";

/// The canonical inject edges (web shape): dependencies before dependents
/// in every plan that carries them. Pinned by test — plans never widen
/// these; a plan missing a dependency mounts nothing.
pub(crate) fn canonical_edges(key: &str) -> &'static [&'static str] {
    match key {
        KEY_COMPACTION => &[KEY_SESSIONS, KEY_LLM],
        KEY_AGENTS => &[KEY_SESSIONS, KEY_LLM, KEY_TOOLS],
        KEY_AGENT_LOOP => &[KEY_SESSIONS, KEY_LLM, KEY_TOOLS],
        _ => &[],
    }
}

/// The keys every profile structurally carries and every canonical edge
/// toward them is therefore always required: the session log and the
/// provider. A plan that carries a dependent without them is refused by
/// the kernel's MissingDependency — never silently narrowed.
const STRUCTURAL_KEYS: &[&str] = &[KEY_SESSIONS, KEY_LLM];

/// The install edges a plan derives for `key`: the canonical edges minus
/// the ones toward profile-OPTIONAL keys the plan omits (the tool surface,
/// the delegation face). Edges toward structural keys never drop — a plan
/// that carries a dependent without its structural dependencies is refused,
/// not narrowed. Canonical order preserved.
fn derived_edges(key: &str, keys: &[&'static str]) -> Vec<&'static str> {
    canonical_edges(key)
        .iter()
        .filter(|dep| keys.contains(dep) || STRUCTURAL_KEYS.contains(dep))
        .copied()
        .collect()
}

/// A resolved profile: fixed mount order (dependencies before dependents)
/// plus each key's derived inject edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfilePlan {
    pub(crate) name: &'static str,
    pub(crate) keys: Vec<&'static str>,
}

impl ProfilePlan {
    /// Per-key install edges, aligned with `keys`: canonical edges, with
    /// edges toward profile-optional keys dropped when the plan omits them.
    pub(crate) fn edges(&self) -> Vec<Vec<&'static str>> {
        self.keys
            .iter()
            .map(|k| derived_edges(k, &self.keys))
            .collect()
    }
}

/// Named profile failure — never a fallback to a default profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileError {
    Unknown { name: String },
}

/// Resolve a profile by name. Pure: no env reads, no I/O — the same input
/// resolves to the byte-identical plan every call.
pub(crate) fn resolve_profile(name: &str) -> Result<ProfilePlan, ProfileError> {
    let (name, keys): (&'static str, Vec<&'static str>) = match name {
        "web" => (
            "web",
            vec![
                KEY_TOOLS,
                KEY_LLM,
                KEY_SESSIONS,
                KEY_SYSTEM_PROMPT,
                KEY_COMPACTION,
                KEY_SANDBOX,
                KEY_AGENTS,
                KEY_AGENT_LOOP,
                KEY_EVIDENCE,
                KEY_SCORING,
            ],
        ),
        "headless" => (
            "headless",
            vec![
                KEY_LLM,
                KEY_SESSIONS,
                KEY_SYSTEM_PROMPT,
                KEY_COMPACTION,
                KEY_SANDBOX,
                KEY_AGENT_LOOP,
                KEY_EVIDENCE,
                KEY_SCORING,
            ],
        ),
        other => {
            return Err(ProfileError::Unknown {
                name: other.to_string(),
            });
        }
    };
    Ok(ProfilePlan { name, keys })
}

/// Failure vocabulary for the session faces: pool acquisition and SQL are
/// distinct, both loud.
#[derive(Debug)]
pub(crate) enum SessionError {
    Pool(String),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Pool(m) => write!(f, "session pool: {m}"),
            SessionError::Sql(e) => write!(f, "session store: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<rusqlite::Error> for SessionError {
    fn from(e: rusqlite::Error) -> Self {
        SessionError::Sql(e)
    }
}

fn pool_conn(
    pool: &Pool,
) -> Result<r2d2::PooledConnection<crate::pool::SqliteConnectionManager>, SessionError> {
    pool.get().map_err(|e| SessionError::Pool(e.to_string()))
}

/// `ctx.tools`: the registry (provider face) plus the definitions the
/// driver was built with.
#[derive(Clone)]
pub(crate) struct ToolsSvc {
    registry: Arc<ToolRegistry>,
    tools: Vec<ToolDef>,
    injects: Vec<&'static str>,
}

impl Service for ToolsSvc {
    fn key(&self) -> &'static str {
        KEY_TOOLS
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl ToolsSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) fn definitions(&self) -> &[ToolDef] {
        &self.tools
    }
    pub(crate) fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }
    pub(crate) fn execute_tool(
        &self,
        name: &str,
        env: &ExecutionEnv,
        input_json: &str,
    ) -> brain_engine_sdk::env::ToolResult {
        self.registry.execute(name, env, input_json)
    }
}

/// `ctx.llm`: the provider seam, identity by NAME only — never key
/// material, never configuration.
#[derive(Clone)]
pub(crate) struct LlmSvc {
    provider: Arc<dyn LlmProvider>,
    injects: Vec<&'static str>,
}

impl Service for LlmSvc {
    fn key(&self) -> &'static str {
        KEY_LLM
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl LlmSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) fn name(&self) -> &str {
        self.provider.name()
    }
}

/// `ctx.sessions`: the durable session-log face over the driver's pool.
#[derive(Clone)]
pub(crate) struct SessionsSvc {
    pool: Pool,
    injects: Vec<&'static str>,
}

impl Service for SessionsSvc {
    fn key(&self) -> &'static str {
        KEY_SESSIONS
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl SessionsSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) fn append(
        &self,
        run_id: i64,
        kind: &str,
        payload_json: &str,
        idempotency_key: &str,
        now: i64,
    ) -> Result<(bool, i64), SessionError> {
        let conn = pool_conn(&self.pool)?;
        session_log::append(&conn, run_id, kind, payload_json, idempotency_key, now)
            .map_err(SessionError::Sql)
    }

    pub(crate) fn replay(
        &self,
        run_id: i64,
        cap: usize,
    ) -> Result<Vec<SessionEventRow>, SessionError> {
        let conn = pool_conn(&self.pool)?;
        session_log::replay(&conn, run_id, cap).map_err(SessionError::Sql)
    }

    /// Claim the run's ownership lease. `session_log::acquire` requires the
    /// caller to hold a `BEGIN IMMEDIATE` transaction; this face begins and
    /// commits exactly that one transaction — the driver's own discipline,
    /// not a new one.
    pub(crate) fn claim(&self, run_id: i64, owner: &str) -> Result<(), SessionError> {
        let mut conn = pool_conn(&self.pool)?;
        let mut tx = WorkflowTx::begin(&mut conn).map_err(SessionError::Sql)?;
        session_log::acquire(tx.tx(), run_id, owner).map_err(SessionError::Sql)?;
        tx.commit().map_err(SessionError::Sql)?;
        Ok(())
    }

    /// Release the ownership lease (same one-transaction shape as `claim`).
    pub(crate) fn release(&self, run_id: i64, owner: &str) -> Result<(), SessionError> {
        let mut conn = pool_conn(&self.pool)?;
        let mut tx = WorkflowTx::begin(&mut conn).map_err(SessionError::Sql)?;
        session_log::release(tx.tx(), run_id, owner).map_err(SessionError::Sql)?;
        tx.commit().map_err(SessionError::Sql)?;
        Ok(())
    }
}

/// `ctx.agents`: the delegation face over `subagents::delegate_owned_budgeted`
/// with the budget handle the driver's config carries (a fresh root budget
/// when the config carries none).
#[derive(Clone)]
pub(crate) struct AgentsSvc {
    pool: Pool,
    host: Arc<SqliteWorkflowHost>,
    env: ExecutionEnv,
    tools: Vec<ToolDef>,
    provider: Arc<dyn LlmProvider>,
    budget: ExchangeBudget,
    injects: Vec<&'static str>,
}

impl Service for AgentsSvc {
    fn key(&self) -> &'static str {
        KEY_AGENTS
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl AgentsSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) fn budget(&self) -> &ExchangeBudget {
        &self.budget
    }

    pub(crate) async fn delegate(
        &self,
        run_id: i64,
        spec: &SubagentSpec,
        invocation_key: &str,
        owner: &str,
        cancel: &CancellationToken,
    ) -> Result<SubagentOutcome, LoopError> {
        delegate_owned_budgeted(
            &self.pool,
            &self.host,
            &self.env,
            &self.tools,
            Arc::clone(&self.provider),
            run_id,
            spec,
            invocation_key,
            owner,
            cancel,
            &self.budget,
        )
        .await
    }
}

/// `ctx.agentLoop`: run control over the driver itself.
#[derive(Clone)]
pub(crate) struct AgentLoopSvc {
    driver: Arc<LoopDriver>,
    injects: Vec<&'static str>,
}

impl Service for AgentLoopSvc {
    fn key(&self) -> &'static str {
        KEY_AGENT_LOOP
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl AgentLoopSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) async fn run_turns(
        &self,
        run_id: i64,
        input: &str,
        cancel: &CancellationToken,
    ) -> Result<RunOutcome, LoopError> {
        self.driver.run_turns(run_id, input, cancel).await
    }
}

/// `ctx.compaction`: policy plus planning over the replayed window.
#[derive(Clone)]
pub(crate) struct CompactionSvc {
    pool: Pool,
    policy: CompactionPolicy,
    injects: Vec<&'static str>,
}

impl Service for CompactionSvc {
    fn key(&self) -> &'static str {
        KEY_COMPACTION
    }
    fn inject(&self) -> &[&str] {
        self.edges()
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl CompactionSvc {
    pub(crate) fn edges(&self) -> &[&'static str] {
        &self.injects
    }
    pub(crate) fn policy(&self) -> CompactionPolicy {
        self.policy
    }

    pub(crate) fn plan_window(
        &self,
        run_id: i64,
        cap: usize,
    ) -> Result<Option<CompactionSplit>, SessionError> {
        let conn = pool_conn(&self.pool)?;
        let events = session_log::replay(&conn, run_id, cap).map_err(SessionError::Sql)?;
        Ok(compaction::plan(&events))
    }

    pub(crate) fn reconstruct(
        &self,
        events: &[ContextEvent],
    ) -> Result<compaction::EffectiveContext, ContextError> {
        compaction::reconstruct(events)
    }
}

/// The kernel-side owners each adapter needs, constructor-injected. A
/// profile key whose owners are absent is an assembly error named BEFORE
/// anything mounts — never a silent skip.
#[derive(Default)]
pub(crate) struct ServiceOwners {
    pub(crate) pool: Option<Pool>,
    pub(crate) provider: Option<Arc<dyn LlmProvider>>,
    pub(crate) registry: Option<Arc<ToolRegistry>>,
    pub(crate) tools: Option<Vec<ToolDef>>,
    pub(crate) env: Option<ExecutionEnv>,
    pub(crate) config: Option<LoopConfig>,
    pub(crate) driver: Option<Arc<LoopDriver>>,
}

/// One built service: the ordered/audited face (installed) and the typed
/// `require` face (provided) are separate instances of the same thin
/// adapter over the same Arcs.
pub(crate) enum Built {
    Tools(ToolsSvc),
    Llm(LlmSvc),
    Sessions(SessionsSvc),
    Agents(AgentsSvc),
    AgentLoop(AgentLoopSvc),
    Compaction(CompactionSvc),
    SystemPrompt(brain_engine_sdk::services::SystemPromptSvc),
    Sandbox(brain_engine_sdk::services::SandboxSvc),
    Evidence(brain_engine_sdk::services::EvidenceSvc),
    Scoring(brain_engine_sdk::services::ScoringSvc),
}

impl Built {
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Built::Tools(s) => s.key(),
            Built::Llm(s) => s.key(),
            Built::Sessions(s) => s.key(),
            Built::Agents(s) => s.key(),
            Built::AgentLoop(s) => s.key(),
            Built::Compaction(s) => s.key(),
            Built::SystemPrompt(s) => s.key(),
            Built::Sandbox(s) => s.key(),
            Built::Evidence(s) => s.key(),
            Built::Scoring(s) => s.key(),
        }
    }

    pub(crate) fn injects(&self) -> Vec<&'static str> {
        match self {
            Built::Tools(s) => s.edges().to_vec(),
            Built::Llm(s) => s.edges().to_vec(),
            Built::Sessions(s) => s.edges().to_vec(),
            Built::Agents(s) => s.edges().to_vec(),
            Built::AgentLoop(s) => s.edges().to_vec(),
            Built::Compaction(s) => s.edges().to_vec(),
            Built::SystemPrompt(_) => Vec::new(),
            Built::Sandbox(_) => Vec::new(),
            Built::Evidence(_) => Vec::new(),
            Built::Scoring(_) => Vec::new(),
        }
    }

    fn boxed(&self) -> Box<dyn Service + Send> {
        match self {
            Built::Tools(s) => Box::new(s.clone()),
            Built::Llm(s) => Box::new(s.clone()),
            Built::Sessions(s) => Box::new(s.clone()),
            Built::Agents(s) => Box::new(s.clone()),
            Built::AgentLoop(s) => Box::new(s.clone()),
            Built::Compaction(s) => Box::new(s.clone()),
            Built::SystemPrompt(_) => Box::new(brain_engine_sdk::services::SystemPromptSvc),
            Built::Sandbox(_) => Box::new(brain_engine_sdk::services::SandboxSvc),
            Built::Evidence(_) => Box::new(brain_engine_sdk::services::EvidenceSvc),
            Built::Scoring(_) => Box::new(brain_engine_sdk::services::ScoringSvc),
        }
    }

    fn provide(&self, ctx: &mut Context) -> Result<(), KernelError> {
        match self {
            Built::Tools(s) => ctx.provide(s.clone()),
            Built::Llm(s) => ctx.provide(s.clone()),
            Built::Sessions(s) => ctx.provide(s.clone()),
            Built::Agents(s) => ctx.provide(s.clone()),
            Built::AgentLoop(s) => ctx.provide(s.clone()),
            Built::Compaction(s) => ctx.provide(s.clone()),
            Built::SystemPrompt(_) => ctx.provide(brain_engine_sdk::services::SystemPromptSvc),
            Built::Sandbox(_) => ctx.provide(brain_engine_sdk::services::SandboxSvc),
            Built::Evidence(_) => ctx.provide(brain_engine_sdk::services::EvidenceSvc),
            Built::Scoring(_) => ctx.provide(brain_engine_sdk::services::ScoringSvc),
        }
    }
}

/// Assemble failure vocabulary: a missing owner names the key and the
/// owner; anything the kernel refuses surfaces as its own named error.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AssembleError {
    MissingOwner {
        key: &'static str,
        owner: &'static str,
    },
    Kernel(KernelError),
}

impl std::fmt::Display for AssembleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssembleError::MissingOwner { key, owner } => {
                write!(f, "service `{key}` missing owner `{owner}`")
            }
            AssembleError::Kernel(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssembleError {}

fn missing_owner(key: &'static str, owner: &'static str) -> AssembleError {
    AssembleError::MissingOwner { key, owner }
}

/// Build one key's service from the owners. Every absent owner is named
/// here, before any mounting happens.
fn build(
    key: &'static str,
    edges: Vec<&'static str>,
    owners: &ServiceOwners,
    host: &Arc<SqliteWorkflowHost>,
) -> Result<Built, AssembleError> {
    Ok(match key {
        KEY_TOOLS => {
            let Some(registry) = owners.registry.clone() else {
                return Err(missing_owner(KEY_TOOLS, "tool registry"));
            };
            let Some(tools) = owners.tools.clone() else {
                return Err(missing_owner(KEY_TOOLS, "tool definitions"));
            };
            Built::Tools(ToolsSvc {
                registry,
                tools,
                injects: edges,
            })
        }
        KEY_LLM => {
            let Some(provider) = owners.provider.clone() else {
                return Err(missing_owner(KEY_LLM, "provider"));
            };
            Built::Llm(LlmSvc {
                provider,
                injects: edges,
            })
        }
        KEY_SESSIONS => {
            let Some(pool) = owners.pool.clone() else {
                return Err(missing_owner(KEY_SESSIONS, "pool"));
            };
            Built::Sessions(SessionsSvc {
                pool,
                injects: edges,
            })
        }
        KEY_AGENTS => {
            let Some(pool) = owners.pool.clone() else {
                return Err(missing_owner(KEY_AGENTS, "pool"));
            };
            let Some(env) = owners.env.clone() else {
                return Err(missing_owner(KEY_AGENTS, "execution environment"));
            };
            let Some(tools) = owners.tools.clone() else {
                return Err(missing_owner(KEY_AGENTS, "tool definitions"));
            };
            let Some(provider) = owners.provider.clone() else {
                return Err(missing_owner(KEY_AGENTS, "provider"));
            };
            let budget = owners
                .config
                .as_ref()
                .and_then(|c| c.budget_accounting.clone())
                .unwrap_or_else(|| ExchangeBudget::new(None));
            Built::Agents(AgentsSvc {
                pool,
                host: Arc::clone(host),
                env,
                tools,
                provider,
                budget,
                injects: edges,
            })
        }
        KEY_AGENT_LOOP => {
            let Some(driver) = owners.driver.clone() else {
                return Err(missing_owner(KEY_AGENT_LOOP, "loop driver"));
            };
            Built::AgentLoop(AgentLoopSvc {
                driver,
                injects: edges,
            })
        }
        KEY_COMPACTION => {
            let Some(pool) = owners.pool.clone() else {
                return Err(missing_owner(KEY_COMPACTION, "pool"));
            };
            let Some(config) = owners.config.as_ref() else {
                return Err(missing_owner(KEY_COMPACTION, "loop config"));
            };
            Built::Compaction(CompactionSvc {
                pool,
                policy: config.compaction,
                injects: edges,
            })
        }
        KEY_SYSTEM_PROMPT => Built::SystemPrompt(brain_engine_sdk::services::SystemPromptSvc),
        KEY_SANDBOX => Built::Sandbox(brain_engine_sdk::services::SandboxSvc),
        KEY_EVIDENCE => Built::Evidence(brain_engine_sdk::services::EvidenceSvc),
        KEY_SCORING => Built::Scoring(brain_engine_sdk::services::ScoringSvc),
        other => return Err(missing_owner(other, "known service key")),
    })
}

/// Assemble a profile plan into a mounted context. Phase 1 builds every
/// key's service (naming any missing owner); phase 2 mounts in plan order
/// through the kernel's audited, inject-enforced face and provides the
/// typed face. Nothing mounts unless every owner was present, and a plan
/// whose dependencies are absent is refused by the kernel's own
/// MissingDependency before the service's mount runs.
pub(crate) fn assemble(
    plan: &ProfilePlan,
    owners: &ServiceOwners,
    host: &Arc<SqliteWorkflowHost>,
) -> Result<Context, AssembleError> {
    let edges = plan.edges();
    let mut built = Vec::with_capacity(plan.keys.len());
    for (key, key_edges) in plan.keys.iter().zip(&edges) {
        built.push(build(key, key_edges.clone(), owners, host)?);
    }
    let mut ctx = Context::new();
    for svc in &built {
        install_audited(host, &mut ctx, svc.boxed()).map_err(AssembleError::Kernel)?;
        svc.provide(&mut ctx).map_err(AssembleError::Kernel)?;
    }
    Ok(ctx)
}

/// Tear a mounted context down in the exact reverse of the given order,
/// every unmount audited on the host chain.
pub(crate) fn disassemble(
    ctx: &mut Context,
    host: &Arc<SqliteWorkflowHost>,
    keys: &[&'static str],
) -> Result<(), KernelError> {
    for key in keys.iter().rev() {
        uninstall_audited(host, ctx, key)?;
    }
    Ok(())
}

/// What the renderer may see: constructor-injected values only. There is
/// no env access, no provider material, and no config echo beyond the
/// fixed scalar fields below — anything absent renders as a named absent.
pub(crate) struct InspectContext<'a> {
    pub(crate) config: Option<&'a LoopConfig>,
    pub(crate) provider_name: Option<&'a str>,
}

/// The bounded, env-blind renderer: profile name, mounted keys in install
/// order, the loop's scalar configuration, provider NAME only, and the
/// sandbox posture as the honest denial. Deterministic by construction —
/// the same plan and view render byte-identically, whatever the process
/// environment holds.
pub(crate) fn inspect(plan: &ProfilePlan, ctx: &InspectContext<'_>) -> String {
    let mut out = String::new();
    out.push_str("profile: ");
    out.push_str(plan.name);
    out.push('\n');
    out.push_str("services:");
    for key in &plan.keys {
        out.push(' ');
        out.push_str(key);
    }
    out.push('\n');
    let Some(config) = ctx.config else {
        for field in [
            "max_turns: absent",
            "session_replay_cap: absent",
            "tool_output_cap: absent",
            "tool_timeout_ms: absent",
            "compaction: absent",
            "token_budget: absent",
            "budget_accounting: absent",
        ] {
            out.push_str(field);
            out.push('\n');
        }
        out.push_str("provider: ");
        out.push_str(ctx.provider_name.unwrap_or("absent"));
        out.push('\n');
        out.push_str("sandbox: unavailable (denied)\n");
        return out;
    };
    out.push_str(&format!("max_turns: {}\n", config.max_turns));
    out.push_str(&format!(
        "session_replay_cap: {}\n",
        config.session_replay_cap
    ));
    out.push_str(&format!("tool_output_cap: {}\n", config.tool_output_cap));
    out.push_str(&format!(
        "tool_timeout_ms: {}\n",
        config.tool_timeout.as_millis()
    ));
    out.push_str(&format!(
        "compaction: {}/{}/{}\n",
        config.compaction.just_before_call,
        config.compaction.tool_result_clearing,
        config.compaction.selective_retention
    ));
    out.push_str(&format!(
        "token_budget: {}\n",
        if config.token_budget.is_some() {
            "configured"
        } else {
            "absent"
        }
    ));
    out.push_str(&format!(
        "budget_accounting: {}\n",
        if config.budget_accounting.is_some() {
            "configured"
        } else {
            "absent"
        }
    ));
    out.push_str("provider: ");
    out.push_str(ctx.provider_name.unwrap_or("absent"));
    out.push('\n');
    out.push_str("sandbox: unavailable (denied)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::provider::{LoopbackProvider, StreamEvent, scripted_text};
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::proficiency::{Proficiency, env_for};
    use brain_engine_sdk::env::{DenyAll, FsSeam, create_read_tool};
    use brain_engine_sdk::harness::AgentHarness;
    use brain_engine_sdk::plugin::{KernelError, install_audited};
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    /// In-memory seam for adapter tests: real reads, recorded writes.
    #[derive(Default)]
    struct MemFs {
        files: StdMutex<HashMap<String, String>>,
    }
    impl FsSeam for MemFs {
        fn read(&self, path: &str) -> Result<String, EnvError> {
            self.files
                .lock()
                .ok()
                .and_then(|g| g.get(path).cloned())
                .ok_or_else(|| EnvError::NotFound(path.into()))
        }
        fn write(&self, path: &str, content: &str) -> Result<(), EnvError> {
            if let Ok(mut g) = self.files.lock() {
                g.insert(path.into(), content.into());
            }
            Ok(())
        }
        fn exec(&self, command: &str) -> Result<String, EnvError> {
            Ok(format!("ran {command}"))
        }
    }
    use brain_engine_sdk::env::EnvError;

    fn staged() -> (Pool, Arc<SqliteWorkflowHost>) {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
        let pool: Pool = r2d2::Pool::builder().max_size(2).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
        (pool.clone(), Arc::new(SqliteWorkflowHost::new(pool)))
    }

    struct Stage {
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        provider: Arc<LoopbackProvider>,
        tools: Vec<ToolDef>,
        registry: Arc<ToolRegistry>,
        env: ExecutionEnv,
        config: LoopConfig,
        driver: Arc<LoopDriver>,
    }

    /// Everything a profile needs, built the way the driver is built: the
    /// same pool, host, provider, env, definitions, and config.
    fn stage(script: Vec<Vec<StreamEvent>>) -> Stage {
        let (pool, host) = staged();
        let harness = Arc::new(AgentHarness::new(host.clone(), "test-model", "steward"));
        let provider = LoopbackProvider::new("loopback-fixture", script);
        let fs = Arc::new(MemFs::default());
        fs.write("a.txt", "steward file body").ok();
        let env = ExecutionEnv {
            fs: fs.clone() as Arc<dyn FsSeam>,
            read_only: false,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let tools = vec![create_read_tool()];
        let mut registry = ToolRegistry::new();
        for def in &tools {
            registry.register(def.clone()).unwrap();
        }
        let config = LoopConfig::default();
        let driver = Arc::new(LoopDriver::new(
            pool.clone(),
            host.clone(),
            harness,
            provider.clone(),
            tools.clone(),
            env.clone(),
            config.clone(),
            "",
            crate::agentloop::hooks::LoopHooks::pass_through(),
        ));
        Stage {
            pool,
            host,
            provider,
            tools,
            registry: Arc::new(registry),
            env,
            config,
            driver,
        }
    }

    fn owners_of(s: &Stage) -> ServiceOwners {
        let provider: Arc<dyn LlmProvider> = s.provider.clone();
        ServiceOwners {
            pool: Some(s.pool.clone()),
            provider: Some(provider),
            registry: Some(s.registry.clone()),
            tools: Some(s.tools.clone()),
            env: Some(s.env.clone()),
            config: Some(s.config.clone()),
            driver: Some(s.driver.clone()),
        }
    }

    fn audit_rows(pool: &Pool) -> Vec<(String, String)> {
        let conn = pool.get().unwrap();
        // Lifecycle rows only: the host write path may also carry compliance
        // decision-evidence rows beside each audit; the mount/unmount
        // discipline is what this fixture pins.
        let mut stmt = conn
            .prepare(
                "SELECT target_hash, detail_hash FROM audit_events
                 WHERE actor = 'plugin' AND detail_hash IN (?1, ?2) ORDER BY id",
            )
            .unwrap();
        stmt.query_map(
            [crate::audit::hash("mount"), crate::audit::hash("unmount")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    }

    /// THE one behavioral RED available in R8: missing-owner assembly is
    /// refused with a named error before any mount, and a plan whose key's
    /// dependencies are absent is refused with the kernel's own
    /// MissingDependency. The first half pins the owner check; the second
    /// pins inject enforcement through the assembly path.
    #[test]
    fn assemble_refuses_missing_owner_and_missing_injection_named() {
        let (pool, host) = staged();

        // Missing owner: ctx.llm with no provider — named, before any mount.
        let plan = ProfilePlan {
            name: "probe",
            keys: vec![KEY_LLM],
        };
        assert!(matches!(
            assemble(&plan, &ServiceOwners::default(), &host),
            Err(AssembleError::MissingOwner {
                key: KEY_LLM,
                owner: "provider"
            })
        ));
        let mounts = audit_rows(&pool);
        assert!(mounts.is_empty(), "no mount side effect before the refusal");

        // Missing injection: ctx.compaction's owners exist, but the plan
        // omits ctx.sessions — the kernel's MissingDependency must surface.
        let plan = ProfilePlan {
            name: "probe",
            keys: vec![KEY_COMPACTION],
        };
        let owners = ServiceOwners {
            pool: Some(pool),
            config: Some(LoopConfig::default()),
            ..Default::default()
        };
        assert!(matches!(
            &assemble(&plan, &owners, &host),
            Err(AssembleError::Kernel(KernelError::MissingDependency {
                service: KEY_COMPACTION,
                needs,
            })) if *needs == KEY_SESSIONS
        ));
    }

    #[test]
    fn profiles_resolve_deterministically_and_differ_exactly_in_the_pinned_keys() {
        let web_a = resolve_profile("web").unwrap();
        let web_b = resolve_profile("web").unwrap();
        assert_eq!(web_a, web_b, "same input, byte-identical plan");
        assert_eq!(
            web_a.keys,
            vec![
                KEY_TOOLS,
                KEY_LLM,
                KEY_SESSIONS,
                KEY_SYSTEM_PROMPT,
                KEY_COMPACTION,
                KEY_SANDBOX,
                KEY_AGENTS,
                KEY_AGENT_LOOP,
                KEY_EVIDENCE,
                KEY_SCORING,
            ]
        );
        let headless = resolve_profile("headless").unwrap();
        let expected_headless: Vec<&'static str> = web_a
            .keys
            .iter()
            .copied()
            .filter(|k| *k != KEY_TOOLS && *k != KEY_AGENTS)
            .collect();
        assert_eq!(headless.keys, expected_headless);
        assert_eq!(headless.keys.len(), 8);
        assert_eq!(
            resolve_profile("agent-profile"),
            Err(ProfileError::Unknown {
                name: "agent-profile".into()
            })
        );
    }

    #[test]
    fn inject_edges_are_the_declared_contract_and_intersect_in_headless() {
        let edge_of = |plan: &ProfilePlan, key: &str| -> Vec<&'static str> {
            plan.edges()
                .into_iter()
                .zip(plan.keys.iter().copied())
                .find(|(_, k)| *k == key)
                .map(|(e, _)| e)
                .unwrap()
        };
        let web = resolve_profile("web").unwrap();
        // Web carries every key, so every derived edge IS the canonical
        // contract, exactly.
        for key in &web.keys {
            assert_eq!(edge_of(&web, key), canonical_edges(key).to_vec());
        }
        assert_eq!(edge_of(&web, KEY_COMPACTION), vec![KEY_SESSIONS, KEY_LLM]);
        assert_eq!(
            edge_of(&web, KEY_AGENTS),
            vec![KEY_SESSIONS, KEY_LLM, KEY_TOOLS]
        );
        assert_eq!(
            edge_of(&web, KEY_AGENT_LOOP),
            vec![KEY_SESSIONS, KEY_LLM, KEY_TOOLS]
        );
        assert!(edge_of(&web, KEY_TOOLS).is_empty());
        assert!(edge_of(&web, KEY_LLM).is_empty());
        assert!(edge_of(&web, KEY_SESSIONS).is_empty());
        // Headless drops ctx.tools, so the loop's tool edge drops with it.
        let headless = resolve_profile("headless").unwrap();
        assert_eq!(
            edge_of(&headless, KEY_AGENT_LOOP),
            vec![KEY_SESSIONS, KEY_LLM]
        );
        assert_eq!(
            edge_of(&headless, KEY_COMPACTION),
            vec![KEY_SESSIONS, KEY_LLM]
        );
    }

    #[test]
    fn assemble_mounts_in_plan_order_and_disassemble_is_the_exact_reverse() {
        let stage = stage(vec![]);
        let plan = resolve_profile("web").unwrap();
        let owners = owners_of(&stage);
        let mut ctx = assemble(&plan, &owners, &stage.host).unwrap();
        assert_eq!(ctx.mounted_keys(), plan.keys, "install order is plan order");
        disassemble(&mut ctx, &stage.host, &plan.keys).unwrap();
        assert!(ctx.mounted_keys().is_empty());

        let mut expected = Vec::new();
        for key in &plan.keys {
            expected.push((crate::audit::hash(key), crate::audit::hash("mount")));
        }
        for key in plan.keys.iter().rev() {
            expected.push((crate::audit::hash(key), crate::audit::hash("unmount")));
        }
        assert_eq!(
            audit_rows(&stage.pool),
            expected,
            "audited reverse-order cleanup"
        );
    }

    #[test]
    fn reload_compaction_leaves_every_other_key_mounted_and_never_duplicates() {
        let stage = stage(vec![]);
        let plan = resolve_profile("web").unwrap();
        let owners = owners_of(&stage);
        let mut ctx = assemble(&plan, &owners, &stage.host).unwrap();
        ctx.reload("ctx.compaction").unwrap();
        let keys = ctx.mounted_keys();
        assert_eq!(
            keys.len(),
            plan.keys.len(),
            "reload reclaims, never duplicates"
        );
        for key in &plan.keys {
            assert!(keys.contains(key), "{key} stays mounted through a reload");
        }
        disassemble(&mut ctx, &stage.host, &plan.keys).unwrap();
        assert!(ctx.mounted_keys().is_empty());
    }

    #[test]
    fn effect_handle_reversal_through_an_assembled_context() {
        let stage = stage(vec![]);
        let plan = resolve_profile("web").unwrap();
        let owners = owners_of(&stage);
        let mut ctx = assemble(&plan, &owners, &stage.host).unwrap();
        let undo_log: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(Vec::new()));
        let log_for_undo = undo_log.clone();
        let handle = ctx.effect("probe", move |_ctx: &mut Context| {
            move |_ctx: &mut Context| {
                if let Ok(mut g) = log_for_undo.lock() {
                    g.push("undone");
                }
            }
        });
        drop(handle);
        assert_eq!(
            undo_log.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["undone"]
        );
        assert_eq!(ctx.mounted_keys().len(), plan.keys.len());
    }

    #[test]
    fn denied_mount_lands_on_the_audit_chain() {
        let stage = stage(vec![]);
        let plan = resolve_profile("web").unwrap();
        let owners = owners_of(&stage);
        let mut ctx = assemble(&plan, &owners, &stage.host).unwrap();
        // A second ctx.compaction is a Duplicate — and its denial lands on
        // the chain like every mount decision.
        let duplicate = build(KEY_COMPACTION, vec![], &owners, &stage.host).unwrap();
        assert!(matches!(
            install_audited(&stage.host, &mut ctx, duplicate.boxed()),
            Err(KernelError::Duplicate { key }) if key == KEY_COMPACTION
        ));
        let denied: Vec<(String, String)> = audit_rows(&stage.pool)
            .into_iter()
            .filter(|(target, _)| *target == crate::audit::hash(KEY_COMPACTION))
            .collect();
        assert!(denied.contains(&(
            crate::audit::hash(KEY_COMPACTION),
            crate::audit::hash("mount")
        )));
        // The status of that mount row is `denied` — read it back directly,
        // pinned to the mount detail so decision-evidence rows never shadow it.
        let conn = stage.pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM audit_events WHERE actor = 'plugin' AND target_hash = ?1
                 AND detail_hash = ?2 ORDER BY id DESC LIMIT 1",
                [
                    crate::audit::hash(KEY_COMPACTION),
                    crate::audit::hash("mount"),
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "denied");
    }

    #[test]
    fn definition_is_provider_and_consumer_independent() {
        let def = create_read_tool();
        let definition = (
            def.name.clone(),
            def.description.clone(),
            def.schema_json.clone(),
        );

        // Two provider-side environments over the SAME definition: the full
        // operator-provisioned env and the proficiency-narrowed env differ
        // ONLY in capability, and the definition executes identically.
        let fs = Arc::new(MemFs::default());
        fs.write("a.txt", "steward file body").ok();
        let full = ExecutionEnv {
            fs: fs.clone() as Arc<dyn FsSeam>,
            read_only: false,
            allow_process: true,
            root: "/".into(),
            allowed_commands: vec!["ls".into()],
        };
        let narrowed = env_for(&full, Proficiency::L1);
        assert!(narrowed.read_only && !full.read_only);

        let mut registry_a = ToolRegistry::new();
        registry_a.register(def.clone()).unwrap();
        let mut registry_b = ToolRegistry::new();
        registry_b.register(def.clone()).unwrap();
        assert_eq!(
            registry_a.execute("read", &full, "a.txt"),
            registry_b.execute("read", &narrowed, "a.txt"),
            "identically-shaped execution under both provider environments"
        );
        assert_eq!(
            registry_a.execute("read", &full, "a.txt").unwrap(),
            "steward file body"
        );

        // The consumer swaps environments without touching the definition.
        let swapped = ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        registry_a.execute("read", &swapped, "a.txt").unwrap_err();
        assert_eq!(
            (
                def.name.clone(),
                def.description.clone(),
                def.schema_json.clone()
            ),
            definition,
            "the definition is untouched by every consumer/env swap"
        );
    }

    #[test]
    fn sessions_adapter_appends_and_replays_over_the_driver_pool() {
        let stage = stage(vec![]);
        let svc = SessionsSvc {
            pool: stage.pool.clone(),
            injects: vec![],
        };
        let (created_first, seq_first) = svc
            .append(1, "user", r#"{"text":"hi"}"#, "k-one", 1_000)
            .unwrap();
        assert!(created_first);
        let (created_again, seq_again) = svc
            .append(1, "user", r#"{"text":"hi"}"#, "k-one", 1_001)
            .unwrap();
        assert!(!created_again, "idempotent replay is a no-op receipt");
        assert_eq!(seq_first, seq_again);
        svc.append(1, "assistant", r#"{"text":"hello"}"#, "k-two", 1_002)
            .unwrap();
        let rows = svc.replay(1, 10).unwrap();
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "assistant"]);
    }

    #[test]
    fn sessions_adapter_claim_and_release_over_the_one_tx_shape() {
        let stage = stage(vec![]);
        let svc = SessionsSvc {
            pool: stage.pool.clone(),
            injects: vec![],
        };
        svc.claim(1, "adapter-owner").unwrap();
        // A second owner cannot take the claim while held.
        assert!(svc.claim(1, "second-owner").is_err());
        svc.release(1, "adapter-owner").unwrap();
        // After release the claim is gone: releasing again fails loud.
        assert!(svc.release(1, "adapter-owner").is_err());
    }

    #[test]
    fn llm_adapter_exposes_name_only() {
        let provider: Arc<dyn LlmProvider> = LoopbackProvider::new("loopback-fixture", vec![]);
        let svc = LlmSvc {
            provider,
            injects: vec![],
        };
        assert_eq!(svc.name(), "loopback-fixture");
    }

    #[test]
    fn compaction_adapter_delegates_plan_and_reconstruct() {
        let stage = stage(vec![]);
        let svc = CompactionSvc {
            pool: stage.pool.clone(),
            policy: stage.config.compaction,
            injects: vec![],
        };
        assert_eq!(svc.policy(), stage.config.compaction);
        let sessions = SessionsSvc {
            pool: stage.pool.clone(),
            injects: vec![],
        };
        sessions
            .append(1, "user", r#"{"text":"hi"}"#, "k", 1)
            .unwrap();
        let direct = {
            let conn = stage.pool.get().unwrap();
            session_log::replay(&conn, 1, 50).unwrap()
        };
        assert_eq!(
            svc.plan_window(1, 50).unwrap(),
            compaction::plan(&direct),
            "adapter plan equals core plan over the same window"
        );
        let effective = svc.reconstruct(&[]).unwrap();
        assert!(effective.summary.is_none() && effective.tail.is_empty());
    }

    #[test]
    fn agentloop_adapter_runs_a_turn_through_the_driver() {
        let stage = stage(vec![scripted_text("all done")]);
        let svc = AgentLoopSvc {
            driver: stage.driver.clone(),
            injects: vec![],
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = CancellationToken::new();
        let outcome = rt.block_on(svc.run_turns(1, "hello", &cancel)).unwrap();
        assert!(matches!(outcome, RunOutcome::Completed { .. }));
        assert_eq!(stage.provider.requests().len(), 1);
    }

    #[test]
    fn web_assembles_all_ten_headless_eight_and_typed_faces_resolve() {
        let stage = stage(vec![]);
        let owners = owners_of(&stage);
        let web = resolve_profile("web").unwrap();
        let ctx = assemble(&web, &owners, &stage.host).unwrap();
        assert_eq!(ctx.mounted_keys(), web.keys);
        ctx.require::<ToolsSvc>().unwrap();
        ctx.require::<SessionsSvc>().unwrap();
        ctx.require::<LlmSvc>().unwrap();
        ctx.require::<AgentsSvc>().unwrap();
        ctx.require::<AgentLoopSvc>().unwrap();
        ctx.require::<CompactionSvc>().unwrap();
        ctx.require::<brain_engine_sdk::services::SystemPromptSvc>()
            .unwrap();
        ctx.require::<brain_engine_sdk::services::SandboxSvc>()
            .unwrap();
        ctx.require::<brain_engine_sdk::services::EvidenceSvc>()
            .unwrap();
        ctx.require::<brain_engine_sdk::services::ScoringSvc>()
            .unwrap();

        let headless = resolve_profile("headless").unwrap();
        let ctx = assemble(&headless, &owners, &stage.host).unwrap();
        assert_eq!(ctx.mounted_keys(), headless.keys);
        // No tool surface, no delegation face — the honest headless shape.
        assert!(ctx.require::<ToolsSvc>().is_err());
        assert!(ctx.require::<AgentsSvc>().is_err());
        ctx.require::<SessionsSvc>().unwrap();
        ctx.require::<AgentLoopSvc>().unwrap();
    }

    #[test]
    fn inspect_is_deterministic_bounded_and_env_blind() {
        let stage = stage(vec![]);
        let plan = resolve_profile("web").unwrap();
        let view = InspectContext {
            config: Some(&stage.config),
            provider_name: Some(stage.provider.name()),
        };
        let first = inspect(&plan, &view);
        let second = inspect(&plan, &view);
        assert_eq!(first, second, "two calls, byte-equal output");

        // Bounded, fixed fields: exactly these eleven lines, nothing else —
        // no unbounded echo of config, session data, or provider internals.
        const FIELDS: &[&str] = &[
            "profile: ",
            "services:",
            "max_turns: ",
            "session_replay_cap: ",
            "tool_output_cap: ",
            "tool_timeout_ms: ",
            "compaction: ",
            "token_budget: ",
            "budget_accounting: ",
            "provider: ",
            "sandbox: unavailable (denied)",
        ];
        let lines: Vec<&str> = first.lines().collect();
        assert_eq!(lines.len(), FIELDS.len());
        for (line, prefix) in lines.iter().zip(FIELDS) {
            assert!(line.starts_with(prefix), "unexpected field: {line}");
        }

        // Env-blind: the focused suite runs with BRAIN_R8_INSPECT_PROBE
        // planted on the command line; its canary value must never appear.
        assert!(!first.contains("env-probe-canary-7f3a"));
        assert!(!first.contains("BRAIN_"));

        // Provider identity is NAME only; the loopback fixture has no key
        // material, and the renderer's vocabulary has no slot for one.
        assert!(first.contains("provider: loopback-fixture"));
        assert!(first.contains("token_budget: absent"));
        assert!(first.contains("budget_accounting: absent"));

        // A view without owners renders named absents, never blanks.
        let empty = InspectContext {
            config: None,
            provider_name: None,
        };
        let rendered = inspect(&plan, &empty);
        assert!(rendered.contains("max_turns: absent"));
        assert!(rendered.contains("provider: absent"));
        assert!(rendered.contains("sandbox: unavailable (denied)"));
    }
}
