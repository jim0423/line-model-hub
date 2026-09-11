//! Anthropic Claude (api.anthropic.com) — non-OpenAI-compat native driver.
//!
//! Messages API:
//!   POST {base_url}/v1/messages
//!     headers: x-api-key: <key>, anthropic-version: 2023-06-01,
//!              content-type: application/json
//!     body:    { model, system, messages, tools, max_tokens, stream: true }
//!
//! SSE event types:
//!   message_start                       — initial metadata (id, model, usage)
//!   content_block_start                 — {"type":"text"} or {"type":"tool_use"}
//!   content_block_delta (text)          — {"delta":{"type":"text_delta","text":"…"}}
//!   content_block_delta (tool_use)      — {"delta":{"type":"input_json_delta","partial_json":"…"}}
//!   content_block_stop                  — block finished
//!   message_delta                       — {"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":N}}
//!   message_stop                        — final sentinel
//!
//! Anthropic does NOT emit a parallel reasoning channel; it returns the
//! model's reasoning as plain text inside the same text content blocks. To
//! preserve the same UX as the OpenAI/MiniMax drivers we expose that text
//! as Delta events (we do NOT emit ReasoningDelta) — reasoning visibility
//! is only useful when a provider splits it out of the visible response.
//!
//! Tools are mapped from our `ToolDefinition` (JSON Schema) into Anthropic's
//! `{ name, description, input_schema }` shape. Anthropic only supports
//! top-level object schemas, which matches our domain.

