//! User preferences extraction and management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub source: Option<String>,
}

pub struct PreferencesManager {
    db: Arc<tokio_rusqlite::Connection>,
}

impl PreferencesManager {
    pub const fn new(db: Arc<tokio_rusqlite::Connection>) -> Self {
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
        let category = category.to_string();
        let key = key.to_string();
        let value = value.to_string();
        let source = source.map(|s| s.to_string());
        
        self.db.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT OR REPLACE INTO preferences 
                 (category, key, value, confidence, source, updated_at) 
                 VALUES (?, ?, ?, ?, ?, datetime('now'))",
                rusqlite::params![&category, &key, &value, confidence, &source],
            )?;
            Ok(())
        }).await.map_err(Into::into)?;
        
        info!("Set preference: {} / {} = {}", category, key, value);
        Ok(())
    }

    /// Get a specific preference
    #[allow(dead_code)]
    pub async fn get_preference(&self, category: &str, key: &str) -> Result<Option<Preference>> {
        let category = category.to_string();
        let key = key.to_string();
        
        self.db.call(move |conn| -> rusqlite::Result<Option<Preference>> {
            let mut stmt = conn.prepare(
                "SELECT category, key, value, confidence, source 
                 FROM preferences 
                 WHERE category = ? AND key = ?"
            )?;
            
            let pref = stmt.query_row([&category, &key], |row| {
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
                Err(e) => Err(e),
            }
        }).await.map_err(Into::into)
    }

    /// Get all preferences in a category
    #[allow(dead_code)]
    pub async fn get_category(&self, category: &str) -> Result<Vec<Preference>> {
        let category = category.to_string();
        
        self.db.call(move |conn| -> rusqlite::Result<Vec<Preference>> {
            let mut stmt = conn.prepare(
                "SELECT category, key, value, confidence, source 
                 FROM preferences 
                 WHERE category = ? 
                 ORDER BY key"
            )?;
            
            let prefs = stmt.query_map([&category], |row| {
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
        }).await.map_err(Into::into)
    }

    /// Get all preferences as a map
    #[allow(dead_code)]
    pub async fn get_all(&self) -> Result<HashMap<String, HashMap<String, String>>> {
        self.db.call(|conn| -> rusqlite::Result<HashMap<String, HashMap<String, String>>> {
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
        }).await.map_err(Into::into)
    }

    /// Format preferences for system prompt
    #[allow(dead_code)]
    pub async fn format_for_prompt(&self) -> Result<String> {
        let prefs = self.get_all().await?;
        
        if prefs.is_empty() {
            return Ok(String::new());
        }
        
        let mut output = String::from("\n## User Preferences\n");
        
        for (category, items) in prefs {
            output = format!("{output}\n### {category}\n");
            for (key, value) in items {
                output = format!("{output}- {key}: {value}\n");
            }
        }
        
        Ok(output)
    }

    /// Remove a preference
    #[allow(dead_code)]
    pub async fn remove_preference(&self, category: &str, key: &str) -> Result<bool> {
        let category = category.to_string();
        let key = key.to_string();
        
        self.db.call(move |conn| -> rusqlite::Result<bool> {
            let rows = conn.execute(
                "DELETE FROM preferences WHERE category = ? AND key = ?",
                [&category, &key],
            )?;
            Ok(rows > 0)
        }).await.map_err(Into::into)
    }

    /// Clear all preferences in a category
    #[allow(dead_code)]
    pub async fn clear_category(&self, category: &str) -> Result<usize> {
        let category = category.to_string();
        
        let rows = self.db.call(move |conn| -> rusqlite::Result<usize> {
            let rows = conn.execute(
                "DELETE FROM preferences WHERE category = ?",
                [&category],
            )?;
            Ok(rows)
        }).await.map_err(Into::into)?;
        
        info!("Cleared {} preferences from category: {}", rows, category);
        Ok(rows)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn create_test_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        
        conn.execute(
            "CREATE TABLE preferences (
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
        ).unwrap();
        
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_and_get_preference() {
        let db = create_test_db();
        let manager = PreferencesManager::new(db);
        
        manager.set_preference("schedule", "wake_time", "7:00AM", 0.9, Some("user")).await.unwrap();
        
        let pref = manager.get_preference("schedule", "wake_time").await.unwrap();
        assert!(pref.is_some());
        let pref = pref.unwrap();
        assert_eq!(pref.value, "7:00AM");
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(pref.confidence, 0.9);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_category() {
        let db = create_test_db();
        let manager = PreferencesManager::new(db);
        
        manager.set_preference("schedule", "wake_time", "7:00AM", 1.0, None).await.unwrap();
        manager.set_preference("schedule", "work_start", "9:00AM", 1.0, None).await.unwrap();
        manager.set_preference("communication", "style", "formal", 1.0, None).await.unwrap();
        
        let schedule_prefs = manager.get_category("schedule").await.unwrap();
        assert_eq!(schedule_prefs.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_format_for_prompt() {
        let db = create_test_db();
        let manager = PreferencesManager::new(db);
        
        manager.set_preference("schedule", "wake_time", "7:00AM", 1.0, None).await.unwrap();
        
        let formatted = manager.format_for_prompt().await.unwrap();
        assert!(formatted.contains("## User Preferences"));
        assert!(formatted.contains("schedule"));
        assert!(formatted.contains("wake_time"));
        assert!(formatted.contains("7:00AM"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_remove_preference() {
        let db = create_test_db();
        let manager = PreferencesManager::new(db);
        
        manager.set_preference("test", "key", "value", 1.0, None).await.unwrap();
        let removed = manager.remove_preference("test", "key").await.unwrap();
        assert!(removed);
        
        let pref = manager.get_preference("test", "key").await.unwrap();
        assert!(pref.is_none());
    }
}
