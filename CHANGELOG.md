# Changelog

All notable changes to LINE Model Hub are documented here. Versions follow
[Semantic Versioning](https://semver.org/).

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