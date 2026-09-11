import { useEffect, useState } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import {
    cancelChat,
    chat,
    HubConfig,
    listProviders,
    ModelInfo,
    ProviderSummary,
    requestSendConfirm,
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
                    Line 小幫手
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
                    <ToolCard key={t.id} trace={t} />
                ))}
            </div>
        </div>
    );
}

/**
 * Render a single line-desktop-mcp tool call as a human-friendly card
 * instead of dumping the raw JSON. Each tool gets its own icon, accent,
 * and a one-line summary; details (arguments, result preview) collapse
 * under a "details" toggle so the chat stays readable.
 *
 * For send-class tools (send_message_*, send_file_manual, draft overwrite,
 * forward) the card also carries an "Approve" button that pops a native
 * confirmation dialog before the tool is allowed to run.
 */
function ToolCard({ trace }: { trace: ToolCallTrace }) {
    const v = visualize(trace);
    const [approval, setApproval] = useState<
        "idle" | "pending" | "approved" | "denied"
    >(trace.blocked ? "denied" : "idle");

    const askNative = async () => {
        const args = trace.args ?? {};
        const chatroom =
            typeof args.chatroom === "string" ? (args.chatroom as string) : "";
        const text =
            typeof args.text === "string"
                ? (args.text as string)
                : JSON.stringify(args);
        setApproval("pending");
        try {
            const ok = await requestSendConfirm({
                chatroom,
                text,
                tool: trace.name,
            });
            setApproval(ok ? "approved" : "denied");
        } catch (e) {
            setApproval("denied");
        }
    };

    return (
        <div
            className={`border rounded-lg overflow-hidden text-xs ${v.border} ${v.bg}`}
        >
            <div className="flex items-center gap-2 px-3 py-2">
                <span className="text-base">{v.icon}</span>
                <span className="font-semibold text-white/90">
                    {v.title}
                </span>
                <span className="ml-auto text-white/40 font-mono text-[10px]">
                    {trace.name}
                </span>
            </div>
            <div className="px-3 pb-2 text-white/80">{v.summary}</div>
            {v.requiresApproval && (
                <div className="px-3 pb-2 flex items-center gap-2">
                    <button
                        onClick={askNative}
                        disabled={approval !== "idle"}
                        className={`px-3 py-1 rounded text-xs font-semibold transition-colors ${
                            approval === "approved"
                                ? "bg-emerald-500/30 text-emerald-200"
                                : approval === "denied"
                                  ? "bg-rose-500/20 text-rose-300"
                                  : approval === "pending"
                                    ? "bg-white/10 text-white/50 cursor-wait"
                                    : "bg-emerald-500 hover:bg-emerald-400 text-white"
                        }`}
                    >
                        {approval === "approved"
                            ? "✅ 已確認送出"
                            : approval === "denied"
                              ? "❌ 已拒絕"
                              : approval === "pending"
                                ? "等待確認…"
                                : "🛡 確認送出"}
                    </button>
                    {approval !== "idle" && (
                        <button
                            onClick={() => setApproval("idle")}
                            className="text-[10px] text-white/40 hover:text-white/70"
                        >
                            重設
                        </button>
                    )}
                </div>
            )}
            {(trace.args !== undefined || trace.result_preview) && (
                <details className="border-t border-white/10">
                    <summary className="px-3 py-1.5 cursor-pointer text-white/40 hover:text-white/60 select-none">
                        顯示參數與結果
                    </summary>
                    <div className="px-3 py-2 space-y-2 bg-black/20">
                        {trace.args !== undefined && (
                            <pre className="font-mono text-white/60 whitespace-pre-wrap">
                                {JSON.stringify(trace.args, null, 2)}
                            </pre>
                        )}
                        {trace.result_preview && (
                            <pre className="font-mono text-white/40 whitespace-pre-wrap">
                                ↳ {trace.result_preview}
                                {trace.result_preview.length >= 200
                                    ? "…"
                                    : ""}
                            </pre>
                        )}
                    </div>
                </details>
            )}
            {trace.blocked && (
                <div className="px-3 py-1.5 bg-amber-500/20 text-amber-300 text-[11px] border-t border-amber-500/30">
                    ⚠ 已被守衛攔下（需手動確認）
                </div>
            )}
        </div>
    );
}

interface CardVisual {
    icon: string;
    title: string;
    summary: string;
    border: string;
    bg: string;
    /** true if this card should pop a native confirmation dialog before
     *  the tool actually runs (send / file / draft overwrite class). */
    requiresApproval: boolean;
}

