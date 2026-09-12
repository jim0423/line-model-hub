# Changelog

All notable changes to Line 小幫手 are documented here. Versions follow
[Semantic Versioning](https://semver.org/).


## [0.5.0] - 2026-09-12

v0.5.0 is tuned to the line-desktop-mcp **v3.0.0** release. It covers
the new 29-tool surface, fail-closed named-chat verification flow, and
typed image / audio blocks returned by \`get_line_local_messages\`.

### ✨ New

- **Typed image / audio attachments in tool results**
  (\`crates/line-hub-core/src/mcp.rs\`, \`commands.rs::UiAttachment\`)
  \`call_tool_structured()\` decodes the MCP \`content: Content[]\` blocks
  into typed variants (\`Text\` / \`Image\` / \`Audio\` / \`Unsupported\`).
  Image bytes ride the Tauri event channel as inline \`data:\` URLs so
  the React frontend can render \`<img src=...>\` without any extra
  \`file://\` plumbing on the way.

- **Agentic loop wired into the chat command**
  (\`commands.rs::chat\` + \`conversation::run_turn\`)
  \`chat()\` now drives \`conversation::run_turn\` instead of streaming
  directly into a \`tokio::spawn\`. AI can decide to call a tool, get
  the result back, and chain into another provider turn — up to 8
  tool turns per message — before returning a final reply.

- **Native send-guard in the agentic loop**
  (\`commands.rs::TauriSendGuard\`)
  When the model wants to invoke any \`send_*\` / \`stage_line_reply\`
  tool the Tauri command layer now pops a real OS confirmation
  dialog (\`tauri-plugin-dialog\`) inline. Approvals survive across
  tool calls within one \`run_turn\` session; refusals still let the
  loop continue with a \`SendBlocked\` trace.

- **\`get_line_capabilities\` health card**
  (\`commands.rs::fetch_capabilities\`, \`App.tsx\`)
  Right after \`spawn_mcp\` the UI requests
  \`get_line_capabilities({mode:"all"})\` and renders a "LINE feature
  map" (tool count, direct / uia / guided_ui /
  unavailable_windows buckets). Returns a synthetic "spawn MCP
  first" payload if the child is not running.

- **Asia/Taipei timezone helpers**
  (\`crates/line-hub-core/src/tz.rs\`)
  \`tz::local_midnight_to_utc(date)\` and \`tz::today_local()\` keep all
  date filters flowing into the bridge in \`+08:00\` regardless of the
  host system timezone — matching line-desktop-mcp v3.0.0 own
  fixed-Asia/Taipei behaviour. Three unit tests cover round-trip and
  bad-input cases.

- **\`categorize_tool\` extended to all 29 tools**
  (\`commands.rs::categorize_tool\`)
  Replaces the v0.4.0 8-category heuristic with explicit \`matches!\`
  arms for every tool exposed by line-desktop-mcp v3.0.0:
  \`get_line_local_messages\` (its own bucket so the system prompt
  can recommend it), the four \`confirm_*\` visual-confirmation tools
  (caller must inspect screenshot), \`prepare_line_workflow\`,
  \`get_line_poll_state\`, etc. No more silent fallthrough to \`other\`.

- **\`build_system_prompt\` v3.0.0 hints**
  (\`commands.rs::build_system_prompt\`)
  System prompt now nudges the model to (a) prefer
  \`get_line_local_messages\` over the older paging tools, (b) look at
  the screenshot before issuing any confirmation token, (c) plan
  mentions / replies / polls with \`prepare_line_workflow\` before
  executing, and (d) treat all date / time filters as Asia/Taipei
  (UTC+08:00).

### 🐛 Fixed

- **\`mcp.rs::registry_sink()\` was \`unimplemented!()\`**
  The v0.4.0 code panicked the first time \`initialize()\` ran because
  the published registry lived behind an unreachable \`Mutex\` stub.
  v0.5.0 replaces it with \`Arc<RwLock<ToolRegistry>>\` and the spawn
  flow writes through the lock instead of crashing.

- **\`McpClient\` registry was empty after spawn**
  Symptom: any code path that called \`mcp.tool_registry()\` immediately
  after \`spawn_mcp\` got a zero-length registry. \`initialize()\` now
  writes through the lock before returning.

### 🧪 Tests

- \`mcp::tests\` adds 5 cases: text / image / audio block decoding,
  unsupported block fallback, oversize-image rejection.
- \`tz::tests\` adds 3 cases: midnight round-trip, bad-date rejection,
  \`today_local()\` stays in range.
- Workspace count now **45+** lib tests passing (was 34 in v0.4.0).

## [0.4.0] - 2026-09-11

### ✨ New

- **OS keyring API key storage** (`crates/line-hub-core/src/keyring.rs`)
  Every API key now lives in the operating system's secret store rather
  than the on-disk JSON config. Specifically:
  - Windows → Credential Manager (`wincred`, encrypted via DPAPI)
  - macOS → Keychain
  - Linux → Secret Service (gnome-keyring / kwallet)
  Namespaced under `com.tt-openclaw.line-xiaobangshou` so other apps
  cannot collide. Three new Tauri commands wire it to the UI:
  `set_provider_keyring_key`, `list_keyring_providers`,
  `delete_provider_keyring_key`. Empty-string writes delete the entry.

- **Per-provider recommended `max_tokens`**
  Anthropic requires an explicit `max_tokens`; previously we hardcoded
  2048 which truncates long Claude responses mid-stream. New
  `recommended_max_tokens(provider_id)` returns:
  - Anthropic → 8192 (Claude Sonnet/Opus budget)
  - MiniMax / OpenAI → 4096
  - Ollama → 4096
  - other → 2048

- **System prompt injection** in `commands.rs::chat`
  Before the user message lands, we prepend a hub-managed system prompt
  that primes the model on guardrails, available tool categories, and
  the active chatroom. Built via `build_system_prompt(tools, active_chat)`
  using a compact categoriser (`categorize_tool`) so the prompt stays
  short even with 24 tools loaded.

- **Streaming SQLite flush** in the background `chat` task
  Every 8 delta events we INSERT OR REPLACE a partial assistant turn
  into `~/.line-hub/history.sqlite3` using `seq = i64::MAX` as a
  sentinel. A crash mid-stream now loses at most 8 deltas instead of
  the entire response.

### 🧪 Tests

- `keyring` adds 4 tests (round-trip / missing-get / missing-delete /
  list configured).
- `e2e` integration suite adds 8 tests covering:
  `config_defaults_inject_minimax`, `history_and_config_coexist`,
  `anthropic_request_serialises_with_strict_wire_shape`,
  `anthropic_tool_use_block_serialises_back_to_content_blocks`,
  `message_enum_round_trip_for_anthropic_style_assistant`,
  `streaming_fixture_emits_done_after_message_stop`,
  `user_only_request_serialises_cleanly_through_minimax_path`,
  `shared_arc_clones`.

Total: **34 tests passing** across the workspace.

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