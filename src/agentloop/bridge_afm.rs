//! The AFM bridge client — the kernel's one egress to `brain-afm`.
//!
//! The bridge is a child process speaking JSONL over stdio (see
//! `shell-macos/Sources/brain-afm/main.swift` for the protocol's other half).
//! This module owns the child, multiplexes turns by request id, and honors
//! the drop-cancel contract the loop's provider seam established: dropping
//! the returned receiver triggers `op:cancel`, which kills the TURN, not the
//! session — the conversation's live state is the bridge's, and it survives.
//!
//! WHY A SEPARATE SEAM FROM `LlmProvider`. The provider trait is STATELESS by
//! contract (each `stream()` receives the whole message list — built for HTTP
//! providers). The AFM session is STATEFUL: the framework has no seed-history
//! API in the shape the loop would need per turn, and the trimmer
//! (`historyTransform`) lives inside the bridge. The chat route therefore
//! drives this client directly with per-turn prompts; a stateless
//! `AfmBridgeProvider` adapter is OWED to the round that puts tools in the
//! chat (LoopDriver adoption), and is not built speculatively here.
//!
//! Process-wide singleton via `chat_bridge()`, following the `screen.rs`
//! CLASSIFIER precedent: a child-process handle is process state, spawned
//! once from `config::afm_bridge_path()`, absent → the chat route answers
//! with a named refusal (fail-open for every other surface of the server).
//!
//! ponytail: no env knobs read inside (the path comes from config at the one
//! spawn site), no keys, no network — the child's only network is Apple's
//! own framework (PCC).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

/// One streamed event from a turn, or its terminal outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeEvent {
    /// An incremental text fragment. `replace` marks a wholesale revision
    /// (the model re-wrote its running answer): the receiver must discard
    /// prior deltas, not append.
    Delta { text: String, replace: bool },
    /// The turn's terminal success.
    Final {
        text: String,
        tier: String,
        turn: i64,
        input_tokens: i64,
        output_tokens: i64,
    },
    /// The turn's terminal failure, by the bridge's named code
    /// (`conversation_does_not_fit` carries `shortfall_tokens`).
    Failed {
        code: String,
        message: Option<String>,
        shortfall_tokens: Option<i64>,
    },
}

/// The measured machine facts, cached by the bridge for its lifetime.
/// Wire names are the SWIFT side's camelCase (the protocol's shape is owned
/// by `brain-afm`; this struct mirrors it, drift surfaces in the pins).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeCapabilities {
    pub window: i64,
    pub budget: Option<i64>,
    pub languages: Vec<String>,
    pub pcc_window: Option<i64>,
    #[serde(default)]
    pub pcc_eligible: bool,
    pub pcc_generation: String,
    #[serde(default)]
    pub escalation_permitted: bool,
}

/// One turn request. `instructions` reaches the bridge only on a session's
/// FIRST turn (the bridge refuses a changed instruction later — a prefix
/// cache is invalidated by an instruction change, so merging would be a lie).
#[derive(Debug, Clone)]
pub struct BridgeTurn {
    pub session: String,
    pub prompt: String,
    pub instructions: Option<String>,
    pub max_response_tokens: Option<i64>,
}

/// Client-side failure taxonomy: transport-shaped, never the bridge's named
/// turn codes (those arrive as [`BridgeEvent::Failed`]).
#[derive(Debug)]
pub enum BridgeError {
    Unavailable(String),
    Protocol(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Unavailable(detail) => {
                write!(f, "the AFM bridge is not available: {detail}")
            }
            BridgeError::Protocol(detail) => {
                write!(f, "the AFM bridge protocol broke: {detail}")
            }
        }
    }
}

impl std::error::Error for BridgeError {}

/// The live child + the pending-turn table.
pub struct AfmBridge {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<BridgeEvent>>>>,
    capabilities: OnceLock<Arc<BridgeCapabilities>>,
}

