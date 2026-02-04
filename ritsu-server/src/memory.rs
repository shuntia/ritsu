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

        // Generate summary with LLM (no lock held)
        let conversation_text = conversations.iter()
            .map(|(role, content)| format!("{role}: {content}"))
            .collect::<Vec<_>>()
            .join("\n");
        
        let messages = vec![
            crate::llm::Message {
                role: "user".to_string(),
                content: format!("Summarize the following conversation into key points:\n\n{conversation_text}"),
            }
        ];
        
        let response = llm_client.generate(&messages, None).await
            .unwrap_or_else(|_| crate::llm::LlmResponse {
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

        // Generate monthly summary with LLM (no lock held)
        let summaries_text = summaries.iter()
            .map(|(date, summary)| format!("{date}: {summary}"))
            .collect::<Vec<String>>()
            .join("\n\n");
        
        let messages = vec![
            crate::llm::Message {
                role: "user".to_string(),
                content: format!("Create a comprehensive monthly summary from these daily summaries:\n\n{summaries_text}"),
            }
        ];
        
        let response = llm_client.generate(&messages, None).await
            .unwrap_or_else(|_| crate::llm::LlmResponse {
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
        let base = self.get_system_prompt("base").await?
            .unwrap_or_else(|| "You are Ritsu, a helpful AI assistant.".to_string());
        
        let ai_generated = self.get_system_prompt("ai_generated").await?;

        Ok(ai_generated.map_or_else(
            || base.clone(),
            |ai| format!("{base}\n\n{ai}"),
        ))
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
}
