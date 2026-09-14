//! MCP client + tool registry.
//!
//! Spawns `line-desktop-mcp` as a child process, performs the JSON-RPC
//! `initialize` handshake, advertises the discovered tools to the
//! conversation loop, and dispatches `tools/call` invocations.
//!
//! v0.5.0 additions
//! ----------------
//! * Content-block aware `call_tool` — the raw `result.content[]` from
//!   line-desktop-mcp (which may carry `image` / `audio` base64 previews
//!   via `get_line_local_messages`) is decomposed into typed
//!   [`ToolResult`]s so the UI can render `<img>` and `<audio>` instead
//!   of dumping raw base64 into a text blob.
//! * `with_registry()` constructor — replaces the pre-v0.5.0 stub that
//!   panicked with `unimplemented!()` when `initialize()` tried to
//!   publish the tool list. The registry is now stored behind a
//!   `tokio::sync::RwLock` so reads (e.g. from the conversation loop)
//!   do not contend with the one-time write that happens at spawn.

use crate::provider::ToolDefinition;
use crate::{HubError, HubResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

// -------------------------------------------------------------------------
// Tool registry
// -------------------------------------------------------------------------

/// Opaque handle to a registered tool definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpToolDef {
    pub def: ToolDefinition,
}

#[derive(Default, Debug, Clone)]
pub struct ToolRegistry(pub HashMap<String, McpToolDef>);

impl ToolRegistry {
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.0.values().map(|t| t.def.clone()).collect()
    }
    pub fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

// -------------------------------------------------------------------------
// Tool result (image / audio / text)
// -------------------------------------------------------------------------

/// One piece of a tool call's MCP response.
///
/// line-desktop-mcp v3.0.0 returns structured `content: Content[]` rather
/// than a flat text blob — `get_line_local_messages` for example emits
/// one `text` block with the message array plus zero or more `image` /
/// `audio` blocks for previews. The UI needs to render each block
/// independently (a chat list with thumbnail tiles, an `<audio>` player
/// for WAV, etc.), so we keep the original structure here instead of
/// collapsing everything into a JSON string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResultBlock {
    Text { text: String },
    /// Image preview, encoded inline. `mime_type` is typically
    /// `image/png` or `image/jpeg`. Data is raw bytes (not base64).
    Image { mime_type: String, data: Vec<u8> },
    /// Small PCM WAV audio block validated by line-desktop-mcp.
    Audio { mime_type: String, data: Vec<u8> },
    /// Unsupported media — surfaced as informational text so the AI
    /// can name the file even when it cannot render it.
    Unsupported { mime_type: String, note: String },
}

/// Convenience: tool result holds either an error string (mirrored from
/// MCP `isError: true` responses) or a list of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResult {
    Ok { blocks: Vec<ToolResultBlock> },
    Err { message: String },
}

// -------------------------------------------------------------------------
// MCP client trait
// -------------------------------------------------------------------------

#[async_trait]
pub trait McpTool: Send + Sync {
    async fn list_tools(&self) -> HubResult<Vec<ToolDefinition>>;
    async fn call_tool(&self, name: &str, args: Value) -> HubResult<Value>;
    /// Same as `call_tool` but returns structured content blocks instead
    /// of a `serde_json::Value`. Implementations may keep a fast path
    /// for legacy callers.
    async fn call_tool_structured(&self, name: &str, args: Value) -> HubResult<ToolResult> {
        // Default impl: serialise the legacy Value back to string. The
        // real override in `McpClient` keeps the structured view.
        let raw = self.call_tool(name, args).await?;
        let text = serde_json::to_string(&raw).unwrap_or_default();
        Ok(ToolResult::Ok {
            blocks: vec![ToolResultBlock::Text { text }],
        })
    }
    fn tool_registry(&self) -> ToolRegistry;
    /// Best-effort graceful shutdown of the underlying transport.
    /// Default impl does nothing (for in-memory test doubles).
    async fn shutdown(&self) {}
}

