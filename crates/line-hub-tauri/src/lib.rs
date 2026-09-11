//! LINE Model Hub — Tauri shell.
//!
//! This crate wraps `line-hub-core` for desktop use. It exposes:
//!   - Tauri commands the React frontend calls via `invoke()`
//!   - Tauri events streamed from Rust → React (chat tokens, tool trace)
//!   - A native approval dialog before any LINE send
//!
//! Architecture:
//!     React UI
//!        │  invoke()
//!        ▼
//!     Tauri commands  ──►  line-hub-core  ──►  AI provider (M3/OpenAI/...)
//!        ▲                            │
//!        │ events                     ▼
//!     React        line-desktop-mcp child process

mod commands;
mod state;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,line_hub_core=debug")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let state = AppState::new();
            app.manage(Arc::new(Mutex::new(state)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_models,
            commands::list_providers,
            commands::load_config,
            commands::save_config,
            commands::mcp_status,
            commands::spawn_mcp,
            commands::shutdown_mcp,
            commands::chat,
            commands::cancel_chat,
            commands::request_send_confirm,
            commands::list_history_sessions,
            commands::rename_history_session,
            commands::delete_history_session,
            commands::load_history_turns,
            commands::append_history_turn,
            commands::set_provider_keyring_key,
            commands::list_keyring_providers,
            commands::delete_provider_keyring_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LINE Model Hub");
}