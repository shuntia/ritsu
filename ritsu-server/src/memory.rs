//! Memory management and compaction

use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use tracing::info;

pub struct MemoryManager<'a> {
    conn: &'a Connection,
}

#[allow(dead_code)]
impl<'a> MemoryManager<'a> {
    #[must_use]
    pub const fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Store a conversation message
    pub fn store_conversation(&self, role: &str, content: &str) -> Result<()> {
        let date = chrono::Utc::now().date_naive();
        
        self.conn.execute(
            "INSERT INTO daily_conversations (date, role, content) VALUES (?1, ?2, ?3)",
            (date.to_string(), role, content),
        )?;
        
        Ok(())
    }

    /// Create a note
    pub fn create_note(&self, content: &str, tags: &[String]) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        
        self.conn.execute(
            "INSERT INTO notes (content, tags) VALUES (?1, ?2)",
            (content, tags_json),
        )?;
        
        Ok(self.conn.last_insert_rowid())
    }

    /// Compact daily conversations into a daily summary
    pub fn compact_daily(&self, date: NaiveDate) -> Result<()> {
        info!("Compacting conversations for {date}");

        // Get all conversations for the date
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM daily_conversations 
             WHERE date = ?1 ORDER BY timestamp"
        )?;
        
        let conversations: Vec<(String, String)> = stmt
            .query_map([date.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if conversations.is_empty() {
            info!("No conversations to compact for {date}");
            return Ok(());
        }

        // TODO: Use LLM to generate summary
        let summary = format!("Day had {} conversations (placeholder summary)", conversations.len());
        let tags = serde_json::to_string(&Vec::<String>::new())?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let count = conversations.len() as i32;

        // Insert daily summary
        self.conn.execute(
            "INSERT OR REPLACE INTO daily_summaries (date, summary, tags, conversation_count) 
             VALUES (?1, ?2, ?3, ?4)",
            (date.to_string(), summary, tags, count),
        )?;

        info!("Created daily summary for {date} with {count} conversations");
        Ok(())
    }

    /// Compact daily summaries into a monthly summary
    pub fn compact_monthly(&self, year: i32, month: u32) -> Result<()> {
        let year_month = format!("{year:04}-{month:02}");
        info!("Compacting daily summaries for {year_month}");

        // Get all daily summaries for the month
        let mut stmt = self.conn.prepare(
            "SELECT date, summary FROM daily_summaries 
             WHERE date LIKE ?1 ORDER BY date"
        )?;
        
        let summaries: Vec<(String, String)> = stmt
            .query_map([format!("{year_month}%")], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if summaries.is_empty() {
            info!("No daily summaries to compact for {year_month}");
            return Ok(());
        }

        // TODO: Use LLM to generate monthly summary
        let summary = format!("Month had {} days with activity (placeholder summary)", summaries.len());
        let tags = serde_json::to_string(&Vec::<String>::new())?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let days_count = summaries.len() as i32;

        // Insert monthly summary
        self.conn.execute(
            "INSERT OR REPLACE INTO monthly_summaries (year_month, summary, tags, days_included) 
             VALUES (?1, ?2, ?3, ?4)",
            (&year_month, summary, tags, days_count),
        )?;

        info!("Created monthly summary for {year_month} with {days_count} days");
        Ok(())
    }

    /// Rotate out old daily conversations (40+ days old)
    pub fn rotate_old_conversations(&self, rotation_days: u32) -> Result<()> {
        let cutoff_date = chrono::Utc::now().date_naive() - chrono::Duration::days(i64::from(rotation_days));
        
        let deleted = self.conn.execute(
            "DELETE FROM daily_conversations WHERE date < ?1",
            [cutoff_date.to_string()],
        )?;

        if deleted > 0 {
            info!("Rotated out {deleted} old conversation entries before {cutoff_date}");
        }

        Ok(())
    }

    /// Store or update system prompt
    pub fn store_system_prompt(&self, prompt_type: &str, content: &str) -> Result<()> {
        // Deactivate existing prompts of this type
        self.conn.execute(
            "UPDATE system_prompts SET active = FALSE WHERE prompt_type = ?1",
            [prompt_type],
        )?;

        // Get next version number
        let version: i32 = self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM system_prompts WHERE prompt_type = ?1",
                [prompt_type],
                |row| row.get(0),
            )?;

        // Insert new prompt
        self.conn.execute(
            "INSERT INTO system_prompts (prompt_type, content, version, active) 
             VALUES (?1, ?2, ?3, TRUE)",
            (prompt_type, content, version),
        )?;

        info!("Stored system prompt type '{prompt_type}' version {version}");
        Ok(())
    }

    /// Get active system prompt
    pub fn get_system_prompt(&self, prompt_type: &str) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT content FROM system_prompts 
             WHERE prompt_type = ?1 AND active = TRUE 
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

    /// Build effective system prompt (base + AI-generated)
    pub fn build_effective_prompt(&self) -> Result<String> {
        let base = self.get_system_prompt("base")?
            .unwrap_or_else(|| "You are Ritsu, a self-triggering AI agent.".to_string());
        
        let ai_additions = self.get_system_prompt("ai_generated")?;

        Ok(match ai_additions {
            Some(additions) => format!("{base}\n\n--- AI-Generated Context ---\n{additions}"),
            None => base,
        })
    }

    /// Store idle analysis results
    pub fn store_idle_analysis(
        &self, 
        analysis_type: &str, 
        findings: &str, 
        prompted_changes: Option<&str>
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO idle_analyses (analysis_type, findings, prompted_changes) 
             VALUES (?1, ?2, ?3)",
            (analysis_type, findings, prompted_changes),
        )?;

        info!("Stored {analysis_type} idle analysis");
        Ok(())
    }
}