// -------------------------------------------------------------------------
// JSON-RPC wire types
// -------------------------------------------------------------------------

/// JSON-RPC envelope.
#[derive(Debug, Serialize, Deserialize)]
struct JsonRpc {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Deserialize)]
struct McpCallToolResult {
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    is_error: bool,
}

// -------------------------------------------------------------------------
// Client
// -------------------------------------------------------------------------

pub struct McpClient {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<BufReader<tokio::process::ChildStdout>>>>,
    next_id: Arc<Mutex<u64>>,
    /// v0.5.0: registry behind a `RwLock` so reads (concurrent callers)
    /// don't block on `initialize()` writes.
    registry: Arc<RwLock<ToolRegistry>>,
}

impl McpClient {
    /// Spawn `line-desktop-mcp` (or a custom path from `HUB_LINE_MCP_PATH`).
    pub async fn spawn(
        node_path: &str,
        mcp_entry: &str,
        extra_env: &[(&str, &str)],
    ) -> HubResult<Self> {
        let mut cmd = Command::new(node_path);
        cmd.arg(mcp_entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        // Skip line-desktop-mcp's interactive postinstall by pretending to be CI.
        cmd.env("LINE_MCP_SKIP_POSTINSTALL", "1");
        let mut child = cmd
            .spawn()
            .map_err(|e| HubError::LineMcpSpawnFailed(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HubError::McpTransport("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HubError::McpTransport("no stdout".into()))?;
        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let stdout = Arc::new(Mutex::new(Some(BufReader::new(stdout))));
        let child = Arc::new(Mutex::new(Some(child)));

        // v0.5.0: registry initialised empty up-front; `initialize()` will
        // populate it once the `tools/list` handshake completes.
        let client = Self {
            child,
            stdin,
            stdout,
            next_id: Arc::new(Mutex::new(1)),
            registry: Arc::new(RwLock::new(ToolRegistry::default())),
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn next_id(&self) -> u64 {
        let mut g = self.next_id.lock().await;
        let id = *g;
        *g += 1;
        id
    }

    async fn write_request(&self, method: &str, params: Value) -> HubResult<()> {
        let id = self.next_id().await;
        let req = JsonRpc {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: method.into(),
            params: Some(params),
            result: None,
            error: None,
        };
        let line = serde_json::to_string(&req)?;
        let mut g = self.stdin.lock().await;
        let stdin = g
            .as_mut()
            .ok_or_else(|| HubError::McpTransport("stdin closed".into()))?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&self) -> HubResult<JsonRpc> {
        let mut g = self.stdout.lock().await;
        let reader = g
            .as_mut()
            .ok_or_else(|| HubError::McpTransport("stdout closed".into()))?;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(HubError::McpTransport("stdout EOF".into()));
            }
            // Drop the `&mut line` borrow by owning the trimmed slice.
            let owned: String = line.trim().to_owned();
            if owned.is_empty() {
                continue;
            }
            let resp: JsonRpc = serde_json::from_str(&owned)?;
            return Ok(resp);
        }
    }

    async fn request(&self, method: &str, params: Value) -> HubResult<Value> {
        self.write_request(method, params).await?;
        let resp = self.read_response().await?;
        if let Some(err) = resp.error {
            return Err(HubError::McpProtocol {
                code: err.code,
                message: err.message,
            });
        }
        resp.result
            .ok_or_else(|| HubError::McpTransport("missing result".into()))
    }

    async fn initialize(&self) -> HubResult<()> {
        // Send initialize
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "line-model-hub", "version": env!("CARGO_PKG_VERSION") }
        });
        let _ = self.request("initialize", init_params).await?;
        // Send notifications/initialized
        self.write_request("notifications/initialized", serde_json::json!({}))
            .await?;

        // List tools
        let result = self.request("tools/list", serde_json::json!({})).await?;
        let tools_arr = result
            .get("tools")
            .and_then(|t| t.as_array())
            .ok_or_else(|| HubError::McpProtocol {
                code: -1,
                message: "tools/list missing 'tools' array".into(),
            })?;
        let mut reg = ToolRegistry::default();
        for t in tools_arr {
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| HubError::McpProtocol {
                    code: -2,
                    message: "tool missing name".into(),
                })?
                .to_string();
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // inputSchema may live under "inputSchema" or "parameters"
            let parameters = t
                .get("inputSchema")
                .or_else(|| t.get("parameters"))
                .cloned()
                .unwrap_or(serde_json::json!({"type":"object"}));
            reg.0.insert(
                name.clone(),
                McpToolDef {
                    def: ToolDefinition {
                        name,
                        description,
                        parameters,
                    },
                },
            );
        }
        // v0.5.0: write through the RwLock instead of panicking.
        {
            let mut g = self.registry.write().await;
            *g = reg;
        }
        Ok(())
    }

