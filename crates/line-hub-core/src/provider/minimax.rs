//! MiniMax M3 / M2.7 — default provider for LINE Model Hub.
//!
//! Speaks OpenAI-compat SSE at https://api.minimax.io/v1/chat/completions.
//! Key difference from OpenAI: M3/M2.7 emit `<thinking>` tags inside the
//! content channel AND a parallel `reasoning` channel.

use super::openai_compat::{drive_compat_stream, OpenAICompatConfig};
use super::types::*;
use crate::provider::Provider;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use reqwest_eventsource::EventSource;
use std::pin::Pin;

pub struct MiniMaxProvider {
    cfg: OpenAICompatConfig,
    http: Client,
}

impl MiniMaxProvider {
    pub fn new(entry: &crate::config::ProviderEntry) -> HubResult<Self> {
        if entry.api_key.is_empty() {
            return Err(HubError::Config(
                "MiniMax API key not set. Configure via Settings.".into(),
            ));
        }
        let cfg = OpenAICompatConfig {
            label: "minimax",
            base_url: entry
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.minimax.io".into()),
            api_key: entry.api_key.clone(),
            default_model: entry.default_model.clone().unwrap_or_else(|| "MiniMax-M3".into()),
            extra_models: Vec::new(),
            strip_thinking_tags: true,
            has_reasoning_channel: true,
        };
        Ok(Self {
            cfg,
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| HubError::Config(format!("HTTP client init: {e}")))?,
        })
    }
}

#[async_trait]
impl Provider for MiniMaxProvider {
    fn id(&self) -> ProviderId {
        ProviderId::MiniMax
    }
    fn label(&self) -> &'static str {
        "MiniMax"
    }
    fn list_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "MiniMax-M3".into(),
                label: "★ MiniMax M3 (default)".into(),
                tier: ModelTier::Balanced,
                description: Some(
                    "Reasoning + tool use. Best default for LINE automation.".into(),
                ),
                is_default: true,
            },
            ModelInfo {
                id: "MiniMax-M2.7".into(),
                label: "MiniMax M2.7".into(),
                tier: ModelTier::Reasoning,
                description: Some("Deeper reasoning. Slower; content can be sparse.".into()),
                is_default: false,
            },
            ModelInfo {
                id: "MiniMax-M2.7-highspeed".into(),
                label: "MiniMax M2.7 highspeed".into(),
                tier: ModelTier::Fast,
                description: Some("⚠ Rate limit prone.".into()),
                is_default: false,
            },
        ]
    }

    async fn chat(
        &self,
        req: ChatRequest,
    ) -> HubResult<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        let wire = super::openai_compat::WireChatRequest {
            model: &req.model,
            messages: &req.messages,
            tools: req
                .tools
                .iter()
                .map(|t| super::openai_compat::WireTool {
                    kind: "function",
                    function: super::openai_compat::WireToolFn {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    },
                })
                .collect(),
            tool_choice: if req.tools.is_empty() { None } else { Some("auto") },
            temperature: req.temperature,
            max_tokens: req.max_tokens.or(Some(2048)),
            stream: true,
        };
        let url = format!("{}/v1/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let req_builder = self
            .http
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .header("Content-Type", "application/json")
            .json(&wire);
        let es = EventSource::new(req_builder).map_err(|e| HubError::Provider {
            provider: "minimax".into(),
            message: format!("EventSource init: {e}"),
        })?;
        let state = super::openai_compat::CompatStreamState::new(true, true);
        Ok(drive_compat_stream(es, state))
    }
}