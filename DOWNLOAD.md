# 一鍵 build LINE Model Hub Windows installer

> 用 GitHub Actions 在雲端 build 出 `.msi` 與 `-setup.exe`，
> Jim 直接從 **Releases 頁面**下載測試。

## 為什麼不在本地 build？

- Linux 主機沒有 **MSVC 工具鏈**（`link.exe`）+ **WebView2 SDK**
- `cargo tauri build` 在 Windows 跑要 Visual Studio Build Tools + WiX
- 雲端 GitHub Actions 跑 windows-latest runner，**3-5 分鐘**出 installer

## 怎麼觸發？

### 方式 A：push tag（推薦）

```bash
cd /home/tt-openclaw/workspace/line-model-hub
git init
git add -A
git commit -m "v0.1.0 initial release"
git tag v0.1.0
git remote add origin https://github.com/jim/line-model-hub.git
git push -u origin main --tags
```

→ 5 分鐘後 https://github.com/jim/line-model-hub/releases/tag/v0.1.0 有：

```
line-model-hub.exe            # 單一執行檔（≈ 8 MB）
LINE Model Hub_0.1.0_x64_en-US.msi     # WiX 安裝包
LINE Model Hub_0.1.0_x64-setup.exe    # NSIS 安裝包
```

### 方式 B：手動 trigger

1. 把程式 push 到 GitHub（隨便一個 commit）
2. 到 repo 頁面 → Actions → "build-windows" → Run workflow
3. 跑完後在 run 頁面下載 Artifacts

### 方式 C：本地 Windows build

如果 Jim 那邊有 Windows 機器，直接照 [BUILDING.md](BUILDING.md) 跑
`cargo tauri build`，不需要 GitHub Actions。

## 下載後

```powershell
# 用 MSI（企業部署）
msiexec /i "LINE Model Hub_0.1.0_x64_en-US.msi"

# 或 NSIS（個人安裝，推薦）
.\"LINE Model Hub_0.1.0_x64-setup.exe"

# 跑起來
& "C:\Program Files\LINE Model Hub\line-model-hub.exe"
```

第一次啟動：
1. ⚙ Settings → 貼 MiniMax API key
2. ⚙ Settings → 貼 line-desktop-mcp 的 server.js 路徑
3. 點 **Spawn LINE MCP** → 看是否 `LINE MCP · 24`

詳細 SOP 見 [BUILDING.md](BUILDING.md) 與 [README.md](README.md)。