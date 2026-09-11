//! Local Ollama / LM Studio (OpenAI-compat). Dynamic model list from
//! GET /v1/models.

use super::openai_compat::{drive_compat_stream, OpenAICompatConfig};
use super::types::*;
use crate::provider::Provider;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use reqwest_eventsource::EventSource;
use std::pin::Pin;

pub struct OllamaProvider {
    cfg: OpenAICompatConfig,
    http: Client,
}

impl OllamaProvider {
    pub fn new(entry: &crate::config::ProviderEntry) -> HubResult<Self> {
        let cfg = OpenAICompatConfig {
            label: "ollama",
            base_url: entry
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".into()),
            // Ollama doesn't require a key; LM Studio accepts anything.
            api_key: if entry.api_key.is_empty() {
                "ollama".to_string()
            } else {
                entry.api_key.clone()
            },
            default_model: entry.default_model.clone().unwrap_or_else(|| "qwen2.5:14b".into()),
            extra_models: Vec::new(),
            strip_thinking_tags: false,
            has_reasoning_channel: false,
        };
        Ok(Self {
            cfg,
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(300)) // local is slow
                .build()
                .map_err(|e| HubError::Config(format!("HTTP client init: {e}")))?,
        })
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Ollama
    }
    fn label(&self) -> &'static str {
        "Ollama / LM Studio"
    }
    fn list_models(&self) -> Vec<ModelInfo> {
        // Static fallback — runtime enrichment via GET /v1/models happens in
        // `OllamaProvider::discover()`. The UI merges both lists.
        vec![
            ModelInfo {
                id: "qwen2.5:14b".into(),
                label: "Qwen 2.5 14B (recommended local)".into(),
                tier: ModelTier::Balanced,
                description: Some("Good tool-use + Chinese support.".into()),
                is_default: true,
            },
            ModelInfo {
                id: "llama3.3:70b".into(),
                label: "Llama 3.3 70B".into(),
                tier: ModelTier::Flagship,
                description: Some("Large, slow, English-first.".into()),
                is_default: false,
            },
            ModelInfo {
                id: "gpt-oss:20b".into(),
                label: "gpt-oss 20B".into(),
                tier: ModelTier::Balanced,
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
            provider: "ollama".into(),
            message: format!("EventSource init: {e}"),
        })?;
        let state = super::openai_compat::CompatStreamState::new(false, false);
        Ok(drive_compat_stream(es, state))
    }
}