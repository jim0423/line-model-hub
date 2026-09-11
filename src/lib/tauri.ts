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