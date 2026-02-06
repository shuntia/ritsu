//! Conversation session management

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub turn_number: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_results: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationSession {
    pub session_id: String,
    pub turn_count: i64,
    pub started_at: String,
    pub last_activity: String,
    pub title: Option<String>,
}

pub struct ConversationManager {
    db: Arc<Mutex<Connection>>,
}

impl ConversationManager {
    pub const fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Create or resume a conversation session
    #[allow(clippy::significant_drop_tightening)]
    pub async fn get_or_create_session(&self, session_id: &str) -> Result<ConversationSession> {
        let conn = self.db.lock().await;
        
        // Try to get existing session
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_count, started_at, last_activity, title 
             FROM conversations WHERE session_id = ?"
        )?;
        
        let session = stmt.query_row([session_id], |row| {
            Ok(ConversationSession {
                session_id: row.get(0)?,
                turn_count: row.get(1)?,
                started_at: row.get(2)?,
                last_activity: row.get(3)?,
                title: row.get(4)?,
            })
        });
        
        if let Ok(s) = session {
            // Update last_activity
            conn.execute(
                "UPDATE conversations SET last_activity = datetime('now') WHERE session_id = ?",
                [session_id],
            )?;
            Ok(s)
        } else {
            // Create new session
            conn.execute(
                "INSERT INTO conversations (session_id, metadata) VALUES (?, '{}')",
                [session_id],
            )?;
            
            Ok(ConversationSession {
                session_id: session_id.to_string(),
                turn_count: 0,
                started_at: chrono::Utc::now().to_rfc3339(),
                last_activity: chrono::Utc::now().to_rfc3339(),
                title: None,
            })
        }
    }

    /// Add a turn to the conversation
    #[allow(clippy::significant_drop_tightening)]
    pub async fn add_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<&str>,
        tool_results: Option<&str>,
    ) -> Result<i64> {
        let conn = self.db.lock().await;
        
        // Get current turn count
        let turn_count: i64 = conn.query_row(
            "SELECT turn_count FROM conversations WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )?;
        
        let turn_number = turn_count + 1;
        
        // Insert turn
        conn.execute(
            "INSERT INTO conversation_turns 
             (session_id, turn_number, role, content, tool_calls, tool_results) 
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![session_id, turn_number, role, content, tool_calls, tool_results],
        )?;
        
        // Update conversation turn count and last_activity
        conn.execute(
            "UPDATE conversations 
             SET turn_count = ?, last_activity = datetime('now') 
             WHERE session_id = ?",
            rusqlite::params![turn_number, session_id],
        )?;
        
        info!("Added turn {} to session {}", turn_number, session_id);
        Ok(turn_number)
    }

    /// Get conversation history (last N turns)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn get_history(&self, session_id: &str, limit: i64) -> Result<Vec<ConversationTurn>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT turn_number, role, content, tool_calls, tool_results 
             FROM conversation_turns 
             WHERE session_id = ? 
             ORDER BY turn_number DESC 
             LIMIT ?"
        )?;
        
        let turns = stmt.query_map([session_id, &limit.to_string()], |row| {
            Ok(ConversationTurn {
                turn_number: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tool_calls: row.get(3)?,
                tool_results: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        
        // Reverse to get chronological order
        Ok(turns.into_iter().rev().collect())
    }

    /// Get all active sessions (active in last 24 hours)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn get_active_sessions(&self) -> Result<Vec<ConversationSession>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_count, started_at, last_activity, title 
             FROM conversations 
             WHERE last_activity > datetime('now', '-1 day')
             ORDER BY last_activity DESC"
        )?;
        
        let sessions = stmt.query_map([], |row| {
            Ok(ConversationSession {
                session_id: row.get(0)?,
                turn_count: row.get(1)?,
                started_at: row.get(2)?,
                last_activity: row.get(3)?,
                title: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        
        Ok(sessions)
    }

    /// Clean up old conversations (older than 30 days)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn cleanup_old_sessions(&self, days: i64) -> Result<usize> {
        let conn = self.db.lock().await;
        
        let deleted = conn.execute(
            "DELETE FROM conversations 
             WHERE last_activity < datetime('now', ? || ' days')",
            [format!("-{days}")],
        )?;
        
        if deleted > 0 {
            info!("Cleaned up {} old conversation sessions", deleted);
        }
        
        Ok(deleted)
    }

    /// Generate a title for a session based on first few turns
    #[allow(clippy::significant_drop_tightening)]
    pub async fn generate_title(&self, session_id: &str, llm_client: &crate::llm::LlmClient) -> Result<String> {
        // Build context from first few turns
        let context = {
            let conn = self.db.lock().await;
            
            // Get first 2-3 turns to generate title from
            let mut stmt = conn.prepare(
                "SELECT role, content FROM conversation_turns 
                 WHERE session_id = ? 
                 ORDER BY turn_number 
                 LIMIT 4"
            )?;
            
            let turns: Vec<(String, String)> = stmt.query_map([session_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
            
            if turns.is_empty() {
                return Ok("New Conversation".to_string());
            }
            
            // Build context from turns
            let mut context = String::new();
            for (role, content) in &turns {
                context.push_str(&format!("{}: {}\n", role, content));
            }
            
            context
        }; // conn and stmt dropped here
        
        // Ask LLM to generate a short title
        let messages = vec![
            crate::llm::Message {
                role: "user".to_string(),
                content: format!(
                    "Based on this conversation start, generate a very short title (2-5 words max):\n\n{}\n\nTitle:",
                    context
                ),
            }
        ];
        
        let response = llm_client.generate(&messages, None).await?;
        
        let title = response.content
            .trim()
            .trim_matches('"')
            .to_string();
        
        // Store the title
        let conn = self.db.lock().await;
        conn.execute(
            "UPDATE conversations SET title = ? WHERE session_id = ?",
            [&title, session_id],
        )?;
        
        Ok(title)
    }

    /// Check if session needs title generation (has turns but no title)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn needs_title_generation(&self, session_id: &str) -> Result<bool> {
        let conn = self.db.lock().await;
        
        let result: Option<(i64, Option<String>)> = conn.query_row(
            "SELECT turn_count, title FROM conversations WHERE session_id = ?",
            [session_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        ).optional()?;
        
        if let Some((turn_count, title)) = result {
            Ok(turn_count >= 2 && title.is_none())
        } else {
            Ok(false)
        }
    }

    /// Clear all conversations and turns (for testing)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn clear_all(&self) -> Result<()> {
        let conn = self.db.lock().await;
        
        conn.execute("DELETE FROM conversation_turns", [])?;
        conn.execute("DELETE FROM conversations", [])?;
        
        info!("Cleared all conversations and turns");
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn create_test_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        
        // Create minimal schema for testing
        conn.execute(
            "CREATE TABLE conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT UNIQUE NOT NULL,
                started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                turn_count INTEGER DEFAULT 0,
                metadata TEXT
            )",
            [],
        ).unwrap();
        
        conn.execute(
            "CREATE TABLE conversation_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_number INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls TEXT,
                tool_results TEXT,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (session_id) REFERENCES conversations(session_id) ON DELETE CASCADE
            )",
            [],
        ).unwrap();
        
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_session() {
        let db = create_test_db();
        let manager = ConversationManager::new(db);
        
        let session = manager.get_or_create_session("test_session").await.unwrap();
        assert_eq!(session.session_id, "test_session");
        assert_eq!(session.turn_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_add_turn() {
        let db = create_test_db();
        let manager = ConversationManager::new(db);
        
        let _session = manager.get_or_create_session("test_session").await.unwrap();
        
        let turn_number = manager.add_turn(
            "test_session",
            "user",
            "Hello",
            None,
            None,
        ).await.unwrap();
        
        assert_eq!(turn_number, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_history() {
        let db = create_test_db();
        let manager = ConversationManager::new(db);
        
        let _session = manager.get_or_create_session("test_session").await.unwrap();
        
        manager.add_turn("test_session", "user", "Hello", None, None).await.unwrap();
        manager.add_turn("test_session", "assistant", "Hi there!", None, None).await.unwrap();
        
        let history = manager.get_history("test_session", 10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "Hi there!");
    }
}
