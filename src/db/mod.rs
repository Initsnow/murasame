use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub character_id: String,
    pub title: String,
    pub protocol: String,
    pub last_response_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub token_count: i64,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;

        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                character_id TEXT NOT NULL,
                title TEXT NOT NULL,
                protocol TEXT NOT NULL,
                last_response_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_character ON sessions(character_id);
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id ASC);
            "#,
        )?;

        // Non-destructive schema migration for existing databases
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN character_id TEXT NOT NULL DEFAULT 'murasame'",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_character ON sessions(character_id)",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE messages ADD COLUMN token_count INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id ASC)",
            [],
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn list_sessions(&self, character_id: &str) -> Result<Vec<ChatSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, character_id, title, protocol, last_response_id, created_at, updated_at FROM sessions WHERE character_id = ?1 ORDER BY updated_at DESC"
        )?;

        let rows = stmt.query_map(params![character_id], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                character_id: row.get(1)?,
                title: row.get(2)?,
                protocol: row.get(3)?,
                last_response_id: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<ChatSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, character_id, title, protocol, last_response_id, created_at, updated_at FROM sessions WHERE id = ?1"
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                character_id: row.get(1)?,
                title: row.get(2)?,
                protocol: row.get(3)?,
                last_response_id: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;

        if let Some(r) = rows.next() {
            Ok(Some(r?))
        } else {
            Ok(None)
        }
    }

    pub fn create_session(
        &self,
        character_id: &str,
        title: &str,
        protocol: &str,
    ) -> Result<ChatSession> {
        let id = format!("{:x}", rand::random::<u128>());
        let now = now_ts();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, character_id, title, protocol, last_response_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)",
            params![&id, character_id, title, protocol, now, now],
        )?;

        Ok(ChatSession {
            id,
            character_id: character_id.to_string(),
            title: title.to_string(),
            protocol: protocol.to_string(),
            last_response_id: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get_or_create_default_session(
        &self,
        character_id: &str,
        default_protocol: &str,
    ) -> Result<ChatSession> {
        let list = self.list_sessions(character_id)?;
        if let Some(first) = list.into_iter().next() {
            return Ok(first);
        }
        self.create_session(character_id, "新会话", default_protocol)
    }

    pub fn update_session_title(&self, id: &str, title: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = now_ts();
        conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )?;
        Ok(())
    }

    pub fn update_session_response_id(
        &self,
        id: &str,
        last_response_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = now_ts();
        conn.execute(
            "UPDATE sessions SET last_response_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![last_response_id, now, id],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, token_count, created_at FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                token_count: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn add_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        token_count: i64,
    ) -> Result<ChatMessage> {
        let conn = self.conn.lock().unwrap();
        let now = now_ts();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, token_count, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, role, content, token_count, now],
        )?;
        let id = conn.last_insert_rowid();

        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(ChatMessage {
            id,
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            token_count,
            created_at: now,
        })
    }

    pub fn clear_messages(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = now_ts();
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session_id],
        )?;
        conn.execute(
            "UPDATE sessions SET last_response_id = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_db_lifecycle() -> Result<()> {
        let db = Database::open(":memory:")?;
        let s = db.get_or_create_default_session("murasame", "hermes")?;
        assert_eq!(s.character_id, "murasame");
        assert_eq!(s.title, "新会话");
        assert_eq!(s.protocol, "hermes");

        // Another character
        let s2 = db.get_or_create_default_session("eris", "openai")?;
        assert_eq!(s2.character_id, "eris");

        let murasame_sessions = db.list_sessions("murasame")?;
        assert_eq!(murasame_sessions.len(), 1);
        assert_eq!(murasame_sessions[0].id, s.id);

        let eris_sessions = db.list_sessions("eris")?;
        assert_eq!(eris_sessions.len(), 1);
        assert_eq!(eris_sessions[0].id, s2.id);

        // Add messages
        let m1 = db.add_message(&s.id, "user", "你好，丛雨！", 12)?;
        assert_eq!(m1.role, "user");
        assert_eq!(m1.content, "你好，丛雨！");
        assert_eq!(m1.token_count, 12);

        let m2 = db.add_message(&s.id, "assistant", "你好！有什么我可以帮你的？", 20)?;
        assert_eq!(m2.role, "assistant");
        assert_eq!(m2.token_count, 20);

        let msgs = db.get_messages(&s.id)?;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].token_count, 12);
        assert_eq!(msgs[1].token_count, 20);

        // Update response id
        db.update_session_response_id(&s.id, Some("resp_12345"))?;
        let updated_s = db.get_session(&s.id)?.unwrap();
        assert_eq!(updated_s.last_response_id.as_deref(), Some("resp_12345"));

        // Rename session
        db.update_session_title(&s.id, "测试会话")?;
        let renamed_s = db.get_session(&s.id)?.unwrap();
        assert_eq!(renamed_s.title, "测试会话");

        // Clear messages
        db.clear_messages(&s.id)?;
        let cleared_msgs = db.get_messages(&s.id)?;
        assert_eq!(cleared_msgs.len(), 0);

        // Delete session
        db.delete_session(&s.id)?;
        let deleted = db.get_session(&s.id)?;
        assert!(deleted.is_none());

        Ok(())
    }
}
