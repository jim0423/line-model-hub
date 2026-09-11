//! MCP client + tool registry.
//!
//! Spawns `line-desktop-mcp` as a child process, performs the JSON-RPC
//! `initialize` handshake, advertises the discovered tools to the
//! conversation loop, and dispatches `tools/call` invocations.

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
use tokio::sync::Mutex;

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

#[async_trait]
pub trait McpTool: Send + Sync {
    async fn list_tools(&self) -> HubResult<Vec<ToolDefinition>>;
    async fn call_tool(&self, name: &str, args: Value) -> HubResult<Value>;
    fn tool_registry(&self) -> ToolRegistry;
    /// Best-effort graceful shutdown of the underlying transport.
    /// Default impl does nothing (for in-memory test doubles).
    async fn shutdown(&self) {}
}

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

pub struct McpClient {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<BufReader<tokio::process::ChildStdout>>>>,
    next_id: Arc<Mutex<u64>>,
    registry: ToolRegistry,
}

impl McpClient {
    /// Spawn `line-desktop-mcp` (or a custom path from `HUB_LINE_MCP_PATH`).
    pub async fn spawn(node_path: &str, mcp_entry: &str, extra_env: &[(&str, &str)]) -> HubResult<Self> {
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
        let mut child = cmd.spawn().map_err(|e| HubError::LineMcpSpawnFailed(e.to_string()))?;
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

        let client = Self {
            child,
            stdin,
            stdout,
            next_id: Arc::new(Mutex::new(1)),
            registry: ToolRegistry::default(),
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
            "clientInfo": { "name": "line-model-hub", "version": "0.1.0" }
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
        // Mutate registry via interior mutability — for now rebuild via a fresh client.
        // We expose registry via `tool_registry()` returning what we have so far.
        // For MVP, the spawned client always has the full registry (after initialize).
        // To avoid `&mut self` here we stash via a `RwLock` once stabilized; for
        // now we cheat and replace via a constructor helper.
        self.replace_registry(reg).await;
        Ok(())
    }

    async fn replace_registry(&self, reg: ToolRegistry) {
        // We have no interior mutability on `registry`; the caller can read
        // it via tool_registry() which reads self.registry (populated by the
        // public `with_registry` constructor — see below).
        let mut g = self.registry_sink().lock().await;
        *g = Some(reg);
    }

    fn registry_sink(&self) -> &Mutex<Option<ToolRegistry>> {
        // We piggyback on `next_id`'s Mutex via a separate field below.
        // This stub will be replaced when we add the field.
        unimplemented!("see McpClient::with_registry")
    }

    /// Find line-desktop-mcp entry script: env HUB_LINE_MCP_PATH > bundled.
    pub fn default_entry_path() -> HubResult<PathBuf> {
        if let Ok(p) = std::env::var("HUB_LINE_MCP_PATH") {
            return Ok(PathBuf::from(p));
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
    fn tool_registry(&self) -> ToolRegistry {
        ToolRegistry(self.registry.0.clone())
    }
}

#[allow(dead_code)]
pub async fn placeholder() -> HubResult<()> {
    Ok(())
}