impl AfmBridge {
    /// Spawn the child and start its reader task. The reader is the ONLY
    /// consumer of stdout; every line routes by id to the registered sender,
    /// and a dead child fails all pending turns at once.
    pub fn spawn(path: &Path) -> Result<Arc<Self>, BridgeError> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| BridgeError::Unavailable(format!("spawn {path:?}: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BridgeError::Protocol("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::Protocol("no stdout".into()))?;

        let pending: Arc<Mutex<HashMap<String, mpsc::Sender<BridgeEvent>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Some(value) = serde_json::from_str::<serde_json::Value>(&line).ok() else {
                    continue;
                };
                let id = value.get("id").and_then(|v| v.as_str()).map(str::to_owned);
                route_line(&reader_pending, id, value);
            }
            // Child exit: every pending turn fails at once, by name — a
            // caller holding a stream must learn the bridge died, not hang
            // until the router's timeout.
            fail_all_pending(
                &reader_pending,
                "bridge_exited",
                "the bridge process exited",
            );
        });

        Ok(Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            capabilities: OnceLock::new(),
        }))
    }

    /// The measured capabilities, requested once and cached for the child's
    /// lifetime (the bridge caches its own side identically).
    pub async fn capabilities(&self) -> Result<Arc<BridgeCapabilities>, BridgeError> {
        if let Some(cached) = self.capabilities.get() {
            return Ok(Arc::clone(cached));
        }
        let (tx, mut rx) = mpsc::channel::<BridgeEvent>(16);
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut pending = self.pending.lock().expect("bridge pending lock");
            pending.insert(id.clone(), tx);
        }
        self.write_line(
            &json!({ "id": id, "op": "capabilities" }),
        )?;
        // Deltas never precede a capabilities reply, but the reader speaks
        // one event type; treat Final-ish payloads generically below.
        while let Some(event) = rx.recv().await {
            match event {
                BridgeEvent::Failed { code, message, .. } => {
                    self.pending.lock().expect("bridge pending lock").remove(&id);
                    return Err(BridgeError::Protocol(format!(
                        "capabilities failed: {code} {message:?}"
                    )));
                }
                BridgeEvent::Delta { .. } => continue,
                BridgeEvent::Final { text, .. } => {
                    let caps: BridgeCapabilities = serde_json::from_str(&text)
                        .map_err(|e| BridgeError::Protocol(format!("capabilities body: {e}")))?;
                    let caps = Arc::new(caps);
                    let _ = self.capabilities.set(Arc::clone(&caps));
                    self.pending.lock().expect("bridge pending lock").remove(&id);
                    return Ok(caps);
                }
            }
        }
        Err(BridgeError::Unavailable(
            "the bridge closed before answering capabilities".into(),
        ))
    }

    /// Begin one streamed turn. DROPPING the receiver cancels the turn (the
    /// bridge abandons its task; the session survives).
    pub async fn turn(
        &self,
        request: BridgeTurn,
    ) -> Result<(String, mpsc::Receiver<BridgeEvent>), BridgeError> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel::<BridgeEvent>(64);
        self.pending.lock().expect("bridge pending lock").insert(id.clone(), tx);
        let mut body = json!({
            "id": id,
            "op": "turn",
            "session": request.session,
            "prompt": request.prompt,
        });
        if let Some(instructions) = &request.instructions {
            body["instructions"] = json!(instructions);
        }
        if let Some(cap) = request.max_response_tokens {
            body["maxResponseTokens"] = json!(cap);
        }
        if self.write_line(&body).is_err() {
            self.pending.lock().expect("bridge pending lock").remove(&id);
            return Err(BridgeError::Unavailable(
                "writing to the bridge failed — the child is likely dead".into(),
            ));
        }
        Ok((id, rx))
    }

    /// Cancel an in-flight turn by its SSE-facing request id. The bridge
    /// treats an unknown id as a no-op ("already done" == "never existed").
    pub fn cancel(&self, bridge_id: &str) {
        let _ = self.write_line(&json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "op": "cancel",
            "target": bridge_id,
        }));
    }

    fn write_line(&self, value: &serde_json::Value) -> Result<(), BridgeError> {
        let mut stdin = self.stdin.lock().expect("bridge stdin lock");
        let write = serde_json::to_writer(&mut *stdin, value)
            .map_err(|e| BridgeError::Unavailable(format!("bridge stdin: {e}")))
            .and_then(|()| {
                stdin.write_all(b"\n").map_err(|e| {
                    BridgeError::Unavailable(format!("bridge stdin: {e}"))
                })
            })
            .and_then(|()| {
                stdin.flush().map_err(|e| {
                    BridgeError::Unavailable(format!("bridge stdin: {e}"))
                })
            });
        write
    }

    /// Whether the child is still alive (best-effort; the reader's
    /// fail-all-pending is the authoritative signal).
    #[allow(dead_code)]
    pub fn alive(&self) -> bool {
        self.child
            .lock()
            .expect("bridge child lock")
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false)
    }
}

