//! Memory management and compaction
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::doc_markdown)]

use anyhow::Result;
use chrono::NaiveDate;
use std::sync::Arc;
use tokio_rusqlite::rusqlite;
use tracing::info;

use crate::llm::LlmClient;

/// Tool usage record: (tool_name, arguments, success, result, timestamp)
type ToolUsageRecord = (String, String, bool, String, String);

pub struct MemoryManager {
    conn: Arc<tokio_rusqlite::Connection>,
    #[allow(dead_code)]
    db_path: String,
}

#[allow(dead_code)]
impl MemoryManager {
    #[must_use]
    pub fn new(conn: Arc<tokio_rusqlite::Connection>, db_path: String) -> Self {
        Self { conn, db_path }
    }

    /// Store a conversation message
    pub async fn store_conversation(&self, role: &str, content: &str) -> Result<()> {
        let date = chrono::Utc::now().date_naive();
        let role = role.to_string();
        let content = content.to_string();
        
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO daily_conversations (date, role, content) VALUES (?1, ?2, ?3)",
                (date.to_string(), &role, &content),
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    /// Create a note
    pub async fn create_note(&self, content: &str, tags: &[String]) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        let content = content.to_string();
        
        let id = self.conn.call(move |conn| -> rusqlite::Result<i64> {
            conn.execute(
                "INSERT INTO notes (content, tags) VALUES (?1, ?2)",
                (&content, &tags_json),
            )?;
            Ok(conn.last_insert_rowid())
        }).await?;
        Ok(id)
    }

    /// Compact daily conversations into a daily summary
    pub async fn compact_daily(&self, date: &NaiveDate, llm_client: &LlmClient, task_context: Option<&str>) -> Result<()> {
        info!("Compacting conversations for {date}");

        // Get all conversations for the date
        let date_str = date.to_string();
        let conversations = self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT role, content FROM daily_conversations 
                 WHERE date = ?1 ORDER BY timestamp"
            )?;
            
