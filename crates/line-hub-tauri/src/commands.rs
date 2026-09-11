//! Tauri commands — IPC surface between React frontend and Rust core.

use crate::state::AppState;
use line_hub_core::config::{HubConfig, ProviderEntry};
use line_hub_core::mcp::{McpClient, McpTool};
use line_hub_core::provider::{
    ChatRequest, Message, ModelInfo, Provider, ProviderId, StreamEvent, ToolDefinition,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tokio::sync::Mutex;

// -------------------------------------------------------------------------
// Types crossing the IPC boundary
// -------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ProviderSummary {
    pub id: String,
    pub label: String,
    pub models: Vec<ModelInfo>,
    pub default_model: Option<String>,
    pub is_configured: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChatArgs {
    pub session_id: String,
    pub user_input: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventPayload {
    Delta { text: String },
    Reasoning { text: String },
    ToolStart { id: String, name: String, args_preview: String },
    ToolArgs { id: String, args: serde_json::Value },
    ToolDone { id: String, name: String, result_preview: String },
    SendBlocked { chat: String, message: String },
    Done,
    Error { message: String },
}

impl From<&StreamEvent> for UiEventPayload {
    fn from(e: &StreamEvent) -> Self {
        match e {
            StreamEvent::Delta { text } => Self::Delta { text: text.clone() },
            StreamEvent::ReasoningDelta { text } => Self::Reasoning { text: text.clone() },
            StreamEvent::ToolCallStart { id, name, .. } => Self::ToolStart {
                id: id.clone(),
                name: name.clone(),
                args_preview: String::new(),
            },
            StreamEvent::ToolCallDelta { .. } => {
                // Args are accumulated by run_turn and emitted as a single
                // ToolArgs event when the turn completes; deltas are not
                // streamed to the UI to avoid flicker.
                Self::ToolStart {
                    id: String::new(),
                    name: String::new(),
                    args_preview: String::new(),
                }
            }
            StreamEvent::Done { .. } => Self::Done,
            StreamEvent::Error { message, .. } => Self::Error { message: message.clone() },
        }
    }
}

// -------------------------------------------------------------------------
// Commands
// -------------------------------------------------------------------------

#[tauri::command]
pub async fn list_providers(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<ProviderSummary>, String> {
    let cfg = HubConfig::load().await.map_err(|e| e.to_string())?;
    let st = state.lock().await;
    let mut out = Vec::new();
    for entry in &cfg.providers {
        let id_str = entry.id.as_str().to_string();
        let models = match entry.id {
            ProviderId::MiniMax => line_hub_core::provider::MiniMaxProvider::new(entry)
                .map(|p| p.list_models())
                .unwrap_or_default(),
            ProviderId::OpenAI => line_hub_core::provider::OpenAIProvider::new(entry)
                .map(|p| p.list_models())
                .unwrap_or_default(),
            ProviderId::Anthropic => line_hub_core::provider::AnthropicProvider::new(entry)
                .map(|p| p.list_models())
                .unwrap_or_default(),
            ProviderId::Ollama => line_hub_core::provider::OllamaProvider::new(entry)
                .map(|p| p.list_models())
                .unwrap_or_default(),
        };
        out.push(ProviderSummary {
            is_configured: !entry.api_key.is_empty(),
            default_model: entry.default_model.clone(),
            id: id_str,
            label: label_for(entry.id),
            models,
        });
    }
    Ok(out)
}

fn label_for(id: ProviderId) -> String {
    match id {
        ProviderId::MiniMax => "MiniMax".into(),
        ProviderId::OpenAI => "OpenAI".into(),
        ProviderId::Anthropic => "Anthropic".into(),
        ProviderId::Ollama => "Ollama / LM Studio".into(),
    }
}

#[tauri::command]
pub async fn list_models(provider_id: String) -> Result<Vec<ModelInfo>, String> {
    let cfg = HubConfig::load().await.map_err(|e| e.to_string())?;
    let pid = parse_pid(&provider_id).map_err(|e| e.to_string())?;
    let entry = cfg
        .providers
        .iter()
        .find(|p| p.id == pid)
        .ok_or_else(|| format!("provider {provider_id} not configured"))?;
    let models = match pid {
        ProviderId::MiniMax => line_hub_core::provider::MiniMaxProvider::new(entry)
            .map(|p| p.list_models())
            .unwrap_or_default(),
        ProviderId::OpenAI => line_hub_core::provider::OpenAIProvider::new(entry)
            .map(|p| p.list_models())
            .unwrap_or_default(),
        ProviderId::Anthropic => line_hub_core::provider::AnthropicProvider::new(entry)
            .map(|p| p.list_models())
            .unwrap_or_default(),
        ProviderId::Ollama => line_hub_core::provider::OllamaProvider::new(entry)
            .map(|p| p.list_models())
            .unwrap_or_default(),
    };
    Ok(models)
}

#[tauri::command]
pub async fn load_config() -> Result<HubConfig, String> {
    HubConfig::load().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_config(cfg: HubConfig) -> Result<(), String> {
    cfg.save().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mcp_status(state: State<'_, Arc<Mutex<AppState>>>) -> Result<serde_json::Value, String> {
    let st = state.lock().await;
    Ok(serde_json::json!({
        "providers_loaded": st.providers.read().await.len(),
        "active_sessions": st.sessions.read().await.len(),
    }))
}

// -------------------------------------------------------------------------
// History (local SQLite-backed turn log)
// -------------------------------------------------------------------------

/// List every session in the history store, newest first.
#[tauri::command]
pub async fn list_history_sessions(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<line_hub_core::history::Session>, String> {
    let st = state.lock().await;
    st.history.list_sessions().map_err(|e| e.to_string())
}

/// Rename a session in the sidebar.
#[tauri::command]
pub async fn rename_history_session(
    state: State<'_, Arc<Mutex<AppState>>>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let st = state.lock().await;
    st.history
        .rename_session(&session_id, &title)
        .map_err(|e| e.to_string())
}

/// Delete a session and all of its turns.
#[tauri::command]
pub async fn delete_history_session(
    state: State<'_, Arc<Mutex<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let st = state.lock().await;
    st.history
        .delete_session(&session_id)
        .map_err(|e| e.to_string())
}

/// Load every turn for a session in order.
#[tauri::command]
pub async fn load_history_turns(
    state: State<'_, Arc<Mutex<AppState>>>,
    session_id: String,
) -> Result<Vec<line_hub_core::history::Turn>, String> {
    let st = state.lock().await;
    st.history.load_turns(&session_id).map_err(|e| e.to_string())
}

/// Append one turn to a session.
#[tauri::command]
pub async fn append_history_turn(
    state: State<'_, Arc<Mutex<AppState>>>,
    turn: line_hub_core::history::Turn,
) -> Result<(), String> {
    let st = state.lock().await;
    st.history.append_turn(&turn).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spawn_mcp(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<ToolDefinition>, String> {
    use line_hub_core::config::HubConfig;

    // Resolve the line-desktop-mcp entry path with this priority:
    //   1. `HUB_LINE_MCP_PATH` env var (CI / silent installs)
    //   2. `line_mcp_path` saved in HubConfig (the Settings dialog field)
    //   3. `LINE_MODEL_HUB_BUNDLED` env var pointing at a bundled install dir
    //      (the installer ships node_modules under resources/line-mcp/)
    //
    // If none of the three are set, we surface a *human-readable* error to the
    // UI rather than the cryptic env-var hint.
    let cfg = HubConfig::load().await.ok();
    let configured_path = cfg.as_ref().and_then(|c| c.line_mcp_path.clone());

    let path = std::env::var("HUB_LINE_MCP_PATH")
        .ok()
        .or_else(|| configured_path.clone())
        .or_else(|| {
            std::env::var("LINE_MODEL_HUB_BUNDLED")
                .ok()
                .map(|p| {
                    std::path::Path::new(&p)
                        .join("src")
                        .join("server.js")
                        .to_string_lossy()
                        .into_owned()
                })
        })
        .ok_or_else(|| {
            "Cannot find line-desktop-mcp. Open Settings and paste the path to \
             line-desktop-mcp\\src\\server.js (or set HUB_LINE_MCP_PATH)."
                .to_string()
        })?;

    let node = which_node().ok_or_else(|| {
        "node.exe not found in PATH. Install Node.js from https://nodejs.org/ \
         and restart LINE Model Hub."
            .to_string()
    })?;
    let mcp = McpClient::spawn(
        &node,
        &path,
        &[
            ("LINE_MCP_EXTENSIONS", "1"),
            ("LINE_MCP_SKIP_POSTINSTALL", "1"),
        ],
    )
    .await
    .map_err(|e| e.to_string())?;
    let tools = mcp.list_tools().await.map_err(|e| e.to_string())?;
    let mut st = state.lock().await;
    *st.mcp.write().await = Some(Arc::new(Mutex::new(Box::new(mcp))));
    Ok(tools)
}

#[tauri::command]
pub async fn shutdown_mcp(state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let st = state.lock().await;
    let mut guard = st.mcp.write().await;
    if let Some(mcp) = guard.take() {
        mcp.lock().await.shutdown().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn chat(
    app: AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    args: ChatArgs,
) -> Result<String, String> {
    let cfg = HubConfig::load().await.map_err(|e| e.to_string())?;
    let pid_str = args
        .provider_id
        .clone()
        .or_else(|| cfg.default_provider.as_ref().map(|p| p.as_str().to_string()))
        .ok_or_else(|| "no provider configured".to_string())?;
    let pid = parse_pid(&pid_str).map_err(|e| e.to_string())?;
    let entry = cfg
        .providers
        .iter()
        .find(|p| p.id == pid)
        .ok_or_else(|| format!("provider {pid_str} not configured"))?
        .clone();
    let model = args
        .model
        .clone()
        .or(entry.default_model.clone())
        .ok_or_else(|| "no model selected".to_string())?;

    let provider: Arc<dyn Provider> = match pid {
        ProviderId::MiniMax => Arc::new(line_hub_core::provider::MiniMaxProvider::new(&entry).map_err(|e| e.to_string())?),
        ProviderId::OpenAI => Arc::new(line_hub_core::provider::OpenAIProvider::new(&entry).map_err(|e| e.to_string())?),
        ProviderId::Anthropic => Arc::new(line_hub_core::provider::AnthropicProvider::new(&entry).map_err(|e| e.to_string())?),
        ProviderId::Ollama => Arc::new(line_hub_core::provider::OllamaProvider::new(&entry).map_err(|e| e.to_string())?),
    };

    // Get MCP tools if available
    let tools: Vec<ToolDefinition> = {
        let st = state.lock().await;
        let guard = st.mcp.read().await;
        if let Some(mcp) = guard.as_ref() {
            mcp.lock().await.list_tools().await.unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    // Build conversation with history from session
    let mut history: Vec<Message> = {
        let st = state.lock().await;
        let sessions = st.sessions.read().await;
        sessions.get(&args.session_id).map(|s| s.history.clone()).unwrap_or_default()
    };
    if history.is_empty() {
        // Lazily create the session record on first use.
        let mut st = state.lock().await;
        let mut sessions = st.sessions.write().await;
        sessions.entry(args.session_id.clone()).or_insert_with(|| crate::state::ChatSession {
            history: Vec::new(),
            model: model.clone(),
            provider_id: pid_str.clone(),
        });
    }
    history.push(Message::User {
        content: args.user_input.clone(),
    });

    let req = ChatRequest {
        model: model.clone(),
        messages: history,
        tools,
        temperature: 0.3,
        max_tokens: Some(2048),
    };

    let stream = provider.chat(req).await.map_err(|e| e.to_string())?;
    let mut stream = Box::pin(stream);

    // Stream to UI + collect text
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
        match &ev {
            StreamEvent::Delta { text } => {
                assistant_text.push_str(text);
                let _ = app.emit(&format!("chat:{}", args.session_id), UiEventPayload::from(&ev));
            }
            StreamEvent::ReasoningDelta { text } => {
                reasoning_text.push_str(text);
                let _ = app.emit(&format!("chat:{}", args.session_id), UiEventPayload::from(&ev));
            }
            _ => {
                let _ = app.emit(&format!("chat:{}", args.session_id), UiEventPayload::from(&ev));
            }
        }
        if matches!(ev, StreamEvent::Done { .. } | StreamEvent::Error { .. }) {
            break;
        }
    }

    // Append assistant message to history
    let st = state.lock().await;
    let mut sessions = st.sessions.write().await;
    if let Some(session) = sessions.get_mut(&args.session_id) {
        session.history.push(Message::Assistant {
            content: if assistant_text.is_empty() { None } else { Some(assistant_text.clone()) },
            reasoning: if reasoning_text.is_empty() { None } else { Some(reasoning_text) },
            tool_calls: None,
        });
    }

    Ok(assistant_text)
}

#[tauri::command]
pub async fn cancel_chat(
    state: State<'_, Arc<Mutex<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let st = state.lock().await;
    let cancel = st.cancel.read().await;
    if let Some(notify) = cancel.get(&session_id) {
        notify.notify_waiters();
    }
    Ok(())
}

fn parse_pid(s: &str) -> Result<ProviderId, String> {
    match s {
        "minimax" => Ok(ProviderId::MiniMax),
        "openai" => Ok(ProviderId::OpenAI),
        "anthropic" => Ok(ProviderId::Anthropic),
        "ollama" => Ok(ProviderId::Ollama),
        other => Err(format!("unknown provider id: {other}")),
    }
}

#[cfg(target_os = "windows")]
fn which_node() -> Option<String> {
    // On Windows, prefer `node.exe` from PATH; if not present, error gracefully.
    which("node.exe").or_else(|| which("node"))
}

#[cfg(not(target_os = "windows"))]
fn which_node() -> Option<String> {
    which("node")
}

fn which(name: &str) -> Option<String> {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .ok()
        .map(|o| if o.status.success() { name.to_string() } else { String::new() })
        .filter(|s| !s.is_empty())
}

// -------------------------------------------------------------------------
// Approval dialog (called from conversation loop via custom channel)
// -------------------------------------------------------------------------

/// Frontend-invokable confirmation gate: pops a native OS dialog asking the
/// user to OK/Cancel before any LINE send-style tool call runs. Returns
/// `true` only if the user pressed OK.
///
/// This is the safety net for the `send_message_auto` /
/// `send_message_manual` / `send_file_manual` paths. The frontend shows a
/// `<ToolCard>` for each tool call; when the card detects a send-class
/// tool it calls this command before letting the tool proceed.
#[tauri::command]
pub async fn request_send_confirm(
    app: AppHandle,
    args: SendConfirmArgs,
) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

    let body = format!(
        "Line 小幫手 想要呼叫「{tool}」\n\n\
         聊天室：{chat}\n\n\
         訊息：\n{text}\n\n\
         按「確定」允許送出，按「取消」拒絕。",
        tool = args.tool,
        chat = args.chatroom,
        text = if args.text.chars().count() > 400 {
            let truncated: String = args.text.chars().take(400).collect();
            format!("{truncated}…\n（已截斷，共 {} 字）", args.text.chars().count())
        } else {
            args.text.clone()
        },
    );

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(body)
        .title("LINE 訊息送出確認")
        .buttons(MessageDialogButtons::OkCancel)
        .show(move |ok| {
            let _ = tx.send(ok);
        });
    Ok(rx.await.unwrap_or(false))
}

#[derive(Debug, serde::Deserialize)]
pub struct SendConfirmArgs {
    pub chatroom: String,
    pub text: String,
    #[serde(default = "default_tool_name")]
    pub tool: String,
}

fn default_tool_name() -> String {
    "send_message_auto".to_string()
}