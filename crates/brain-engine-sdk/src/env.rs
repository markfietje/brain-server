//! Execution environment + tool surface.
//!
//! Invariant: tools never touch the filesystem or spawn processes directly —
//! every effect flows through the [`FsSeam`] injected into their
//! [`ExecutionEnv`], and the default seam denies everything (fail-closed).
//! The registry enforces presentation/lookup/execution alignment: what a
//! model is shown, what a caller can look up, and what will actually run are
//! one and the same set (`restrict()` shape).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// The only path tools have to the outside world. Hosts inject a real seam;
/// the SDK ships [`DenyAll`] as the default.
pub trait FsSeam: Send + Sync {
    fn read(&self, path: &str) -> Result<String, EnvError>;
    fn write(&self, path: &str, content: &str) -> Result<(), EnvError>;
    /// Edit = read-modify-write through the same seam, atomic at the seam.
    fn edit(&self, path: &str, find: &str, replace: &str) -> Result<(), EnvError> {
        let content = self.read(path)?;
        if !content.contains(find) {
            return Err(EnvError::Denied("edit target not found".into()));
        }
        self.write(path, content.replacen(find, replace, 1).as_str())
    }
    fn exec(&self, command: &str) -> Result<String, EnvError>;
}

/// Environment failure vocabulary.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum EnvError {
    /// The capability flag or seam refused the operation.
    Denied(String),
    /// The target does not exist.
    NotFound(String),
    /// Infrastructure failure surfaced verbatim from the host seam.
    Internal(String),
}

impl fmt::Display for EnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvError::Denied(m) => write!(f, "denied: {m}"),
            EnvError::NotFound(m) => write!(f, "not found: {m}"),
            EnvError::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}

impl std::error::Error for EnvError {}

/// The fail-closed default: every capability refuses.
pub struct DenyAll;

impl FsSeam for DenyAll {
    fn read(&self, _: &str) -> Result<String, EnvError> {
        Err(EnvError::Denied("no filesystem seam configured".into()))
    }
    fn write(&self, _: &str, _: &str) -> Result<(), EnvError> {
        Err(EnvError::Denied("no filesystem seam configured".into()))
    }
    fn exec(&self, _: &str) -> Result<String, EnvError> {
        Err(EnvError::Denied("no process seam configured".into()))
    }
}

/// Per-turn execution capabilities. Cloned into each tool invocation so a
/// tool cannot widen its own powers.
#[derive(Clone)]
pub struct ExecutionEnv {
    pub fs: Arc<dyn FsSeam>,
    pub read_only: bool,
    pub allow_process: bool,
    /// Working-directory root; seams enforce it, tools never see absolute
    /// paths outside it by convention.
    pub root: String,
    /// Optional command allowlist consulted before `exec`.
    pub allowed_commands: Vec<String>,
}

impl Default for ExecutionEnv {
    fn default() -> Self {
        ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: Vec::new(),
        }
    }
}

impl ExecutionEnv {
    fn check_path(&self, path: &str) -> Result<(), EnvError> {
        // Path-escape guard: anything leaving the declared root is refused
        // before the seam is ever consulted.
        let normalized = path.trim_start_matches('/');
        if normalized.starts_with("..") || normalized.split('/').any(|seg| seg == "..") {
            return Err(EnvError::Denied(format!("path escapes root: {path}")));
        }
        Ok(())
    }

    pub fn read_file(&self, path: &str) -> Result<String, EnvError> {
        self.check_path(path)?;
        self.fs.read(path)
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<(), EnvError> {
        if self.read_only {
            return Err(EnvError::Denied("environment is read-only".into()));
        }
        self.check_path(path)?;
        self.fs.write(path, content)
    }

    pub fn edit_file(&self, path: &str, find: &str, replace: &str) -> Result<(), EnvError> {
        if self.read_only {
            return Err(EnvError::Denied("environment is read-only".into()));
        }
        self.check_path(path)?;
        self.fs.edit(path, find, replace)
    }

    pub fn exec_command(&self, command: &str) -> Result<String, EnvError> {
        if !self.allow_process {
            return Err(EnvError::Denied("process execution disabled".into()));
        }
        // `prepare` pattern: validate against the allowlist BEFORE exec.
        let head = command.split_whitespace().next().unwrap_or("");
        if !self.allowed_commands.is_empty() && !self.allowed_commands.iter().any(|c| c == head) {
            return Err(EnvError::Denied(format!(
                "command `{head}` not in allowlist"
            )));
        }
        self.fs.exec(command)
    }
}

/// What a tool invocation returns.
pub type ToolResult = Result<String, EnvError>;

type Runner = Arc<dyn Fn(&ExecutionEnv, &str) -> ToolResult + Send + Sync>;

/// A registered tool: name, JSON schema (flows into prompt assembly), and a
/// runner that receives a cloned, narrowed env.
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema_json: String,
    run: Runner,
}

impl Clone for ToolDef {
    fn clone(&self) -> Self {
        ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            schema_json: self.schema_json.clone(),
            run: Arc::clone(&self.run),
        }
    }
}

