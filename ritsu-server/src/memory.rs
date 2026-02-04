//! Memory management and compaction
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::doc_markdown)]

use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::llm::LlmClient;

pub struct MemoryManager {
    conn: Arc<Mutex<Connection>>,
}

#[allow(dead_code)]
impl MemoryManager {
    #[must_use]
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Store a conversation message
    pub async fn store_conversation(&self, role: &str, content: &str) -> Result<()> {
        let date = chrono::Utc::now().date_naive();
        
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO daily_conversations (date, role, content) VALUES (?1, ?2, ?3)",
            (date.to_string(), role, content),
        )?;
        
        Ok(())
    }

    /// Create a note
    pub async fn create_note(&self, content: &str, tags: &[String]) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO notes (content, tags) VALUES (?1, ?2)",
            (content, &tags_json),
        )?;
        
        Ok(conn.last_insert_rowid())
    }

    /// Compact daily conversations into a daily summary
    pub async fn compact_daily(&self, date: &NaiveDate, llm_client: &LlmClient) -> Result<()> {
        info!("Compacting conversations for {date}");

        // Get all conversations for the date (collect first, then release lock)
        let conversations = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare(
                "SELECT role, content FROM daily_conversations 
                 WHERE date = ?1 ORDER BY timestamp"
            )?;
            
            let rows = stmt
                .query_map([date.to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            result
        }; // Lock released here

        if conversations.is_empty() {
            info!("No conversations to compact for {date}");
            return Ok(());
        }

        // Get previous day's summary for context
        let previous_summary = if let Some(previous_day) = date.pred_opt() {
            let conn = self.conn.lock().await;
            conn.query_row(
                "SELECT summary FROM daily_summaries WHERE date = ?1",
                [previous_day.to_string()],
                |row| row.get::<_, String>(0),
            ).ok()
        } else {
            None
        };

        // Get system prompt
        let system_prompt = self.build_effective_prompt().await?;

        // Generate summary with LLM (no lock held)
        let conversation_text = conversations.iter()
            .map(|(role, content)| format!("{role}: {content}"))
            .collect::<Vec<_>>()
            .join("\n");
        
        let mut prompt = format!("Summarize the following conversation into key points:\n\n{conversation_text}");
        
        if let Some(prev) = previous_summary {
            prompt = format!("Previous day context: {prev}\n\n{prompt}");
        }
        
        let messages = vec![
            crate::llm::Message {
                role: "system".to_string(),
                content: system_prompt,
            },
            crate::llm::Message {
                role: "user".to_string(),
                content: prompt,
            }
        ];
        
        let response = llm_client.generate(&messages, None).await
            .ok()
            .unwrap_or_else(|| crate::llm::LlmResponse {
                content: format!("Conversation summary for {date} ({} messages)", conversations.len()),
                tool_calls: Vec::new(),
            });
        
        let summary = response.content;
        let tags = serde_json::to_string(&Vec::<String>::new())?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let count = conversations.len() as i32;

        // Insert daily summary (acquire lock again)
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO daily_summaries (date, summary, tags, conversation_count) 
             VALUES (?1, ?2, ?3, ?4)",
            (date.to_string(), summary, tags, count),
        )?;

        info!("Compacted {count} conversations into daily summary for {date}");
        Ok(())
    }

    /// Compact daily summaries into a monthly summary
    pub async fn compact_monthly(&self, year_month: &str, llm_client: &LlmClient) -> Result<()> {
        info!("Compacting daily summaries for {year_month}");

        // Get all daily summaries for the month (collect first, then release lock)
        let summaries = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare(
                "SELECT date, summary FROM daily_summaries 
                 WHERE date LIKE ?1 ORDER BY date"
            )?;
            
            let rows = stmt
                .query_map([format!("{year_month}%")], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            result
        }; // Lock released here

        if summaries.is_empty() {
            info!("No daily summaries to compact for {year_month}");
            return Ok(());
        }

        // Get system prompt
        let system_prompt = self.build_effective_prompt().await?;

        // Get previous month's summary for continuity
        let prev_month_summary = {
            let conn = self.conn.lock().await;
            conn.query_row(
                "SELECT summary FROM monthly_summaries 
                 WHERE year_month < ?1 ORDER BY year_month DESC LIMIT 1",
                [year_month],
                |row| row.get::<_, String>(0),
            ).ok()
        };

        // Generate monthly summary with LLM (no lock held)
        let summaries_text = summaries.iter()
            .map(|(date, summary)| format!("{date}: {summary}"))
            .collect::<Vec<String>>()
            .join("\n\n");
        
        let mut prompt = format!("Create a comprehensive monthly summary from these daily summaries:\n\n{summaries_text}\n\nIdentify key themes, patterns, and progress.");
        
        if let Some(prev) = prev_month_summary {
            prompt = format!("Previous month: {prev}\n\n{prompt}");
        }
        
        let messages = vec![
            crate::llm::Message {
                role: "system".to_string(),
                content: system_prompt,
            },
            crate::llm::Message {
                role: "user".to_string(),
                content: prompt,
            }
        ];
        
        let response = llm_client.generate(&messages, None).await
            .ok()
            .unwrap_or_else(|| crate::llm::LlmResponse {
                content: format!("Monthly summary for {year_month} ({} days)", summaries.len()),
                tool_calls: Vec::new(),
            });
        
        let summary = response.content;
        let tags = serde_json::to_string(&Vec::<String>::new())?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let days_count = summaries.len() as i32;

        // Insert monthly summary (acquire lock again)
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO monthly_summaries (year_month, summary, tags, days_included) 
             VALUES (?1, ?2, ?3, ?4)",
            (year_month, summary, tags, days_count),
        )?;

        info!("Compacted {days_count} daily summaries into monthly summary for {year_month}");
        Ok(())
    }

    /// Rotate old conversations (delete conversations older than rotation_days)
    pub async fn rotate_old_conversations(&self, rotation_days: u32) -> Result<()> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(rotation_days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let conn = self.conn.lock().await;
        let deleted = conn.execute(
            "DELETE FROM daily_conversations WHERE date < ?1",
            [cutoff_date.to_string()],
        )?;

        info!("Rotated {deleted} old conversations (older than {cutoff_date})");
        Ok(())
    }

    /// Store a system prompt (base or AI-generated)
    pub async fn store_system_prompt(&self, prompt_type: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        
        // Deactivate previous prompts of this type
        conn.execute(
            "UPDATE system_prompts SET active = 0 WHERE prompt_type = ?1",
            [prompt_type],
        )?;

        // Get next version number
        let version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM system_prompts WHERE prompt_type = ?1",
                [prompt_type],
                |row| row.get(0),
            )
            .unwrap_or(1);

        // Insert new prompt
        conn.execute(
            "INSERT INTO system_prompts (prompt_type, content, version, active) 
             VALUES (?1, ?2, ?3, 1)",
            (prompt_type, content, version),
        )?;

        info!("Stored {prompt_type} system prompt (version {version})");
        Ok(())
    }

    /// Get the active system prompt of a specific type
    pub async fn get_system_prompt(&self, prompt_type: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        
        let result = conn.query_row(
            "SELECT content FROM system_prompts 
             WHERE prompt_type = ?1 AND active = 1 
             ORDER BY version DESC LIMIT 1",
            [prompt_type],
            |row| row.get(0),
        );

        match result {
            Ok(content) => Ok(Some(content)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Build the effective system prompt (base + AI-generated)
    pub async fn build_effective_prompt(&self) -> Result<String> {
        // First try to read from .config/system_prompt.md
        let config_prompt = std::fs::read_to_string(".config/system_prompt.md")
            .ok()
            .or_else(|| {
                // Try from home directory
                dirs::home_dir()
                    .and_then(|home| std::fs::read_to_string(home.join(".config/ritsu/system_prompt.md")).ok())
            });

        // Then get database prompts
        let base = self.get_system_prompt("base").await?;
        let ai_generated = self.get_system_prompt("ai_generated").await?;

        // Build the prompt in order: config file → database base → AI generated
        let mut parts = Vec::new();

        if let Some(config) = config_prompt {
            parts.push(config);
        } else if let Some(base_prompt) = base {
            parts.push(base_prompt);
        } else {
            parts.push("You are Ritsu, a helpful AI assistant.".to_string());
        }

        if let Some(ai) = ai_generated {
            parts.push(ai);
        }

        Ok(parts.join("\n\n"))
    }

    /// Store an idle analysis result
    pub async fn store_idle_analysis(
        &self,
        analysis_type: &str,
        findings: &std::collections::HashMap<String, String>,
        prompted_changes: Option<&str>,
    ) -> Result<()> {
        let findings_json = serde_json::to_string(findings)?;
        
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO idle_analyses (analysis_type, findings, prompted_changes) 
             VALUES (?1, ?2, ?3)",
            (analysis_type, findings_json, prompted_changes),
        )?;

        info!("Stored {analysis_type} idle analysis");
        Ok(())
    }

    /// Query recent conversations
    pub async fn query_recent_conversations(&self, days: u32) -> Result<Vec<(String, String, String)>> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT date, role, content FROM daily_conversations 
             WHERE date >= ?1 ORDER BY date DESC, id DESC LIMIT 100"
        )?;
        
        let rows = stmt.query_map([cutoff_date.to_string()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;

        let results: Vec<(String, String, String)> = rows.filter_map(Result::ok).collect();
        Ok(results)
    }

    /// Query daily summaries
    pub async fn query_daily_summaries(&self, days: u32) -> Result<Vec<(String, String)>> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT date, summary FROM daily_summaries 
             WHERE date >= ?1 ORDER BY date DESC"
        )?;
        
        let rows = stmt.query_map([cutoff_date.to_string()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        let results: Vec<(String, String)> = rows.filter_map(Result::ok).collect();
        Ok(results)
    }

    /// Query monthly summaries
    pub async fn query_monthly_summaries(&self, months: u32) -> Result<Vec<(String, String, i32)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT year_month, summary, days_included FROM monthly_summaries 
             ORDER BY year_month DESC LIMIT ?1"
        )?;
        
        let rows = stmt.query_map([months], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;

        let results: Vec<(String, String, i32)> = rows.filter_map(Result::ok).collect();
        Ok(results)
    }

    /// Get daily summary for a specific date
    pub async fn get_daily_summary(&self, date: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let result = conn.query_row(
            "SELECT summary FROM daily_summaries WHERE date = ?1",
            [date],
            |row| row.get::<_, String>(0),
        ).ok();
        Ok(result)
    }

    /// Query notes
    pub async fn query_notes(&self, limit: u32) -> Result<Vec<(i64, String, String)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, content, tags FROM notes 
             ORDER BY created_at DESC LIMIT ?1"
        )?;
        
        let rows = stmt.query_map([limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;

        let results: Vec<(i64, String, String)> = rows.filter_map(Result::ok).collect();
        Ok(results)
    }

    /// Get recent conversations (last N days)
    pub async fn get_recent_conversations_days(&self, days: i64) -> Result<Vec<(String, String, String, String)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT timestamp, role, content, user_id FROM conversations 
             WHERE datetime(timestamp) >= datetime('now', ? || ' days')
             ORDER BY timestamp DESC"
        )?;
        
        let rows = stmt.query_map([format!("-{}", days)], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        let results = rows.filter_map(Result::ok).collect();
        Ok(results)
    }

    /// Get recent summaries (daily or monthly)
    pub async fn get_summaries(&self, summary_type: &str, limit: i64) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT date, summary, tags FROM summaries 
             WHERE type = ?1
             ORDER BY date DESC LIMIT ?2"
        )?;
        
        let rows = stmt.query_map((summary_type, limit), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;

        let results = rows.filter_map(Result::ok).collect();
        Ok(results)
    }
}
