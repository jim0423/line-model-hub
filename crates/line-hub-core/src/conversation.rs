//! Agentic conversation loop.
//!
//! Drives the provider ↔ MCP tool ↔ provider cycle with bounded depth and
//! a mandatory human-approval gate before any tool that sends a message.

use crate::mcp::{McpTool, ToolRegistry, ToolResult, ToolResultBlock};
use crate::provider::{ChatRequest, Message, Provider, StreamEvent, ToolCall, ToolDefinition};
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;

#[async_trait]
pub trait SendGuard: Send + Sync {
    /// Called BEFORE any tool that would mutate LINE state on the user's behalf.
    /// Implementations MUST show the user the exact chat + message + tool name
    /// and return `Ok(true)` only on explicit approval.
    async fn approve_send(
        &self,
        chat: &str,
        message: &str,
        tool: &str,
    ) -> HubResult<bool>;
}

pub struct NoopGuard;
#[async_trait]
impl SendGuard for NoopGuard {
    async fn approve_send(&self, _chat: &str, _message: &str, _tool: &str) -> HubResult<bool> {
        Ok(false) // refuse by default — never auto-send
    }
}

#[derive(Clone)]
pub struct LoopConfig {
    pub max_tool_turns: usize,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_tool_turns: 8,
            model: "MiniMax-M3".into(),
            temperature: 0.3,
            max_tokens: Some(2048),
        }
    }
}

pub enum UiEvent {
    /// Streamed content delta (thinking already stripped).
    Delta(String),
    /// Streamed reasoning delta (provider-dependent).
    Reasoning(String),
    /// A tool call started; the UI may show "🔧 {name}" with args accumulating.
    ToolStart { id: String, name: String, args_preview: String },
    /// Tool call arguments complete; result is being awaited.
    /// `attachment_count` is the number of previews already attached to
    /// the args payload (so the UI can render a 📎 badge before decoding).
    ToolArgs {
        id: String,
        args: serde_json::Value,
        attachment_count: usize,
    },
    /// Tool call finished. `attachments` carries the typed content blocks
    /// (text / image / audio / unsupported) so the UI can render each one
    /// without first re-parsing the preview text.
    ToolDone {
        id: String,
        name: String,
        result_preview: String,
        attachments: Vec<ToolResultBlock>,
    },
    /// A send was blocked by the guard.
    SendBlocked { chat: String, message: String },
    /// Final assistant turn complete.
    Done,
    Error(String),
}

/// Tools considered "sending" — they mutate LINE state on the user's behalf
/// and MUST pass the SendGuard before invocation.
pub const SEND_TOOLS: &[&str] = &[
    "send_message_auto",
    "send_message_manual",
    "send_file_manual",
    "clear_line_draft",
    "set_line_draft",
];

