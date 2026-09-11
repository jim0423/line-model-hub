//! Provider-agnostic types shared across the conversation loop, MCP layer,
//! and UI.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    /// OpenAI / GPT-5 / o1 / GPT-4o via api.openai.com
    OpenAI,
    /// MiniMax M3 / M2.7 via api.minimax.io
    MiniMax,
    /// Anthropic Claude via api.anthropic.com
    Anthropic,
    /// Local Ollama / LM Studio (OpenAI-compatible)
    Ollama,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::MiniMax => "minimax",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // always "function" for OpenAI-compat
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Arguments as a JSON-encoded STRING (OpenAI/MiniMax wire format).
    /// Decoded by the conversation loop before invoking the tool.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub tier: ModelTier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// Flagship / most capable (slowest, most expensive)
    Flagship,
    /// Balanced default (M3, GPT-5-mini, Sonnet, qwen2.5:14b)
    Balanced,
    /// Reasoning-optimised (M2.7, o1, Opus)
    Reasoning,
    /// Fastest (highspeed variants, haiku, Flash)
    Fast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

fn default_temperature() -> f32 {
    0.3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Visible response text delta. Already stripped of `<thinking>` tags.
    Delta { text: String },
    /// Reasoning / thinking content delta (provider-specific, optional).
    /// Surfaced to the UI in a collapsible "Thinking…" panel.
    ReasoningDelta { text: String },
    /// A tool call is starting. `index` is its position within this turn.
    ToolCallStart { id: String, name: String, index: usize },
    /// Incremental arguments delta for an existing tool call. Concatenate in
    /// `index` order until `Done`.
    ToolCallDelta { index: usize, arguments_delta: String },
    /// Stream completed. `usage` is best-effort (only some providers emit it
    /// per chunk; we surface the last seen values).
    Done {
        stop_reason: String,
        usage: Option<Usage>,
    },
    /// Stream aborted with a non-recoverable error.
    Error { message: String, retriable: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}