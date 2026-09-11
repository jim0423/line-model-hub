use thiserror::Error;

pub type HubResult<T> = Result<T, HubError>;

#[derive(Debug, Error)]
pub enum HubError {
    #[error("provider error in {provider}: {message}")]
    Provider { provider: String, message: String },

    #[error("invalid arguments for {tool}: {message}")]
    InvalidArguments { tool: String, message: String },

    #[error("send refused by user (chat={chat})")]
    SendRefused { chat: String },

    #[error("MCP transport error: {0}")]
    McpTransport(String),

    #[error("MCP protocol error: code={code} message={message}")]
    McpProtocol { code: i32, message: String },

    #[error("config error: {0}")]
    Config(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("line-desktop-mcp not found. Set HUB_LINE_MCP_PATH or install via bundled resource")]
    LineMcpNotFound,

    #[error("line-desktop-mcp spawn failed: {0}")]
    LineMcpSpawnFailed(String),
}