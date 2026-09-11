//! Anthropic Claude (api.anthropic.com) — non-OpenAI-compat.
//!
//! Native Messages API: POST /v1/messages with `messages`, `tools`, SSE
//! response with event types `message_start`, `content_block_start`,
//! `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`.

use super::types::*;
use crate::provider::Provider;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    default_model: String,
    http: Client,
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
            http: Client::builder()
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
        // TODO: implement Anthropic Messages SSE driver
        // (event types: message_start, content_block_start with tool_use,
        //  content_block_delta with input_json_delta, message_delta with stop_reason)
        let _ = req;
        Err(HubError::Provider {
            provider: "anthropic".into(),
            message: "Anthropic provider pending v1.1 implementation".into(),
        })
    }
}