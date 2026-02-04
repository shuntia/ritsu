//! Database operations and schema management

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub struct Database {
    pub connection: Arc<Mutex<Connection>>,
}

impl Database {
    /// Create a new database connection and initialize schema
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create database directory")?;
        }

        let conn = Connection::open(path)
            .context("Failed to open database")?;
        
        let db = Self { connection: Arc::new(Mutex::new(conn)) };
        
        // Initialize schema synchronously in a blocking task
        let conn_clone = db.connection.clone();
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let conn = conn_clone.lock().await;
                Self::initialize_schema_sync(&conn)
            })
        })?;
        
        Ok(db)
    }

    /// Initialize or migrate database schema
    fn initialize_schema_sync(conn: &Connection) -> Result<()> {
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

        info!("Database schema initialized successfully");
        Ok(())
    }
}
