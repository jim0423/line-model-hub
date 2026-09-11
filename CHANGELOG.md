# Changelog

All notable changes to Line 小幫手 are documented here. Versions follow
[Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-09-11

### ✨ New

- **Full Anthropic Claude SSE driver** (`crates/line-hub-core/src/provider/anthropic.rs`)
  Previously a stub returning "pending v1.1 implementation" — now wires
  the native `POST /v1/messages` protocol with `x-api-key` and
  `anthropic-version: 2023-06-01` headers. Translates every Anthropic
  event type into our `StreamEvent`:
  - `message_start` → seed usage
  - `content_block_start` (text) → no-op
  - `content_block_start` (tool_use) → `ToolCallStart`
  - `content_block_delta` (text_delta) → `Delta`
  - `content_block_delta` (input_json_delta) → `ToolCallDelta`
  - `content_block_delta` (thinking_delta) → `ReasoningDelta`
  - `message_delta` → merge output_tokens, capture stop_reason
  - `message_stop` → `Done`
  Lists `claude-opus-4-5`, `claude-sonnet-4-5` (default),
  `claude-haiku-4-5`. Maps our wire-format messages to Anthropic's
  content blocks (system goes to top-level, assistant content becomes
  text/tool_use array, tool results fold into a user message carrying
  an array of `tool_result` blocks).

- **Multi-session background chat**
  `chat` no longer blocks the IPC reply until the stream finishes — it
  spawns a `tokio::spawn` task per session, returns immediately, and
  emits events on `chat:<session_id>`. Concretely:
  - A `tokio::sync::Notify` per session lets `cancel_chat` abort
    cleanly without dropping the half-streamed response.
  - The frontend's `listen<UiEvent>(chat:${activeSessionId})` now
    re-subscribes whenever the active session changes (deps include
    `activeSessionId`), so events from a different session become
    invisible until you switch back.
  - `onCancel` now maps to `cancel_chat(activeSessionId)` instead of
    the global constant.

### 🧪 Tests

- `crates/line-hub-core/src/provider/anthropic.rs` adds 6 unit tests:
  - `request_serialises_with_anthropic_shape`
  - `parses_text_delta_event`
  - `parses_tool_use_input_json_delta_event`
  - `parses_thinking_delta_event`
  - `parses_message_delta_with_stop_reason_and_usage`
  - `parses_tool_use_block_start`

Total: **22 tests passing** across the workspace.

## [0.2.1] - 2026-09-11

### ✨ New

- **Local conversation history** — every turn is now persisted to a
  SQLite database at `~/.line-hub/history.sqlite3` (in-memory fallback
  if the disk store fails to open). Closing and relaunching the app
  restores the conversation exactly as you left it.
- **Session sidebar** — the left rail now lists every saved conversation,
  newest first. Click to switch; hover to rename (✎) or delete (×).
  Brand-new sessions are created with the `+ 新對話` button.
- **5 new Tauri commands** wired to the history store:
  `list_history_sessions`, `load_history_turns`, `append_history_turn`,
  `rename_history_session`, `delete_history_session`.
- **Cancel streaming button** — the chat footer's send button flips to
  `Cancel` while a response is in flight. Pressing it calls
  `cancel_chat` and stops the SSE stream cleanly.
- **`HistoryStore::open_in_memory`** — graceful fallback so a broken
  on-disk store does not crash startup.

### 🧪 Tests

- `crates/line-hub-core/src/history.rs` adds 3 unit tests:
  - `append_and_load_round_trip`
  - `list_sessions_orders_by_updated_at_desc`
  - `delete_session_cascades_turns`

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