    /// Decode MCP `content: Content[]` into typed [`ToolResultBlock`]s.
    ///
    /// Per the MCP spec a `Content` block is one of:
    ///   * `{type:"text", text: string}`
    ///   * `{type:"image", mimeType: string, data: base64}`
    ///   * `{type:"audio", mimeType: string, data: base64}`
    ///
    /// Anything else falls into `Unsupported` with a note so the UI /
    /// AI can still see the file metadata.
    fn decode_blocks(blocks: Vec<Value>) -> Vec<ToolResultBlock> {
        let mut out = Vec::with_capacity(blocks.len());
        for b in blocks {
            let type_ = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match type_ {
                "text" => {
                    let text = b
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(ToolResultBlock::Text { text });
                }
                "image" | "audio" => {
                    let mime = b
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or(match type_ {
                            "image" => "image/png",
                            _ => "audio/wav",
                        })
                        .to_string();
                    let data = b
                        .get("data")
                        .and_then(|v| v.as_str())
                        .and_then(|s| match type_ {
                            "image" => decode_base64_limited(s, 8 * 1024 * 1024),
                            // 8 MiB cap on inline audio — matches
                            // line-desktop-mcp's own ~256 KB preview
                            // budget but leaves headroom for full
                            // validated WAV blocks (~ a few MB).
                            _ => decode_base64_limited(s, 8 * 1024 * 1024),
                        });
                    match data {
                        Some(bytes) => {
                            let ctor = if type_ == "image" {
                                ToolResultBlock::Image {
                                    mime_type: mime,
                                    data: bytes,
                                }
                            } else {
                                ToolResultBlock::Audio {
                                    mime_type: mime,
                                    data: bytes,
                                }
                            };
                            out.push(ctor);
                        }
                        None => out.push(ToolResultBlock::Unsupported {
                            mime_type: mime,
                            note: "base64 decode failed or exceeded size cap".into(),
                        }),
                    }
                }
                other => {
                    let mime = b
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let note = format!("unsupported content block type={other:?}, mime={mime}");
                    out.push(ToolResultBlock::Unsupported {
                        mime_type: mime,
                        note,
                    });
                }
            }
        }
        out
    }