/// Route one parsed stdout line to its pending sender (or cache capabilities
/// in-flight via a synthetic Final — see `capabilities()`).
fn route_line(
    pending: &Arc<Mutex<HashMap<String, mpsc::Sender<BridgeEvent>>>>,
    id: Option<String>,
    value: serde_json::Value,
) {
    // The bridge's capabilities reply carries the payload as top-level keys,
    // not as a Final event — normalize it here so `capabilities()` can treat
    // one event shape.
    let event = if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        BridgeEvent::Final {
            text: value.to_string(),
            tier: "capabilities".into(),
            turn: 0,
            input_tokens: 0,
            output_tokens: 0,
        }
    } else {
        match value.get("event").and_then(|v| v.as_str()) {
            Some("delta") => BridgeEvent::Delta {
                text: value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                replace: value
                    .get("replace")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            Some("final") => BridgeEvent::Final {
                text: value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                tier: value
                    .get("tier")
                    .and_then(|v| v.as_str())
                    .unwrap_or("on_device")
                    .to_owned(),
                turn: value.get("turn").and_then(|v| v.as_i64()).unwrap_or(0),
                input_tokens: value
                    .get("inputTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                output_tokens: value
                    .get("outputTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            },
            Some("error") => BridgeEvent::Failed {
                code: value
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned(),
                message: value
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                shortfall_tokens: value.get("shortfallTokens").and_then(|v| v.as_i64()),
            },
            _ => return,
        }
    };

    let terminal = !matches!(event, BridgeEvent::Delta { .. });
    let Some(id) = id else { return };
    let mut pending = pending.lock().expect("bridge pending lock");
    if let Some(tx) = pending.get(&id) {
        let _ = tx.try_send(event);
    }
    if terminal {
        pending.remove(&id);
    }
}

fn fail_all_pending(pending: &Arc<Mutex<HashMap<String, mpsc::Sender<BridgeEvent>>>>, code: &str, message: &str) {
    let mut pending = pending.lock().expect("bridge pending lock");
    for (_, tx) in pending.drain() {
        let _ = tx.try_send(BridgeEvent::Failed {
            code: code.to_owned(),
            message: Some(message.to_owned()),
            shortfall_tokens: None,
        });
    }
}

/// The process-wide chat bridge, spawned once from the configured path.
/// `None` means "no bridge binary here" — the chat route's named refusal,
/// never a crash: every other surface of the server is unaffected.
static CHAT_BRIDGE: OnceLock<Option<Arc<AfmBridge>>> = OnceLock::new();

pub fn chat_bridge() -> Option<Arc<AfmBridge>> {
    CHAT_BRIDGE
        .get_or_init(|| match crate::config::afm_bridge_path() {
            Some(path) if path.is_file() => AfmBridge::spawn(&path)
                .map_err(|e| {
                    tracing::warn!("chat bridge spawn failed ({e}); chat turns will refuse by name");
                    e
                })
                .ok(),
            _ => {
                tracing::info!(
                    "no AFM bridge binary; chat turns answer chat_bridge_unavailable"
                );
                None
            }
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned protocol: drive the REAL bridge binary when it is built
    /// (the suite's own build produces it), asserting the wire contract the
    /// handler depends on. Skipped-with-prejudice when the binary is absent
    /// — the shell-macos suite owns the binary's own pins.
    #[tokio::test]
    async fn the_real_bridge_answers_capabilities_and_a_refusal_by_name() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            repo_root.join("shell-macos/.build/out/Products/Debug/brain-afm"),
            repo_root.join("shell-macos/.build/arm64-apple-darwin/debug/brain-afm"),
        ];
        let Some(binary) = candidates.iter().find(|p| p.is_file()) else {
            eprintln!("brain-afm not built; run `swift build` in shell-macos — SKIPPING (owned by the Swift suite)");
            return;
        };
        let bridge = AfmBridge::spawn(binary).expect("spawn the real bridge");
        let caps = bridge.capabilities().await.expect("capabilities");
        assert_eq!(caps.window, 4096, "the on-device window is the measured 4096");
        assert_eq!(
            caps.pcc_window,
            Some(32768),
            "the PCC window reads back 32,768 even while generation is refused"
        );
        // A turn BEFORE capabilities on a FRESH bridge child is refused by
        // name — spawn a second child to prove the ordering law.
        let fresh = AfmBridge::spawn(binary).expect("spawn fresh");
        let (_bridge_id, mut rx) = fresh
            .turn(BridgeTurn {
                session: "s".into(),
                prompt: "hi".into(),
                instructions: None,
                max_response_tokens: None,
            })
            .await
            .expect("turn begins");
        match rx.recv().await {
            Some(BridgeEvent::Failed { code, .. }) => {
                assert_eq!(code, "bad_request", "the refusal is named");
            }
            other => panic!("expected the named refusal, got {other:?}"),
        }
    }
}