/**
 * Map a line-desktop-mcp tool name + parsed args to a human-friendly card.
 * 24 tools are bucketed into 7 categories matching docs/features.md so we
 * can give every call a sensible icon and label without writing 24 ifs.
 */
function visualize(trace: ToolCallTrace): CardVisual {
    const args = trace.args ?? {};
    const argStr = (k: string, fallback = "—") =>
        typeof args[k] === "string" && (args[k] as string).length
            ? (args[k] as string)
            : fallback;
    const name = trace.name;

    // ── Send tools (require native confirmation in agentic loop) ───────────
    if (name === "send_message_auto") {
        return {
            icon: "📤",
            title: "即將送出訊息",
            summary: `給「${argStr("chatroom", "未指定")}」：${truncate(argStr("text"), 80)}`,
            border: "border-emerald-500/30",
            bg: "bg-emerald-500/[0.04]",

            requiresApproval: true,        };
    }
    if (name === "send_message_manual") {
        return {
            icon: "✍️",
            title: "即將把訊息放進輸入框",
            summary: `給「${argStr("chatroom")}」：${truncate(argStr("text"), 80)}`,
            border: "border-sky-500/30",
            bg: "bg-sky-500/[0.04]",

            requiresApproval: true,        };
    }

    // ── Draft tools ────────────────────────────────────────────────────────
    if (name === "set_line_draft") {
        return {
            icon: "📝",
            title: "即將覆蓋草稿",
            summary: `「${argStr("chatroom")}」：${truncate(argStr("text"), 80)}`,
            border: "border-amber-500/30",
            bg: "bg-amber-500/[0.04]",

            requiresApproval: true,        };
    }
    if (name === "get_line_draft") {
        return {
            icon: "📝",
            title: "讀取草稿",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "clear_line_draft") {
        return {
            icon: "🧹",
            title: "清除草稿",
            summary: `「${argStr("chatroom")}」`,
            border: "border-amber-500/30",
            bg: "bg-amber-500/[0.04]",

            requiresApproval: true,        };
    }

    // ── Read history ───────────────────────────────────────────────────────
    if (
        name === "get_line_chatroom_history_short" ||
        name === "get_line_chatroom_history_default" ||
        name === "get_line_chatroom_history_long"
    ) {
        return {
            icon: "📜",
            title: "讀取近期訊息",
            summary: `「${argStr("chatroom")}」（${labelForHistoryRange(name)}）`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "get_line_chat_messages") {
        return {
            icon: "📜",
            title: "結構化讀取訊息",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }

    // ── Search / verify ────────────────────────────────────────────────────
    if (name === "search_line_chat_messages") {
        return {
            icon: "🔎",
            title: "搜尋訊息",
            summary: `在「${argStr("chatroom")}」找「${truncate(argStr("query"), 40)}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "verify_line_message") {
        return {
            icon: "✅",
            title: "核對訊息",
            summary: `在「${argStr("chatroom")}」確認文字是否存在`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }

    // ── Export ─────────────────────────────────────────────────────────────
    if (name === "export_line_chat_history") {
        const fmt = (args.format as string) ?? "txt";
        return {
            icon: "💾",
            title: `匯出聊天紀錄（${fmt.toUpperCase()}）`,
            summary: `「${argStr("chatroom")}」→ ${argStr("path")}`,
            border: "border-fuchsia-500/30",
            bg: "bg-fuchsia-500/[0.04]",

            requiresApproval: false,        };
    }

    // ── File / reply / forward / copy / translate ──────────────────────────
    if (name === "send_file_manual") {
        return {
            icon: "📎",
            title: "準備附件（待手動確認上傳）",
            summary: `「${argStr("chatroom")}」→ ${argStr("file_path")}`,
            border: "border-emerald-500/30",
            bg: "bg-emerald-500/[0.04]",

            requiresApproval: true,        };
    }
    if (name === "stage_line_reply") {
        return {
            icon: "↩️",
            title: "準備引用回覆",
            summary: `「${argStr("chatroom")}」`,
            border: "border-sky-500/30",
            bg: "bg-sky-500/[0.04]",

            requiresApproval: true,        };
    }
    if (name === "copy_line_message") {
        return {
            icon: "📋",
            title: "複製訊息",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "translate_line_message") {
        return {
            icon: "🌐",
            title: "翻譯訊息",
            summary: `「${argStr("chatroom")}」→ ${argStr("target_lang", "auto")}`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "stage_line_forward") {
        return {
            icon: "➡️",
            title: "準備轉傳",
            summary: `「${argStr("chatroom")}」`,
            border: "border-sky-500/30",
            bg: "bg-sky-500/[0.04]",

            requiresApproval: true,        };
    }

    // ── Navigation & status ────────────────────────────────────────────────
    if (name === "open_line_chat") {
        return {
            icon: "🚪",
            title: "開啟聊天室",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "open_line_chat_feature") {
        return {
            icon: "🧭",
            title: "導向功能頁",
            summary: `「${argStr("chatroom")}」→ ${argStr("feature")}`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "get_line_status") {
        return {
            icon: "🩺",
            title: "查看 LINE 狀態",
            summary: "LINE 視窗 / GUI 環境",
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "get_line_capabilities") {
        return {
            icon: "🧩",
            title: "列出工具能力",
            summary: "查看 24 個工具可用範圍",
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "get_line_workflow") {
        return {
            icon: "🗺️",
            title: "查詢操作流程",
            summary: `流程：${argStr("workflow_name", "auto")}`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "get_line_ui_state") {
        return {
            icon: "🪟",
            title: "讀取介面狀態",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }
    if (name === "confirm_line_chat_view") {
        return {
            icon: "🔒",
            title: "確認聊天室視圖",
            summary: `「${argStr("chatroom")}」`,
            border: "border-white/10",
            bg: "bg-white/[0.02]",

            requiresApproval: false,        };
    }

    // ── Fallback for unknown / new tools ───────────────────────────────────
    return {
        icon: "🔧",
        title: trace.name,
        summary: "",
        border: "border-white/10",
        bg: "bg-white/[0.02]",
        requiresApproval: false,
    };
}

function truncate(s: string, max: number): string {
    return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

function labelForHistoryRange(name: string): string {
    if (name.endsWith("_short")) return "小範圍";
    if (name.endsWith("_long")) return "較多範圍";
    return "一般範圍";
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

    // Always show the four canonical providers even if the Rust backend
    // hasn't pushed an entry yet — otherwise the user has no way to add
    // their first API key. See HubConfig::load in crates/line-hub-core.
    const knownProviders = [
        { id: "minimax", label: "MiniMax API Key", placeholder: "sk-cp-..." },
        { id: "openai", label: "OpenAI API Key (optional)", placeholder: "sk-..." },
        { id: "anthropic", label: "Anthropic API Key (optional)", placeholder: "sk-ant-..." },
        { id: "ollama", label: "Ollama base URL (optional)", placeholder: "http://localhost:11434/v1" },
    ];

    return (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4">
            <div className="bg-ink-900 border border-white/10 rounded-xl w-full max-w-lg p-6 space-y-4">
                <h2 className="text-lg font-semibold">Settings</h2>
                {knownProviders.map((kp) => {
                    const existing = cfg?.providers.find((p) => p.id === kp.id);
                    const baseUrl = (existing as any)?.base_url as string | undefined;
                    return (
                        <div key={kp.id}>
                            <label className="block text-xs text-white/60 mb-1">
                                {kp.label}
                            </label>
                            <input
                                type={kp.id === "ollama" ? "text" : "password"}
                                value={
                                    kp.id === "ollama"
                                        ? (baseUrl ?? keys[kp.id] ?? "")
                                        : (keys[kp.id] ?? existing?.api_key ?? "")
                                }
                                onChange={(e) =>
                                    setKeys((k) => ({
                                        ...k,
                                        [kp.id]: e.target.value,
                                    }))
                                }
                                placeholder={kp.placeholder}
                                className="w-full bg-white/5 border border-white/10 rounded px-3 py-2 text-sm font-mono"
                            />
                        </div>
                    );
                })}
                <div>
                    <label className="block text-xs text-white/60 mb-1">
                        line-desktop-mcp entry path (server.js)
                    </label>
                    <input
                        value={mcpPath || cfg?.line_mcp_path || ""}
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
                        onClick={() => {
                            const merged: HubConfig = {
                                ...(cfg as HubConfig),
                                providers:
                                    cfg?.providers.map((p) => ({
                                        ...p,
                                        api_key:
                                            keys[p.id] !== undefined
                                                ? keys[p.id]
                                                : p.api_key,
                                        ...(p.id === "ollama" && keys.ollama
                                            ? { base_url: keys.ollama }
                                            : {}),
                                    })) ?? [],
                                line_mcp_path: mcpPath || cfg?.line_mcp_path || null,
                            };
                            onSave(merged);
                        }}
                        className="px-4 py-2 rounded text-sm bg-emerald-500 hover:bg-emerald-400"
                    >
                        Save
                    </button>
                </div>
            </div>
        </div>
    );
}