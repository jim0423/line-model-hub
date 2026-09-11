# LINE Model Hub vs line-desktop-mcp v1.2.0 — 功能差距分析

**Date**: 2026-09-11
**Compared against**: https://github.com/bensonmaxai/line-desktop-mcp/blob/main/docs/features.md
**Our current**: v0.1.3 (in flight via GitHub Actions)

## 1. 我們「自動繼承」的功能（24 個工具全支援）

因為 LINE Model Hub 是 **MCP client**，不是自己實作工具 — 我們 spawn `line-desktop-mcp` child process，把所有 24 個 tool 透過 JSON-RPC 公開給 AI。所有 line-desktop-mcp 的工具**原生支援**，不需要我們重新實作：

| 分類 | 工具 | 支援 |
|------|------|------|
| **1. 讀取歷史 (4)** | `get_line_chatroom_history_short` | ✅ 自動 |
| | `get_line_chatroom_history_default` | ✅ 自動 |
| | `get_line_chatroom_history_long` | ✅ 自動 |
| | `get_line_chat_messages` | ✅ 自動 |
| **2. 搜尋/核對 (2)** | `search_line_chat_messages` | ✅ 自動 |
| | `verify_line_message` | ✅ 自動 |
| **3. 發送文字 (2)** | `send_message_auto` | ✅ 自動（**client UI 不會原生擋**） |
| | `send_message_manual` | ✅ 自動 |
| **4. 管理草稿 (3)** | `get_line_draft` | ✅ 自動 |
| | `set_line_draft` | ✅ 自動 |
| | `clear_line_draft` | ✅ 自動 |
| **5. 匯出聊天 (1)** | `export_line_chat_history` | ✅ 自動 |
| **6. 附件/引用/操作 (5)** | `send_file_manual` | ✅ 自動 |
| | `stage_line_reply` | ✅ 自動 |
| | `copy_line_message` | ✅ 自動 |
| | `translate_line_message` | ✅ 自動 |
| | `stage_line_forward` | ✅ 自動 |
| **7. 導覽/狀態 (7)** | `get_line_capabilities` | ✅ 自動 |
| | `get_line_workflow` | ✅ 自動 |
| | `get_line_status` | ✅ 自動 |
| | `open_line_chat` | ✅ 自動 |
| | `get_line_ui_state` | ✅ 自動 |
| | `confirm_line_chat_view` | ✅ 自動 |
| | `open_line_chat_feature` | ✅ 自動 |

**所有 24 個工具呼叫 AI 都能用** — 因為 line-desktop-mcp 是 child server。

---

## 2. 我們「多出來」的功能（line-desktop-mcp 沒有的）

### A. 多 AI provider 切換（4 個 provider）
- `line-desktop-mcp` 綁定 **Codex CLI**（單一 AI）
- 我們支援 **MiniMax M3**、**OpenAI**、**Anthropic**、**Ollama** 自由切換

### B. 多模型切換（每個 provider 多 model）
- MiniMax: M3 / M2.7 / ...
- OpenAI: GPT-4o / GPT-4-Turbo / GPT-3.5
- Anthropic: Claude 3.5 Sonnet / Opus / Haiku
- Ollama: 本機任意

### C. 跨平台 desktop shell
- 用 **Tauri 2**（Rust + React + WebView2）而不是 CLI 命令列
- 給使用者 **GUI 對話框**而不是 terminal prompt

### D. 串流回應（streaming tokens）
- `line-desktop-mcp` 透過 Codex CLI 必須等整段 AI 輸出
- 我們 **SSE stream** 邊收邊顯示（已實作 `reqwest-eventsource` + thinking-tag stripping）

### E. MiniMax M3 特殊處理
- **平行 `delta.reasoning` channel** → `StreamEvent::ReasoningDelta`（與一般 content 分開路由）
- 思考 tags 未閉合 → buffer carry-over（不會丟字）
- 11 個 SSE unit tests 守護

### F. API key 安全儲存（規劃中）
- `line-desktop-mcp` 把 LINE bot token 放環境變數
- 我們用 **`keyring` crate**（Windows Credential Manager）— **目前還沒串接到 UI**

### G. Send-confirm dialog
- 規劃 **native Tauri dialog** 在 AI 提議發送訊息時跳「確認」按鈕
- 防止 AI 自己 send 而沒人 review

---

## 3. 我們「沒做到」的 line-desktop-mcp 功能

### ⚠️ 1. Safety Guards（**部分缺失**，高優先）
line-desktop-mcp 有 4 個安全機制防止 AI 把使用者剛編輯的內容蓋掉：

| Guard | line-desktop-mcp | 我們 |
|-------|------------------|------|
| `set_line_draft`/`clear_line_draft` 需要 `expectedDraft` 匹配 | ✅ | ❌ **沒實作** — 雖然是 line-desktop-mcp 內部檢查，我們 Rust client 不會攔 |
| `export_line_chat_history` SHA-256 + readback + no-overwrite | ✅ | ❌ 同上 |
| `confirm_line_chat_view` header-pixel token gate | ✅ | ❌ 同上 |
| Cross-process lock `~/.line-desktop-mcp/operation.lock` | ✅ | ❌ 同上 |

