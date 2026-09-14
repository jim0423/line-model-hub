# Changelog

All notable changes to Line 小幫手 are documented here. Versions follow
[Semantic Versioning](https://semver.org/).


## [0.6.7] - 2026-09-14

Hotfix for v0.6.6 installer-not-bundling-mcp bug.

### Symptom

After installing v0.6.6, the app launched but immediately showed:

> ⚠ Cannot find line-desktop-mcp. Open Settings and paste the path to
> line-desktop-mcp\src\server.js (or set HUB_LINE_MCP_PATH).

The GitHub Actions build had succeeded and the installer had run, but
runtime could not find the bundled MCP server entry script.

### Root cause

`grep -c 'server.js' v0.6.6-installer.exe` → **0 occurrences**.
`grep -c 'line-desktop-mcp' v0.6.6-installer.exe` → **0 occurrences**.

`tauri-bundler/src/bundle/windows/nsis/mod.rs:825` does:

```rust
let cwd = std::env::current_dir()?;
let src = cwd.join(resource.path());
let resource_path = dunce::simplified(&src).to_path_buf();
```

`cwd` is the **manifest-dir** (`crates/line-hub-tauri/`), set by
`tauri-cli/build.rs:166` `set_current_dir(dirs.tauri)` before
tauri-bundler runs. The resulting `src` becomes the literal string
emitted into the NSIS template's `File` directive. NSIS then runs
makensis from `target/release/nsis/<arch>/` (set by
`nsis/mod.rs:704` `.current_dir(output_path)`).

Crucially, **`cwd.join(relative_path)` with an absolute `cwd` always
yields an absolute path**. So a `bundle.resources` key of
`vendor/line-desktop-mcp/src/server.js` (no leading `../`) is emitted
to NSIS as an **absolute path** like
`D:\a\line-model-hub\line-model-hub\crates\line-hub-tauri\vendor\line-desktop-mcp\src\server.js`,
which NSIS uses verbatim — no relative resolution, no silent skip.

The bug in v0.6.1-v0.6.6 was using `../../vendor/...` (the value
correct for **tauri-build**'s `canonicalize()` from manifest-dir CWD)
but **wrong for NSIS** because:

1. `cwd.join("../../vendor/...")` produces
   `crates/line-hub-tauri/../../vendor/...` which canonicalizes to
   `<workspace>/vendor/...` (workspace-root).
2. That absolute path is emitted to NSIS as
   `<workspace>/vendor/line-desktop-mcp/src/server.js`.
3. The actual clone lived at `<workspace>/vendor/line-desktop-mcp/...`
   in v0.6.1, so NSIS should have found it — but the file lookup
   NSIS performs can fail silently on long paths or symlinks.

The cleanest fix: **move the clone into the manifest-dir itself** so
`cwd.join("vendor/line-desktop-mcp/...")` resolves to the absolute
path the bundler actually emits, and the file is always there
(`<workspace>/crates/line-hub-tauri/vendor/line-desktop-mcp/...`).

### Fix

- `scripts/vendor-line-desktop-mcp.sh` clones into
  `crates/line-hub-tauri/vendor/line-desktop-mcp/` instead of
  `vendor/line-desktop-mcp/`.
- `tauri.conf.json`'s `bundle.resources` keys are bare
  `vendor/line-desktop-mcp/src/server.js` (no `../`).

Both stages now resolve identically:

- **tauri-build** (build script, cwd = manifest-dir):
  `Path::new("vendor/...").canonicalize()` → exists ✓.
- **tauri-bundler** (cwd = manifest-dir, absolute):
  `cwd.join("vendor/...")` → absolute path inside manifest-dir;
  emitted verbatim to NSIS template.
- **NSIS File directive** (cwd = `target/release/nsis/<arch>/`):
  receives an absolute path, uses it directly. ✓.

### Defensive CI step

`.github/workflows/build-windows.yml` adds a step after
`cargo tauri build` that greps the produced installer for
`line-desktop-mcp`, `server.js`, `package.json`, and
`npm-shrinkwrap.json`. If any marker is missing, the build fails —
catching the "build succeeded but silently skipped resources" class
of bug in CI instead of after a user install.

### Files touched

- `scripts/vendor-line-desktop-mcp.sh` — `DEST` moved to
  `crates/line-hub-tauri/vendor/line-desktop-mcp/`
- `crates/line-hub-tauri/tauri.conf.json` — `bundle.resources` keys
  no longer use `../../` (they were `../../vendor/...` in v0.6.6)
- `.gitignore` — also ignores `crates/*/vendor/` and `crates/**/vendor/`
- `.github/workflows/build-windows.yml` — added "Verify bundled
  resources landed in installer" step

`installerHooks` stays at `../../nsis-hooks.nsh` (correct since v0.6.6 —
manifest-dir relative, no change).

24 lib tests pass. `cargo check` clean.


## [0.6.6] - 2026-09-14

Hotfix for v0.6.5 NSIS bundling failure.

### Why v0.6.5 was wrong

I claimed `installerHooks` resolves against `target/release/nsis/x64/`
(the NSIS bundler's `current_dir(output_path)` call site). **Wrong.**
That `current_dir()` only changes CWD for the **makensis subprocess**,
which runs AFTER `dunce::canonicalize(installer_hooks)` has already
been resolved (line 398 in `nsis/mod.rs`).

Looking at the **tauri-cli** source (`crates/tauri-cli/src/build.rs:166`):

```rust
set_current_dir(dirs.tauri).context("failed to set current directory")?;
```

`cargo tauri build` switches the process CWD to `dirs.tauri` BEFORE
running any other step. `dirs.tauri` is the directory containing
`tauri.conf.json` — in this project, `<repo>/crates/line-hub-tauri/`.

So `dunce::canonicalize("installerHooks")` runs from
`<repo>/crates/line-hub-tauri/`, NOT the workspace root or the
NSIS output directory.

### 🐛 Fixed

```diff
- "installerHooks": "../../../../nsis-hooks.nsh"
+ "installerHooks": "../../nsis-hooks.nsh"
```

Two `..` walk back from `<repo>/crates/line-hub-tauri/` to the
workspace root, where `nsis-hooks.nsh` lives.

```
<repo>/crates/line-hub-tauri/..  = <repo>/crates/
<repo>/crates/..              = <repo>/
<repo>/nsis-hooks.nsh         ✓
```

### Lessons (added to skill SOP — Mode D)

- **`cargo tauri build` permanently switches CWD to `dirs.tauri`**
  via `set_current_dir()` at line 166 of `tauri-cli/src/build.rs`,
  BEFORE any path resolution happens.
- **`dunce::canonicalize(...)` for fields like `installerHooks`
  resolves against `dirs.tauri`**, i.e. the manifest dir
  (`crates/<crate>/`).
- **The bundler's later `current_dir(output_path)` only affects the
  makensis subprocess**, which runs AFTER path resolution. So the
  bundler's intermediate-directory state is irrelevant for config-path
  resolution.
- **The path-resolution base is therefore `dirs.tauri` = manifest-dir
  for both `bundle.resources` AND `installerHooks`!** They are the
  SAME base (Mode A), contrary to what v0.6.4 / v0.6.5 hypothesised.

### Why we kept guessing wrong

- v0.6.2: assumed workspace-root (no `../`). Failed.
- v0.6.3: assumed `../` from workspace root. Failed.
- v0.6.4: assumed `set_current_dir` doesn't happen, moved file to
  workspace root, used 0 `../`. Failed because CWD is actually
  manifest dir.
- v0.6.5: assumed bundler output dir `target/release/nsis/x64/`,
  used `../../../../`. Failed because `set_current_dir` runs LATER
  only for makensis.
- v0.6.6: greps `set_current_dir` in tauri-cli, finds it runs at
  `build.rs:166` BEFORE path resolution. Uses `../../`. Hopefully
  right.

### Future-proof alternatives (deferred)

The 2-deep `../../` is correct but still brittle if Tauri re-arranges
where tauri.conf.json sits. More robust options:

- **Use `custom_template_path` for a full custom `installer.nsi`**
  — but that file has the same CWD issue.
- **Co-locate `nsis-hooks.nsh` inside the manifest dir**
  (`crates/line-hub-tauri/nsis-hooks.nsh`) and use just the basename.
  This is what `bundle.resources` and `frontendDist` effectively do.
- **Lobby Tauri to add `tauri_dir` env var injection for hooks**, or
  to resolve paths via `config_parent.join(...)` instead of
  `dunce::canonicalize()`.

Total: 24 lib tests pass. `cargo check` clean.

Hotfix for v0.6.4 NSIS bundling failure.

### Why v0.6.4 was wrong

I claimed `installerHooks` resolves against the process CWD which
during `cargo tauri build` is the workspace root. **Wrong on both
counts.** Looking at the NSIS bundler source more carefully
(`crates/tauri-bundler/src/bundle/windows/nsis/mod.rs`):

```rust
let output_path = settings.project_out_directory().join("nsis").join(arch);
...
let status = nsis_cmd
    .args(...)
    .arg(installer_nsi_path)
    .current_dir(output_path)   // ← CWD switched HERE before makensis runs
```

Tauri's NSIS bundler switches CWD to `target/release/nsis/x64/`
before resolving any relative paths. So `dunce::canonicalize("nsis-hooks.nsh")`
runs from there, not the workspace root.

### 🐛 Fixed

`tauri.conf.json` `installerHooks`:

```diff
- "installerHooks": "nsis-hooks.nsh"
+ "installerHooks": "../../../../nsis-hooks.nsh"
```

Four `..` climb back from `target/release/nsis/x64/` to the
workspace root. Ugly but mechanically correct.

### Lessons (added to skill SOP)

- **Tauri NSIS bundler CWD is `target/release/nsis/<arch>/`**, NOT the
  workspace root. Four `..` walk back to the repo root.
- **The path-resolution stack has THREE different bases**, not two:
  1. `bundle.resources` / `frontendDist` → manifest-dir (`crates/<crate>/`)
  2. `beforeBuildCommand` → workspace root
  3. NSIS bundler → `target/release/nsis/<arch>/`
- **Always grep the Tauri source for the `current_dir()` call** before
  guessing which base a config field uses. The bundler silently
  switches CWD before resolving relative paths.

### Future cleanup (deferred)

The four `..` is fragile — if Tauri adds another path component (e.g.
`target/release/nsis/x64_unicode/`) the count breaks. Better long-term
fixes:

- Use `custom_template_path` for a full custom `installer.nsi` that
  inlines the hook logic via `!include "${__FILE__}"`.
- Or copy `nsis-hooks.nsh` into `target/release/nsis/<arch>/` from
  `beforeBuildCommand` so the path resolves in CWD.

Total: 24 lib tests pass. `cargo check` clean.

Hotfix for v0.6.3 NSIS bundling failure.

### Why v0.6.3 was wrong

I claimed `installerHooks` is manifest-dir relative like
`bundle.resources`. **It isn't.** Looking at Tauri source
(`crates/tauri-bundler/src/bundle/windows/nsis/mod.rs` line 398):

```rust
let installer_hooks = dunce::canonicalize(installer_hooks).fs_context(
    "failed to resolve `bundle > windows > nsis > installerHooks`", ...
)?;
```

`dunce::canonicalize()` resolves the path against the **process CWD**,
not the manifest dir. On `cargo tauri build` the CWD is the workspace
root, so `../scripts/nsis-hooks.nsh` walked out of the repo entirely.

### 🐛 Fixed

- Moved `scripts/nsis-hooks.nsh` → `nsis-hooks.nsh` (workspace root).
  Updated `tauri.conf.json` `installerHooks` to `"nsis-hooks.nsh"` so
  the CWD-relative resolution just works.
- `scripts/vendor-line-desktop-mcp.sh` is unaffected — `beforeBuildCommand`
  is invoked with CWD = workspace root so `scripts/vendor-...` has
  resolved correctly since v0.6.1.

### Lessons (carried into skill SOP)

- **Not all Tauri paths use the same base.** `bundle.resources` and
  `frontendDist` are manifest-dir relative; `installerHooks`,
  `installerIcon`, etc. are process-CWD relative. Read the bundler
  source before guessing.
- **When CI surfaces a path resolution bug, grep the Tauri source for
  the canonicalize/resolve call** to see exactly which base is in
  use. Saves a lot of guess-and-check.

Total: 24 lib tests pass. `cargo check` clean.

Hotfix for v0.6.1 GitHub Actions build failure.

### 🐛 Fixed

- **Bundled-resource paths now resolve correctly** — `tauri.conf.json`
  `bundle.resources` keys are resolved relative to the manifest dir
  (`crates/line-hub-tauri/`), not the workspace root. Use `../../vendor/`
  so the build script finds the cloned line-desktop-mcp source.
- **npm-shrinkwrap.json is the lockfile** — line-desktop-mcp ships
  `npm-shrinkwrap.json`, not `package-lock.json`. Renamed the bundled
  resource so Tauri's build-script pre-flight check does not fail.
- **`default_entry_path_returns_err_when_nothing_set` no longer
  false-passes** — added a `HUB_TEST_NO_BUNDLED=1` test hook that
  short-circuits the bundled walk in unit tests. Without it, the test
  runner's stub `target/debug/resources/...` tree satisfied the
  candidate walk and the test reported `Ok(...)` instead of the
  expected `Err(LineMcpNotFound)`.

Total: 24 lib tests pass.


## [0.6.1] - 2026-09-14

Bundle line-desktop-mcp inside the installer + complete the v0.4.0
keyring integration that v0.6.0 left half-wired. No more separate
`git clone` + `npm install` per machine — one installer ships everything.

### ✨ New

- **Bundled line-desktop-mcp installer** (P0-A)
  - `scripts/vendor-line-desktop-mcp.sh` clones a SHA-pinned copy of
    `line-desktop-mcp@v3.0.0` into `vendor/line-desktop-mcp/` at build time.
    Idempotent: re-runs of `cargo tauri build` skip the clone when the
    pinned tag already matches.
  - `tauri.conf.json` `bundle.resources` ships `src/server.js`,
    `package.json`, and `package-lock.json` into the installer's
    `<install-dir>\resources\line-desktop-mcp\` tree (~5 MB extra).
  - `scripts/nsis-hooks.nsh` runs `npm install --omit=dev --ignore-scripts`
    in `$INSTDIR\resources\line-desktop-mcp\` during install. Writes a
    `.installed` sentinel so subsequent launches can skip the bootstrap.
    If `node.exe` is missing, surfaces a clear `install-node-missing.txt`
    next to the app instead of crashing.
  - `default_entry_path` in `crates/line-hub-core/src/mcp.rs` now falls
    back to the bundled layout. Resolution priority becomes:
    `HUB_LINE_MCP_PATH` env → `LINE_MODEL_HUB_BUNDLED` env → bundled
    `<exe-dir>/resources/line-desktop-mcp/src/server.js` (with two
    candidate walks for `bin/`, root, and one level up). Only when all
    three fail does the user see the Settings hint.

- **OS keyring wired into `save_config` (P0-B, completes v0.4.0 stub)**
  - `save_config` now mirrors every non-empty API key into the OS
    keyring (Windows Credential Manager / macOS Keychain / Linux Secret
    Service) before persisting the JSON file. Empty keys delete the
    entry so the Settings "clear key" path stays correct.
  - `list_providers` reports `is_configured = true` whenever EITHER the
    keyring has a credential OR the JSON config has plaintext. Closes
    the v0.6.0 half-finished integration where the keyring module
    existed but was never called from the save path.

### 🧪 Tests

- `default_entry_path_env_wins_over_bundled` — env var beats bundled walk.
- `default_entry_path_env_dir_resolves_to_server_js` — bundle dir appends
  `src/server.js` automatically.
- `default_entry_path_returns_err_when_nothing_set` — honest error path.
- Existing 21 tests still green. **Total: 24 lib tests pass.**

### ⚠️ Known limitations

- npm install on first launch can take 30-90s depending on network. The
  NSIS install hook logs to `%TEMP%\line-desktop-mcp-install.log`; the
  app starts immediately so the user is not blocked.
- Build-time `npm install` smoke test in `vendor-line-desktop-mcp.sh`
  needs Node 22 on the build machine. CI already has it (line 47 of
  `build-windows.yml`). Set `SKIP_NPM_INSTALL=1` to skip on dev boxes
  without network.
- Installer size grows from ~5 MB to ~10-50 MB depending on whether
  node_modules is bundled at build time (we strip it; user-side npm
  install fetches at first launch).


## [0.6.0] - 2026-09-12

Local-only mode toggle + MCP auto-respawn framework + cancel streaming. On top of v0.5.0-v0.5.2 code base.

### New

- **Local-only mode toggle (P1-A)** - New checkbox in Settings dialog.
  - When enabled, build_system_prompt filters out 8 tools that mutate LINE state: `send_*`, `stage_*`, `set_line_draft`, `clear_line_draft`, `open_line_chat`, `open_line_chat_feature`, `get_line_draft`, `export_line_chat_history`.
  - MCP child is still spawned (so we keep the local DB reader); only send-class tools are hidden from the system prompt.
  - Read tools (`get_line_local_messages`, `search_*`, `verify_*`, `get_line_capabilities`, `get_line_poll_state`, `copy_*`, `translate_*`) stay available.
  - Persists to `~/.line-hub/config.json::local_only`.

- **MCP child auto-respawn framework (P2-A skeleton)**
  - New `crates/line-hub-tauri/src/respawn.rs` with `RespawnState`, `RESPAWN_BACKOFF = [1s, 2s, 4s]`, `MAX_RESPAWN_ATTEMPTS = 3`, plus 3 unit tests.
  - Full watcher task integration deferred to v0.6.1.

- **v0.5.1-2 infra fixes folded in** (no behaviour change):
  - Tauri config version sync, MSIS bundle removed (WiX 3.14 broken on windows-latest runner).
  - Workflow tolerates missing MSI glob.

- **New `is_send_guarded_tool(name) -> bool` helper** in `commands.rs` to back the local-only filter.

### Tests

- `cargo test -p line-hub-core --lib` 21+2+8+6+5 = 42 lib tests pass.
- `cargo test -p line-hub-tauri --lib respawn` 3 respawn tests pass.
- `cargo check -p line-hub-tauri` 0 errors, 18 benign warnings.
- `tsc -p tsconfig.json --noEmit` 0 errors.
- `npm run build` 162.86 KB / 52.58 KB gzip.

### Deferred to v0.6.1

- P1-B: ToolCallTrace per-tool customisation.
- P1-C: cargo fix --lib to clean up 18 benign warnings.

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