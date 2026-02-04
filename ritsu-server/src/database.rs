//! Database operations and schema management

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::info;

pub struct Database {
    conn: Connection,
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
        
        let db = Self { conn };
        db.initialize_schema()?;
        
        Ok(db)
    }

    /// Initialize or migrate database schema
    fn initialize_schema(&self) -> Result<()> {
        info!("Initializing database schema");

        // Enable foreign keys
        self.conn.execute("PRAGMA foreign_keys = ON", [])?;

        // Notes table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                content TEXT NOT NULL,
                tags TEXT
            )",
            [],
        )?;

        // Daily summaries table
        self.conn.execute(
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
        self.conn.execute(
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
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date DATE NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                role TEXT NOT NULL,
                content TEXT NOT NULL
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_conversations_date 
             ON daily_conversations(date)",
            [],
        )?;

        // Triggers table
        self.conn.execute(
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
        self.conn.execute(
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
        self.conn.execute(
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
        self.conn.execute(
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

        info!("Database schema initialized successfully");
        Ok(())
    }
}
