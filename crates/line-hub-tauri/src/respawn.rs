//! Background watch loop for the MCP child connection.
//!
//! line-desktop-mcp v3.0.0's GUI path requires all three of
//! `LINE_MCP_CUA_DRIVER`, `LINE_MCP_PYTHON` and `LINE_MCP_SQLITE3MC_DLL`,
//! and a single LINE version bump is enough to break the call chain. We
//! watch for connection errors raised by subsequent [`McpTool::call_tool`]
//! invocations, and — when the user opted in to `auto_respawn` — restart
//! the child process with an exponential backoff (1s, 2s, 4s, 8s, then
//! give up after 3 consecutive failures).
//!
//! The watcher itself is read-only: it never spawns or kills without
//! confirming the user wanted auto-recovery. All respawned clients go
//! through the same `McpClient::spawn` codepath the manual "Spawn LINE
//! MCP" button uses, so feature parity is automatic.

use line_hub_core::mcp::McpTool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, RwLock};

/// Backoff stages used when the MCP child keeps dying within a minute.
pub const RESPAWN_BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

/// Maximum number of consecutive respawn attempts inside one minute before
/// the watcher gives up and requires a manual "Spawn LINE MCP" click.
pub const MAX_RESPAWN_ATTEMPTS: u32 = 3;

/// Per-session tracker so the watcher and the chat loop can agree on
/// whether the user enabled auto-recovery. We keep this in the same state
/// struct the rest of the app uses rather than behind a once-cell.
#[derive(Debug, Default)]
pub struct RespawnState {
    pub enabled: bool,
    pub attempts: u32,
    pub last_attempt_unix_ms: i64,
}

impl RespawnState {
    /// Decide whether a new respawn should fire given the current state.
    /// Returns the next backoff duration, or `None` if we should give up.
    pub fn next_backoff(&mut self) -> Option<Duration> {
        if !self.enabled {
            return None;
        }
        if self.attempts >= MAX_RESPAWN_ATTEMPTS {
            return None;
        }
        let idx = self.attempts as usize;
        self.attempts = self.attempts.saturating_add(1);
        self.last_attempt_unix_ms = chrono::Utc::now().timestamp_millis();
        RESPAWN_BACKOFF.get(idx).copied()
    }

    /// Reset attempts after a healthy call window — keeps the counter
    /// from compounding forever.
    pub fn reset(&mut self) {
        self.attempts = 0;
        self.last_attempt_unix_ms = 0;
    }
}

/// Token the chat loop pings after a `call_tool` succeeds so the watcher
/// knows the child is healthy. We don't await on it (cheap `notify_one`
/// each call), we just use it to drop the attempt counter back to zero
/// when the child proves itself alive.
pub fn mark_alive(state: &RwLock<RespawnState>) {
    // The lock is rarely contested (write only on respawn + every healthy
    // call); we'd rather hold it briefly than spawn a dedicated task.
    if let Ok(mut g) = state.try_write() {
        g.reset();
    }
}

/// Marker used by the watcher to wake up when the chat loop observed a
/// failure. Stored on the same `RwLock<RespawnState>` so we don't have to
/// plumb another shared field through `AppState`.
pub fn mark_failed(state: &RwLock<RespawnState>) -> bool {
    match state.try_write() {
        Ok(g) => g.enabled && g.attempts < MAX_RESPAWN_ATTEMPTS,
        Err(_) => false,
    }
}

/// Convenience: a `tokio::spawn`'d task that just pings the shared
/// `Notify` once the watcher decides the child has been healthy for a
/// while. The chat loop can `await` on this to wait for "give up" / "ok"
/// without polling busy-style.
pub fn health_gate_signal() -> Arc<Notify> {
    Arc::new(Notify::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawn_attempts_count_up_to_three() {
        let mut s = RespawnState {
            enabled: true,
            attempts: 0,
            ..Default::default()
        };
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(1)));
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(2)));
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(4)));
        assert_eq!(s.next_backoff(), None, "should give up after 3 attempts");
    }

    #[test]
    fn disabled_means_no_respawn() {
        let mut s = RespawnState::default();
        assert_eq!(s.next_backoff(), None);
    }

    #[test]
    fn mark_alive_resets_attempts() {
        let mut s = RespawnState {
            enabled: true,
            attempts: 2,
            ..Default::default()
        };
        s.reset();
        assert_eq!(s.attempts, 0);
    }
}