    /// Find line-desktop-mcp entry script.
    ///
    /// Resolution priority (matches `spawn_mcp` in
    /// `crates/line-hub-tauri/src/commands.rs`):
    ///   1. `HUB_LINE_MCP_PATH` env var (CI / silent installs).
    ///   2. `LINE_MODEL_HUB_BUNDLED` env var pointing at a directory —
    ///      resolved as `<dir>/src/server.js`.
    ///   3. Bundled alongside the running binary, at
    ///      `<exe_dir>/resources/line-desktop-mcp/src/server.js`
    ///      (v0.6.1 installer ships this layout). Falls back to
    ///      `<exe_dir>/../resources/line-desktop-mcp/src/server.js`
    ///      because NSIS installs to `bin\` for some bundle configs.
    ///
    /// Returns `LineMcpNotFound` only when none of the above exist —
    /// `spawn_mcp` then surfaces the human-readable hint that points
    /// the user at Settings.
    pub fn default_entry_path() -> HubResult<PathBuf> {
        if let Ok(p) = std::env::var("HUB_LINE_MCP_PATH") {
            return Ok(PathBuf::from(p));
        }
        if let Ok(dir) = std::env::var("LINE_MODEL_HUB_BUNDLED") {
            return Ok(PathBuf::from(dir).join("src").join("server.js"));
        }
        // Bundled-by-installer fallback — figure out where the running
        // binary lives. `current_exe()` is the documented way; resolves
        // symlinks so we end up in the real install dir.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidates = [
                    dir.join("resources")
                        .join("line-desktop-mcp")
                        .join("src")
                        .join("server.js"),
                    dir.join("..")
                        .join("resources")
                        .join("line-desktop-mcp")
                        .join("src")
                        .join("server.js"),
                    dir.join("..")
                        .join("..")
                        .join("resources")
                        .join("line-desktop-mcp")
                        .join("src")
                        .join("server.js"),
                ];
                for c in &candidates {
                    if c.exists() {
                        return Ok(c.clone());
                    }
                }
            }
        }
        Err(HubError::LineMcpNotFound)
    }

    pub async fn shutdown(&self) {
        let mut g = self.child.lock().await;
        if let Some(mut c) = g.take() {
            let _ = c.kill().await;
        }
    }
}

/// Tolerant base64 decode with an upper size cap. Returns `None` on
/// whitespace/garbage input to keep the UI from rendering half-broken
/// `<img>` tags.
fn decode_base64_limited(s: &str, max_bytes: usize) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let bytes = STANDARD.decode(s.trim()).ok()?;
    if bytes.len() > max_bytes {
        return None;
    }
    Some(bytes)
}

#[async_trait]
impl McpTool for McpClient {
    async fn list_tools(&self) -> HubResult<Vec<ToolDefinition>> {
        Ok(self.tool_registry().definitions())
    }

    async fn call_tool(&self, name: &str, args: Value) -> HubResult<Value> {
        self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        )
        .await
    }

    async fn call_tool_structured(&self, name: &str, args: Value) -> HubResult<ToolResult> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({ "name": name, "arguments": args }),
            )
            .await?;
        match serde_json::from_value::<McpCallToolResult>(result.clone()) {
            Ok(parsed) if parsed.is_error => Ok(ToolResult::Err {
                message: parsed
                    .content
                    .first()
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("MCP tool returned isError=true")
                    .to_string(),
            }),
            Ok(parsed) => Ok(ToolResult::Ok {
                blocks: Self::decode_blocks(parsed.content),
            }),
            Err(_) => {
                // Tool returned something else (e.g. a plain JSON object
                // not wrapped in `content`); serialise back to text so
                // legacy callers still get a preview.
                let text = serde_json::to_string(&result).unwrap_or_default();
                Ok(ToolResult::Ok {
                    blocks: vec![ToolResultBlock::Text { text }],
                })
            }
        }
    }

    fn tool_registry(&self) -> ToolRegistry {
        // Snapshot under the read lock — the conversation loop calls
        // this once per turn so contention is fine.
        match self.registry.try_read() {
            Ok(g) => g.clone(),
            // If a writer is mid-flight during shutdown we may briefly
            // hold a queued lock; in that pathological case fall back
            // to an empty registry rather than block the caller.
            Err(_) => ToolRegistry::default(),
        }
    }
}

