// Tauri command bindings — mirror of Rust commands in crates/line-hub-tauri/src/commands.rs

import { invoke } from "@tauri-apps/api/core";

export interface ModelInfo {
    id: string;
    label: string;
    tier: "flagship" | "balanced" | "reasoning" | "fast";
    description?: string | null;
    is_default: boolean;
}

export interface ProviderSummary {
    id: string;
    label: string;
    models: ModelInfo[];
    default_model?: string | null;
    is_configured: boolean;
}

export interface HubConfig {
    default_provider?: "minimax" | "openai" | "anthropic" | "ollama" | null;
    default_model?: string | null;
    providers: Array<{
        id: "minimax" | "openai" | "anthropic" | "ollama";
        api_key: string;
        base_url?: string | null;
        default_model?: string | null;
    }>;
    line_mcp_path?: string | null;
}

export interface ToolDefinition {
    name: string;
    description: string;
    parameters: any;
}

export const listProviders = () =>
    invoke<ProviderSummary[]>("list_providers");

export const listModels = (providerId: string) =>
    invoke<ModelInfo[]>("list_models", { providerId });

export const loadConfig = () => invoke<HubConfig>("load_config");

export const saveConfig = (cfg: HubConfig) =>
    invoke<void>("save_config", { cfg });

export const mcpStatus = () =>
    invoke<{ providers_loaded: number; active_sessions: number }>("mcp_status");

export const appendHistoryTurn = (turn: HistoryTurn) =>
    invoke<void>("append_history_turn", { turn });

export const listHistorySessions = () =>
    invoke<HistorySession[]>("list_history_sessions");

export const loadHistoryTurns = (sessionId: string) =>
    invoke<HistoryTurn[]>("load_history_turns", { sessionId });

export const renameHistorySession = (sessionId: string, title: string) =>
    invoke<void>("rename_history_session", { sessionId, title });

export const deleteHistorySession = (sessionId: string) =>
    invoke<void>("delete_history_session", { sessionId });

// History row shapes — mirror `line_hub_core::history`.
export interface HistoryTurn {
    session_id: string;
    seq: number;
    role: string;
    content: string;
    reasoning?: string | null;
    tool_trace?: string | null;
    ts: number;
}

export interface HistorySession {
    id: string;
    title: string;
    created_at: number;
    updated_at: number;
}

/**
 * Pop a native confirmation dialog before sending a LINE message.
 * Returns `true` only if the user pressed OK in the OS dialog.
 *
 * Wired to `request_send_confirm` in crates/line-hub-tauri/src/commands.rs
 * which uses `tauri-plugin-dialog::ask`. This is the user-facing safety
 * gate: any AI tool call that wants to deliver text to a real chatroom
 * must round-trip through here.
 */
export const requestSendConfirm = (args: {
    chatroom: string;
    text: string;
    tool?: string;
}) => invoke<boolean>("request_send_confirm", { args });

export const spawnMcp = () => invoke<ToolDefinition[]>("spawn_mcp");

export const shutdownMcp = () => invoke<void>("shutdown_mcp");

export const chat = (args: {
    session_id: string;
    user_input: string;
    provider_id?: string;
    model?: string;
}) => invoke<string>("chat", { args });

export const cancelChat = (sessionId: string) =>
    invoke<void>("cancel_chat", { sessionId });