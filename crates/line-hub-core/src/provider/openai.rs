//! OpenAI provider (api.openai.com).

use super::openai_compat::{drive_compat_stream, OpenAICompatConfig};
use super::types::*;
use crate::provider::Provider;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use reqwest_eventsource::EventSource;
use std::pin::Pin;

pub struct OpenAIProvider {
    cfg: OpenAICompatConfig,
    http: Client,
}

impl OpenAIProvider {
    pub fn new(entry: &crate::config::ProviderEntry) -> HubResult<Self> {
        if entry.api_key.is_empty() {
            return Err(HubError::Config("OpenAI API key not set".into()));
        }
        let cfg = OpenAICompatConfig {
            label: "openai",
            base_url: entry
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".into()),
            api_key: entry.api_key.clone(),
            default_model: entry.default_model.clone().unwrap_or_else(|| "gpt-5-mini".into()),
            extra_models: Vec::new(),
            strip_thinking_tags: false,
            has_reasoning_channel: false,
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
impl Provider for OpenAIProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OpenAI
    }
    fn label(&self) -> &'static str {
        "OpenAI"
    }
    fn list_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-5".into(),
                label: "GPT-5".into(),
                tier: ModelTier::Flagship,
                description: None,
                is_default: false,
            },
            ModelInfo {
                id: "gpt-5-mini".into(),
                label: "GPT-5 mini".into(),
                tier: ModelTier::Balanced,
                description: Some("Default for OpenAI provider.".into()),
                is_default: true,
            },
            ModelInfo {
                id: "o1".into(),
                label: "o1 (reasoning)".into(),
                tier: ModelTier::Reasoning,
                description: None,
                is_default: false,
            },
            ModelInfo {
                id: "gpt-4o".into(),
                label: "GPT-4o (legacy)".into(),
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
            max_tokens: req.max_tokens,
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
            provider: "openai".into(),
            message: format!("EventSource init: {e}"),
        })?;
        let state = super::openai_compat::CompatStreamState::new(false, false);
        Ok(drive_compat_stream(es, state))
    }
}