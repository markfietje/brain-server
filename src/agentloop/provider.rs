//! The provider/stream seam — the loop's ONE egress to a model.
//!
//! The trait is kernel-side and async-runtime-shaped (the SDK stays
//! dependency-free); the message vocabulary is value-typed and
//! provider-neutral so no vendor SDK leaks into loop logic. Streaming is
//! channel-based: [`LlmProvider::stream`] hands back a receiver, the
//! provider's sender task owns the egress, and DROPPING THE RECEIVER IS THE
//! CANCEL — no detached producer can outlive a cancelled turn (the .69
//! TimeoutLayer lesson, applied at the seam instead of a middleware).
//!
//! What this module deliberately does NOT do: no real provider ships here
//! (zero provider code in-tree is the law of this seam — keys, egress,
//! per-provider caps belong to the provider line that follows), no retries
//! (a failed stream is the caller's typed error, never an in-seam retry
//! loop), no env knobs (provider selection is constructor injection;
//! `BRAIN_*` wiring arrives with the provider line so env-truth never
//! carries a dead name).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// Bound on the stream channel (bounds law): a provider cannot buffer
/// unbounded deltas ahead of a slow consumer — backpressure starts here.
pub(crate) const STREAM_CHANNEL_CAP: usize = 64;

/// Chat role. Two-value, provider-neutral; system text rides the request
/// struct, tool schemas ride the request struct — neither is a "message".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Assistant,
}

impl Role {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One conversational message (input history or a prior assistant turn).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChatMessage {
    pub role: Role,
    pub text: String,
}

/// A tool as presented to the model: name + description + JSON schema.
/// Presentation/lookup/execution alignment is the SDK registry's law; this
/// is only the wire shape the prompt assembler emits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema_json: String,
}

/// A streamed completion request. Value-typed end to end: the system prompt
/// is a cache-stable prefix by construction (deterministic assembly lives in
/// the loop), tools ride at the end, and nothing in here is provider-native.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProviderRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
}

/// Token accounting for one assistant message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub(crate) fn total(self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// A completed tool invocation request inside a stream.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

/// Why a message ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// No tool calls — the conversational end of a turn.
    EndTurn,
    /// The model asked for tools; the loop executes and continues.
    ToolUse,
    /// A budget/cap cut the message short.
    MaxTokens,
    /// The provider refused mid-message (content policy, auth).
    Refused,
}

/// One streamed event, in arrival order. Tool-call arguments arrive as
/// concatenated deltas; the loop assembles them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StreamEvent {
    MessageStart,
    TextDelta(String),
    ToolCallDelta {
        id: String,
        name: String,
        arguments_delta: String,
    },
    MessageEnd {
        stop_reason: StopReason,
        usage: Usage,
    },
}

