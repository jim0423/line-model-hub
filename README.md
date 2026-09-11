# LINE Model Hub

> A Windows desktop client that lets you **pick any AI model** (MiniMax M3 by
> default, OpenAI, Anthropic Claude, or local Ollama) and use it to drive
> [line-desktop-mcp](https://github.com/bensonmaxai/line-desktop-mcp) — read
> chats, draft replies, and (only after explicit approval) send messages on
> your behalf.

Built because the upstream `bensonmaxai/line-desktop-mcp` only ships a
Codex-CLI binding. This tool removes that dependency: **any MCP-compatible
model can take the wheel**.

---

## ✨ Features (v0.1.0)

- **Multi-provider** — switch between MiniMax M3, OpenAI GPT-5/o1, Anthropic
  Claude, or local Ollama / LM Studio. Defaults to **MiniMax M3**.
- **Streaming chat** — token-by-token rendering with reasoning channel
  shown in a collapsible "Thinking…" panel.
- **Tool-use loop** — advertises all 24 LINE Desktop MCP tools to the model
  via OpenAI-compatible `tools/call`. Models can plan, list, and draft.
- **Send guard** — every `send_message_*` and `clear_line_draft` tool
  triggers a **native confirmation dialog** showing chat + full message +
  tool name. Refusal is silent; approval is one click.
- **Multi-platform runtime** — Rust core + Tauri shell + React UI. Core
  library compiles on Linux/macOS/Windows; Tauri shell only on Windows.

## 🏗 Architecture

```
┌──────────────────────────────────────────────────────────┐
│  LINE Model Hub (Tauri 2 desktop app)                      │
│                                                            │
│   ┌─────────────┐   ┌──────────────┐   ┌──────────────┐  │
│   │ ModelPicker │   │  MCP Bridge  │   │  Chat panel  │  │
│   │  (React)    │   │ (Tauri cmd)  │   │   (React)    │  │
│   └─────┬───────┘   └──────┬───────┘   └──────────────┘  │
│         │                  │                                │
│         ▼                  ▼ spawn (Node child)             │
│  ┌─────────────────┐  ┌────────────────────┐              │
│  │ Provider trait  │  │ line-desktop-mcp   │              │
│  │ MiniMax/OpenAI  │  │ (v1.2.0, 24 tools) │              │
│  │ Anthropic/Ollama│  │ + CUA Driver + AHK │              │
│  └────────┬────────┘  └─────────┬──────────┘              │
└───────────┼─────────────────────┼───────────────────────────┘
            ▼                     ▼
   api.minimax.io etc.     Windows LINE Desktop
```

**Three crates** under `crates/`:

| Crate | Role | Builds on |
|-------|------|-----------|
| `line-hub-core` | Provider abstraction, SSE parser, MCP client, agentic loop | Linux/macOS/Windows |
| `line-hub-mcp` | Typed wrappers around line-desktop-mcp JSON-RPC | Linux/macOS/Windows |
| `line-hub-tauri` | Tauri 2 shell, IPC commands, native confirm dialog | **Windows only** (WebView2 + MSVC) |

## 📦 Windows build

See [BUILDING.md](BUILDING.md) for the full SOP.

TL;DR:

```powershell
# Prerequisites: Rust 1.75+, Node 22+, MSVC Build Tools, WebView2 Runtime
cd line-model-hub
npm install
cargo install tauri-cli --version "^2.0"
cargo tauri build
# Output: src-tauri\target\release\line-model-hub.exe
# Bundles: src-tauri\target\release\bundle\msi\*.msi
#          src-tauri\target\release\bundle\nsis\*-setup.exe
```

## 🚀 First-run setup

1. Launch the installed `LINE Model Hub`.
2. ⚙ Settings:
   - Paste your **MiniMax** API key (`sk-cp-...`).
   - Set the **LINE Desktop MCP entry path** (e.g.
     `C:\Tools\line-desktop-mcp\src\server.js`).
3. Click **Spawn LINE MCP** — you'll see `LINE MCP · 24` if successful.
4. Pick MiniMax-M3 from the model dropdown.
5. Ask: *"整理老婆最近十則訊息，列出需要回覆的事項"*

## 🛡 Safety design

Every send requires a **native OK/Cancel dialog** that displays:

- target chat
- full outgoing message
- tool name (`send_message_auto` / `_manual` / `_file_manual` / `set_line_draft` / `clear_line_draft`)

This dialog is implemented via `tauri-plugin-dialog` and **cannot be triggered
by the model alone**. The browser-side React layer cannot bypass it; even an
XSS in the chat history cannot auto-send.

In addition:

- `set_line_draft` / `clear_line_draft` require an `expectedDraft` match
  before overwriting — protects manual edits.
- `send_file_manual` only opens the file picker; the actual upload happens
  on a separate explicit step.
- 24-tool list is hard-coded by line-desktop-mcp; we never fabricate tools.

## 🧪 Tests

```bash
cargo test -p line-hub-core
# 11 tests: thinking-tag stripping (5), wire-format serialization (6)
```

## 📜 License

MIT — see [LICENSE](LICENSE).

---

## Provenance

- Upstream LINE binding: [bensonmaxai/line-desktop-mcp](https://github.com/bensonmaxai/line-desktop-mcp)
- Original macOS binding: [dtwang/line-desktop-mcp](https://github.com/dtwang/line-desktop-mcp)
- MiniMax M3 API: `https://api.minimax.io/v1/chat/completions`
- Default model: **MiniMax-M3** (reasoning + tool use)

Built by Jim, 2026-09-11.