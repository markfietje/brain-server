//! The AFM-stdio provider — the GDL loop's transport over the bridge.
//!
//! D-P1 (the pilot round's transport decision): the GDL provider seam gains
//! an `afm-stdio` kind BESIDE the HTTP one, and the loop's propose calls
//! ride [`bridge_afm`] — the kernel's ONE egress to `brain-afm`, the chat
//! spine's own client — translated into the bridge's JSONL-over-stdio
//! protocol. The alternatives were DECLINED with reasons: a Swift-side HTTP
//! server would add a network listener to a process whose transport is
//! deliberately stdio (a new egress surface, the loopback law churned); a
//! hosted OpenAI-compatible provider moves case data off the machine (the
//! local-first posture refuses it for the pilot). One provider kind, one
//! transport, zero new egress — this module speaks to a child it never
//! spawns (the shared `chat_bridge()` owns the one spawn site) and opens no
//! socket of its own.
//!
//! THE STATELESS→STATEFUL BRIDGE. The [`LlmProvider`] trait is stateless
//! (each `stream()` receives the whole message list); the bridge's session
//! is stateful (instructions lock to a session's first turn). An exchange is
//! therefore a FRESH bridge session whose single turn carries the whole
//! conversation: the system prompt rides `instructions`, the rendered
//! message list rides `prompt`, and no cross-exchange state survives — the
//! trait's contract, kept by construction.
//!
//! WHY DELTAS ARE NOT FORWARDED. The child's guided generation may REVISE
//! its running answer (a `replace` delta — the child marks it), and the loop
//! assembles assistant text by concatenating `TextDelta`s; forwarding a
//! revision would fabricate text the model never said. The final event's
//! SETTLED answer is the only text this seam vouches for, so it crosses as
//! one `TextDelta` before `MessageEnd`. Streaming fidelity is worthless to
//! a headless propose loop; correctness of the assembled artifact is the
//! whole point.
//!
//! ponytail: no env knobs (the kind comes from config at the construction
//! seam), no keys (the child needs none), no network — the child's only
//! network is Apple's own framework (PCC), refused on every machine
//! measured so far.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::bridge_afm::{AfmBridge, BridgeError, BridgeEvent, BridgeTurn};
use super::provider::{
    ChatMessage, LlmProvider, ProviderError, ProviderRequest, STREAM_CHANNEL_CAP, StopReason,
    StreamEvent, Usage,
};

/// The response-token cap sent with every turn — the child's own clamp
/// ceiling (`min(max(maxResponse ?? 512, 16), 1024)` in the child's main).
/// GDL artifacts are JSON documents; the pilot asks for the largest answer
/// the child permits and lets the loop's own gates judge what arrives.
pub(crate) const AFM_RESPONSE_TOKEN_CAP: i64 = 1024;

/// The loop-side rail against a wedged child. The chat spine can stream
/// without a deadline (a human watches the SSE stream), but the GDL loop's
/// caller is a synchronous POST — an unbounded exchange would hold it
/// forever. 120 s is far above any on-device artifact generation the chat
/// spine has produced; the pilot's latency measurement (PILOT_EVIDENCE)
/// records the actual maximum against this headroom.
pub(crate) const AFM_EXCHANGE_DEADLINE: Duration = Duration::from_secs(120);

/// The provider: the shared bridge client plus nothing else. Construction
/// requests capabilities (the child refuses any turn that precedes them —
/// `bad_request` by name), which is also the one honest launch-time check
/// that the child is alive and answering.
pub(crate) struct AfmBridgeProvider {
    bridge: Arc<AfmBridge>,
}

impl AfmBridgeProvider {
    /// Construct over the shared bridge. The capabilities request is the
    /// ordering law made executable: after it returns, every later `turn`
    /// on this child is legal.
    pub(crate) async fn for_bridge(bridge: Arc<AfmBridge>) -> Result<Self, BridgeError> {
        bridge.capabilities().await?;
        Ok(Self { bridge })
    }
}

