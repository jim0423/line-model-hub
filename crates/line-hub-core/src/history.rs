//! Local persistence of chat turns.
//!
//! Each session is a flat ordered list of `Turn` records keyed by
//! `(session_id, seq)`. We use SQLite because it ships with `rusqlite`'s
//! `bundled` feature — no system-level install of libsqlite3 needed.
//!
//! Schema is intentionally tiny; the UI builds the rendered chat on top
//! of this, so we don't need a fancy relational model yet.

use crate::error::{HubError, HubResult};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub session_id: String,
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub reasoning: Option<String>,
    /// JSON-encoded `Vec<ToolCallTrace>` for assistant turns.
    pub tool_trace: Option<String>,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone)]
pub struct HistoryStore {
    conn: std::sync::Arc<std::sync::Mutex<Connection>>,
}

impl HistoryStore {
    pub fn open() -> HubResult<Self> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(HubError::from)?;
        }
        let conn = Connection::open(&path).map_err(HubError::from)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
        })
    }

    /// Open a transient in-memory store. Used as a fallback when the on-disk
    /// store cannot be opened — losing persistence is preferable to a
    /// hard crash on startup.
    pub fn open_in_memory() -> HubResult<Self> {
        let conn = Connection::open_in_memory().map_err(HubError::from)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
        })
    }

    pub fn db_path() -> PathBuf {
        let mut p = dirs_home();
        p.push(".line-hub");
        p.push("history.sqlite3");
        p
    }

    fn migrate(conn: &Connection) -> HubResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS turns (
                session_id  TEXT NOT NULL,
                seq         INTEGER NOT NULL,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                reasoning   TEXT,
                tool_trace  TEXT,
                ts          INTEGER NOT NULL,
                PRIMARY KEY (session_id, seq),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_turns_session
                ON turns (session_id, seq);
            "#,
        )
        .map_err(HubError::from)?;
        Ok(())
    }

    /// Make sure a session row exists; create it if not.
    pub fn ensure_session(&self, session_id: &str) -> HubResult<()> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, title, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?3)",
            params![session_id, default_title(session_id), now],
        )
        .map_err(HubError::from)?;
        Ok(())
    }

    pub fn list_sessions(&self) -> HubResult<Vec<Session>> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT id, title, created_at, updated_at \
                 FROM sessions ORDER BY updated_at DESC",
            )
            .map_err(HubError::from)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Session {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    created_at: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            })
            .map_err(HubError::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(HubError::from)?);
        }
        Ok(out)
    }

    pub fn rename_session(&self, id: &str, title: &str) -> HubResult<()> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )
        .map_err(HubError::from)?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> HubResult<()> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        conn.execute("DELETE FROM turns WHERE session_id = ?1", params![id])
            .map_err(HubError::from)?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(HubError::from)?;
        Ok(())
    }

    pub fn append_turn(&self, turn: &Turn) -> HubResult<()> {
        self.ensure_session(&turn.session_id)?;
        let conn = self.conn.lock().expect("history mutex poisoned");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO turns \
             (session_id, seq, role, content, reasoning, tool_trace, ts) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                turn.session_id,
                turn.seq,
                turn.role,
                turn.content,
                turn.reasoning,
                turn.tool_trace,
                if turn.ts == 0 { now } else { turn.ts },
            ],
        )
        .map_err(HubError::from)?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, turn.session_id],
        )
        .map_err(HubError::from)?;
        Ok(())
    }

    pub fn next_seq(&self, session_id: &str) -> HubResult<i64> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let last: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM turns WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .map_err(HubError::from)?;
        Ok(last.unwrap_or(0) + 1)
    }

    pub fn load_turns(&self, session_id: &str) -> HubResult<Vec<Turn>> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT session_id, seq, role, content, reasoning, tool_trace, ts \
                 FROM turns WHERE session_id = ?1 ORDER BY seq ASC",
            )
            .map_err(HubError::from)?;
        let rows = stmt
            .query_map(params![session_id], |r| {
                Ok(Turn {
                    session_id: r.get(0)?,
                    seq: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    reasoning: r.get(4)?,
                    tool_trace: r.get(5)?,
                    ts: r.get(6)?,
                })
            })
            .map_err(HubError::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(HubError::from)?);
        }
        Ok(out)
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_title(session_id: &str) -> String {
    // Use the first 8 chars of the session id (or the whole thing if
    // shorter) — keeps the sidebar tidy without forcing a rename.
    let trimmed: String = session_id.chars().take(8).collect();
    if trimmed.is_empty() {
        "新對話".to_string()
    } else {
        format!("對話 {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Use a per-test tempfile so concurrent test runs do not stomp on the
    /// real ~/.line-hub/history.sqlite3.
    fn fresh_store() -> HistoryStore {
        let tmp = env::temp_dir().join(format!(
            "line-hub-history-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&tmp);
        // Point HOME at the temp dir; dirs_home() falls back to USERPROFILE
        // and finally "." so we set both for good measure.
        env::set_var("HOME", &tmp);
        env::set_var("USERPROFILE", &tmp);
        let store = HistoryStore::open().expect("open history store");
        store
    }

    #[test]
    fn append_and_load_round_trip() {
        let store = fresh_store();
        store.ensure_session("alpha").unwrap();
        store
            .append_turn(&Turn {
                session_id: "alpha".into(),
                seq: 1,
                role: "user".into(),
                content: "hi".into(),
                reasoning: None,
                tool_trace: None,
                ts: 0,
            })
            .unwrap();
        store
            .append_turn(&Turn {
                session_id: "alpha".into(),
                seq: 2,
                role: "assistant".into(),
                content: "hello back".into(),
                reasoning: Some("thinking".into()),
                tool_trace: None,
                ts: 0,
            })
            .unwrap();

        let turns = store.load_turns("alpha").unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "hi");
        assert_eq!(turns[1].content, "hello back");
        assert_eq!(turns[1].reasoning.as_deref(), Some("thinking"));
    }

    #[test]
    fn list_sessions_orders_by_updated_at_desc() {
        let store = fresh_store();
        store.ensure_session("old").unwrap();
        store.ensure_session("new").unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        // "new" was inserted last so its updated_at >= "old"'s.
        assert_eq!(sessions[0].id, "new");
        assert_eq!(sessions[1].id, "old");
    }

    #[test]
    fn delete_session_cascades_turns() {
        let store = fresh_store();
        store.ensure_session("ephemeral").unwrap();
        store
            .append_turn(&Turn {
                session_id: "ephemeral".into(),
                seq: 1,
                role: "user".into(),
                content: "x".into(),
                reasoning: None,
                tool_trace: None,
                ts: 0,
            })
            .unwrap();
        store.delete_session("ephemeral").unwrap();
        assert!(store.load_turns("ephemeral").unwrap().is_empty());
        assert!(store.list_sessions().unwrap().is_empty());
    }
}