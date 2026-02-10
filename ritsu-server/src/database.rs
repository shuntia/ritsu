//! Database operations and schema management

use anyhow::{Context, Result};
use tokio_rusqlite::rusqlite;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

/// Helper to convert `tokio_rusqlite` errors to `anyhow`
fn convert_db_result<T>(res: std::result::Result<T, tokio_rusqlite::Error<rusqlite::Error>>) -> Result<T> {
    res.map_err(|e| anyhow::anyhow!("{e:?}"))
}

pub struct Database {
    pub connection: Arc<tokio_rusqlite::Connection>,
}

impl Database {
    /// Create a new database connection and initialize schema
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await
                .context("Failed to create database directory")?;
        }

        // Open connection using tokio_rusqlite
        let conn = tokio_rusqlite::Connection::open(&path).await
            .context("Failed to open database")?;
        
        // Initialize schema
        convert_db_result(conn.call(|conn| {
            Self::initialize_schema_sync(conn)
        }).await)?;
        
        let db = Self { connection: Arc::new(conn) };
        Ok(db)
    }

    /// Initialize or migrate database schema
    #[allow(clippy::too_many_lines)]
    fn initialize_schema_sync(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        info!("Initializing database schema");

        // Enable foreign keys
        conn.execute("PRAGMA foreign_keys = ON", [])?;

        // Notes table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                content TEXT NOT NULL,
                tags TEXT
            )",
            [],
        )?;

        // Daily summaries table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_summaries (
                date DATE PRIMARY KEY,
                summary TEXT NOT NULL,
                tags TEXT,
                conversation_count INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Monthly summaries table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS monthly_summaries (
                year_month TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                tags TEXT,
                days_included INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Daily conversations table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date DATE NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                role TEXT NOT NULL,
                content TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_conversations_date 
             ON daily_conversations(date)",
            [],
        )?;

        // Triggers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS triggers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                trigger_type TEXT NOT NULL,
                schedule TEXT NOT NULL,
                enabled BOOLEAN DEFAULT TRUE,
                created_by TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                metadata TEXT
            )",
            [],
        )?;

        // System prompts table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS system_prompts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                prompt_type TEXT NOT NULL,
                content TEXT NOT NULL,
                version INTEGER DEFAULT 1,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                active BOOLEAN DEFAULT TRUE
            )",
            [],
        )?;

        // Idle analyses table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS idle_analyses (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                analysis_type TEXT NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                findings TEXT NOT NULL,
                prompted_changes TEXT
            )",
            [],
        )?;

        // Tasks table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                tags TEXT,
                due_date TIMESTAMP,
                created_by TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                completed_at TIMESTAMP
            )",
            [],
        )?;

        // Tool usage tracking table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tool_name TEXT NOT NULL,
                arguments TEXT NOT NULL,
                success BOOLEAN NOT NULL,
                result TEXT NOT NULL,
                execution_time_ms INTEGER,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                triggered_by TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_usage_timestamp 
             ON tool_usage(timestamp)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_usage_tool_name 
             ON tool_usage(tool_name)",
            [],
        )?;

        // Cron exceptions table: store cancelled specific occurrences for cron triggers
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cron_exceptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                trigger_name TEXT NOT NULL,
                occurrence TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_cron_exceptions_trigger_occurrence 
             ON cron_exceptions(trigger_name, occurrence)",
            [],
        )?;

        // Conversation sessions table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT UNIQUE NOT NULL,
                started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                turn_count INTEGER DEFAULT 0,
                title TEXT,
                metadata TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_conversations_session_id 
             ON conversations(session_id)",
            [],
        )?;

        // Conversation turns table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversation_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_number INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls TEXT,
                tool_results TEXT,
                thinking TEXT,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (session_id) REFERENCES conversations(session_id) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_conversation_turns_session 
             ON conversation_turns(session_id, turn_number)",
            [],
        )?;

        // Migration: Add title column to existing conversations table
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(conversations)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        
        if !columns.contains(&"title".to_string()) {
            conn.execute("ALTER TABLE conversations ADD COLUMN title TEXT", [])?;
        }

        // Migration: Add thinking column to existing conversation_turns table
        let turn_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(conversation_turns)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        
        if !turn_columns.contains(&"thinking".to_string()) {
            conn.execute("ALTER TABLE conversation_turns ADD COLUMN thinking TEXT", [])?;
        }

        // User preferences table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS preferences (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL DEFAULT 1.0,
                source TEXT,
                extracted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(category, key)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_preferences_category 
             ON preferences(category)",
            [],
        )?;

        info!("Database schema initialized successfully");
        Ok(())
    }
    
    /// Execute a database operation in a blocking context
    /// Use this helper for all DB operations from async contexts
    pub async fn execute_blocking<F, T>(db_path: String, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_path)?;
            f(&conn)
        })
        .await
        .context("Task panicked")?
    }
}

/// Helper function for filesystem operations
pub async fn read_file_async(path: impl AsRef<Path> + Send + 'static) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read file: {}", path.as_ref().display()))
    })
    .await
    .context("Task panicked")?
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_database_creation() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("ritsu_test_{}.db", std::process::id()));
        
        let result = Database::new(&db_path).await;
        assert!(result.is_ok());
        
        // Cleanup
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_database_schema_tables() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("ritsu_test_schema_{}.db", std::process::id()));
        
        let db = Database::new(&db_path).await.unwrap();
        
        // Check if key tables exist
        let tables = vec![
            "notes", "conversations", "daily_summaries", "monthly_summaries",
            "triggers", "tasks", "tool_usage", "idle_analyses",
            "conversations", "conversation_turns", "preferences"
        ];
        
        for table in tables {
            let result = db.connection.call(move |conn| -> rusqlite::Result<i64> {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
                    [table],
                    |row| row.get(0),
                )
            }).await;
            assert!(result.unwrap() > 0, "Table '{table}' should exist");
        }
        
        // Cleanup
        let _ = std::fs::remove_file(&db_path);
    }
}
