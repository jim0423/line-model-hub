import { useEffect, useState } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import {
    cancelChat,
    chat,
    HubConfig,
    listProviders,
    ModelInfo,
    ProviderSummary,
    saveConfig,
    spawnMcp,
} from "./lib/tauri";

interface UiEvent {
    kind:
        | "delta"
        | "reasoning"
        | "tool_start"
        | "tool_args"
        | "tool_done"
        | "send_blocked"
        | "done"
        | "error";
    text?: string;
    id?: string;
    name?: string;
    args_preview?: string;
    args?: any;
    result_preview?: string;
    chat?: string;
    message?: string;
    message_text?: string;
}

interface TurnMessage {
    role: "user" | "assistant";
    content: string;
    reasoning?: string;
    toolTrace?: ToolCallTrace[];
    partial?: boolean;
}

interface ToolCallTrace {
    id: string;
    name: string;
    args?: any;
    args_preview?: string;
    result_preview?: string;
    blocked?: boolean;
}

const SESSION = "default";

export default function App() {
    const [providers, setProviders] = useState<ProviderSummary[]>([]);
    const [providerId, setProviderId] = useState<string>("minimax");
    const [model, setModel] = useState<string>("MiniMax-M3");
    const [turns, setTurns] = useState<TurnMessage[]>([]);
    const [input, setInput] = useState("");
    const [sending, setSending] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [mcpTools, setMcpTools] = useState<string[]>([]);
    const [showSettings, setShowSettings] = useState(false);

    useEffect(() => {
        listProviders()
            .then((ps) => {
                setProviders(ps);
                const configured = ps.find((p) => p.is_configured);
                if (configured) setProviderId(configured.id);
                const def = ps.find((p) => p.is_configured)?.models.find(
                    (m) => m.is_default
                );
                if (def) setModel(def.id);
            })
            .catch((e) => setError(String(e)));
    }, []);

    useEffect(() => {
        let unlisten: UnlistenFn | undefined;
        listen<UiEvent>(`chat:${SESSION}`, (e) => {
            const ev = e.payload;
            setTurns((prev) => {
                const next = [...prev];
                const last = next[next.length - 1];
                if (!last || last.role !== "assistant") return next;
                if (ev.kind === "delta" && ev.text) {
                    next[next.length - 1] = {
                        ...last,
                        content: last.content + ev.text,
                        partial: true,
                    };
                } else if (ev.kind === "reasoning" && ev.text) {
                    next[next.length - 1] = {
                        ...last,
                        reasoning: (last.reasoning ?? "") + ev.text,
                    };
                } else if (ev.kind === "tool_start" && ev.id && ev.name) {
                    next[next.length - 1] = {
                        ...last,
                        toolTrace: [
                            ...(last.toolTrace ?? []),
                            { id: ev.id, name: ev.name, args_preview: ev.args_preview ?? "" },
                        ],
                    };
                } else if (ev.kind === "tool_args" && ev.id) {
                    next[next.length - 1] = {
                        ...last,
                        toolTrace: (last.toolTrace ?? []).map((t) =>
                            t.id === ev.id ? { ...t, args: ev.args } : t
                        ),
                    };
                } else if (ev.kind === "tool_done" && ev.id) {
                    next[next.length - 1] = {
                        ...last,
                        toolTrace: (last.toolTrace ?? []).map((t) =>
                            t.id === ev.id
                                ? { ...t, result_preview: ev.result_preview ?? "" }
                                : t
                        ),
                    };
                } else if (ev.kind === "send_blocked") {
                    next[next.length - 1] = {
                        ...last,
                        toolTrace: (last.toolTrace ?? []).map((t) =>
                            t.name.includes("send") || t.name.includes("draft")
                                ? { ...t, blocked: true }
                                : t
                        ),
                    };
                } else if (ev.kind === "done") {
                    next[next.length - 1] = { ...last, partial: false };
                    setSending(false);
                } else if (ev.kind === "error") {
                    next[next.length - 1] = { ...last, partial: false };
                    setError(ev.message ?? "unknown error");
                    setSending(false);
                }
                return next;
            });
        }).then((u) => (unlisten = u));
        return () => unlisten?.();
    }, []);

    const models: ModelInfo[] =
        providers.find((p) => p.id === providerId)?.models ?? [];

    const onSend = async () => {
        if (!input.trim() || sending) return;
        const userText = input;
        setInput("");
        setSending(true);
        setError(null);
        setTurns((t) => [
            ...t,
            { role: "user", content: userText },
            { role: "assistant", content: "", toolTrace: [], partial: true },
        ]);
        try {
            await chat({
                session_id: SESSION,
                user_input: userText,
                provider_id: providerId,
                model,
            });
        } catch (e: any) {
            setError(String(e));
            setSending(false);
        }
    };

    const onCancel = async () => {
        await cancelChat(SESSION);
        setSending(false);
    };

    const onSpawnMcp = async () => {
        try {
            const tools = await spawnMcp();
            setMcpTools(tools.map((t) => t.name));
        } catch (e: any) {
            setError(String(e));
        }
    };

    const onSaveSettings = async (cfg: HubConfig) => {
        try {
            await saveConfig(cfg);
            const ps = await listProviders();
            setProviders(ps);
            setShowSettings(false);
        } catch (e: any) {
            setError(String(e));
        }
    };

    return (
        <div className="h-full flex flex-col">
            <header className="border-b border-white/10 px-6 py-3 flex items-center gap-4">
                <div className="text-lg font-semibold tracking-tight">
                    LINE Model Hub
                </div>
                <select
                    className="bg-white/5 border border-white/10 rounded px-2 py-1 text-sm"
                    value={providerId}
                    onChange={(e) => {
                        setProviderId(e.target.value);
                        const def = providers
                            .find((p) => p.id === e.target.value)
                            ?.models.find((m) => m.is_default);
                        if (def) setModel(def.id);
                    }}
                >
                    {providers.map((p) => (
                        <option key={p.id} value={p.id}>
                            {p.label}
                            {p.is_configured ? "" : " ⚠"}
                        </option>
                    ))}
                </select>
                <select
                    className="bg-white/5 border border-white/10 rounded px-2 py-1 text-sm"
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                >
                    {models.map((m) => (
                        <option key={m.id} value={m.id}>
                            {m.is_default ? "★ " : ""}
                            {m.label}
                        </option>
                    ))}
                </select>
                <div className="flex-1" />
                <button
                    className="px-3 py-1 rounded bg-emerald-500/20 text-emerald-300 text-sm hover:bg-emerald-500/30"
                    onClick={onSpawnMcp}
                    title={
                        mcpTools.length
                            ? `${mcpTools.length} LINE tools loaded`
                            : "Spawn line-desktop-mcp"
                    }
                >
                    {mcpTools.length
                        ? `LINE MCP · ${mcpTools.length}`
                        : "Spawn LINE MCP"}
                </button>
                <button
                    className="px-3 py-1 rounded bg-white/5 text-sm hover:bg-white/10"
                    onClick={() => setShowSettings(true)}
                >
                    ⚙ Settings
                </button>
            </header>

            <main className="flex-1 overflow-auto scroll-thin px-6 py-4 space-y-4">
                {turns.length === 0 && (
                    <div className="text-white/40 text-sm">
                        開始跟 AI 對話，它可以用 LINE MCP 幫你讀取訊息、整理草稿、或在獲得你確認後送出訊息。
                    </div>
                )}
                {turns.map((t, i) =>
                    t.role === "user" ? (
                        <UserBubble key={i} content={t.content} />
                    ) : (
                        <AssistantBubble
                            key={i}
                            content={t.content}
                            reasoning={t.reasoning}
                            toolTrace={t.toolTrace}
                            partial={t.partial}
                        />
                    )
                )}
                {error && (
                    <div className="text-red-400 text-sm">⚠ {error}</div>
                )}
            </main>

            <footer className="border-t border-white/10 p-4 flex gap-2">
                <textarea
                    rows={1}
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.shiftKey) {
                            e.preventDefault();
                            onSend();
                        }
                    }}
                    placeholder="輸入訊息… (Enter 送出 / Shift+Enter 換行)"
                    disabled={sending}
                    className="flex-1 bg-white/5 border border-white/10 rounded px-3 py-2 text-sm resize-none disabled:opacity-50"
                />
                {sending ? (
                    <button
                        onClick={onCancel}
                        className="px-4 py-2 rounded bg-red-500/20 text-red-300 hover:bg-red-500/30"
                    >
                        Cancel
                    </button>
                ) : (
                    <button
                        onClick={onSend}
                        disabled={!input.trim()}
                        className="px-4 py-2 rounded bg-emerald-500 text-white hover:bg-emerald-400 disabled:opacity-40"
                    >
                        送出
                    </button>
                )}
            </footer>

            {showSettings && (
                <SettingsDialog
                    cfg={
                        providers.length
                            ? {
                                  default_provider: providerId as any,
                                  default_model: model,
                                  providers: providers.map((p) => ({
                                      id: p.id as any,
                                      api_key: "",
                                      base_url: null,
                                      default_model: p.default_model,
                                  })),
                              }
                            : null
                    }
                    onClose={() => setShowSettings(false)}
                    onSave={onSaveSettings}
                />
            )}
        </div>
    );
}