impl ToolDef {
    pub fn new(
        name: &str,
        description: &str,
        schema_json: &str,
        run: impl Fn(&ExecutionEnv, &str) -> ToolResult + Send + Sync + 'static,
    ) -> Self {
        ToolDef {
            name: name.to_string(),
            description: description.to_string(),
            schema_json: schema_json.to_string(),
            run: Arc::new(run),
        }
    }

    pub fn execute(&self, env: &ExecutionEnv, input_json: &str) -> ToolResult {
        (self.run)(env, input_json)
    }
}

// -- built-in tool factories (the createReadTool/createBashTool shape) ------

/// Read-only file reader.
pub fn create_read_tool() -> ToolDef {
    ToolDef::new(
        "read",
        "read a file",
        r#"{"path":"string"}"#,
        |env, input| env.read_file(input),
    )
}

/// File writer (refused outright under a read-only env).
pub fn create_write_tool() -> ToolDef {
    ToolDef::new(
        "write",
        "write a file",
        r#"{"path":"string","content":"string"}"#,
        |env, input| {
            let (path, content) = split_two(input);
            env.write_file(&path, &content)?;
            Ok(format!("wrote {path}"))
        },
    )
}

/// Single-replacement editor (refused under a read-only env).
pub fn create_edit_tool() -> ToolDef {
    ToolDef::new(
        "edit",
        "edit a file in place",
        r#"{"path":"string","find":"string","replace":"string"}"#,
        |env, input| {
            let mut parts = input.split('\n');
            let (Some(path), Some(find), rest) = (
                parts.next(),
                parts.next(),
                parts.collect::<Vec<_>>().join("\n"),
            ) else {
                return Err(EnvError::Denied("malformed edit input".into()));
            };
            env.edit_file(path, find, &rest)?;
            Ok(format!("edited {path}"))
        },
    )
}

/// Process runner behind the allowlist gate.
pub fn create_bash_tool() -> ToolDef {
    ToolDef::new(
        "bash",
        "run a shell command",
        r#"{"cmd":"string"}"#,
        |env, cmd| env.exec_command(cmd),
    )
}

fn split_two(input: &str) -> (String, String) {
    match input.split_once('\n') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (input.to_string(), String::new()),
    }
}

// -- registry ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ListingMode {
    /// Deferred set empty: additions ride along additively (cache-friendly).
    Additive,
    /// A mid-session addition forced a full re-send of the tool list.
    FullResend { cache_miss: bool },
}