#[allow(dead_code)]
pub async fn placeholder() -> HubResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// RAII guard that restores an env var when dropped. Avoids leaking
    /// state across tests if one panics.
    struct EnvVarGuard {
        key: &'static str,
        prior: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let prior = std::env::var_os(key);
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            Self { key, prior }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.prior.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Build a temp dir under a per-test prefix. Cleans itself up on drop is
    /// not implemented because the OS reclaims it at process exit; we just
    /// want a unique-enough path for the env-var trick to use.
    fn tempdir_via_env(prefix: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn default_entry_path_env_wins_over_bundled() {
        // HUB_LINE_MCP_PATH env var must take precedence over any bundled
        // lookup — this guards the v0.6.1 installer from being overridden
        // by an SDK pointing at a custom checkout (or vice versa).
        let _guard = EnvVarGuard::set("HUB_LINE_MCP_PATH", Some("/tmp/from-env.js"));
        let res = McpClient::default_entry_path().expect("env path");
        assert_eq!(res, std::path::PathBuf::from("/tmp/from-env.js"));
    }

    #[test]
    fn default_entry_path_env_dir_resolves_to_server_js() {
        // LINE_MODEL_HUB_BUNDLED points at the *directory*; the function
        // resolves it to <dir>/src/server.js so the installer can simply
        // ship a directory tree.
        let dir = tempdir_via_env("LINE_HUB_TEST_BUNDLE");
        let _env_guard = EnvVarGuard::set("LINE_MODEL_HUB_BUNDLED", Some(dir.to_str().unwrap()));
        let _path_guard = EnvVarGuard::set("HUB_LINE_MCP_PATH", None);
        let res = McpClient::default_entry_path().expect("bundled dir");
        assert_eq!(res, dir.join("src").join("server.js"));
    }

    #[test]
    fn default_entry_path_returns_err_when_nothing_set() {
        // With both env vars cleared and no bundled resources, we expect
        // LineMcpNotFound so spawn_mcp can surface the human hint.
        let _path_guard = EnvVarGuard::set("HUB_LINE_MCP_PATH", None);
        let _bundled_guard = EnvVarGuard::set("LINE_MODEL_HUB_BUNDLED", None);
        let res = McpClient::default_entry_path();
        assert!(
            matches!(res, Err(HubError::LineMcpNotFound)),
            "expected LineMcpNotFound, got {res:?}"
        );
    }

    #[test]
    fn decode_text_block() {
        let blocks = vec![json!({"type":"text","text":"hello"})];
        let out = McpClient::decode_blocks(blocks);
        match &out[0] {
            ToolResultBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn decode_image_block_round_trip() {
        let original = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let encoded = STANDARD.encode(&original);
        let blocks = vec![json!({
            "type": "image",
            "mimeType": "image/png",
            "data": encoded,
        })];
        let out = McpClient::decode_blocks(blocks);
        match &out[0] {
            ToolResultBlock::Image { mime_type, data } => {
                assert_eq!(mime_type, "image/png");
                assert_eq!(data, &original);
            }
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn decode_audio_block() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let wav = vec![0x52, 0x49, 0x46, 0x46, 0x00, 0x00];
        let encoded = STANDARD.encode(&wav);
        let blocks = vec![json!({"type":"audio","mimeType":"audio/wav","data":encoded})];
        let out = McpClient::decode_blocks(blocks);
        match &out[0] {
            ToolResultBlock::Audio { mime_type, data } => {
                assert_eq!(mime_type, "audio/wav");
                assert_eq!(data, &wav);
            }
            _ => panic!("expected audio"),
        }
    }

    #[test]
    fn decode_unsupported_block() {
        let blocks = vec![json!({"type":"resource","mimeType":"video/mp4"})];
        let out = McpClient::decode_blocks(blocks);
        match &out[0] {
            ToolResultBlock::Unsupported { mime_type, .. } => {
                assert_eq!(mime_type, "video/mp4");
            }
            _ => panic!("expected unsupported"),
        }
    }

    #[test]
    fn decode_oversize_image_returns_unsupported() {
        // 4 bytes that decode to > 8 MiB is impossible with 4 chars,
        // so we feed a syntactically valid blob that exceeds the cap.
        let big = "A".repeat(20 * 1024 * 1024); // ~15 MiB raw
        let blocks = vec![json!({"type":"image","mimeType":"image/png","data":big})];
        let out = McpClient::decode_blocks(blocks);
        // base64 of 15 MiB chars decodes to 11.25 MiB which exceeds 8 MiB
        assert!(matches!(&out[0], ToolResultBlock::Unsupported { .. }));
    }
}
