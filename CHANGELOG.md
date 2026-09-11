# Changelog

All notable changes to Line 小幫手 are documented here. Versions follow
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-09-11

### ✨ New

- **Product rename**: 「LINE Model Hub」 → **「Line 小幫手」**.
  Updated `productName`, app identifier (`com.tt-openclaw.line-xiaobangshou`),
  window title, all docs, and the in-app header.
- **Per-tool UI cards** (`ToolCard` + `visualize` in `src/App.tsx`).
  Each of the 24 line-desktop-mcp tools now renders a distinct icon, accent
  border, and one-line summary instead of dumping raw JSON. Categories
  covered: send, draft, history, search/verify, export, file/reply/copy/
  translate/forward, navigation/status.
- **Native send-confirm gate** (`request_send_confirm` Tauri command +
  🛡 button on send-class cards). Before any `send_message_auto`,
  `send_message_manual`, `send_file_manual`, `set_line_draft`,
  `clear_line_draft`, `stage_line_reply`, or `stage_line_forward` call,
  a native OK/Cancel dialog pops with chatroom + message preview. Approval
  state is recorded on the card (✅ / ❌ / 重設).
- **Settings dialog hardened**: always renders the four canonical
  provider rows (MiniMax / OpenAI / Anthropic / Ollama) regardless of
  whether the backend has pushed entries yet.
- **Config loader hardened** (`HubConfig::load` now calls
  `ensure_minimax_default` on both file-missing and JSON-decoded paths).
- **New tests** (`crates/line-hub-core/tests/config_defaults.rs`):
  `ensure_minimax_default_adds_minimax_to_empty_config` +
  `ensure_minimax_default_is_idempotent`.

### 🐛 Fixed

- `spawn_mcp` was ignoring `HubConfig.line_mcp_path` (the value saved
  from the Settings dialog) — only the `HUB_LINE_MCP_PATH` /
  `LINE_MODEL_HUB_BUNDLED` env vars were checked. Result: users saw
  `⚠ set HUB_LINE_MCP_PATH or LINE_MODEL_HUB_BUNDLED` even after pasting
  the correct path. New priority is env → HubConfig → bundled, with a
  human-readable error string.

## [0.1.0] — 2026-09-11

### Added
- **Multi-provider support**: MiniMax M3 (default), OpenAI, Anthropic Claude,
  Ollama / LM Studio.
- **Streaming chat** with token-by-token UI updates.
- **Tool-use loop** integrating all 24 line-desktop-mcp tools.
- **Native send-approval dialog** — every LINE mutation requires explicit
  user OK/Cancel.
- **Thinking-tag stripping** — MiniMax M3 / M2.7 emit `` and
  `` blocks; both are filtered from the visible chat, with
  reasoning content surfaced in a collapsible "Thinking…" panel.
- **Multi-crate Rust workspace** (`line-hub-core`, `line-hub-mcp`,
  `line-hub-tauri`) for clean separation of portable logic and the
  Windows-only Tauri shell.
- **11 unit tests** for the SSE parser and OpenAI-compat wire format.
- **Tauri 2 shell** with React 18 + Vite 5 + Tailwind 3 frontend.
- **MSI + NSIS** bundling targets for Windows installers.

### Verified
- Live MiniMax M3 API tests (text-only, tool use, streaming tool calls,
  multi-turn agentic loop, 24-tool context).
- Rust core compiles on Linux/macOS/Windows (edition 2021).
- Frontend `npm run build` succeeds: 151 KB JS / 49 KB gzipped.
- `cargo check -p line-hub-tauri` succeeds on Linux (full link requires
  Windows MSVC + WebView2).

### Known limitations
- Anthropic provider is stub-only (real SSE driver pending v1.1).
- Tauri shell `cargo tauri build` only runs on Windows.
- No MSI/NSIS test signing in CI yet (manual `signtool` step documented
  in BUILDING.md).
- LM Studio model list is static (auto-discovery via `/v1/models` is
  v1.2).

[0.1.0]: #010--2026-09-11