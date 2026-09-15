//! Tauri commands — IPC surface between React frontend and Rust core.

use crate::state::AppState;
use line_hub_core::config::{HubConfig, ProviderEntry};
use line_hub_core::conversation::{self, LoopConfig, SendGuard, UiEvent};
use line_hub_core::mcp::{McpClient, McpTool, ToolResult, ToolResultBlock};
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

/// Attachment block surfaced to the UI from a tool call's response.
///
/// We mirror the [`ToolResultBlock`] shape but serialise image bytes as
/// a base64 data-URL so Tauri's Tauri event channel can carry them to
/// the React frontend without any extra file:// plumbing on the way.
/// The frontend reverses the encoding back into `<img src=...>`.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiAttachment {
    Text {
        text: String,
    },
    Image {
        /// data: URL — `data:image/png;base64,XXXX`.
        src: String,
        mime_type: String,
        bytes: usize,
    },
    Audio {
        src: String,
        mime_type: String,
        bytes: usize,
    },
    Unsupported {
        mime_type: String,
        note: String,
    },
}

impl From<&ToolResultBlock> for UiAttachment {
    fn from(b: &ToolResultBlock) -> Self {
        match b {
            ToolResultBlock::Text { text } => Self::Text { text: text.clone() },
            ToolResultBlock::Image { mime_type, data } => {
                use base64::engine::general_purpose::STANDARD;
                use base64::Engine;
                let encoded = STANDARD.encode(data);
                Self::Image {
                    src: format!("data:{mime_type};base64,{encoded}"),
                    mime_type: mime_type.clone(),
                    bytes: data.len(),
                }
            }
            ToolResultBlock::Audio { mime_type, data } => {
                use base64::engine::general_purpose::STANDARD;
                use base64::Engine;
                let encoded = STANDARD.encode(data);
                Self::Audio {
                    src: format!("data:{mime_type};base64,{encoded}"),
                    mime_type: mime_type.clone(),
                    bytes: data.len(),
                }
            }
            ToolResultBlock::Unsupported { mime_type, note } => Self::Unsupported {
                mime_type: mime_type.clone(),
                note: note.clone(),
            },
        }
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventPayload {
    Delta { text: String },
    Reasoning { text: String },
    ToolStart { id: String, name: String, args_preview: String },
    ToolArgs {
        id: String,
        args: serde_json::Value,
        /// Number of attachments the model received. Surfaced so the
        /// UI can show a 📎 badge before any decoding happens.
        attachment_count: usize,
    },
    ToolDone {
        id: String,
        name: String,
        result_preview: String,
        attachments: Vec<UiAttachment>,
    },
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
        // v0.6.1: provider is configured if EITHER the keyring has a
        // secret OR the config.json has plaintext (legacy / dev mode).
        // This keeps the Settings ⚠ badge honest across both paths.
        let keyring_hit = line_hub_core::keyring::get_api_key(&id_str)
            .ok()
            .flatten()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        out.push(ProviderSummary {
            is_configured: keyring_hit || !entry.api_key.is_empty(),
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
    // v0.6.1: mirror non-empty API keys to the OS keyring so they are
    // encrypted at rest and never persist as plaintext JSON. Empty keys
    // delete the entry instead — keeps Settings "clear key" working.
    for entry in &cfg.providers {
        let id = entry.id.as_str();
        if entry.api_key.is_empty() {
            // Best-effort delete: if there is nothing stored, ignore the
            // resulting `Ok(false)` — we do not want a missing entry to
            // fail the whole save.
            let _ = line_hub_core::keyring::delete_api_key(id);
        } else {
            line_hub_core::keyring::set_api_key(id, &entry.api_key)
                .map_err(|e| format!("keyring set({id}): {e}"))?;
        }
    }
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
pub async fn set_local_only(
    state: State<'_, Arc<Mutex<AppState>>>,
    enabled: bool,
) -> Result<bool, String> {
    // Update the runtime flag while holding the lock, then drop the
    // lock before the second `await` so we never hold a `MutexGuard`
    // across a config save roundtrip.
    {
        let st = state.lock().await;
        let mut g = st.local_only.write().await;
        *g = enabled;
    }
    // Persist to the on-disk config so the toggle survives a relaunch.
    let mut cfg = HubConfig::load().await.unwrap_or_default();
    cfg.local_only = enabled;
    cfg.save().await.map_err(|e| format!("save config: {e}"))?;
    tracing::info!(enabled, "local_only mode toggled");
    Ok(enabled)
}

#[tauri::command]
pub async fn get_local_only(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<bool, String> {
    let st = state.lock().await;
    let g = st.local_only.read().await;
    Ok(*g)
}

#[tauri::command]
pub async fn spawn_mcp(
    app: AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<ToolDefinition>, String> {
    use line_hub_core::config::HubConfig;

    // v0.6.8: resolve the line-desktop-mcp entry path with four tiers,
    // highest priority first:
    //
    //   1. `HUB_LINE_MCP_PATH` env var — CI smoke tests + power-users.
    //   2. `line_mcp_path` saved in HubConfig (Settings dialog field).
    //   3. `LINE_MODEL_HUB_BUNDLED` env var → `src/server.js` — for dev
    //      who happen to have the source tree checked out somewhere
    //      outside the installer's data dir.
    //   4. `crate::vendor::ensure_vendor_installed()` — extracts the
    //      embedded source tree baked into the exe by `build.rs` on first
    //      launch and runs `npm install` once per machine.
    //
    // Tiers 3 and 4 historically failed because the NSIS bundler
    // silently dropped `bundle.resources` (verified in CI run
    // 34828069067 log line 1465). Embedding in build.rs and unpacking
    // on first launch sidesteps the bundler entirely.
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
        .or_else(|| {
            // Last resort: extract the embedded vendor tree to a
            // per-user data dir and run npm install there.
            let identifier = app.config().identifier.clone();
            Some(candidate_vendor_path(identifier))
        })
        .ok_or_else(|| {
            "Cannot find line-desktop-mcp. Open Settings and paste the path to \
             line-desktop-mcp\\src\\server.js (or set HUB_LINE_MCP_PATH)."
                .to_string()
        })?;

    let resolved_path = if std::path::Path::new(&path).exists() {
        // Tiers 1-3 already point at a real file on disk — use as-is.
        path
    } else {
        // Tier 4 marker: paths ending in `<...>/src/server.js` that do
        // not yet exist on disk trigger the vendor-extract flow.
        let identifier = app.config().identifier.clone();
        crate::vendor::ensure_vendor_installed(&identifier)
            .await
            .map_err(|e| format!("vendor extract failed: {e}"))?
            .to_string_lossy()
            .into_owned()
    };

    let node = which_node().ok_or_else(|| {
        "node.exe not found in PATH. Install Node.js from https://nodejs.org/ \
         and restart LINE Model Hub."
            .to_string()
    })?;
    let mcp = McpClient::spawn(
        &node,
        &resolved_path,
        &[
            ("LINE_MCP_EXTENSIONS", "1"),
            ("LINE_MCP_SKIP_POSTINSTALL", "1"),
        ],
    )
    .await
    .map_err(|e| e.to_string())?;
    let tools = mcp.list_tools().await.map_err(|e| e.to_string())?;
    let mut st = state.lock().await;
    *st.mcp.write().await = Some(Arc::new(mcp) as Arc<dyn McpTool>);
    Ok(tools)
}

/// Stand-in path used solely so `or_else(|| Some(...))` above can pick
/// the vendor branch. The actual on-disk resolution happens in
/// `spawn_mcp` once we know whether the path already exists.
fn candidate_vendor_path(_identifier: String) -> String {
    // Sentinel: spawn_mcp checks `Path::new(&path).exists()` before
    // returning; this value never reaches McpClient.
    std::path::Path::new("vendor")
        .join("line-desktop-mcp")
        .join("src")
        .join("server.js")
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub async fn shutdown_mcp(state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let st = state.lock().await;
    let mut guard = st.mcp.write().await;
    if let Some(mcp) = guard.take() {
        mcp.shutdown().await;
    }
    Ok(())
}

/// Thin wrapper around the `get_line_capabilities` MCP tool.
///
/// Called by the frontend right after `spawn_mcp` succeeds to render a
/// "LINE feature map" card (which tools can run on this LINE build, which
/// require the optional `LINE_MCP_CUA_DRIVER` / `LINE_MCP_PYTHON` deps,
/// which are explicitly unavailable on the current platform, etc.). The
/// MCP tool itself is read-only and listed by default in v3.0.0, so it's
/// safe to call as the very first interaction after spawn.
///
/// Falls back to a synthetic unavailable payload if the MCP child is not
/// running, which lets the UI show a "spawn MCP first" hint without
/// crashing on cold start.
#[tauri::command]
pub async fn fetch_capabilities(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<serde_json::Value, String> {
    let st = state.lock().await;
    let guard = st.mcp.read().await;
    let Some(mcp_arc) = guard.as_ref().cloned() else {
        return Ok(serde_json::json!({
            "ok": false,
            "code": "LINE_MCP_NOT_SPAWNED",
            "note": "Spawn LINE MCP first.",
        }));
    };
    // Drop the read lock before the await so a long-running MCP call does
    // not block other readers (e.g. `chat()` cloning its own snapshot).
    drop(guard);
    let raw = mcp_arc
        .call_tool("get_line_capabilities", serde_json::json!({"mode": "all"}))
        .await
        .map_err(|e| format!("get_line_capabilities failed: {e}"))?;
    Ok(raw)
}

#[tauri::command]
pub async fn chat(
    app: AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    args: ChatArgs,
) -> Result<String, String> {
    // -- Resolve provider / model ---------------------------------------
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

    // -- Snapshot the MCP client (if any) -------------------------------
    // run_turn needs `&Arc<dyn McpTool>`, so we clone the Arc out of the
    // state lock before any await. If no MCP has been spawned, we fall
    // back to a no-op proxy that returns an empty tool list — the
    // conversation loop then degenerates to a single non-agentic turn,
    // which matches the v0.4.0 behaviour when no MCP was running.
    let mcp_arc: Arc<dyn McpTool> = {
        let st = state.lock().await;
        let guard = st.mcp.read().await;
        guard
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Arc::new(NoopMcp) as Arc<dyn McpTool>)
    };

    // -- Conversation history + system prompt ---------------------------
    let mut history: Vec<Message> = {
        let st = state.lock().await;
        let sessions = st.sessions.read().await;
        sessions
            .get(&args.session_id)
            .map(|s| s.history.clone())
            .unwrap_or_default()
    };
    if history.is_empty() {
        // Lazily create the session record on first use so the model
        // and provider are remembered for the next turn.
        let st = state.lock().await;
        let mut sessions = st.sessions.write().await;
        sessions
            .entry(args.session_id.clone())
            .or_insert_with(|| crate::state::ChatSession {
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
        let tools_for_prompt = mcp_arc.list_tools().await.unwrap_or_default();
        let local_only = *state.lock().await.local_only.read().await;
        let prompt = build_system_prompt(&tools_for_prompt, None, local_only);
        history.insert(0, Message::System { content: prompt });
    }

    // -- Loop config + send guard ---------------------------------------
    let loop_cfg = LoopConfig {
        max_tool_turns: 8,
        model: model.clone(),
        temperature: 0.3,
        max_tokens: recommended_max_tokens(&pid_str),
    };
    let send_guard = TauriSendGuard { app: app.clone() };

    // -- Cancellation token (registered before the spawn so cancel_chat
    //    can find it the instant the user hits Stop) ---------------------
    let cancel_notify = Arc::new(tokio::sync::Notify::new());
    {
        let st = state.lock().await;
        st.cancel
            .write()
            .await
            .insert(args.session_id.clone(), cancel_notify.clone());
    }

    // -- Move everything into the spawn ---------------------------------
    let session_id = args.session_id.clone();
    let user_input = args.user_input.clone();
    let app_for_spawn = app.clone();
    let state_for_spawn = state.inner().clone();
    let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Backing buffers the closure mutates; we keep one Arc clone in the
    // outer scope so the cancel branch can read whatever was emitted
    // right before the user hit Stop.
    let assistant_text_buf = Arc::new(std::sync::Mutex::new(String::new()));
    let reasoning_text_buf = Arc::new(std::sync::Mutex::new(String::new()));

    tokio::spawn(async move {
        // -- Drive the agentic loop under a cancel-aware select ---------
        let run_fut = conversation::run_turn(
            &*provider,
            &mcp_arc,
            &send_guard,
            &loop_cfg,
            &mut history,
            &user_input,
            on_event_fn(
                cancel_flag.clone(),
                assistant_text_buf.clone(),
                reasoning_text_buf.clone(),
                session_id.clone(),
                app_for_spawn.clone(),
                state_for_spawn.clone(),
            ),
        );

        tokio::select! {
            run_result = run_fut => {
                if let Err(e) = run_result {
                    let _ = app_for_spawn.emit(
                        &format!("chat:{session_id}"),
                        UiEventPayload::Error { message: e.to_string() },
                    );
                }
            }
            _ = cancel_notify.notified() => {
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                // run_fut is dropped here, which cancels any in-flight
                // MCP call / SSE stream the loop was waiting on. The
                // provider's history is updated only when run_turn
                // completes a turn; we synthesise the partial assistant
                // message below so the cancel state is recoverable.
                let text_snapshot = assistant_text_buf
                    .lock()
                    .expect("assistant_text mutex poisoned")
                    .clone();
                let reasoning_snapshot = reasoning_text_buf
                    .lock()
                    .expect("reasoning_text mutex poisoned")
                    .clone();
                history.push(Message::Assistant {
                    content: if text_snapshot.is_empty() {
                        None
                    } else {
                        Some(text_snapshot)
                    },
                    reasoning: if reasoning_snapshot.is_empty() {
                        None
                    } else {
                        Some(reasoning_snapshot)
                    },
                    tool_calls: None,
                });
                let _ = app_for_spawn.emit(
                    &format!("chat:{session_id}"),
                    UiEventPayload::Error { message: "cancelled by user".into() },
                );
            }
        }

        // -- Persist final history back into the in-memory session ------
        let st = state_for_spawn.lock().await;
        if let Some(session) = st.sessions.write().await.get_mut(&session_id) {
            session.history = history;
        }
        // Drop the cancellation token for this session — chat is over.
        st.cancel.write().await.remove(&session_id);
    });

    // Return immediately so the frontend can keep typing / switching sessions.
    Ok(args.session_id.clone())
}

/// Helper: build the sync `FnMut(UiEvent) + Send` adapter that
/// `conversation::run_turn` requires. Kept as a free function so the
/// closure's environment stays readable instead of being inlined into
/// the spawn body.
fn on_event_fn(
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    assistant_text_buf: Arc<std::sync::Mutex<String>>,
    reasoning_text_buf: Arc<std::sync::Mutex<String>>,
    session_id: String,
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
) -> impl FnMut(UiEvent) + Send {
    const STREAM_FLUSH_EVERY: usize = 8;
    let mut dirty_events: usize = 0;
    move |ev: UiEvent| {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let is_delta = matches!(&ev, UiEvent::Delta(_));
        match &ev {
            UiEvent::Delta(t) => assistant_text_buf
                .lock()
                .expect("assistant_text mutex poisoned")
                .push_str(t),
            UiEvent::Reasoning(t) => reasoning_text_buf
                .lock()
                .expect("reasoning_text mutex poisoned")
                .push_str(t),
            _ => {}
        }
        let payload = ui_event_to_payload(ev);
        let _ = app.emit(&format!("chat:{session_id}"), payload);

        if is_delta {
            dirty_events += 1;
            if dirty_events >= STREAM_FLUSH_EVERY {
                dirty_events = 0;
                let st_clone = state.clone();
                let session_id_clone = session_id.clone();
                let text_snapshot = assistant_text_buf
                    .lock()
                    .expect("assistant_text mutex poisoned")
                    .clone();
                let reasoning_snapshot = reasoning_text_buf
                    .lock()
                    .expect("reasoning_text mutex poisoned")
                    .clone();
                tokio::spawn(async move {
                    let st = st_clone.lock().await;
                    let partial = line_hub_core::history::Turn {
                        session_id: session_id_clone,
                        seq: std::i64::MAX, // sentinel — REPLACE wins
                        role: "assistant".into(),
                        content: text_snapshot,
                        reasoning: if reasoning_snapshot.is_empty() {
                            None
                        } else {
                            Some(reasoning_snapshot)
                        },
                        tool_trace: None,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    let _ = st.history.append_turn(&partial);
                });
            }
        }
    }
}

/// Map a `conversation::UiEvent` (the agentic loop's progress signal)
/// to the corresponding `UiEventPayload` that ships over the Tauri
/// event channel. Kept as a free function so the closure body in
/// `on_event_fn` stays focused on emission + persistence.
fn ui_event_to_payload(ev: UiEvent) -> UiEventPayload {
    match ev {
        UiEvent::Delta(text) => UiEventPayload::Delta { text },
        UiEvent::Reasoning(text) => UiEventPayload::Reasoning { text },
        UiEvent::ToolStart {
            id,
            name,
            args_preview,
        } => UiEventPayload::ToolStart {
            id,
            name,
            args_preview,
        },
        UiEvent::ToolArgs {
            id,
            args,
            attachment_count,
        } => UiEventPayload::ToolArgs {
            id,
            args,
            attachment_count,
        },
        UiEvent::ToolDone {
            id,
            name,
            result_preview,
            attachments,
        } => {
            // Typed blocks ride through as `UiAttachment`s; the React
            // side reverses the base64 data URLs back into `<img>` /
            // `<audio>` tags.
            let ui_attachments: Vec<UiAttachment> =
                attachments.iter().map(UiAttachment::from).collect();
            UiEventPayload::ToolDone {
                id,
                name,
                result_preview,
                attachments: ui_attachments,
            }
        }
        UiEvent::SendBlocked { chat, message } => UiEventPayload::SendBlocked { chat, message },
        UiEvent::Done => UiEventPayload::Done,
        UiEvent::Error(message) => UiEventPayload::Error { message },
    }
}

/// `SendGuard` that pops a native OS confirmation dialog before any
/// LINE-sending tool runs. The agentic loop calls `approve_send`
/// synchronously; we return an async future that resolves once the
/// user clicks OK or Cancel.
struct TauriSendGuard {
    app: AppHandle,
}

#[async_trait::async_trait]
impl SendGuard for TauriSendGuard {
    async fn approve_send(
        &self,
        chat: &str,
        message: &str,
        tool: &str,
    ) -> line_hub_core::HubResult<bool> {
        let body = format!(
            "Line 小幫手 想要呼叫「{tool}」\n\n\
             聊天室：{chat}\n\n\
             訊息：\n{text}\n\n\
             按「確定」允許送出，按「取消」拒絕。",
            text = if message.chars().count() > 400 {
                let truncated: String = message.chars().take(400).collect();
                format!("{truncated}…\n（已截斷，共 {} 字）", message.chars().count())
            } else {
                message.to_string()
            },
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.app
            .dialog()
            .message(body)
            .title("LINE 訊息送出確認")
            .buttons(MessageDialogButtons::OkCancel)
            .show(move |ok| {
                let _ = tx.send(ok);
            });
        Ok(rx.await.unwrap_or(false))
    }
}

/// Fallback `McpTool` used when the user has not yet spawned a real
/// `McpClient`. Returns an empty tool list so the agentic loop
/// degenerates to a single non-agentic turn (matching the v0.4.0
/// behaviour when MCP was offline).
struct NoopMcp;

#[async_trait::async_trait]
impl McpTool for NoopMcp {
    async fn list_tools(
        &self,
    ) -> line_hub_core::HubResult<
        Vec<line_hub_core::provider::ToolDefinition>,
    > {
        Ok(Vec::new())
    }

    async fn call_tool(
        &self,
        _name: &str,
        _args: serde_json::Value,
    ) -> line_hub_core::HubResult<serde_json::Value> {
        Err(line_hub_core::HubError::McpTransport(
            "no MCP server running — call spawn_mcp first".into(),
        ))
    }

    async fn call_tool_structured(
        &self,
        _name: &str,
        _args: serde_json::Value,
    ) -> line_hub_core::HubResult<ToolResult> {
        Ok(ToolResult::Err {
            message: "no MCP server running — call spawn_mcp first".into(),
        })
    }

    fn tool_registry(&self) -> line_hub_core::mcp::ToolRegistry {
        line_hub_core::mcp::ToolRegistry::default()
    }

    async fn shutdown(&self) {}
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
fn build_system_prompt(tools: &[ToolDefinition], active_chat: Option<&str>, local_only: bool) -> String {
    let mut cats: Vec<&str> = Vec::new();
    for t in tools {
        if local_only && is_send_guarded_tool(&t.name) {
            // Local-only users have not installed the LINE GUI prerequisites
            // (CUA Driver, Python, SQLite3MC), so the bridge refuses any
            // tool that mutates chat state. Hide them from the prompt
            // entirely instead of letting the model waste a turn on a
            // tool that will inevitably fail at runtime.
            continue;
        }
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
         - For chat history, prefer `get_line_local_messages` (the v3.0.0 default — 31-day window, cursor pagination, optional image previews). Avoid the older `get_line_chatroom_history_*` paging tools unless `get_line_local_messages` is unavailable on the current LINE build.\n\
         - For visual workflows that need user eyeball confirmation (`confirm_line_chat_view`, `confirm_line_reply_source_target`, `get_line_ui_state`, `get_line_poll_state`), the user MUST look at the returned screenshot before the corresponding token is issued — never accept a confirmation without that screenshot.\n\
         - Use `get_line_workflow` + `prepare_line_workflow` to plan mentions, replies and polls BEFORE executing; both are pure validators that don't touch LINE.\n\
         - Use `get_line_capabilities` to discover what is currently wired up before relying on a tool.\n\
         - For bulk operations (export, history lookup) prefer a single tool call over several speculative ones.\n\
         - All date / time filters are interpreted in Asia/Taipei (UTC+08:00).\n\
         \n\
         Respond in the language the user writes in (繁體中文 by default).{chat_hint}"
    )
}

/// v0.6.0: which tools mutate LINE state and therefore need the GUI
/// prerequisites (`LINE_MCP_CUA_DRIVER` + `LINE_MCP_PYTHON` +
/// `LINE_MCP_SQLITE3MC_DLL`). Local-only mode filters these out of the
/// system prompt so users who haven't installed them don't burn turns
/// on tools that will return `LINE_CHAT_VERIFICATION_UNAVAILABLE`.
///
/// Mirrors the SEND_TOOLS list in `line_hub_core::conversation` so that
/// any drift between the two will be caught by the integration tests.
fn is_send_guarded_tool(name: &str) -> bool {
    if name.starts_with("send_") {
        return true;
    }
    matches!(
        name,
        "stage_line_reply"
            | "stage_line_forward"
            | "set_line_draft"
            | "clear_line_draft"
            | "open_line_chat"
            | "open_line_chat_feature"
            | "get_line_draft"
            | "export_line_chat_history"
    )
}

/// Maps one tool name to a short category label used by `build_system_prompt`.
///
/// Tool names track the 29 tools exposed by `line-desktop-mcp` v3.0.0
/// (up from 24 in v1.x). The categories are kept coarse on purpose so
/// the system prompt stays compact even with 29 tools loaded.
fn categorize_tool(name: &str) -> &'static str {
    if name.starts_with("send_") || name == "stage_line_reply" || name == "stage_line_forward" {
        "send (guarded)"
    } else if matches!(
        name,
        "set_line_draft" | "get_line_draft" | "clear_line_draft"
    ) {
        "draft"
    } else if name.starts_with("export_") {
        "export"
    } else if name.starts_with("search_") || name.starts_with("verify_") {
        "search/verify"
    } else if matches!(
        name,
        "get_line_chat_messages"
            | "get_line_chatroom_history_short"
            | "get_line_chatroom_history_default"
            | "get_line_chatroom_history_long"
    ) {
        "read history"
    } else if name == "get_line_local_messages" {
        // v3.0.0's flagship read tool — keep its own bucket so the
        // system prompt can hint to prefer it over the older paging tools.
        "read history (local + media preview)"
    } else if matches!(
        name,
        "copy_line_message"
            | "translate_line_message"
            | "send_file_manual"
            | "stage_line_reply"
            | "stage_line_forward"
    ) {
        "compose"
    } else if matches!(
        name,
        "open_line_chat"
            | "open_line_chat_feature"
            | "get_line_capabilities"
            | "get_line_status"
    ) {
        "navigate / status"
    } else if matches!(
        name,
        "get_line_workflow" | "prepare_line_workflow"
    ) {
        "workflow planning"
    } else if matches!(
        name,
        "get_line_ui_state"
            | "confirm_line_chat_view"
            | "get_line_reply_source_target"
            | "confirm_line_reply_source_target"
    ) {
        "visual confirmation (caller must inspect screenshot)"
    } else if name == "get_line_poll_state" {
        "polls (read-only)"
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