/// Provider failure vocabulary. Loud and typed; the loop decides policy.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) enum ProviderError {
    /// Egress unavailable (transport, rate limit, no script left). Retryable
    /// only by the CALLER's policy, never silently in the seam.
    Unavailable(String),
    /// The provider refused the request outright (policy, moderation, auth).
    Refused(String),
    /// The consumer dropped the stream — producer-side acknowledgement of
    /// the drop-cancel contract.
    Cancelled,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unavailable(m) => write!(f, "provider unavailable: {m}"),
            ProviderError::Refused(m) => write!(f, "provider refused: {m}"),
            ProviderError::Cancelled => write!(f, "provider stream cancelled"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// The seam itself. One implementation per provider; the loop accepts an
/// `Arc<dyn LlmProvider>` and knows nothing else about egress.
pub(crate) trait LlmProvider: Send + Sync {
    /// Stable label for audit rows and telemetry.
    fn name(&self) -> &str;

    /// Begin one streamed completion. Events arrive on the returned receiver
    /// in order; the sender task ends when the script/exchange ends or the
    /// receiver drops (the cancel contract). Errors HERE mean the exchange
    /// never started; errors ON the channel are mid-stream failures.
    fn stream(
        &self,
        req: ProviderRequest,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, ProviderError>>, ProviderError>;
}

// -- the loopback fixture (test-only carrier; never a production provider) --

/// Scripted-turn provider: each `stream()` call pops one scripted turn and
/// replays it. Deterministic, zero egress, records every request so tests
/// can assert exactly what the loop asked for. Asking for more turns than
/// the script holds is a typed [`ProviderError::Unavailable`] — a loop that
/// over-runs its scenario fails loud, never silently idles.
pub(crate) struct LoopbackProvider {
    label: String,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
    requests: Mutex<Vec<ProviderRequest>>,
    /// Live sender tasks (incremented on stream, decremented on task exit) —
    /// the observable end of the drop-cancel contract.
    live_senders: Arc<AtomicUsize>,
}

impl LoopbackProvider {
    pub(crate) fn new(label: &str, script: Vec<Vec<StreamEvent>>) -> Arc<Self> {
        Arc::new(LoopbackProvider {
            label: label.to_string(),
            turns: Mutex::new(VecDeque::from(script)),
            requests: Mutex::new(Vec::new()),
            live_senders: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Every request the loop made, in order (test assertions).
    pub(crate) fn requests(&self) -> Vec<ProviderRequest> {
        self.requests.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Sender tasks still alive; 0 means every stream ended.
    pub(crate) fn live_senders(&self) -> usize {
        self.live_senders.load(Ordering::SeqCst)
    }
}

impl LlmProvider for LoopbackProvider {
    fn name(&self) -> &str {
        &self.label
    }

    fn stream(
        &self,
        req: ProviderRequest,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, ProviderError>>, ProviderError> {
        let turn = {
            let mut turns = self
                .turns
                .lock()
                .map_err(|_| ProviderError::Unavailable("loopback script lock poisoned".into()))?;
            turns
                .pop_front()
                .ok_or_else(|| ProviderError::Unavailable("loopback script exhausted".into()))?
        };
        if let Ok(mut g) = self.requests.lock() {
            g.push(req);
        }
        let (tx, rx) = mpsc::channel(STREAM_CHANNEL_CAP);
        let live = Arc::clone(&self.live_senders);
        live.fetch_add(1, Ordering::SeqCst);
        // The sender task is the egress stand-in: it ends when the script
        // drains, OR when the receiver drops (send fails) — either way the
        // decrement guard runs, so live_senders() is the cancel observable.
        tokio::spawn(async move {
            for event in turn {
                if tx.send(Ok(event)).await.is_err() {
                    break;
                }
            }
            live.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(rx)
    }
}

// -- script builders (shared by every loopback-driven test in this line) ----

/// A plain text turn that ends the exchange (no tool calls).
pub(crate) fn scripted_text(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart,
        StreamEvent::TextDelta(text.to_string()),
        StreamEvent::MessageEnd {
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 4,
            },
        },
    ]
}

/// A turn that asks for exactly one tool call.
pub(crate) fn scripted_tool_call(id: &str, name: &str, arguments_json: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart,
        StreamEvent::ToolCallDelta {
            id: id.to_string(),
            name: name.to_string(),
            arguments_delta: arguments_json.to_string(),
        },
        StreamEvent::MessageEnd {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
            },
        },
    ]
}

/// A turn that emits text AND asks for one tool call.
pub(crate) fn scripted_text_then_tool(
    text: &str,
    id: &str,
    name: &str,
    arguments_json: &str,
) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart,
        StreamEvent::TextDelta(text.to_string()),
        StreamEvent::ToolCallDelta {
            id: id.to_string(),
            name: name.to_string(),
            arguments_delta: arguments_json.to_string(),
        },
        StreamEvent::MessageEnd {
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 6,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn loopback_streams_scripted_turn_in_order() {
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("hello")]);
        let rt = rt();
        rt.block_on(async {
            let mut rx = provider
                .stream(ProviderRequest {
                    system_prompt: "s".into(),
                    messages: vec![],
                    tools: vec![],
                })
                .unwrap();
            let first = rx.recv().await.unwrap().unwrap();
            assert_eq!(first, StreamEvent::MessageStart);
            let second = rx.recv().await.unwrap().unwrap();
            assert_eq!(second, StreamEvent::TextDelta("hello".into()));
            let last = rx.recv().await.unwrap().unwrap();
            assert_eq!(
                last,
                StreamEvent::MessageEnd {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 4
                    }
                }
            );
            // Script drained: channel closes, no phantom events.
            assert!(rx.recv().await.is_none());
        });
    }

    #[test]
    fn script_exhaustion_is_unavailable_never_silent() {
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("once")]);
        let rt = rt();
        rt.block_on(async {
            let _rx = provider
                .stream(ProviderRequest {
                    system_prompt: "s".into(),
                    messages: vec![],
                    tools: vec![],
                })
                .unwrap();
            let second = provider.stream(ProviderRequest {
                system_prompt: "s".into(),
                messages: vec![],
                tools: vec![],
            });
            assert!(
                matches!(second, Err(ProviderError::Unavailable(m)) if m.contains("exhausted")),
                "an over-running loop gets a typed refusal, not an empty stream"
            );
        });
    }

    #[test]
    fn dropping_the_receiver_cancels_the_sender() {
        // Enough events to overflow the channel bound: the sender cannot
        // finish before the drop, so the test exercises the cancel arm.
        let big_turn: Vec<StreamEvent> = (0..STREAM_CHANNEL_CAP * 4)
            .map(|i| StreamEvent::TextDelta(format!("d{i}")))
            .collect();
        let provider = LoopbackProvider::new("loopback", vec![big_turn]);
        let rt = rt();
        rt.block_on(async {
            let mut rx = provider
                .stream(ProviderRequest {
                    system_prompt: "s".into(),
                    messages: vec![],
                    tools: vec![],
                })
                .unwrap();
            // Consume a little, then drop mid-stream.
            let _ = rx.recv().await.unwrap().unwrap();
            drop(rx);
            // The sender observes the closed channel and exits; poll with a
            // deadline so a broken contract fails the test, not the timer.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while provider.live_senders() > 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "sender task outlived the dropped receiver"
                );
                tokio::task::yield_now().await;
            }
            assert_eq!(provider.live_senders(), 0);
        });
    }

    #[test]
    fn requests_are_recorded_for_assertions() {
        let provider =
            LoopbackProvider::new("loopback", vec![scripted_text("a"), scripted_text("b")]);
        let rt = rt();
        rt.block_on(async {
            for text in ["a", "b"] {
                let mut rx = provider
                    .stream(ProviderRequest {
                        system_prompt: format!("sys-{text}"),
                        messages: vec![ChatMessage {
                            role: Role::User,
                            text: text.into(),
                        }],
                        tools: vec![ToolSpec {
                            name: "read".into(),
                            description: "read a file".into(),
                            schema_json: r#"{"path":"string"}"#.into(),
                        }],
                    })
                    .unwrap();
                while let Some(ev) = rx.recv().await {
                    if ev.is_err() {
                        break;
                    }
                }
            }
            let requests = provider.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].system_prompt, "sys-b");
            assert_eq!(requests[0].tools[0].name, "read");
            assert_eq!(requests[1].messages[0].text, "b");
        });
    }

    #[test]
    fn stream_channel_cap_is_pinned() {
        // Bounds law: the cap is a pinned number, not a feeling.
        assert_eq!(STREAM_CHANNEL_CAP, 64);
    }
}
