# Line 小幫手 — Specification

## 1. Purpose

A Windows desktop application that gives the user **freedom to choose the AI
model** driving their LINE Desktop interactions. The reference upstream
project [`bensonmaxai/line-desktop-mcp`](https://github.com/bensonmaxai/line-desktop-mcp)
only ships a Codex-CLI binding. This tool removes that coupling.

## 2. Goals (v0.1.0)

1. **Multi-provider** chat with full tool-use support. Default = MiniMax M3.
2. **Stream tokens + reasoning** to the UI in real time.
3. **Wire-protocol compatibility** with line-desktop-mcp via JSON-RPC over stdio.
4. **Native approval gate** before any LINE mutation.
5. **Clean Rust multi-crate** workspace so the core logic compiles on every
   platform and only the GUI shell is Windows-specific.

## 3. Non-goals (v0.1.0)

- Mobile or macOS GUI (core lib is portable; GUI is Windows-only)
- Multi-account LINE switching
- Local model training / fine-tuning
- Cloud relay / proxy — all traffic is direct from the user's machine

## 4. User stories

### 4.1 Read recent messages
> User: "整理老婆最近十則訊息，列出需要回覆的事項"
>
> Assistant: → `get_line_chatroom_history_short(chatName="老婆", messageLimit=10)` →
              processes the JSON → returns a structured summary.
>
> No approval needed (read-only).

### 4.2 Compose a draft
> User: "幫我回老婆說我今天會加班"
>
> Assistant: → `set_line_draft(chatName="老婆", message="今天加班...")` →
              confirms draft text → returns draft URL.
>
> The draft is *in LINE only*. The assistant never auto-sends.

### 4.3 Send (approval required)
> Assistant wants to call `send_message_auto(chatName="老婆", message="...")`.
>
> Native dialog appears:
>
>     ⚠ Line 小幫手 wants to call send_message_auto
>     Chat: 老婆
>     Message: 今天加班...
>     [Cancel] [OK]
>
> User clicks OK → message is sent.
> User clicks Cancel → `{"error": "User declined"}` is fed back to the model.

## 5. Architecture

```
React UI  ⇄  Tauri commands  ⇄  line-hub-core  ⇄  AI provider
                                       │
                                       └─→  line-hub-mcp  ⇄  line-desktop-mcp child
                                                                       │
                                                                       └─→  LINE Desktop
```

### 5.1 line-hub-core

| Module | Role |
|--------|------|
| `provider::Provider` | Trait every provider implements |
| `provider::minimax` | MiniMax M3 / M2.7 (default) |
| `provider::openai` | OpenAI Chat Completions + Responses |
| `provider::anthropic` | Anthropic Messages (stub) |
| `provider::ollama` | Local Ollama / LM Studio |
| `provider::openai_compat` | Shared SSE parser + wire-format |
| `conversation` | Agentic loop, send-guard |
| `mcp` | JSON-RPC client to line-desktop-mcp child |
| `config` | Persistent config in `~/.line-hub/config.json` |

### 5.2 line-hub-tauri

| Module | Role |
|--------|------|
| `lib.rs` | Tauri builder, plugin registration, command handler list |
| `state.rs` | AppState (active providers, sessions, MCP client) |
| `commands.rs` | All `#[tauri::command]` IPC handlers |

### 5.3 Frontend (React + Vite + Tailwind)

| File | Role |
|------|------|
| `src/App.tsx` | Top-level layout, chat list, model picker, send bar |
| `src/lib/tauri.ts` | Typed wrappers around `invoke()` |
| `src/index.css` | Tailwind base |

## 6. Wire protocol

### 6.1 Provider ↔ Hub

OpenAI-compatible Chat Completions with `tools: [{type: "function", ...}]`
and `tool_choice: "auto"`. Streaming via SSE `data: ` chunks; the parser
emits normalised `StreamEvent`s.

MiniMax additions:

- `delta.reasoning` channel (parallel to `content`).
- `<thinking>...</thinking>` and `<think>...</think>` tags embedded in
  `delta.content`. Both stripped from the visible stream, with the
  reasoning content routed to the collapsible "Thinking…" panel.

### 6.2 Hub ↔ line-desktop-mcp

JSON-RPC 2.0 over stdio (one newline-delimited JSON message per line).
Methods used:

| Method | Direction | Purpose |
|--------|-----------|---------|
| `initialize` | hub → mcp | protocol handshake + capabilities |
| `notifications/initialized` | hub → mcp | initialised ack |
| `tools/list` | hub → mcp | discover 24 tools |
| `tools/call` | hub → mcp | invoke one tool |

MCP child is spawned with `LINE_MCP_EXTENSIONS=1` so all 24 are enabled.
We also set `LINE_MCP_SKIP_POSTINSTALL=1` because the bundled postinstall
is interactive and we already have all deps installed.

## 7. Safety properties

| Property | Mechanism |
|----------|-----------|
| No auto-send | Native dialog must return `true` before `send_*` runs |
| No accidental draft overwrite | `set_line_draft` requires `expectedDraft` match |
| No tool fabrication | Tools come from `tools/list`; never user-injected |
| Bounded loop | `max_tool_turns = 8` default |
| API key safety | OS keyring via `keyring` crate (v0.2; v0.1 stores in config.json for simplicity) |
| Crash recovery | Operation lock at `~/.line-desktop-mcp/operation.lock` (provided by MCP server) |

## 8. Build & distribution

| Artefact | Path |
|----------|------|
| Standalone exe | `crates/line-hub-tauri/target/release/line-model-hub.exe` |
| MSI installer | `crates/line-hub-tauri/target/release/bundle/msi/*.msi` |
| NSIS installer | `crates/line-hub-tauri/target/release/bundle/nsis/*-setup.exe` |

Both installers embed the React bundle and the Tauri shell. line-desktop-mcp
is **not** bundled in v0.1 — users install it separately and point Settings at
its `server.js` path. Bundling line-desktop-mcp is a v1.0 target.

## 9. Open issues (deferred to v0.2+)

- Anthropic Messages SSE driver implementation
- API key encryption via `keyring` crate (currently plaintext in
  `~/.line-hub/config.json`; acceptable for dev, must fix before public
  release)
- Conversation export (Markdown / JSON)
- Multi-session UI (currently single fixed session `default`)
- Ollama dynamic model discovery via `GET /v1/models`