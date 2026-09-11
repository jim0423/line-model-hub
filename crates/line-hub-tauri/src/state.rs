//! Shared application state held inside Tauri's State container.

use line_hub_core::mcp::McpTool;
use line_hub_core::provider::{ChatRequest, Provider, StreamEvent};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct AppState {
    /// Active provider instances keyed by ProviderId.
    pub providers: RwLock<HashMap<String, Arc<dyn Provider>>>,
    /// Active MCP client (None until `spawn_mcp` is called).
    pub mcp: RwLock<Option<Arc<Mutex<Box<dyn McpTool>>>>>,
    /// Active chat sessions keyed by session_id.
    pub sessions: RwLock<HashMap<String, ChatSession>>,
    /// Cancellation tokens per session.
    pub cancel: RwLock<HashMap<String, Arc<tokio::sync::Notify>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            mcp: RwLock::new(None),
            sessions: RwLock::new(HashMap::new()),
            cancel: RwLock::new(HashMap::new()),
        }
    }
}

pub struct ChatSession {
    pub history: Vec<line_hub_core::provider::Message>,
    pub model: String,
    pub provider_id: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-export stream events so command handlers can name them.
pub use line_hub_core::provider::StreamEvent as ReEvent;

// Forward helper
pub async fn stream_to_deltas<F>(
    mut stream: std::pin::Pin<Box<dyn futures::Stream<Item = StreamEvent> + Send>>,
    mut on_event: F,
) -> Result<String, line_hub_core::HubError>
where
    F: FnMut(StreamEvent) + Send,
{
    use futures::StreamExt;
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        match &ev {
            StreamEvent::Delta { text: t } => text.push_str(t),
            StreamEvent::Done { .. } => {
                on_event(ev);
                return Ok(text);
            }
            StreamEvent::Error { message, .. } => {
                on_event(ev.clone());
                return Err(line_hub_core::HubError::Provider {
                    provider: "<stream>".into(),
                    message: message.clone(),
                });
            }
            _ => {}
        }
        on_event(ev);
    }
    Ok(text)
}

// Re-export ChatRequest to keep command handlers concise.
pub use line_hub_core::provider::ChatRequest as Req;