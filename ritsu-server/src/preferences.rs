//! User preferences extraction and management

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub source: Option<String>,
}

pub struct PreferencesManager {
    db: Arc<Mutex<Connection>>,
}

impl PreferencesManager {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Set or update a preference
    pub async fn set_preference(
        &self,
        category: &str,
        key: &str,
        value: &str,
        confidence: f64,
        source: Option<&str>,
    ) -> Result<()> {
        let conn = self.db.lock().await;
        
        conn.execute(
            "INSERT OR REPLACE INTO preferences 
             (category, key, value, confidence, source, updated_at) 
             VALUES (?, ?, ?, ?, ?, datetime('now'))",
            rusqlite::params![category, key, value, confidence, source],
        )?;
        
        info!("Set preference: {} / {} = {}", category, key, value);
        Ok(())
    }

    /// Get a specific preference
    pub async fn get_preference(&self, category: &str, key: &str) -> Result<Option<Preference>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT category, key, value, confidence, source 
             FROM preferences 
             WHERE category = ? AND key = ?"
        )?;
        
        let pref = stmt.query_row([category, key], |row| {
            Ok(Preference {
                category: row.get(0)?,
                key: row.get(1)?,
                value: row.get(2)?,
                confidence: row.get(3)?,
                source: row.get(4)?,
            })
        });
        
        match pref {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get all preferences in a category
    pub async fn get_category(&self, category: &str) -> Result<Vec<Preference>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT category, key, value, confidence, source 
             FROM preferences 
             WHERE category = ? 
             ORDER BY key"
        )?;
        
        let prefs = stmt.query_map([category], |row| {
            Ok(Preference {
                category: row.get(0)?,
                key: row.get(1)?,
                value: row.get(2)?,
                confidence: row.get(3)?,
                source: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        
        Ok(prefs)
    }

    /// Get all preferences as a map
    pub async fn get_all(&self) -> Result<HashMap<String, HashMap<String, String>>> {
        let conn = self.db.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT category, key, value FROM preferences ORDER BY category, key"
        )?;
        
        let mut prefs: HashMap<String, HashMap<String, String>> = HashMap::new();
        
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, // category
                row.get::<_, String>(1)?, // key
                row.get::<_, String>(2)?, // value
            ))
        })?;
        
        for row in rows {
            let (category, key, value) = row?;
            prefs.entry(category).or_default().insert(key, value);
        }
        
        Ok(prefs)
    }

    /// Format preferences for system prompt
    pub async fn format_for_prompt(&self) -> Result<String> {
        let prefs = self.get_all().await?;
        
        if prefs.is_empty() {
            return Ok(String::new());
        }
        
        let mut output = String::from("\n## User Preferences\n");
        
        for (category, items) in prefs {
            output.push_str(&format!("\n### {}\n", category));
            for (key, value) in items {
                output.push_str(&format!("- {}: {}\n", key, value));
            }
        }
        
        Ok(output)
    }

    /// Remove a preference
    pub async fn remove_preference(&self, category: &str, key: &str) -> Result<bool> {
        let conn = self.db.lock().await;
        
        let rows = conn.execute(
            "DELETE FROM preferences WHERE category = ? AND key = ?",
            [category, key],
        )?;
        
        Ok(rows > 0)
    }

    /// Clear all preferences in a category
    pub async fn clear_category(&self, category: &str) -> Result<usize> {
        let conn = self.db.lock().await;
        
        let rows = conn.execute(
            "DELETE FROM preferences WHERE category = ?",
            [category],
        )?;
        
        info!("Cleared {} preferences from category: {}", rows, category);
        Ok(rows)
    }
}
