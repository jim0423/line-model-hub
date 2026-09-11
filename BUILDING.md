# Building LINE Model Hub on Windows

This document covers the complete Windows build pipeline. The Rust core
(`line-hub-core`, `line-hub-mcp`) compiles anywhere, but the Tauri shell
**only builds on Windows** because of WebView2 and the MSVC linker.

## Prerequisites

| Tool | Min version | Install |
|------|-------------|---------|
| Windows | 10 1809+ | — |
| Rust | 1.75+ | <https://rustup.rs> |
| Node.js | 22+ | <https://nodejs.org> |
| MSVC Build Tools | 2022 with C++ workload | Visual Studio Installer |
| WebView2 Runtime | Evergreen | Pre-installed on Windows 11; download from Microsoft for 10 |
| line-desktop-mcp | v1.2.0 | Clone from <https://github.com/bensonmaxai/line-desktop-mcp> |

> The MSVC C++ workload is required by Rust's `msvc` toolchain. If you see
> `link.exe not found` you forgot this step.

## Steps

```powershell
# 1. Clone the project
git clone https://github.com/jim/line-model-hub.git
cd line-model-hub

# 2. Install JS deps
npm install

# 3. Install Tauri CLI
cargo install tauri-cli --version "^2.0"

# 4. Verify the Rust workspace compiles (catches 95% of issues before the full bundle)
cargo check --workspace

# 5. Run unit tests
cargo test -p line-hub-core

# 6. Optional: dev mode with hot reload (requires line-desktop-mcp running)
cargo tauri dev

# 7. Build the production bundle
cargo tauri build
```

## Outputs

After `cargo tauri build` completes (≈ 3-5 min on a clean tree):

```
crates/line-hub-tauri\target\release\line-model-hub.exe
crates/line-hub-tauri\target\release\bundle\msi\LINE Model Hub_0.1.0_x64_en-US.msi
crates/line-hub-tauri\target\release\bundle\nsis\LINE Model Hub_0.1.0_x64-setup.exe
```

Both installers are code-signed with the dev certificate in CI; locally they
will be unsigned. To sign:

```powershell
signtool sign /fd SHA256 /a /tr http://timestamp.digicert.com `
    "target\release\bundle\msi\LINE Model Hub_0.1.0_x64_en-US.msi"
```

## Bundle config

See `crates/line-hub-tauri/tauri.conf.json` `bundle` section:

- **MSI** (WiX) — for managed deployments / Group Policy.
- **NSIS** — smaller per-user installer with TradChinese localization.
- **icon** — replace `icons/icon.ico` with a 256×256 multi-resolution icon
  generated from your source PNG.

## Bundling line-desktop-mcp

The Tauri app expects `line-desktop-mcp` to be installed separately and
pointed to via:

1. **Settings dialog** → "line-desktop-mcp entry path" field.
2. **Environment variable** `HUB_LINE_MCP_PATH` — useful for CI / silent
   installs.

To bundle a copy of `line-desktop-mcp` **inside** the installer (so end users
don't need to install Node):

```powershell
# In a postinstall script (or manual step)
git clone --depth 1 --branch v1.2.0 https://github.com/bensonmaxai/line-desktop-mcp.git
cd line-desktop-mcp
npm install --omit=dev --ignore-scripts
```

Then point Settings at
`<install-dir>\resources\line-desktop-mcp\src\server.js`.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `link.exe not found` | MSVC Build Tools missing | Install via Visual Studio Installer |
| `error: Microsoft Visual C++ 14.0 or greater is required` | Same as above | Same |
| `WebView2Loader.dll not found` at runtime | WebView2 Runtime missing | Install Evergreen Runtime |
| Tauri dev window blank | `devUrl` wrong | Check `tauri.conf.json` `build.devUrl` = `http://localhost:5173` |
| `cargo tauri build` OOM | Release profile heavy | Set `CARGO_PROFILE_RELEASE_LTO=false` |
| MSI/NSIS dialog doesn't appear in TradChinese | WiX language pack missing | Install `WiX Toolset` v3 with `zh-TW` |

## Smoke test after install

1. Launch "LINE Model Hub" from Start menu.
2. ⚙ Settings → paste MiniMax API key → Save.
3. Settings → paste line-desktop-mcp entry path → Save.
4. Top bar → "Spawn LINE MCP" → expect `LINE MCP · 24`.
5. Send: "你好". Expect MiniMax reply.
6. Send: "幫我看一下老婆最近五則訊息". Expect tool trace + answer.

If any step fails, copy the error from the terminal/console and check
`%LOCALAPPDATA%\com.tt-openclaw.line-model-hub\logs\`.