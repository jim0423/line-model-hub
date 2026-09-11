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

// -------------------------------------------------------------------------
// Secure API key storage (OS keyring)
// -------------------------------------------------------------------------

/// Store an API key in the OS keyring (Windows Credential Manager / macOS
/// Keychain / Linux Secret Service). Empty keys delete the entry instead.
#[tauri::command]
pub async fn set_provider_keyring_key(
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    if api_key.is_empty() {
        line_hub_core::keyring::delete_api_key(&provider_id)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    line_hub_core::keyring::set_api_key(&provider_id, &api_key)
        .map_err(|e| e.to_string())
}

/// Returns a map of `provider_id -> bool` for the four canonical providers,
/// where `true` means a credential is currently stored. We do NOT echo the
/// secret out of the process — only presence/absence.
#[tauri::command]
pub async fn list_keyring_providers() -> Result<Vec<String>, String> {
    Ok(line_hub_core::keyring::list_configured_providers())
}

/// Explicitly delete a provider's key from the keyring. Useful when the
/// user wants to wipe a provider from their machine.
#[tauri::command]
pub async fn delete_provider_keyring_key(provider_id: String) -> Result<bool, String> {
    line_hub_core::keyring::delete_api_key(&provider_id).map_err(|e| e.to_string())
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
    // Inject the hub-managed system prompt if the caller didn't already
    // supply one. We prepend it as a fresh `Message::System`; providers
    // like Anthropic will hoist it to the top-level `system` field on
    // their next turn. This is the only place we control the model's
    // behaviour from outside the user's messages.
    let has_system = history.iter().any(|m| matches!(m, Message::System { .. }));
    if !has_system {
        let prompt = build_system_prompt(&tools, None);
        history.insert(0, Message::System { content: prompt });
    }

    history.push(Message::User {
        content: args.user_input.clone(),
    });

    let req = ChatRequest {
        model: model.clone(),
        messages: history,
        tools,
        temperature: 0.3,
        max_tokens: recommended_max_tokens(&pid_str),
    };

    let stream = provider.chat(req).await.map_err(|e| e.to_string())?;
    let mut stream = Box::pin(stream);

    // Install a cancellation token for this session so cancel_chat can stop us.
    let cancel_notify = Arc::new(tokio::sync::Notify::new());
    {
        let st = state.lock().await;
        st.cancel
            .write()
            .await
            .insert(args.session_id.clone(), cancel_notify.clone());
    }

    // Multi-session support: dispatch the stream into a detached background
    // task so the UI can switch sessions freely without blocking the chat.
    // Each session gets its own event channel (`chat:<session_id>`) which the
    // frontend listens to on demand.
    let session_id = args.session_id.clone();
    let provider_id_str = pid_str.clone();
    let app_clone = app.clone();
    let state_clone = state.inner().clone();

    tokio::spawn(async move {
        let mut assistant_text = String::new();
        let mut reasoning_text = String::new();
        // Streaming persistence — write a partial snapshot every
        // `STREAM_FLUSH_EVERY` delta events so a crash mid-stream does not
        // cost the user the whole response. The next flush overwrites via
        // `append_turn` (INSERT OR REPLACE on (session_id, seq)).
        const STREAM_FLUSH_EVERY: usize = 8;
        let mut dirty_events: usize = 0;

        loop {
            tokio::select! {
                _ = cancel_notify.notified() => {
                    let _ = app_clone.emit(
                        &format!("chat:{session_id}"),
                        UiEventPayload::from(&StreamEvent::Error {
                            message: "cancelled by user".into(),
                            retriable: false,
                        }),
                    );
                    break;
                }
                ev = futures::StreamExt::next(&mut stream) => {
                    let ev = match ev {
                        Some(ev) => ev,
                        None => break,
                    };
                    match &ev {
                        StreamEvent::Delta { text } => assistant_text.push_str(text),
                        StreamEvent::ReasoningDelta { text } => reasoning_text.push_str(text),
                        _ => {}
                    }
                    let _ = app_clone.emit(
                        &format!("chat:{session_id}"),
                        UiEventPayload::from(&ev),
                    );
                    if matches!(
                        ev,
                        StreamEvent::Done { .. } | StreamEvent::Error { .. }
                    ) {
                        break;
                    }
                    // Periodically flush a partial assistant turn to the
                    // history store so a crash mid-stream does not lose the
                    // work. We coalesce events to avoid hammering SQLite.
                    dirty_events += 1;
                    if dirty_events >= STREAM_FLUSH_EVERY {
                        dirty_events = 0;
                        let st_flush = state_clone.lock().await;
                        let partial = line_hub_core::history::Turn {
                            session_id: session_id.clone(),
                            seq: std::i64::MAX, // sentinel — REPLACE wins
                            role: "assistant".into(),
                            content: assistant_text.clone(),
                            reasoning: if reasoning_text.is_empty() {
                                None
                            } else {
                                Some(reasoning_text.clone())
                            },
                            tool_trace: None,
                            ts: chrono::Utc::now().timestamp_millis(),
                        };
                        let _ = st_flush.history.append_turn(&partial);
                    }
                }
            }
        }

        // Persist assistant turn back into the in-memory session history.
        let st = state_clone.lock().await;
        let mut sessions = st.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.history.push(Message::Assistant {
                content: if assistant_text.is_empty() {
                    None
                } else {
                    Some(assistant_text.clone())
                },
                reasoning: if reasoning_text.is_empty() {
                    None
                } else {
                    Some(reasoning_text)
                },
                tool_calls: None,
            });
        }
        // Drop the cancellation token for this session — chat is over.
        let mut cancel = st.cancel.write().await;
        cancel.remove(&session_id);
        let _ = provider_id_str; // silence unused warning if future-proofed
    });

    // Return immediately so the frontend can keep typing / switching sessions.
    Ok(args.session_id.clone())
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

/// Recommended `max_tokens` per provider. Each provider has its own ceiling
/// — Anthropic demands an explicit value (default 8192), MiniMax / OpenAI
/// tolerate a more conservative budget, and Ollama depends on the local
/// model but a sane default of 4096 keeps streaming responsive.
fn recommended_max_tokens(provider_id: &str) -> Option<u32> {
    match provider_id {
        // Anthropic requires an explicit `max_tokens` — pick a generous
        // budget so we rarely truncate mid-response.
        "anthropic" => Some(8192),
        // MiniMax M3 supports up to 32k context; budget for the full
        // streaming response plus tool calls.
        "minimax" => Some(4096),
        // OpenAI defaults to whatever the model thinks is appropriate.
        "openai" => Some(4096),
        // Ollama models vary — pick a conservative budget that keeps the
        // local experience snappy.
        "ollama" => Some(4096),
        _ => Some(2048),
    }
}

/// Build a compact system prompt that primes the model to use the tools
/// correctly without burning thousands of input tokens on the full schema.
///
/// We deliberately keep this short — the full schemas ride alongside in the
/// request's `tools` field, so the model only needs to know:
///   1. The available categories
///   2. The send-confirm guard exists
///   3. The chat_id alias for the active chatroom
fn build_system_prompt(tools: &[ToolDefinition], active_chat: Option<&str>) -> String {
    let mut cats: Vec<&str> = Vec::new();
    for t in tools {
        let cat = categorize_tool(&t.name);
        if !cats.contains(&cat) {
            cats.push(cat);
        }
    }
    let cats_list = if cats.is_empty() {
        "(no tools loaded yet — call get_line_capabilities)".to_string()
    } else {
        cats.join(", ")
    };

    let chat_hint = match active_chat {
        Some(c) => format!("\n\nActive chatroom: `{c}`. Pass this as `chatroom` for send_* tools unless the user names a different target."),
        None => "\n\nNo chatroom is currently active in the UI — when the user asks to send something, ask which chatroom first.".to_string(),
    };

    format!(
        "You are Line 小幫手, a LINE Desktop assistant.\n\
         \n\
         Available tool categories: {cats_list}.\n\
         \n\
         Important guardrails:\n\
         - Any tool whose name starts with `send_`, `stage_`, or overwrites a draft REQUIRES explicit user confirmation before being invoked. The app surfaces a native dialog; never claim a message was sent without seeing a successful tool result.\n\
         - Read tools (`get_*`, `search_*`, `verify_*`, `copy_*`, `translate_*`) are free to call.\n\
         - Use `get_line_capabilities` to discover what is currently wired up before relying on a tool.\n\
         - For bulk operations (export, history lookup) prefer a single tool call over several speculative ones.\n\
         \n\
         Respond in the language the user writes in (繁體中文 by default).{chat_hint}"
    )
}

fn categorize_tool(name: &str) -> &'static str {
    if name.starts_with("send_") || name == "stage_line_reply" || name == "stage_line_forward" {
        "send (guarded)"
    } else if name.starts_with("set_line_draft")
        || name == "get_line_draft"
        || name == "clear_line_draft"
    {
        "draft"
    } else if name.starts_with("export_") {
        "export"
    } else if name.starts_with("search_") || name.starts_with("verify_") {
        "search/verify"
    } else if name.starts_with("get_line_chat") || name.starts_with("get_line_chatroom_history") {
        "read history"
    } else if name.starts_with("copy_")
        || name.starts_with("translate_")
        || name.starts_with("send_file_")
    {
        "compose"
    } else if name.starts_with("open_") {
        "navigate"
    } else if name.starts_with("get_line_") {
        "status/capabilities"
    } else {
        "other"
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