/// Failure vocabulary for registry operations.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RegistryError {
    UnknownTool(String),
    DuplicateTool(String),
    NotPresented(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::UnknownTool(t) => write!(f, "unknown tool `{t}`"),
            RegistryError::DuplicateTool(t) => write!(f, "duplicate tool `{t}`"),
            RegistryError::NotPresented(t) => write!(f, "tool `{t}` not in presented set"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// One owner for tool truth: registration, presentation, lookup, and
/// execution all consult the same map — misalignment is unrepresentable.
#[derive(Default)]
pub struct ToolRegistry {
    active: HashMap<String, ToolDef>,
    deferred: Vec<ToolDef>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register at session start (idle boundary).
    pub fn register(&mut self, def: ToolDef) -> Result<(), RegistryError> {
        if self.active.contains_key(&def.name) {
            return Err(RegistryError::DuplicateTool(def.name));
        }
        self.active.insert(def.name.clone(), def);
        Ok(())
    }

    /// Mid-session addition: queue for additive loading at the next turn
    /// boundary when the model supports it.
    pub fn defer_loading(&mut self, def: ToolDef) {
        self.deferred.push(def);
    }

    /// Promote deferred tools at a turn boundary.
    pub fn promote_deferred(&mut self) -> Vec<String> {
        let names = self.deferred.iter().map(|d| d.name.clone()).collect();
        for def in std::mem::take(&mut self.deferred) {
            self.active.insert(def.name.clone(), def);
        }
        names
    }

    /// Schemas of the presented set, in sorted-name order (deterministic
    /// prompt assembly). When a mid-session addition is pending, the full
    /// list must be re-sent (counted as a cache miss).
    pub fn present_schemas(&self) -> (Vec<(String, String)>, ListingMode) {
        let mut schemas: Vec<(String, String)> = self
            .active
            .values()
            .map(|d| (d.name.clone(), d.schema_json.clone()))
            .collect();
        schemas.sort_by(|a, b| a.0.cmp(&b.0));
        let mode = if self.deferred.is_empty() {
            ListingMode::Additive
        } else {
            ListingMode::FullResend { cache_miss: true }
        };
        (schemas, mode)
    }

    /// Lookup and execution share the same key space; an unknown name fails
    /// closed identically on both paths.
    pub fn execute(&self, name: &str, env: &ExecutionEnv, input_json: &str) -> ToolResult {
        let def = match self.active.get(name) {
            Some(d) => d,
            // Unknown on lookup == unknown on execution: one fail-closed shape.
            None => {
                return Err(EnvError::Denied(format!("unknown tool `{name}`")));
            }
        };
        def.execute(env, input_json)
    }

    /// Alignment pin: presented names == executable names. Any drift is a
    /// bug caught here, not a silent divergence at prompt time.
    pub fn verify_alignment(&self) -> Result<(), RegistryError> {
        let (presented, _) = self.present_schemas();
        let p: HashSet<&str> = presented.iter().map(|(n, _)| n.as_str()).collect();
        let e: HashSet<&str> = self.active.keys().map(|s| s.as_str()).collect();
        if p != e {
            let missing: Vec<&str> = e.difference(&p).copied().collect();
            return Err(RegistryError::NotPresented(missing.join(",")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// In-memory seam for tests: real reads/writes, gated exec.
    #[derive(Default)]
    struct MemFs {
        files: StdMutex<HashMap<String, String>>,
        execs: StdMutex<Vec<String>>,
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
            if let Ok(mut g) = self.execs.lock() {
                g.push(command.into());
            }
            Ok(format!("ran {command}"))
        }
    }

    fn env() -> (ExecutionEnv, Arc<MemFs>) {
        let fs = Arc::new(MemFs::default());
        (
            ExecutionEnv {
                fs: fs.clone(),
                read_only: false,
                allow_process: true,
                root: "/".into(),
                allowed_commands: vec!["ls".into()],
            },
            fs,
        )
    }

    #[test]
    fn deny_all_default_fails_closed() {
        let env = ExecutionEnv::default();
        assert!(matches!(
            create_read_tool().execute(&env, "x"),
            Err(EnvError::Denied(_))
        ));
        assert!(matches!(
            create_bash_tool().execute(&env, "ls"),
            Err(EnvError::Denied(_))
        ));
    }

    #[test]
    fn read_only_refuses_writes_but_allows_reads() {
        let (mut e, fs) = env();
        e.read_only = true;
        fs.write("a.txt", "hi").ok();
        let out = create_read_tool().execute(&e, "a.txt").unwrap();
        assert_eq!(out, "hi");
        assert!(matches!(
            create_write_tool().execute(&e, "a.txt\nbye"),
            Err(EnvError::Denied(_))
        ));
        assert!(matches!(
            create_edit_tool().execute(&e, "a.txt\nhi\nbye"),
            Err(EnvError::Denied(_))
        ));
    }

    #[test]
    fn bash_gated_by_flag_and_allowlist_before_exec() {
        let (mut e, fs) = env();
        e.allow_process = false;
        assert!(matches!(
            create_bash_tool().execute(&e, "ls"),
            Err(EnvError::Denied(_))
        ));
        assert!(fs.execs.lock().map(|g| g.is_empty()).unwrap_or(false));
        e.allow_process = true;
        // Allowlisted command runs...
        assert!(create_bash_tool().execute(&e, "ls -l").is_ok());
        // ...non-allowlisted is refused before the seam.
        assert!(matches!(
            create_bash_tool().execute(&e, "curl evil.example"),
            Err(EnvError::Denied(_))
        ));
        assert!(
            fs.execs
                .lock()
                .map(|g| g.iter().all(|c| c.starts_with("ls")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn path_escape_refused_before_seam() {
        let (e, _fs) = env();
        assert!(matches!(
            e.read_file("../../etc/passwd"),
            Err(EnvError::Denied(_))
        ));
    }

    #[test]
    fn edit_is_read_modify_write_through_one_seam() {
        let (e, fs) = env();
        fs.write("cfg", "alpha\nbeta").ok();
        create_edit_tool().execute(&e, "cfg\nbeta\ngamma").unwrap();
        assert_eq!(fs.read("cfg").unwrap(), "alpha\ngamma");
        // Missing target refused loudly.
        assert!(matches!(
            create_edit_tool().execute(&e, "cfg\nnope\nx"),
            Err(EnvError::Denied(_))
        ));
    }

    #[test]
    fn registry_alignment_and_additive_loading() {
        let mut reg = ToolRegistry::new();
        reg.register(create_read_tool()).unwrap();
        reg.register(create_bash_tool()).unwrap();
        assert!(reg.verify_alignment().is_ok());
        let (schemas, mode) = reg.present_schemas();
        assert_eq!(mode, ListingMode::Additive);
        assert_eq!(schemas.len(), 2);

        reg.defer_loading(create_write_tool());
        // Pending addition forces a full re-send counted as cache miss.
        let (_, mode) = reg.present_schemas();
        assert_eq!(mode, ListingMode::FullResend { cache_miss: true });
        // Not executable until promoted.
        assert!(matches!(
            reg.execute("write", &ExecutionEnv::default(), "a\nb"),
            Err(EnvError::Denied(_))
        ));
        let promoted = reg.promote_deferred();
        assert_eq!(promoted, ["write"]);
        assert!(reg.verify_alignment().is_ok());

        // Unknown tool fails closed on lookup/execution.
        assert!(reg.execute("nope", &ExecutionEnv::default(), "").is_err());
    }

    #[test]
    fn duplicate_registration_fails_loud() {
        let mut reg = ToolRegistry::new();
        reg.register(create_read_tool()).unwrap();
        assert_eq!(
            reg.register(create_read_tool()).unwrap_err(),
            RegistryError::DuplicateTool("read".into())
        );
    }
}