**現實**：這些 guard 是 **line-desktop-mcp 內部做**的 — 我們純 client 沒權限加。我們能做的是：
- 在我們 Rust client 加 **JSON-RPC 觀察層**：AI 想呼叫 `set_line_draft` 時先在 **我們這層**檢查 `expectedDraft`
- 缺點：增加 30% 程式複雜度、需要 fork 或 override line-desktop-mcp 的合約
- **未實作**

### ⚠️ 2. Send-confirm Dialog（**規劃有，沒實作**）
- AI 提議 send_message_auto → 我們前端應該跳 native confirm
- 我們 `commands.rs` 有 `confirm_send_dialog` function **但 `#[allow(dead_code)]`** — 沒串到 chat loop
- **Agentic loop 沒實作** — v0.1.3 只是「送出 → 等 AI → 顯示 AI 回覆」單趟，不是「AI 來回呼叫 tool 直到決定」

### ⚠️ 3. ToolCallTrace view 太薄
- 我們前端只有 **單一 generic tool trace**：顯示 tool name + args 預覽
- line-desktop-mcp 在前端 vs. 後端 沒有可比性（它純後端）
- 但**理想的 UX**應該：
  - `send_message_auto` 顯示「目標: Demo / 訊息: ...」並要求按鈕「送出」
  - `export_line_chat_history` 顯示「檔案路徑: C:\... / 筆數: 30 / 格式: csv」並讓人改路徑
  - `set_line_draft` 顯示「現有 draft: ... / 即將覆蓋: ...」並顯示 diff
- **v0.1.3 沒做**

### ⚠️ 4. 沒有「建議下一步」workflow guidance
- line-desktop-mcp 有 `get_line_workflow` 工具，告訴 AI「如果你想 reply 給某訊息，步驟是 X/Y/Z」
- 我們的 system prompt **沒主動引導 AI 用 workflow tool**

### ⚠️ 5. 沒有「能力偵測」
- `get_line_capabilities` 我們前端**沒顯示**這個工具的 output — AI 拿到 raw JSON 但 UI 不會視覺化「這個 LINE 帳號能做 / 不能做什麼」

### ⚠️ 6. MCP child failure recovery
- line-desktop-mcp 死了 → 我們 app 還能繼續（雖然 AI 沒 tool 用）
- 但**沒自動 respawn 機制** — 使用者要手動按「Spawn LINE MCP」

### ⚠️ 7. 對話歷史持久化
- 我們 `ChatSession.history: Vec<Message>` **只在記憶體**
- 關 app → 對話不見
- 沒有 sqlite 存對話歷史

### ⚠️ 8. 多 session / 多 chatroom
- 我們只有 `SESSION = "default"` 一個 session
- 不能同時跟「老婆」對話又跟「同事群」對話

### ⚠️ 9. Streaming abort / cancel
- `commands.rs` 有 `cancel_chat` function 但**沒串到 UI**
- AI 講到一半沒辦法按「停」

### ⚠️ 10. Keyring 整合
- `crates/line-hub-core/Cargo.toml` 宣告了 `keyring` 但**沒在 commands.rs 用**
- API key 還在 `~/.line-hub/config.json` **明文** — 不安全

### ⚠️ 11. Anthropic provider SSE driver 是 stub
- 我們有 `provider/anthropic.rs` 但**只 list_models 沒 chat SSE**
- 切到 Anthropic 對話會失敗

### ⚠️ 12. 沒有 icons 品牌
- 我們用 placeholder LINE 綠 icon
- 沒有真的 LINE × LLM 品牌設計

---

## 4. 對齊優先順序建議（Jim 想加什麼先做）

| Prio | 項目 | 工作量 | Jim 影響 |
|------|------|--------|---------|
| **P0** | ToolCallTrace 客製化（看 send / export / draft 各自視覺） | 1-2 天 | 高（AI 變透明） |
| **P0** | Send-confirm dialog（agentic loop + 確認） | 2-3 天 | 高（安全） |
| **P1** | 對話歷史存 sqlite | 1 天 | 中 |
| **P1** | Keyring API key 儲存 | 0.5 天 | 中（安全） |
| **P1** | Cancel streaming 按鈕 | 0.5 天 | 中（UX） |
| **P2** | Anthropic SSE driver 實作 | 1-2 天 | 中（多 provider 完整） |
| **P2** | MCP child 自動 respawn | 0.5 天 | 低 |
| **P3** | Multi-session | 1-2 天 | 低（進階） |
| **P3** | 真正的品牌 icon | 1 天 | 低 |

---

## 5. 簡明一句話

**LINE Model Hub = line-desktop-mcp 的所有功能 + 多 AI provider 自由切換 + Tauri GUI + SSE streaming**。

但 **agentic loop（AI 自主呼叫 tool 來回決策）+ UX 客製化 + 安全 guard 加強**還沒到位，這三個是讓它從「能跑」變「好用」的核心。

---