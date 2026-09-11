//! Provider trait + concrete implementations.
//!
//! Each provider converts its native API into the same [`ChatRequest`] /
//! [`StreamEvent`] contract so the conversation loop is provider-agnostic.

pub mod anthropic;
pub mod minimax;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod types;

pub use types::{
    ChatRequest, FunctionCall, Message, ModelInfo, ProviderId, StreamEvent, ToolCall,
    ToolDefinition, Usage,
};

use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier used in config (e.g. "minimax", "openai").
    fn id(&self) -> ProviderId;

    /// Human-readable display name (e.g. "MiniMax").
    fn label(&self) -> &'static str;

    /// Static + dynamic model catalogue shown in the model picker.
    fn list_models(&self) -> Vec<ModelInfo>;

    /// Whether this provider supports tool use at all. Small local models
    /// may return false → the loop falls back to JSON action planning.
    fn supports_tools(&self) -> bool {
        true
    }

    /// Streaming chat completion. Implementations MUST:
    ///   - Emit `Delta` chunks that the UI can render token-by-token.
    ///   - Emit `ReasoningDelta` for thinking content (provider-dependent).
    ///   - Emit `ToolCallStart` then incremental `ToolCallDelta` chunks.
    ///   - Emit exactly one `Done` event per call (or `Error`).
    ///   - Strip provider-specific thinking tags from `Delta` text.
    async fn chat(
        &self,
        req: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>, crate::HubError>;
}

// Re-export concrete providers so callers can construct them by id.
pub use anthropic::AnthropicProvider;
pub use minimax::MiniMaxProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;

/// Resolve a provider by id from a registry. Reads `~/.line-hub/config.json`.
pub async fn provider_from_id(
    id: ProviderId,
) -> Result<Box<dyn Provider>, crate::HubError> {
    let cfg = crate::config::HubConfig::load().await?;
    let entry = cfg
        .providers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| crate::HubError::Config(format!("provider {} not configured", id)))?;
    Ok(match id {
        ProviderId::OpenAI => Box::new(OpenAIProvider::new(entry)?),
        ProviderId::MiniMax => Box::new(MiniMaxProvider::new(entry)?),
        ProviderId::Anthropic => Box::new(AnthropicProvider::new(entry)?),
        ProviderId::Ollama => Box::new(OllamaProvider::new(entry)?),
    })
}