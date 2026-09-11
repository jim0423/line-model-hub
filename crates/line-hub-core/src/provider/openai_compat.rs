//! OpenAI-compatible shared implementation.
//!
//! Used by OpenAI / MiniMax / Ollama / LM Studio — anything that speaks the
//! `/v1/chat/completions` SSE protocol.

use super::types::*;
use crate::sse::SseStream;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Configuration shared by every OpenAI-compat provider.
#[derive(Debug, Clone)]
pub struct OpenAICompatConfig {
    pub label: &'static str,
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
    pub extra_models: Vec<ModelInfo>,
    /// Strip `<thinking>...</thinking>` tags from Delta text.
    /// Set true for MiniMax (M3/M2.7 emit this).
    pub strip_thinking_tags: bool,
    /// Whether the provider emits a `reasoning` channel alongside `content`.
    /// MiniMax does; OpenAI does not.
    pub has_reasoning_channel: bool,
}

#[derive(Debug, Serialize)]
pub struct WireChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [Message],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'static str>,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct WireTool {
    #[serde(rename = "type")]
    pub kind: &'static str, // "function"
    pub function: WireToolFn,
}

#[derive(Debug, Serialize)]
pub struct WireToolFn {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// One SSE chunk from the wire.
#[derive(Debug, Deserialize)]
pub struct WireChunk {
    #[serde(default)]
    pub choices: Vec<WireChoice>,
    #[serde(default)]
    pub usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
pub struct WireChoice {
    #[serde(default)]
    pub delta: WireDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WireDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WireToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub function: Option<WireFunction>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WireFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WireUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

/// Stream-level state shared by both providers.
pub struct CompatStreamState {
    pub strip_thinking: bool,
    pub has_reasoning: bool,
    /// Accumulator for `<thinking>...</thinking>` so we can detect tags split
    /// across chunks.
    pub thinking_tail: String,
    /// Running buffer of received content to be returned in Delta events.
    pub content_buf: String,
    pub usage: Option<Usage>,
}

impl CompatStreamState {
    pub fn new(strip_thinking: bool, has_reasoning: bool) -> Self {
        Self {
            strip_thinking,
            has_reasoning,
            thinking_tail: String::new(),
            content_buf: String::new(),
            usage: None,
        }
    }

    /// Process a single content delta string, stripping `<thinking>...</thinking>`
    /// tags and emitting the visible portion. Returns `None` if the entire
    /// delta is inside a thinking block.
    pub fn process_content_delta(&mut self, raw: &str) -> Option<String> {
        if !self.strip_thinking {
            return Some(raw.to_string());
        }
        self.thinking_tail.push_str(raw);

        // If we have an unclosed `<thinking>` or `<think>` tag in the tail,
        // we are mid-block: don't emit anything until the tag closes.
        let has_open = self
            .thinking_tail
            .rfind("<think>")
            .or_else(|| self.thinking_tail.rfind("<thinking"))
            .map(|open_idx| {
                let after = &self.thinking_tail[open_idx..];
                !after.contains("</think>") && !after.contains("</thinking>")
            })
            .unwrap_or(false);

        if has_open {
            // Mid-thought: emit nothing yet, keep tail as-is.
            return None;
        }

        // All tags closed (or no tag at all). Strip complete blocks and emit visible.
        let re = regex_thinking();
        let mut visible = String::new();
        let mut last_end = 0;
        for m in re.find_iter(&self.thinking_tail) {
            visible.push_str(&self.thinking_tail[last_end..m.start()]);
            last_end = m.end();
        }
        visible.push_str(&self.thinking_tail[last_end..]);
        self.thinking_tail.clear();

        if visible.is_empty() {
            None
        } else {
            self.content_buf.push_str(&visible);
            Some(visible)
        }
    }
}

fn regex_thinking() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // (?s) = DOTALL, so `.` matches newlines.
        // Match either `<thinking>...</thinking>` or `<think>...</think>`.
        regex::Regex::new(r"(?s)<think(?:ing)?>.*?</think(?:ing)?>")
            .expect("thinking regex must compile")
    })
}

/// Drive the SSE stream for any OpenAI-compat provider.
pub(super) fn drive_compat_stream(
    es: EventSource,
    state: CompatStreamState,
) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send>> {
    Box::pin(async_stream::stream! {
        let mut state = state;
        let mut es = es;
        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => continue,
                Ok(Event::Message(msg)) => {
                    let data = msg.data;
                    if data == "[DONE]" {
                        yield StreamEvent::Done {
                            stop_reason: "stop".into(),
                            usage: state.usage.clone(),
                        };
                        return;
                    }
                    let chunk: WireChunk = match serde_json::from_str(&data) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(error=%e, data=%data, "skip malformed SSE chunk");
                            continue;
                        }
                    };
                    if let Some(u) = chunk.usage {
                        state.usage = Some(Usage {
                            input_tokens: u.prompt_tokens.unwrap_or(0),
                            output_tokens: u.completion_tokens.unwrap_or(0),
                            reasoning_tokens: None,
                        });
                    }
                    for choice in chunk.choices {
                        // Reasoning channel (MiniMax only)
                        if state.has_reasoning {
                            if let Some(r) = &choice.delta.reasoning {
                                if !r.is_empty() {
                                    yield StreamEvent::ReasoningDelta { text: r.clone() };
                                }
                            }
                        }
                        // Visible content (stripped of <thinking>)
                        if let Some(c) = &choice.delta.content {
                            if let Some(visible) = state.process_content_delta(c) {
                                yield StreamEvent::Delta { text: visible };
                            }
                        }
                        // Tool calls (accumulate by index)
                        if let Some(tcs) = &choice.delta.tool_calls {
                            for tc in tcs {
                                let idx = tc.index.unwrap_or(0);
                                if let Some(id) = &tc.id {
                                    let name = tc
                                        .function
                                        .as_ref()
                                        .and_then(|f| f.name.clone())
                                        .unwrap_or_default();
                                    yield StreamEvent::ToolCallStart {
                                        id: id.clone(),
                                        name,
                                        index: idx,
                                    };
                                }
                                if let Some(args) = tc
                                    .function
                                    .as_ref()
                                    .and_then(|f| f.arguments.clone())
                                {
                                    yield StreamEvent::ToolCallDelta {
                                        index: idx,
                                        arguments_delta: args,
                                    };
                                }
                            }
                        }
                        if let Some(reason) = choice.finish_reason {
                            yield StreamEvent::Done {
                                stop_reason: reason,
                                usage: state.usage.clone(),
                            };
                            return;
                        }
                    }
                }
                Err(reqwest_eventsource::Error::InvalidStatusCode(status, resp)) => {
                    let body = resp.text().await.unwrap_or_default();
                    yield StreamEvent::Error {
                        message: format!("HTTP {} — {}", status, body.chars().take(500).collect::<String>()),
                        retriable: status.is_server_error() || status.as_u16() == 429,
                    };
                    return;
                }
                Err(reqwest_eventsource::Error::InvalidContentType(_, resp)) => {
                    let body = resp.text().await.unwrap_or_default();
                    yield StreamEvent::Error {
                        message: format!("invalid content-type — {}", body.chars().take(500).collect::<String>()),
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
            }
        }
        // Stream ended without explicit Done — emit one.
        yield StreamEvent::Done {
            stop_reason: "stream_closed".into(),
            usage: state.usage,
        };
    })
}

/// Marker to keep `SseStream` in the dep graph even if unused.
#[allow(dead_code)]
pub fn _keep(_: &SseStream) {}