use super::types::*;
use crate::provider::Provider;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use std::pin::Pin;

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    default_model: String,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(entry: &crate::config::ProviderEntry) -> HubResult<Self> {
        if entry.api_key.is_empty() {
            return Err(HubError::Config("Anthropic API key not set".into()));
        }
        Ok(Self {
            api_key: entry.api_key.clone(),
            base_url: entry
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into()),
            default_model: entry
                .default_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-5".into()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| HubError::Config(format!("HTTP client init: {e}")))?,
        })
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Anthropic
    }
    fn label(&self) -> &'static str {
        "Anthropic"
    }
    fn list_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "claude-opus-4-5".into(),
                label: "Claude Opus 4.5".into(),
                tier: ModelTier::Flagship,
                description: None,
                is_default: false,
            },
            ModelInfo {
                id: "claude-sonnet-4-5".into(),
                label: "Claude Sonnet 4.5".into(),
                tier: ModelTier::Balanced,
                description: Some("Default for Anthropic provider.".into()),
                is_default: true,
            },
            ModelInfo {
                id: "claude-haiku-4-5".into(),
                label: "Claude Haiku 4.5".into(),
                tier: ModelTier::Fast,
                description: None,
                is_default: false,
            },
        ]
    }

    async fn chat(
        &self,
        req: ChatRequest,
    ) -> HubResult<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        // ── 1. Pull the system message out and map our wire format to Anthropic's.
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<WireMessage> = Vec::with_capacity(req.messages.len());
        for m in &req.messages {
            match m {
                Message::System { content } => {
                    // Anthropic keeps system as a top-level field; concatenate if
                    // multiple are present (shouldn't happen, but be safe).
                    system_prompt = Some(match system_prompt {
                        Some(prev) => format!("{prev}\n\n{content}"),
                        None => content.clone(),
                    });
                }
                Message::User { content } => {
                    api_messages.push(WireMessage {
                        role: "user".into(),
                        content: serde_json::Value::String(content.clone()),
                    });
                }
                Message::Assistant {
                    content,
                    tool_calls,
                    ..
                } => {
                    // Anthropic's assistant content is an array of content blocks.
                    let mut blocks: Vec<serde_json::Value> = Vec::new();
                    if let Some(text) = content {
                        if !text.is_empty() {
                            blocks.push(serde_json::json!({
                                "type": "text",
                                "text": text,
                            }));
                        }
                    }
                    if let Some(tcs) = tool_calls {
                        for tc in tcs {
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.function.name,
                                "input": serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(Default::default())),
                            }));
                        }
                    }
                    api_messages.push(WireMessage {
                        role: "assistant".into(),
                        content: serde_json::Value::Array(blocks),
                    });
                }
                Message::Tool {
                    tool_call_id,
                    content,
                } => {
                    // Anthropic wraps tool results in a single user message
                    // whose content is an array of tool_result blocks.
                    let block = serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content,
                    });
                    // Merge consecutive tool messages into one user message,
                    // mirroring how the OpenAI-compat path does it.
                    if let Some(last) = api_messages.last_mut() {
                        if last.role == "user"
                            && last.content.is_array()
                            && last
                                .content
                                .as_array()
                                .and_then(|arr| arr.first())
                                .and_then(|v| v.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("tool_result")
                        {
                            if let Some(arr) = last.content.as_array_mut() {
                                arr.push(block);
                                continue;
                            }
                        }
                    }
                    api_messages.push(WireMessage {
                        role: "user".into(),
                        content: serde_json::Value::Array(vec![block]),
                    });
                }
            }
        }

        // ── 2. Map tools to Anthropic's `{ name, description, input_schema }`.
        let wire_tools: Vec<WireTool> = req
            .tools
            .iter()
            .map(|t| WireTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();

        let body = WireChatRequest {
            model: &req.model,
            system: system_prompt.as_deref(),
            messages: &api_messages,
            tools: wire_tools,
            max_tokens: req.max_tokens.unwrap_or(8192),
            temperature: req.temperature,
            stream: true,
        };

        // ── 3. Open the SSE stream.
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let req_builder = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(
                serde_json::to_string(&body)
                    .map_err(|e| HubError::Provider {
                        provider: "anthropic".into(),
                        message: format!("serialise request: {e}"),
                    })?,
            );

        let mut es = reqwest_eventsource::EventSource::new(req_builder).map_err(|e| {
            HubError::Provider {
                provider: "anthropic".into(),
                message: format!("EventSource init: {e}"),
            }
        })?;

        // ── 4. Translate Anthropic events → our StreamEvent.
        let output = async_stream::stream! {
            // Track which content blocks we've already emitted a start for.
            // Anthropic increments `index` per content block within a message.
            type BlockState = Option<String>; // tool_use_id when type=="tool_use"
            let mut current_tool_id: Option<String> = None;
            let mut last_stop_reason: String = "end_turn".to_string();
            let mut last_usage: Option<Usage> = None;

            while let Some(ev_res) = es.next().await {
                let ev = match ev_res {
                    Ok(Event::Open) | Ok(Event::Message{ .. }) if false => continue,
                    Ok(Event::Open) => continue,
                    Ok(Event::Message(message)) => message.data,
                    Err(reqwest_eventsource::Error::InvalidStatusCode(code, resp)) => {
                        let status = code.as_u16();
                        let body = resp.text().await.unwrap_or_default();
                        yield StreamEvent::Error {
                            message: format!("HTTP {status}: {body}"),
                            retriable: status >= 500,
                        };
                        return;
                    }
                    Err(reqwest_eventsource::Error::InvalidContentType(_, resp)) => {
                        let body = resp.text().await.unwrap_or_default();
                        yield StreamEvent::Error {
                            message: format!("Invalid content type: {body}"),
                            retriable: false,
                        };
                        return;
                    }
                    Err(e) => {
                        yield StreamEvent::Error {
                            message: format!("SSE error: {e}"),
                            retriable: true,
                        };
                        return;
                    }
                };

                // Anthropic emits `event: <type>` lines; reqwest-eventsource
                // surfaces only the data, so we trust the JSON shape.
                let parsed: Result<WireEvent, _> = serde_json::from_str(&ev);
                let ev = match parsed {
                    Ok(v) => v,
                    Err(_) => continue, // unknown / non-JSON keepalive line
                };

                match ev {
                    WireEvent::MessageStart { message } => {
                        if let Some(u) = message.usage {
                            last_usage = Some(Usage {
                                input_tokens: u.input_tokens.unwrap_or(0),
                                output_tokens: u.output_tokens.unwrap_or(0),
                                reasoning_tokens: None,
                            });
                        }
                    }
                    WireEvent::ContentBlockStart {
                        index: _,
                        content_block,
                    } => match content_block {
                        WireContentBlock::Text { .. } => {
                            current_tool_id = None;
                        }
                        WireContentBlock::ToolUse { id, name, .. } => {
                            current_tool_id = Some(id.clone());
                            yield StreamEvent::ToolCallStart {
                                id,
                                name,
                                index: 0, // Anthropic indexes by block; we collapse to a single tool-call slot
                            };
                        }
                        WireContentBlock::Thinking { .. } => {
                            current_tool_id = None;
                        }
                        WireContentBlock::Unknown => {
                            current_tool_id = None;
                        }
                    },
                    WireEvent::ContentBlockDelta { delta, .. } => match delta {
                        WireDelta::TextDelta { text } => {
                            yield StreamEvent::Delta { text };
                        }
                        WireDelta::InputJsonDelta { partial_json } => {
                            if current_tool_id.is_some() {
                                yield StreamEvent::ToolCallDelta {
                                    index: 0,
                                    arguments_delta: partial_json,
                                };
                            }
                        }
                        WireDelta::ThinkingDelta { thinking } => {
                            // Surface as ReasoningDelta for parity with MiniMax.
                            yield StreamEvent::ReasoningDelta { text: thinking };
                        }
                        WireDelta::Unknown => {}
                    },
                    WireEvent::ContentBlockStop { .. } => {
                        current_tool_id = None;
                    }
                    WireEvent::MessageDelta { delta, usage } => {
                        if let Some(sr) = delta.stop_reason {
                            last_stop_reason = sr;
                        }
                        if let Some(u) = usage {
                            // Anthropic reports output_tokens in message_delta; merge.
                            let merged = match last_usage.take() {
                                Some(mut prev) => {
                                    prev.output_tokens = u.output_tokens.unwrap_or(prev.output_tokens);
                                    prev
                                }
                                None => Usage {
                                    input_tokens: 0,
                                    output_tokens: u.output_tokens.unwrap_or(0),
                                    reasoning_tokens: None,
                                },
                            };
                            last_usage = Some(merged);
                        }
                    }
                    WireEvent::MessageStop => {
                        yield StreamEvent::Done {
                            stop_reason: last_stop_reason.clone(),
                            usage: last_usage.take(),
                        };
                        return;
                    }
                    WireEvent::Ping => {}
                    WireEvent::Unknown => {}
                }
            }

            // Stream ended without a message_stop (rare — usually the connection
            // dropped mid-flight). Emit a Done with an empty stop_reason so the
            // caller still flushes its accumulated state.
            yield StreamEvent::Done {
                stop_reason: last_stop_reason,
                usage: last_usage,
            };
        };

        Ok(Box::pin(output))
    }
}