/// Render the loop's conversation into ONE bridge prompt: roles labeled,
/// in order, deterministic. Every [`ChatMessage`] variant renders — the
/// GDL propose path carries only `User`/`Assistant` (the driver's tool
/// registry is empty by law), but a renderer that silently skips a variant
/// would be a renderer that lies about what the model saw.
pub(crate) fn render_conversation(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        match message {
            ChatMessage::User { text } => {
                out.push_str("[user]\n");
                out.push_str(text);
            }
            ChatMessage::Assistant { text, tool_calls } => {
                out.push_str("[assistant]\n");
                out.push_str(text);
                for call in tool_calls {
                    out.push_str("\n[tool_call ");
                    out.push_str(&call.name);
                    out.push(' ');
                    out.push_str(&call.arguments_json);
                    out.push(']');
                }
            }
            ChatMessage::ToolResult {
                name,
                status,
                output,
                ..
            } => {
                out.push_str("[tool_result ");
                out.push_str(name);
                out.push(' ');
                out.push_str(match status {
                    super::provider::ToolResultStatus::Success => "success",
                    super::provider::ToolResultStatus::Failure => "failure",
                });
                out.push_str("]\n");
                out.push_str(output);
            }
            ChatMessage::Delegation {
                name,
                outcome,
                summary,
                ..
            } => {
                out.push_str("[delegation ");
                out.push_str(name);
                out.push(' ');
                out.push_str(match outcome {
                    super::provider::DelegationOutcome::Completed => "completed",
                    super::provider::DelegationOutcome::BudgetExceeded => "budget_exceeded",
                    super::provider::DelegationOutcome::Capped => "capped",
                    super::provider::DelegationOutcome::Canceled => "canceled",
                });
                out.push_str("]\n");
                out.push_str(summary);
            }
            ChatMessage::Summary { text } => {
                out.push_str("[summary]\n");
                out.push_str(text);
            }
        }
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

impl LlmProvider for AfmBridgeProvider {
    fn name(&self) -> &str {
        "afm-stdio"
    }

    fn stream(
        &self,
        req: ProviderRequest,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, ProviderError>>, ProviderError> {
        // The child has no tool-calling surface (the bridge's own header:
        // "no tools"). A caller handing this transport a tool schema would
        // get a model that can never call it — a capability lie — so a
        // non-empty registry refuses here, loudly, before any turn.
        if !req.tools.is_empty() {
            return Err(ProviderError::Refused);
        }
        let prompt = render_conversation(&req.messages);
        let bridge = Arc::clone(&self.bridge);
        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ProviderError>>(STREAM_CHANNEL_CAP);
        tokio::spawn(async move {
            // One exchange = one fresh bridge session; instructions ride the
            // session's first (and only) turn, so the child's instructions
            // lock is trivially satisfied.
            let turn = BridgeTurn {
                session: format!("gdl-{}", uuid::Uuid::new_v4()),
                prompt,
                instructions: Some(req.system_prompt),
                max_response_tokens: Some(AFM_RESPONSE_TOKEN_CAP),
            };
            let (bridge_id, mut bridge_rx) = match bridge.turn(turn).await {
                Ok(pair) => pair,
                Err(_) => {
                    let _ = tx.send(Err(ProviderError::Unavailable)).await;
                    return;
                }
            };
            if tx.send(Ok(StreamEvent::MessageStart)).await.is_err() {
                bridge.cancel(&bridge_id);
                return;
            }
            let deadline = tokio::time::Instant::now() + AFM_EXCHANGE_DEADLINE;
            loop {
                let event = match tokio::time::timeout_at(deadline, bridge_rx.recv()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        // The child exited mid-turn: a named failure, never
                        // a hang (the reader's fail-all-pending usually
                        // delivers this first as Failed).
                        let _ = tx.send(Err(ProviderError::Unavailable)).await;
                        return;
                    }
                    Err(_elapsed) => {
                        bridge.cancel(&bridge_id);
                        let _ = tx.send(Err(ProviderError::Timeout)).await;
                        return;
                    }
                };
                match event {
                    BridgeEvent::Delta { .. } => {
                        // Not forwarded (the header's revision law); each
                        // delta only proves the child is alive. A loop that
                        // dropped its receiver cancels the TURN here.
                        if tx.is_closed() {
                            bridge.cancel(&bridge_id);
                            return;
                        }
                    }
                    BridgeEvent::Final {
                        text,
                        input_tokens,
                        output_tokens,
                        ..
                    } => {
                        // The settled answer crosses as ONE delta; an
                        // abstention is the child's typed "no answer" and
                        // crosses as an empty one — the gate parses an empty
                        // artifact by its own named error, the honest path
                        // into the retry ladder. The final's claims ride the
                        // chat spine's grounding law, not the loop's gates,
                        // and are deliberately ignored here.
                        let usage = Usage {
                            input_tokens: input_tokens.max(0) as u64,
                            output_tokens: output_tokens.max(0) as u64,
                        };
                        let _ = tx.send(Ok(StreamEvent::TextDelta(text))).await;
                        let _ = tx
                            .send(Ok(StreamEvent::MessageEnd {
                                stop_reason: StopReason::EndTurn,
                                usage,
                            }))
                            .await;
                        return;
                    }
                    BridgeEvent::Failed { code, .. } => {
                        // The child's named codes map into the closed
                        // provider-failure classes: an over-window
                        // conversation is the child refusing the request
                        // (Refused); a dead/absent on-device model is
                        // Unavailable; the rest are refusals too — every
                        // name crosses as a class, never as prose.
                        let mapped = match code.as_str() {
                            "model_unavailable" => ProviderError::Unavailable,
                            _ => ProviderError::Refused,
                        };
                        let _ = tx.send(Err(mapped)).await;
                        return;
                    }
                }
            }
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::agentloop::provider::{DelegationOutcome, ToolResultStatus};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn the_conversation_renders_role_labeled_and_deterministic() {
        let messages = vec![
            ChatMessage::User {
                text: "produce the intake artifact".into(),
            },
            ChatMessage::Assistant {
                text: "{\"is_not\":{}}".into(),
                tool_calls: vec![],
            },
            ChatMessage::User {
                text: "now the triage artifact".into(),
            },
        ];
        assert_eq!(
            render_conversation(&messages),
            "[user]\nproduce the intake artifact\n\n[assistant]\n{\"is_not\":{}}\n\n\
             [user]\nnow the triage artifact"
        );
        // Every variant renders — none is silently skipped (a skipped
        // variant is a lie about what the model saw).
        let every = vec![
            ChatMessage::User { text: "u".into() },
            ChatMessage::Assistant {
                text: "a".into(),
                tool_calls: vec![super::super::provider::ContextToolCall {
                    original_id: "o".into(),
                    id: "c".into(),
                    name: "read".into(),
                    arguments_json: "{}".into(),
                }],
            },
            ChatMessage::ToolResult {
                call_id: "c".into(),
                original_id: "o".into(),
                name: "read".into(),
                status: ToolResultStatus::Failure,
                output: "boom".into(),
                truncated: false,
            },
            ChatMessage::Delegation {
                exchange_id: 1,
                name: "child".into(),
                outcome: DelegationOutcome::Capped,
                summary: "partial".into(),
            },
            ChatMessage::Summary { text: "s".into() },
        ];
        let rendered = render_conversation(&every);
        for fragment in [
            "[user]\nu",
            "[assistant]\na\n[tool_call read {}]",
            "[tool_result read failure]\nboom",
            "[delegation child capped]\npartial",
            "[summary]\ns",
        ] {
            assert!(
                rendered.contains(fragment),
                "the renderer dropped a variant: missing `{fragment}` in\n{rendered}"
            );
        }
    }

    /// THE SOURCE-SCAN PIN (structural): the afm-stdio path opens no socket
    /// and spawns no process of its own. The scanned text is this file with
    /// comment lines stripped — the law is about CODE, and this pin's own
    /// prose must not be able to green or red it.
    #[test]
    fn the_afm_stdio_path_has_no_egress_beyond_the_shared_bridge() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/agentloop/provider_afm.rs"
        ))
        .expect("this module is in the tree");
        let code: String = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !(trimmed.starts_with("//") || trimmed.starts_with("*"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The tokens are assembled from fragments so this scan cannot hit
        // its own literal list — the law is about the file's CODE, and the
        // pin's fixture must not be able to green or red it.
        let banned = [
            ["req", "west"].concat(),
            ["Tcp", "Stream"].concat(),
            ["hy", "per"].concat(),
            ["Http", "Provider"].concat(),
            ["Command::", "new"].concat(),
        ];
        for banned in banned {
            assert!(
                !code.contains(banned.as_str()),
                "the afm-stdio path must not contain `{banned}` — its only egress is the shared bridge's stdio"
            );
        }
        assert!(
            code.contains("bridge_afm::") || code.contains("AfmBridge"),
            "the transport must ride the shared bridge client, not its own child"
        );
    }

    /// THE MIRROR PIN (the A-3/LL-04 drift law, the adapter's half): the
    /// child's protocol facts this adapter DEPENDS ON are pinned against the
    /// Swift source — the clamp the response cap rides, the
    /// capabilities-before-turn ordering the construction enforces, the
    /// instructions lock the fresh-session design satisfies, and the named
    /// error codes the failure mapping reads. A Swift change to any of them
    /// reds THIS pin, in THIS lane.
    #[test]
    fn the_child_protocol_the_adapter_relies_on_is_the_documented_one() {
        let child = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shell-macos/Sources/brain-afm/main.swift"
        ))
        .expect("the bridge child is in the tree");
        for fact in [
            // AFM_RESPONSE_TOKEN_CAP (1024) is the child's clamp ceiling.
            "min(max(maxResponse ?? 512, 16), 1024)",
            // for_bridge's capabilities request is the ordering law.
            "capabilities must be requested before the first turn",
            // The fresh-session design satisfies the instructions lock.
            "instructionsLocked",
            // The failure mapping's named codes.
            "conversation_does_not_fit",
            "model_unavailable",
            "generation_failed",
            "bad_request",
        ] {
            assert!(
                child.contains(fact),
                "the child's protocol drifted from the adapter: missing `{fact}`"
            );
        }
    }

    /// THE REAL-CHILD PIN (behavioral, the C2 rules-door posture): the
    /// afm-stdio kind drives the REAL brain-afm binary end to end —
    /// capabilities, one propose turn, the settled answer as StreamEvents
    /// with real usage. Skipped-with-prejudice when the binary is absent —
    /// the shell-macos suite owns the binary's own pins.
    #[tokio::test]
    async fn the_afm_stdio_kind_drives_the_real_child() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            repo_root.join("shell-macos/.build/out/Products/Debug/brain-afm"),
            repo_root.join("shell-macos/.build/arm64-apple-darwin/debug/brain-afm"),
        ];
        let Some(binary) = candidates.iter().find(|p| p.is_file()) else {
            eprintln!(
                "brain-afm not built; run `swift build` in shell-macos — SKIPPING (owned by the Swift suite)"
            );
            return;
        };
        let bridge = AfmBridge::spawn(binary).expect("spawn the real bridge");
        let provider = AfmBridgeProvider::for_bridge(bridge)
            .await
            .expect("the real child answers capabilities");
        let mut rx = provider
            .stream(ProviderRequest {
                system_prompt:
                    "You are a strict artifact producer. Answer with exactly the requested token."
                        .into(),
                messages: vec![ChatMessage::User {
                    text: "Reply with exactly: ok".into(),
                }],
                tools: vec![],
            })
            .expect("the stream begins");
        let mut started = false;
        let mut text = String::new();
        let mut ended = None;
        while let Some(event) = rx.recv().await {
            match event.expect("no mid-stream failure on the happy turn") {
                StreamEvent::MessageStart => started = true,
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::MessageEnd { stop_reason, usage } => {
                    ended = Some((stop_reason, usage));
                    break;
                }
                StreamEvent::ToolCallDelta { .. } => panic!("the bridge produced no tool calls"),
            }
        }
        assert!(started, "MessageStart precedes everything");
        let (stop_reason, usage) = ended.expect("the exchange ended with MessageEnd");
        assert_eq!(stop_reason, StopReason::EndTurn);
        assert!(!text.trim().is_empty(), "the settled answer is non-empty");
        assert!(
            usage.output_tokens > 0,
            "a real generation reports real output tokens, got {usage:?}"
        );
    }

    /// A non-empty tool registry refuses BEFORE any turn: the child has no
    /// tool surface, and a silently-ignored registry would be a capability
    /// lie. (Red-proof: remove the refusal, the pin reds.)
    #[test]
    fn a_tool_carrying_request_refuses_by_name() {
        let rt = rt();
        rt.block_on(async {
            let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let candidates = [
                repo_root.join("shell-macos/.build/out/Products/Debug/brain-afm"),
                repo_root.join("shell-macos/.build/arm64-apple-darwin/debug/brain-afm"),
            ];
            let Some(binary) = candidates.iter().find(|p| p.is_file()) else {
                eprintln!("brain-afm not built — SKIPPING (owned by the Swift suite)");
                return;
            };
            let bridge = AfmBridge::spawn(binary).expect("spawn the real bridge");
            let provider = AfmBridgeProvider::for_bridge(bridge).await.expect("caps");
            let refused = provider.stream(ProviderRequest {
                system_prompt: "s".into(),
                messages: vec![ChatMessage::User { text: "hi".into() }],
                tools: vec![super::super::provider::ToolSpec {
                    name: "read".into(),
                    description: "read".into(),
                    schema_json: "{}".into(),
                }],
            });
            assert!(
                matches!(refused, Err(ProviderError::Refused)),
                "a tool-carrying request is refused before any child turn"
            );
        });
    }
}