function UserBubble({ content }: { content: string }) {
    return (
        <div className="flex justify-end">
            <div className="max-w-[80%] bg-emerald-500/15 border border-emerald-500/30 rounded-2xl px-4 py-2 text-sm">
                {content}
            </div>
        </div>
    );
}

function AssistantBubble({
    content,
    reasoning,
    toolTrace,
    partial,
}: {
    content: string;
    reasoning?: string;
    toolTrace?: ToolCallTrace[];
    partial?: boolean;
}) {
    return (
        <div className="flex justify-start">
            <div className="max-w-[85%] space-y-2">
                {reasoning && (
                    <details className="bg-white/[0.03] border border-white/10 rounded-lg px-3 py-2 text-xs text-white/50">
                        <summary className="cursor-pointer">
                            💭 Thinking…
                        </summary>
                        <pre className="mt-2 whitespace-pre-wrap font-mono">
                            {reasoning}
                        </pre>
                    </details>
                )}
                {content && (
                    <div className="bg-white/5 border border-white/10 rounded-2xl px-4 py-2 text-sm whitespace-pre-wrap">
                        {content}
                        {partial && (
                            <span className="inline-block w-2 h-3 bg-emerald-400 animate-pulse ml-1 align-middle" />
                        )}
                    </div>
                )}
                {toolTrace?.map((t) => (
                    <div
                        key={t.id}
                        className={`bg-white/[0.03] border rounded-lg px-3 py-2 text-xs font-mono ${
                            t.blocked
                                ? "border-amber-500/40 bg-amber-500/10"
                                : "border-white/10"
                        }`}
                    >
                        <div className="flex items-center gap-2">
                            <span>🔧</span>
                            <span className="font-semibold">{t.name}</span>
                            {t.blocked && (
                                <span className="text-amber-400">
                                    ⚠ blocked by guard
                                </span>
                            )}
                        </div>
                        {t.args !== undefined && (
                            <pre className="mt-2 ml-5 text-white/70 whitespace-pre-wrap">
                                {JSON.stringify(t.args, null, 2)}
                            </pre>
                        )}
                        {t.result_preview && (
                            <pre className="mt-2 ml-5 text-white/50 whitespace-pre-wrap">
                                ↳ {t.result_preview}
                                {t.result_preview.length >= 200 ? "…" : ""}
                            </pre>
                        )}
                    </div>
                ))}
            </div>
        </div>
    );
}