/// One turn of the agentic loop. Public so the Tauri command layer can drive it
/// with its own progress channel.
pub async fn run_turn<P, G, S>(
    provider: &P,
    mcp: &Arc<dyn McpTool>,
    guard: &G,
    cfg: &LoopConfig,
    history: &mut Vec<Message>,
    user_input: &str,
    mut on_event: S,
) -> HubResult<String>
where
    P: Provider + ?Sized,
    G: SendGuard,
    S: FnMut(UiEvent) + Send,
{
    history.push(Message::User {
        content: user_input.into(),
    });

    let tools: Vec<ToolDefinition> = mcp.list_tools().await?;
    let tool_registry: ToolRegistry = mcp.tool_registry();

    for _turn in 0..cfg.max_tool_turns {
        let req = ChatRequest {
            model: cfg.model.clone(),
            messages: history.clone(),
            tools: tools.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        };

        let stream = provider.chat(req).await?;
        let mut stream = Box::pin(stream);

        // Buffer for this turn
        let mut assistant_text = String::new();
        let mut assistant_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut per_tool_args: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        let mut per_tool_ids: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        let mut per_tool_names: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();

        let mut finish_reason = "stop".to_string();

        while let Some(ev) = stream.next().await {
            match ev {
                StreamEvent::Delta { text } => {
                    assistant_text.push_str(&text);
                    on_event(UiEvent::Delta(text));
                }
                StreamEvent::ReasoningDelta { text } => {
                    assistant_reasoning.push_str(&text);
                    on_event(UiEvent::Reasoning(text));
                }
                StreamEvent::ToolCallStart {
                    id,
                    name,
                    index,
                } => {
                    per_tool_ids.insert(index, id.clone());
                    per_tool_names.insert(index, name.clone());
                    per_tool_args.insert(index, String::new());
                    on_event(UiEvent::ToolStart {
                        id,
                        name,
                        args_preview: String::new(),
                    });
                }
                StreamEvent::ToolCallDelta {
                    index,
                    arguments_delta,
                } => {
                    per_tool_args
                        .entry(index)
                        .or_default()
                        .push_str(&arguments_delta);
                }
                StreamEvent::Done {
                    stop_reason,
                    usage: _,
                } => {
                    finish_reason = stop_reason;
                    break;
                }
                StreamEvent::Error { message, .. } => {
                    on_event(UiEvent::Error(message.clone()));
                    return Err(HubError::Provider {
                        provider: provider.label().into(),
                        message,
                    });
                }
            }
        }

        // Materialise tool calls in index order
        for idx in per_tool_ids.keys().copied().collect::<Vec<_>>() {
            let id = per_tool_ids.remove(&idx).unwrap_or_default();
            let name = per_tool_names.remove(&idx).unwrap_or_default();
            let args = per_tool_args.remove(&idx).unwrap_or_default();
            let parsed = serde_json::from_str::<serde_json::Value>(&args).unwrap_or(serde_json::Value::String(args.clone()));
            tool_calls.push(ToolCall {
                id: id.clone(),
                kind: "function".into(),
                function: crate::provider::FunctionCall {
                    name: name.clone(),
                    arguments: args,
                },
            });
            on_event(UiEvent::ToolArgs {
                id,
                args: parsed.clone(),
                // Args themselves carry no inline attachments today —
                // any image / audio previews surface via ToolDone's
                // `attachments` field. The UI uses 0 to know it should
                // expect the badge to light up later.
                attachment_count: 0,
            });
        }

        // Push assistant message
        history.push(Message::Assistant {
            content: if assistant_text.is_empty() {
                None
            } else {
                Some(assistant_text.clone())
            },
            reasoning: if assistant_reasoning.is_empty() {
                None
            } else {
                Some(assistant_reasoning.clone())
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.clone())
            },
        });

        // No tool calls → final reply
        if tool_calls.is_empty() || finish_reason != "tool_calls" {
            on_event(UiEvent::Done);
            return Ok(assistant_text);
        }

        // Execute each tool call sequentially
        for tc in &tool_calls {
            let args_json: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or(serde_json::Value::Null);

            // Send-guard gate
            if SEND_TOOLS.contains(&tc.function.name.as_str()) {
                let chat = args_json
                    .get("chatName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                let msg = args_json
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let approved = guard.approve_send(chat, msg, &tc.function.name).await?;
                if !approved {
                    on_event(UiEvent::SendBlocked {
                        chat: chat.into(),
                        message: msg.into(),
                    });
                    history.push(Message::Tool {
                        tool_call_id: tc.id.clone(),
                        content: serde_json::to_string(&serde_json::json!({
                            "error": "User declined to send",
                            "operationMayHaveCompleted": false
                        }))?,
                    });
                    continue;
                }
            }

            // Invoke tool — use the structured path so image / audio
            // previews ride through as typed blocks rather than a flat
            // text blob.
            let tool_result = mcp
                .call_tool_structured(&tc.function.name, args_json.clone())
                .await;

            // Materialise the result. On error we still emit a `ToolDone`
            // with no attachments and surface the message as `Error` so
            // the UI can keep its tool card visible.
            let (result_str, attachments) = match &tool_result {
                Ok(ToolResult::Ok { blocks }) => {
                    let preview_blocks: Vec<ToolResultBlock> = blocks.clone();
                    let str_repr = serde_json::to_string(
                        &blocks
                            .iter()
                            .map(|b| match b {
                                ToolResultBlock::Text { text } => {
                                    serde_json::json!({"kind": "text", "text": text})
                                }
                                ToolResultBlock::Image { mime_type, .. } => {
                                    serde_json::json!({"kind": "image", "mime_type": mime_type})
                                }
                                ToolResultBlock::Audio { mime_type, .. } => {
                                    serde_json::json!({"kind": "audio", "mime_type": mime_type})
                                }
                                ToolResultBlock::Unsupported { mime_type, note } => {
                                    serde_json::json!({"kind": "unsupported", "mime_type": mime_type, "note": note})
                                }
                            })
                            .collect::<Vec<_>>(),
                    )?;
                    (str_repr, preview_blocks)
                }
                Ok(ToolResult::Err { message }) => {
                    on_event(UiEvent::Error(format!(
                        "{} failed: {}",
                        tc.function.name, message
                    )));
                    (
                        serde_json::to_string(&serde_json::json!({
                            "error": message,
                            "code": "TOOL_EXEC_FAILED"
                        }))?,
                        Vec::new(),
                    )
                }
                Err(e) => {
                    on_event(UiEvent::Error(format!(
                        "{} failed: {}",
                        tc.function.name, e
                    )));
                    (
                        serde_json::to_string(&serde_json::json!({
                            "error": e.to_string(),
                            "code": "TOOL_EXEC_FAILED"
                        }))?,
                        Vec::new(),
                    )
                }
            };

            let preview = if result_str.len() > 200 {
                format!("{}…", &result_str[..200])
            } else {
                result_str.clone()
            };
            on_event(UiEvent::ToolDone {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                result_preview: preview,
                attachments,
            });

            history.push(Message::Tool {
                tool_call_id: tc.id.clone(),
                content: result_str,
            });
        }
    }

    Err(HubError::Config(format!(
        "max_tool_turns={} exceeded",
        cfg.max_tool_turns
    )))
}

#[allow(dead_code)]
pub type DynSendGuard = Arc<dyn SendGuard>;

// Forward-declare the Stream trait usage so unused-imports doesn't complain.
#[allow(dead_code)]
fn _pin<S: Stream + Unpin + Send + 'static>(s: S) -> Pin<Box<dyn Stream<Item = S::Item> + Send>> {
    Box::pin(s)
}