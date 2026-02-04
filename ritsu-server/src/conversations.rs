//! Conversation session management

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

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
}

pub struct ConversationManager {
    db: Arc<Mutex<Connection>>,
}

impl ConversationManager {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Create or resume a conversation session
    pub async fn get_or_create_session(&self, session_id: &str) -> Result<ConversationSession> {
        let conn = self.db.lock().await;
        
        // Try to get existing session
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_count, started_at, last_activity 
             FROM conversations WHERE session_id = ?"
        )?;
        
        let session = stmt.query_row([session_id], |row| {
            Ok(ConversationSession {
                session_id: row.get(0)?,
                turn_count: row.get(1)?,
                started_at: row.get(2)?,
                last_activity: row.get(3)?,
            })
        });
        
        match session {
            Ok(s) => {
                // Update last_activity
                conn.execute(
                    "UPDATE conversations SET last_activity = datetime('now') WHERE session_id = ?",
                    [session_id],
                )?;
                Ok(s)
            }
            Err(_) => {
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
                })
            }
        }
    }

    /// Add a turn to the conversation
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
    pub async fn get_active_sessions(&self) -> Result<Vec<ConversationSession>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_count, started_at, last_activity 
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        
        Ok(sessions)
    }

    /// Clean up old conversations (older than 30 days)
    pub async fn cleanup_old_sessions(&self, days: i64) -> Result<usize> {
        let conn = self.db.lock().await;
        
        let deleted = conn.execute(
            "DELETE FROM conversations 
             WHERE last_activity < datetime('now', ? || ' days')",
            [format!("-{}", days)],
        )?;
        
        if deleted > 0 {
            info!("Cleaned up {} old conversation sessions", deleted);
        }
        
        Ok(deleted)
    }
}