function SettingsDialog({
    cfg,
    onClose,
    onSave,
}: {
    cfg: HubConfig | null;
    onClose: () => void;
    onSave: (cfg: HubConfig) => void;
}) {
    const [keys, setKeys] = useState<Record<string, string>>({});
    const [mcpPath, setMcpPath] = useState("");

    return (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4">
            <div className="bg-ink-900 border border-white/10 rounded-xl w-full max-w-lg p-6 space-y-4">
                <h2 className="text-lg font-semibold">Settings</h2>
                {cfg?.providers.map((p) => (
                    <div key={p.id}>
                        <label className="block text-xs text-white/60 mb-1">
                            {p.id.toUpperCase()} API Key
                        </label>
                        <input
                            type="password"
                            value={keys[p.id] ?? ""}
                            onChange={(e) =>
                                setKeys((k) => ({
                                    ...k,
                                    [p.id]: e.target.value,
                                }))
                            }
                            placeholder={
                                p.id === "minimax"
                                    ? "sk-cp-..."
                                    : p.id === "openai"
                                      ? "sk-..."
                                      : p.id === "anthropic"
                                        ? "sk-ant-..."
                                        : "(optional)"
                            }
                            className="w-full bg-white/5 border border-white/10 rounded px-3 py-2 text-sm font-mono"
                        />
                    </div>
                ))}
                <div>
                    <label className="block text-xs text-white/60 mb-1">
                        line-desktop-mcp entry path (server.js)
                    </label>
                    <input
                        value={mcpPath}
                        onChange={(e) => setMcpPath(e.target.value)}
                        placeholder="C:\\Tools\\line-desktop-mcp\\src\\server.js"
                        className="w-full bg-white/5 border border-white/10 rounded px-3 py-2 text-sm font-mono"
                    />
                </div>
                <div className="flex justify-end gap-2 pt-2">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 rounded text-sm bg-white/5 hover:bg-white/10"
                    >
                        Cancel
                    </button>
                    <button
                        onClick={() =>
                            onSave({
                                ...(cfg as HubConfig),
                                providers:
                                    cfg?.providers.map((p) => ({
                                        ...p,
                                        api_key: keys[p.id] ?? p.api_key,
                                    })) ?? [],
                                line_mcp_path: mcpPath || null,
                            })
                        }
                        className="px-4 py-2 rounded text-sm bg-emerald-500 hover:bg-emerald-400"
                    >
                        Save
                    </button>
                </div>
            </div>
        </div>
    );
}