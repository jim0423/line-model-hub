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
mod respawn;
mod state;
pub mod vendor;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use std::io::Write;
use tracing::info;

    let fmt_layer = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,line_hub_core=debug")),
        );

    // v0.6.14: GUI mode on Windows detaches stdout, so tracing log
    // messages would otherwise disappear into the void. Tee every log
    // line into `%LOCALAPPDATA%\line-hub\line-hub.log` (best-effort)
    // so users can read it back without having to relaunch from a
    // console. File path is fixed to the same dir the vendor
    // extraction fallback resolves to, so it sits next to any
    // diagnostic data Jim might want.
    let log_file_path = std::env::temp_dir().join("line-hub-line-hub.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .ok();
    let log_file = file.as_ref().map(|f| {
        let mf = f.metadata().ok();
        // Truncate if over 1 MB to avoid runaway growth.
        if let Some(m) = mf {
            if m.len() > 1_048_576 {
                let _ = std::fs::File::create(&log_file_path);
            }
        }
        f.try_clone().ok()
    }).flatten();

    let make_writer = move || -> Box<dyn Write + Send> {
        if let Some(ref f) = log_file {
            Box::new(f.try_clone().unwrap())
        } else {
            Box::new(std::io::sink())
        }
    };

    if let Some(_f) = file {
        fmt_layer
            .with_writer(make_writer)
            .init();
    } else {
        fmt_layer.init();
    }
    info!("line-hub tracing initialised; log file: {}", log_file_path.display());

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
            commands::fetch_capabilities,
            commands::set_local_only,
            commands::get_local_only,
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