// ──────────────────────────────────────────────────────────────────────
// Wire types — match Anthropic's `/v1/messages` SSE protocol.
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct WireChatRequest<'a> {
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: &'a [WireMessage],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct WireTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct WireMessage {
    role: String,
    content: serde_json::Value,
}

// `event: message_start`
#[derive(Debug, Deserialize)]
struct WireMessageStart {
    #[serde(default)]
    message: WireMessageStartInner,
}

#[derive(Debug, Default, Deserialize)]
struct WireMessageStartInner {
    #[serde(default)]
    usage: Option<WireUsageInner>,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsageInner {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

// `event: content_block_start`
#[derive(Debug, Deserialize)]
struct WireContentBlockStart {
    index: usize,
    content_block: WireContentBlock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentBlock {
    Text { text: String },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(other)]
    Unknown,
}

// `event: content_block_delta`
#[derive(Debug, Deserialize)]
struct WireContentBlockDelta {
    index: usize,
    delta: WireDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    #[serde(other)]
    Unknown,
}

// `event: message_delta`
#[derive(Debug, Deserialize)]
struct WireMessageDelta {
    #[serde(default)]
    delta: WireMessageDeltaInner,
    #[serde(default)]
    usage: Option<WireUsageInner>,
}

#[derive(Debug, Default, Deserialize)]
struct WireMessageDeltaInner {
    #[serde(default)]
    stop_reason: Option<String>,
}

// `event: <type>` — dispatch on the JSON `type` field.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireEvent {
    MessageStart {
        #[serde(default)]
        message: WireMessageStartInner,
    },
    ContentBlockStart {
        index: usize,
        content_block: WireContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: WireDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        #[serde(default)]
        delta: WireMessageDeltaInner,
        #[serde(default)]
        usage: Option<WireUsageInner>,
    },
    MessageStop,
    Ping,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with_user_and_tool() -> ChatRequest {
        ChatRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![
                Message::System { content: "You are helpful.".into() },
                Message::User { content: "hi".into() },
            ],
            tools: vec![ToolDefinition {
                name: "echo".into(),
                description: "echo back".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"],
                }),
            }],
            temperature: 0.3,
            max_tokens: Some(1024),
        }
    }

    #[test]
    fn request_serialises_with_anthropic_shape() {
        let r = req_with_user_and_tool();
        let mut api_messages = Vec::new();
        let mut system: Option<String> = None;
        for m in &r.messages {
            match m {
                Message::System { content } => system = Some(content.clone()),
                Message::User { content } => api_messages.push(WireMessage {
                    role: "user".into(),
                    content: serde_json::Value::String(content.clone()),
                }),
                _ => {}
            }
        }
        let wire_tools: Vec<WireTool> = r
            .tools
            .iter()
            .map(|t| WireTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();
        let body = WireChatRequest {
            model: &r.model,
            system: system.as_deref(),
            messages: &api_messages,
            tools: wire_tools,
            max_tokens: r.max_tokens.unwrap_or(8192),
            temperature: r.temperature,
            stream: true,
        };
        let s = serde_json::to_string(&body).expect("serialise");
        // Spot-check the Anthropic-specific fields.
        assert!(s.contains("\"system\":\"You are helpful.\""), "{s}");
        assert!(s.contains("\"max_tokens\":1024"), "{s}");
        assert!(s.contains("\"stream\":true"), "{s}");
        assert!(s.contains("\"input_schema\""), "{s}");
        assert!(!s.contains("\"tool_choice\""), "{s}");
    }

    #[test]
    fn parses_text_delta_event() {
        let raw = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let parsed: WireEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            WireEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    WireDelta::TextDelta { text } => assert_eq!(text, "Hello"),
                    other => panic!("wrong delta variant: {other:?}"),
                }
            }
            other => panic!("wrong top-level variant: {other:?}"),
        }
    }

    #[test]
    fn parses_tool_use_input_json_delta_event() {
        let raw = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"text\":"}}"#;
        let parsed: WireEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            WireEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                match delta {
                    WireDelta::InputJsonDelta { partial_json } => {
                        assert_eq!(partial_json, "{\"text\":");
                    }
                    other => panic!("wrong delta variant: {other:?}"),
                }
            }
            other => panic!("wrong top-level variant: {other:?}"),
        }
    }

    #[test]
    fn parses_thinking_delta_event() {
        let raw = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm…"}}"#;
        let parsed: WireEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            WireEvent::ContentBlockDelta { delta, .. } => match delta {
                WireDelta::ThinkingDelta { thinking } => assert_eq!(thinking, "hmm…"),
                other => panic!("wrong delta variant: {other:?}"),
            },
            other => panic!("wrong top-level variant: {other:?}"),
        }
    }

    #[test]
    fn parses_message_delta_with_stop_reason_and_usage() {
        let raw = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        let parsed: WireEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            WireEvent::MessageDelta { delta, usage } => {
                assert_eq!(delta.stop_reason.as_deref(), Some("end_turn"));
                assert_eq!(usage.unwrap().output_tokens, Some(42));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_tool_use_block_start() {
        let raw = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_abc","name":"echo","input":{}}}"#;
        let parsed: WireEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            WireEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                assert_eq!(index, 1);
                match content_block {
                    WireContentBlock::ToolUse { id, name, .. } => {
                        assert_eq!(id, "toolu_abc");
                        assert_eq!(name, "echo");
                    }
                    other => panic!("wrong content block variant: {other:?}"),
                }
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}