            let rows = stmt
                .query_map([&date_str], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        }).await?;

        if conversations.is_empty() {
            info!("No conversations to compact for {date}");
            return Ok(());
        }

        // Get previous day's summary for context
        let previous_summary = if let Some(previous_day) = date.pred_opt() {
            let prev_str = previous_day.to_string();
            self.conn.call(move |conn| -> rusqlite::Result<Option<String>> {
                match conn.query_row(
                    "SELECT summary FROM daily_summaries WHERE date = ?1",
                    [&prev_str],
                    |row| row.get::<_, String>(0),
                ) {
                    Ok(summary) => Ok(Some(summary)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(e),
                }
            }).await?
        } else {
            None
        };

        // Get compact-specific system prompt
        let system_prompt = self.build_background_prompt(Some("compact")).await?;

        // Generate summary with LLM (no lock held)
        let conversation_text = conversations.iter()
            .map(|(role, content)| format!("{role}: {content}"))
            .collect::<Vec<_>>()
            .join("\n");
        
        let mut prompt = format!("Summarize the following conversation into key points:\n\n{conversation_text}");
        
        if let Some(prev) = previous_summary {
            prompt = format!("Previous day context: {prev}\n\n{prompt}");
        }
        
        if let Some(tasks) = task_context {
            prompt = format!("{prompt}\n\n📋 Active Tasks:\n{tasks}\n\nInclude relevant task updates in your summary.");
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

        // Insert daily summary
        let date_str = date.to_string();
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT OR REPLACE INTO daily_summaries (date, summary, tags, conversation_count) 
                 VALUES (?1, ?2, ?3, ?4)",
                (&date_str, &summary, &tags, &count),
            )?;
            Ok(())
        }).await?;

        info!("Compacted {count} conversations into daily summary for {date}");
        Ok(())
    }

    /// Compact daily summaries into a monthly summary
    pub async fn compact_monthly(&self, year_month: &str, llm_client: &LlmClient, task_context: Option<&str>) -> Result<()> {
        info!("Compacting daily summaries for {year_month}");

        // Get all daily summaries for the month
        let pattern = format!("{year_month}%");
        let summaries = self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT date, summary FROM daily_summaries 
                 WHERE date LIKE ?1 ORDER BY date"
            )?;
            
            let rows = stmt
                .query_map([&pattern], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        }).await?;

        if summaries.is_empty() {
            info!("No daily summaries to compact for {year_month}");
            return Ok(());
        }

        // Get compact-specific system prompt
        let system_prompt = self.build_background_prompt(Some("compact")).await?;

        // Get previous month's summary for continuity
        let year_month_str = year_month.to_string();
        let prev_month_summary = self.conn.call(move |conn| -> rusqlite::Result<Option<String>> {
            match conn.query_row(
                "SELECT summary FROM monthly_summaries 
                 WHERE year_month < ?1 ORDER BY year_month DESC LIMIT 1",
                [&year_month_str],
                |row| row.get::<_, String>(0),
            ) {
                Ok(summary) => Ok(Some(summary)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        }).await?;

        // Generate monthly summary with LLM (no lock held)
        let summaries_text = summaries.iter()
            .map(|(date, summary)| format!("{date}: {summary}"))
            .collect::<Vec<String>>()
            .join("\n\n");
        
        let mut prompt = format!("Create a comprehensive monthly summary from these daily summaries:\n\n{summaries_text}\n\nIdentify key themes, patterns, and progress.");
        
        if let Some(prev) = prev_month_summary {
            prompt = format!("Previous month: {prev}\n\n{prompt}");
        }
        
        if let Some(tasks) = task_context {
            prompt = format!("{prompt}\n\n📋 Task Summary:\n{tasks}\n\nInclude task completion patterns and productivity insights.");
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

        // Insert monthly summary
        let year_month_str = year_month.to_string();
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT OR REPLACE INTO monthly_summaries (year_month, summary, tags, days_included) 
                 VALUES (?1, ?2, ?3, ?4)",
                (&year_month_str, &summary, &tags, &days_count),
            )?;
            Ok(())
        }).await?;

        info!("Compacted {days_count} daily summaries into monthly summary for {year_month}");
        Ok(())
    }

    /// Rotate old conversations (delete conversations older than rotation_days)
    pub async fn rotate_old_conversations(&self, rotation_days: u32) -> Result<()> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(rotation_days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let cutoff_str = cutoff_date.to_string();
        let deleted = self.conn.call(move |conn| {
            conn.execute(
                "DELETE FROM daily_conversations WHERE date < ?1",
                [&cutoff_str],
            )
        }).await?;

        info!("Rotated {deleted} old conversations (older than {cutoff_date})");
        Ok(())
    }

    /// Store a system prompt (base or AI-generated)
    pub async fn store_system_prompt(&self, prompt_type: &str, content: &str) -> Result<()> {
        let prompt_type = prompt_type.to_string();
        let content = content.to_string();
        
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            // Deactivate previous prompts of this type
            conn.execute(
                "UPDATE system_prompts SET active = 0 WHERE prompt_type = ?1",
                [&prompt_type],
            )?;

            // Get next version number
            let version: i32 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) + 1 FROM system_prompts WHERE prompt_type = ?1",
                    [&prompt_type],
                    |row| row.get(0),
                )
                .unwrap_or(1);

            // Insert new prompt
            conn.execute(
                "INSERT INTO system_prompts (prompt_type, content, version, active) 
                 VALUES (?1, ?2, ?3, 1)",
                (&prompt_type, &content, &version),
            )?;

            info!("Stored {prompt_type} system prompt (version {version})");
            Ok(())
        }).await?;
        Ok(())
    }

    /// Get the active system prompt of a specific type
    pub async fn get_system_prompt(&self, prompt_type: &str) -> Result<Option<String>> {
        let prompt_type = prompt_type.to_string();
        
        self.conn.call(move |conn| {
            let result = conn.query_row(
                "SELECT content FROM system_prompts 
                 WHERE prompt_type = ?1 AND active = 1 
                 ORDER BY version DESC LIMIT 1",
                [&prompt_type],
                |row| row.get(0),
            );

            match result {
                Ok(content) => Ok(Some(content)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        }).await.map_err(Into::into)
    }

    /// Build the effective system prompt (base + AI-generated)
    pub async fn build_effective_prompt(&self) -> Result<String> {
        self.build_prompt_for_context("general").await
    }

    /// Build system prompt for specific context (chat, background, etc.)
    pub async fn build_prompt_for_context(&self, context: &str) -> Result<String> {
        let mut parts = Vec::new();

        // 1. Load base system prompt
        let base = self.load_base_prompt().await?;
        parts.push(base);

        // 2. Load context-specific prompt
        if context != "general" {
            if let Ok(context_prompt) = Self::load_context_prompt(context).await {
                parts.push(context_prompt);
            }
        }

        // 3. Add AI-generated enhancements
        if let Ok(Some(ai_generated)) = self.get_system_prompt("ai_generated").await {
            parts.push(ai_generated);
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    /// Load base system prompt from various sources
    async fn load_base_prompt(&self) -> Result<String> {
        // Try .config/ritsu/prompts/system_base.md first
        if let Ok(content) = crate::database::read_file_async(".config/ritsu/prompts/system_base.md").await {
            return Ok(content);
        }

        // Try home directory
        if let Some(home) = dirs::home_dir() {
            if let Ok(content) = crate::database::read_file_async(home.join(".config/ritsu/prompts/system_base.md")).await {
                return Ok(content);
            }
        }

        // Fall back to legacy .config/system_prompt.md
        if let Ok(content) = crate::database::read_file_async(".config/system_prompt.md").await {
            return Ok(content);
        }

        // Try home directory legacy location
        if let Some(home) = dirs::home_dir() {
            if let Ok(content) = crate::database::read_file_async(home.join(".config/ritsu/system_prompt.md")).await {
                return Ok(content);
            }
        }

        // Try database
        if let Ok(Some(base)) = self.get_system_prompt("base").await {
            return Ok(base);
        }

        // No custom prompt found - warn user
        eprintln!("⚠ System prompt file not found: ~/.config/ritsu/prompts/system_base.md");
        eprintln!("  Using default system prompt.");
        eprintln!();
        eprintln!("  To customize the system prompt:");
        eprintln!("    mkdir -p ~/.config/ritsu/prompts");
        eprintln!("    echo 'Your custom system prompt' > ~/.config/ritsu/prompts/system_base.md");
        eprintln!();

        // Ultimate fallback
        Ok("You are Ritsu, a helpful AI assistant.".to_string())
    }

    /// Load context-specific prompt (chat, background, etc.)
    async fn load_context_prompt(context: &str) -> Result<String> {
        let filename = match context {
            "chat" => "chat.md",
            "background" => "background.md",
            "compact" => "background/compact.md",
            "pattern" => "background/pattern.md",
            "briefing" => "background/briefing.md",
            _ => return Err(anyhow::anyhow!("Unknown context: {context}")),
        };

        // Try .config/ritsu/prompts/{filename}
        let local_path = format!(".config/ritsu/prompts/{filename}");
        if let Ok(content) = crate::database::read_file_async(local_path).await {
            return Ok(content);
        }

        // Try home directory
        if let Some(home) = dirs::home_dir() {
            let home_path = home.join(format!(".config/ritsu/prompts/{filename}"));
            if let Ok(content) = crate::database::read_file_async(home_path).await {
                return Ok(content);
            }
        }

        Err(anyhow::anyhow!("Context prompt not found: {context}"))
    }

    /// Build chat-specific system prompt
    pub async fn build_chat_prompt(&self) -> Result<String> {
        self.build_prompt_for_context("chat").await
    }

    /// Build background task system prompt based on analysis type
    pub async fn build_background_prompt(&self, analysis_type: Option<&str>) -> Result<String> {
        let context = match analysis_type {
            Some("conversation" | "reflection") => "compact",  // Daily/monthly compaction
            Some("pattern") => "pattern",       // Weekly pattern recognition
            Some("morning_briefing" | "daily_briefing") => "briefing",
            _ => "background",  // Generic background task
        };
        self.build_prompt_for_context(context).await
    }

    /// Store an idle analysis result
    pub async fn store_idle_analysis(
        &self,
        analysis_type: &str,
        findings: &std::collections::HashMap<String, String>,
        prompted_changes: Option<&str>,
    ) -> Result<()> {
        let findings_json = serde_json::to_string(findings)?;
        let analysis_type = analysis_type.to_string();
        let analysis_type_for_log = analysis_type.clone();
        let prompted_changes = prompted_changes.map(String::from);
        
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO idle_analyses (analysis_type, findings, prompted_changes) 
                 VALUES (?1, ?2, ?3)",
                (&analysis_type, &findings_json, &prompted_changes),
            )?;
            Ok(())
        }).await?;

        info!("Stored {analysis_type_for_log} idle analysis");
        Ok(())
    }

    /// Query recent conversations
    pub async fn query_recent_conversations(&self, days: u32) -> Result<Vec<(String, String, String)>> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let cutoff_str = cutoff_date.to_string();
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT date, role, content FROM daily_conversations 
                 WHERE date >= ?1 ORDER BY date DESC, id DESC LIMIT 100"
            )?;
            
            let rows = stmt.query_map([&cutoff_str], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

            let results: Vec<(String, String, String)> = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Query daily summaries
    pub async fn query_daily_summaries(&self, days: u32) -> Result<Vec<(String, String)>> {
        let cutoff_date = chrono::Utc::now()
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(days)))
            .ok_or_else(|| anyhow::anyhow!("Failed to calculate cutoff date"))?;

        let cutoff_str = cutoff_date.to_string();
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT date, summary FROM daily_summaries 
                 WHERE date >= ?1 ORDER BY date DESC"
            )?;
            
            let rows = stmt.query_map([&cutoff_str], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;

            let results: Vec<(String, String)> = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Query monthly summaries
    pub async fn query_monthly_summaries(&self, months: u32) -> Result<Vec<(String, String, i32)>> {
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String, i32)>> {
            let mut stmt = conn.prepare(
                "SELECT year_month, summary, days_included FROM monthly_summaries 
                 ORDER BY year_month DESC LIMIT ?1"
            )?;
            
            let rows = stmt.query_map([&months], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

            let results: Vec<(String, String, i32)> = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get daily summary for a specific date
    pub async fn get_daily_summary(&self, date: &str) -> Result<Option<String>> {
        let date = date.to_string();
        self.conn.call(move |conn| -> rusqlite::Result<Option<String>> {
            let result = conn.query_row(
                "SELECT summary FROM daily_summaries WHERE date = ?1",
                [&date],
                |row| row.get::<_, String>(0),
            ).ok();
            Ok(result)
        }).await.map_err(Into::into)
    }

    /// Query notes
    pub async fn query_notes(&self, limit: u32) -> Result<Vec<(i64, String, String)>> {
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(i64, String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT id, content, tags FROM notes 
                 ORDER BY created_at DESC LIMIT ?1"
            )?;
            
            let rows = stmt.query_map([&limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

            let results: Vec<(i64, String, String)> = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get recent conversations (last N days)
    pub async fn get_recent_conversations_days(&self, days: i64) -> Result<Vec<(String, String, String, String)>> {
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String, String, String)>> {
            let mut stmt = conn.prepare(
                "SELECT ct.timestamp, ct.role, ct.content, ct.session_id as user_id 
                 FROM conversation_turns ct
                 WHERE datetime(ct.timestamp) >= datetime('now', ? || ' days')
                 ORDER BY ct.timestamp DESC"
            )?;
            
            let days_param = format!("-{days}");
            let rows = stmt.query_map([&days_param], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;

            let results = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get recent summaries (daily or monthly)
    pub async fn get_summaries(&self, summary_type: &str, limit: i64) -> Result<Vec<(String, String, String)>> {
        let summary_type = summary_type.to_string();
        
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, String, String)>> {
            let query = match summary_type.as_str() {
                "daily" => "SELECT date, summary, tags FROM daily_summaries ORDER BY date DESC LIMIT ?1",
                "monthly" => "SELECT year_month, summary, tags FROM monthly_summaries ORDER BY year_month DESC LIMIT ?1",
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            
            let mut stmt = conn.prepare(query)?;
            let rows = stmt.query_map([&limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

            let results = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get tool usage statistics
    pub async fn get_tool_usage_stats(&self, days: i64) -> Result<Vec<(String, i64, i64, f64)>> {
        self.conn.call(move |conn| -> rusqlite::Result<Vec<(String, i64, i64, f64)>> {
            let mut stmt = conn.prepare(
                "SELECT tool_name, 
                        COUNT(*) as total_calls,
                        SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_calls,
                        AVG(execution_time_ms) as avg_execution_time
             FROM tool_usage
             WHERE datetime(timestamp) >= datetime('now', ? || ' days')
             GROUP BY tool_name
                 ORDER BY total_calls DESC"
            )?;
            
            let days_param = format!("-{days}");
            let rows = stmt.query_map([&days_param], |row| {
                Ok((
                    row.get(0)?,  // tool_name
                    row.get(1)?,  // total_calls
                    row.get(2)?,  // successful_calls
                    row.get(3)?,  // avg_execution_time
                ))
            })?;

            let results = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get recent tool usage
    pub async fn get_recent_tool_usage(&self, limit: i64) -> Result<Vec<ToolUsageRecord>> {
        self.conn.call(move |conn| -> rusqlite::Result<Vec<ToolUsageRecord>> {
            let mut stmt = conn.prepare(
                "SELECT tool_name, arguments, success, result, timestamp
                 FROM tool_usage
                 ORDER BY timestamp DESC
                 LIMIT ?1"
            )?;
            
            let rows = stmt.query_map([&limit], |row| {
                Ok((
                    row.get(0)?,  // tool_name
                    row.get(1)?,  // arguments
                    row.get(2)?,  // success
                    row.get(3)?,  // result
                    row.get(4)?,  // timestamp
                ))
            })?;

            let results = rows.filter_map(Result::ok).collect();
            Ok(results)
        }).await.map_err(Into::into)
    }

    /// Get tool effectiveness summary
    pub async fn get_tool_effectiveness_summary(&self, days: i64) -> Result<String> {
        let stats = self.get_tool_usage_stats(days).await?;
        
        if stats.is_empty() {
            return Ok(format!("No tool usage in the past {days} days"));
        }

        let mut summary = format!("Tool Usage Summary (Past {days} Days):\n\n");
        
        for (tool_name, total, successful, avg_time) in stats {
            let success_rate = if total > 0 {
                #[allow(clippy::cast_precision_loss)]
                let rate = (successful as f64 / total as f64) * 100.0;
                rate
            } else {
                0.0
            };
            
            summary = format!("{summary}📊 {tool_name}: {total} calls, {success_rate:.1}% success, {avg_time:.0}ms avg\n");
        }
        
        Ok(summary)
    }

    /// Clear all memory tables (for testing)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn clear_all(&self) -> Result<()> {
        self.conn.call(|conn| -> rusqlite::Result<()> {
            conn.execute("DELETE FROM notes", [])?;
            conn.execute("DELETE FROM daily_conversations", [])?;
            conn.execute("DELETE FROM daily_summaries", [])?;
            conn.execute("DELETE FROM monthly_summaries", [])?;
            conn.execute("DELETE FROM idle_analyses", [])?;
            conn.execute("DELETE FROM tool_usage", [])?;
            Ok(())
        }).await?;
        
        info!("Cleared all memory tables");
        Ok(())
    }

    /// Get database statistics
    pub async fn get_database_stats(&self) -> Result<String> {
        self.conn.call(|conn| -> rusqlite::Result<String> {
            let conversations: i64 = conn.query_row(
                "SELECT COUNT(*) FROM daily_conversations",
                [],
                |row| row.get(0),
            )?;
            
            let daily_summaries: i64 = conn.query_row(
                "SELECT COUNT(*) FROM daily_summaries",
                [],
                |row| row.get(0),
            )?;
            
            let monthly_summaries: i64 = conn.query_row(
                "SELECT COUNT(*) FROM monthly_summaries",
                [],
                |row| row.get(0),
            )?;
            
            let notes: i64 = conn.query_row(
                "SELECT COUNT(*) FROM notes",
                [],
                |row| row.get(0),
            )?;
            
            let tasks: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tasks",
                [],
                |row| row.get(0),
            )?;
            
            let triggers: i64 = conn.query_row(
                "SELECT COUNT(*) FROM triggers",
                [],
                |row| row.get(0),
            )?;
            
            Ok(format!(
                "Database Statistics:\n\
                 Conversations: {conversations}\n\
                 Daily Summaries: {daily_summaries}\n\
                 Monthly Summaries: {monthly_summaries}\n\
                 Notes: {notes}\n\
                 Tasks: {tasks}\n\
                 Triggers: {triggers}"
            ))
        }).await.map_err(Into::into)
    }

    /// Export database to JSON
    #[allow(clippy::unused_async)]
    pub async fn export_database(&self) -> Result<String> {
        // For now, return a simple message
        // Full implementation would serialize all tables
        Ok("Database export not yet implemented. Use sqlite3 to export directly.".to_string())
    }

    /// Get memory compaction status
    pub async fn get_compaction_status(&self) -> Result<String> {
        self.conn.call(|conn| -> rusqlite::Result<String> {
            let last_daily: Option<String> = conn.query_row(
                "SELECT date FROM daily_summaries ORDER BY date DESC LIMIT 1",
                [],
                |row| row.get(0),
            ).ok();
            
            let last_monthly: Option<String> = conn.query_row(
                "SELECT year_month FROM monthly_summaries ORDER BY year_month DESC LIMIT 1",
                [],
                |row| row.get(0),
            ).ok();
            
            let oldest_conv: Option<String> = conn.query_row(
                "SELECT date FROM daily_conversations ORDER BY date ASC LIMIT 1",
                [],
                |row| row.get(0),
            ).ok();
            
            Ok(format!(
                "Memory Compaction Status:\n\
                 Last daily summary: {}\n\
                 Last monthly summary: {}\n\
                 Oldest conversation: {}",
                last_daily.unwrap_or_else(|| "None".to_string()),
                last_monthly.unwrap_or_else(|| "None".to_string()),
                oldest_conv.unwrap_or_else(|| "None".to_string()),
            ))
        }).await.map_err(Into::into)
    }

    /// Force memory compaction
    #[allow(clippy::unused_async)]
    pub async fn force_compact(&self) -> Result<()> {
        // For now, just return success
        // Full implementation would run daily compaction logic
        info!("Force compact requested (not yet implemented)");
        Ok(())
    }

    /// Reindex database
    pub async fn reindex_database(&self) -> Result<()> {
        self.conn.call(|conn| -> rusqlite::Result<()> {
            conn.execute("REINDEX", [])?;
            Ok(())
        }).await.map_err(|e| anyhow::anyhow!("DB error: {e}"))?;
        
        info!("Database reindexed");
        Ok(())
    }
}
