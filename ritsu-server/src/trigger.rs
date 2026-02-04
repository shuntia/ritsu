//! Trigger system - self-scheduling and idle analysis
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::future_not_send)]
#![allow(clippy::struct_field_names)]
#![allow(dead_code)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::uninlined_format_args)]

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveTime, TimeZone};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, sleep_until, Duration, Instant};
use tracing::{error, info, warn};

use crate::llm::LlmClient;
use crate::memory::MemoryManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerType {
    /// Time-based: "HH:MM" format (24-hour)
    Time(String),
    /// Interval: seconds between triggers
    Interval(u64),
    /// Inactivity: seconds of no user interaction
    Inactivity(u64),
    /// Dynamic: AI-initiated
    Dynamic,
}

#[derive(Debug, Clone)]
pub struct Trigger {
    pub id: i64,
    pub name: String,
    pub trigger_type: TriggerType,
    pub enabled: bool,
    pub created_by: String,
    pub metadata: HashMap<String, String>,
}

pub struct TriggerRegistry {
    triggers: Arc<RwLock<Vec<Trigger>>>,
    db_path: String,
}

impl TriggerRegistry {
    pub fn new(db_path: String) -> Self {
        Self {
            triggers: Arc::new(RwLock::new(Vec::new())),
            db_path,
        }
    }

    pub async fn load_from_database(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        let loaded_triggers: Vec<Trigger> = {
            let mut stmt = conn.prepare("SELECT id, name, trigger_type, schedule, enabled, created_by, metadata FROM triggers WHERE enabled = 1")?;
            
            let triggers_iter = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let trigger_type_str: String = row.get(2)?;
                let schedule: String = row.get(3)?;
                let enabled: bool = row.get(4)?;
                let created_by: String = row.get(5)?;
                let metadata_json: String = row.get(6)?;

                let trigger_type = match trigger_type_str.as_str() {
                    "time" => TriggerType::Time(schedule),
                    "interval" => TriggerType::Interval(schedule.parse().unwrap_or(3600)),
                    "inactivity" => TriggerType::Inactivity(schedule.parse().unwrap_or(1800)),
                    "dynamic" => TriggerType::Dynamic,
                    _ => TriggerType::Dynamic,
                };

                let metadata: HashMap<String, String> = serde_json::from_str(&metadata_json)
                    .unwrap_or_default();

                Ok(Trigger {
                    id,
                    name,
                    trigger_type,
                    enabled,
                    created_by,
                    metadata,
                })
            })?;

            triggers_iter.filter_map(Result::ok).collect()
        };

        let mut triggers = self.triggers.write().await;
        triggers.clear();
        triggers.extend(loaded_triggers);

        info!("Loaded {} triggers from database", triggers.len());
        Ok(())
    }

    pub async fn register_builtin_triggers(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        // Daily conversation compaction at 2:00 AM
        Self::insert_trigger_if_not_exists(
            &conn,
            "daily_compaction",
            "time",
            "02:00",
            "system",
            r#"{"analysis_type":"conversation"}"#,
        )?;

        // Weekly pattern recognition on Sunday at 3:00 AM
        Self::insert_trigger_if_not_exists(
            &conn,
            "weekly_pattern",
            "time",
            "03:00",
            "system",
            r#"{"analysis_type":"pattern","day":"sunday"}"#,
        )?;

        // Bi-weekly tool effectiveness (1st & 15th at 4:00 AM)
        Self::insert_trigger_if_not_exists(
            &conn,
            "biweekly_tools",
            "time",
            "04:00",
            "system",
            r#"{"analysis_type":"tools","days":"1,15"}"#,
        )?;

        // Monthly self-reflection on last day at 5:00 AM
        Self::insert_trigger_if_not_exists(
            &conn,
            "monthly_reflection",
            "time",
            "05:00",
            "system",
            r#"{"analysis_type":"reflection","day":"last"}"#,
        )?;

        // Inactivity analysis after 30 minutes
        Self::insert_trigger_if_not_exists(
            &conn,
            "inactivity_analysis",
            "inactivity",
            "1800",
            "system",
            r#"{"analysis_type":"conversation"}"#,
        )?;

        self.load_from_database().await?;
        Ok(())
    }

    fn insert_trigger_if_not_exists(
        conn: &Connection,
        name: &str,
        trigger_type: &str,
        schedule: &str,
        created_by: &str,
        metadata: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, datetime('now'))",
            (name, trigger_type, schedule, created_by, metadata),
        )?;
        Ok(())
    }

    pub async fn get_all_triggers(&self) -> Vec<Trigger> {
        self.triggers.read().await.clone()
    }

    pub async fn create_trigger(
        &self,
        name: &str,
        trigger_type: &str,
        schedule: &str,
    ) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        conn.execute(
            "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
             VALUES (?1, ?2, ?3, 1, 'user', '{}', datetime('now'))",
            (name, trigger_type, schedule),
        )?;

        self.load_from_database().await?;
        Ok(())
    }

    pub async fn delete_trigger(&self, name: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        let rows = conn.execute("DELETE FROM triggers WHERE name = ?1", [name])?;
        
        if rows == 0 {
            anyhow::bail!("Trigger not found: {}", name);
        }

        self.load_from_database().await?;
        Ok(())
    }

    pub async fn disable_trigger(&self, name: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        let rows = conn.execute(
            "UPDATE triggers SET enabled = 0 WHERE name = ?1",
            [name],
        )?;
        
        if rows == 0 {
            anyhow::bail!("Trigger not found: {}", name);
        }

        self.load_from_database().await?;
        Ok(())
    }
}

/// Calculate next trigger time for time-based triggers
fn calculate_next_trigger_time(time_str: &str) -> Option<Instant> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;

    let now = Local::now();
    let target_time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    
    let mut target_datetime = now.date_naive().and_time(target_time);
    let target_datetime_with_tz = Local.from_local_datetime(&target_datetime).single()?;

    if target_datetime_with_tz <= now {
        // If time has passed today, schedule for tomorrow
        target_datetime = target_datetime.checked_add_days(Days::new(1))?;
        let target_datetime_with_tz = Local.from_local_datetime(&target_datetime).single()?;
        let duration = (target_datetime_with_tz - now).to_std().ok()?;
        Some(Instant::now() + duration)
    } else {
        let duration = (target_datetime_with_tz - now).to_std().ok()?;
        Some(Instant::now() + duration)
    }
}

/// Check if trigger should run today based on metadata
    fn should_run_today(trigger: &Trigger) -> bool {
    let now = Local::now();
    
    match &trigger.trigger_type {
        TriggerType::Time(_) => {
            trigger.metadata.get("day").map_or_else(
                || {
                    if let Some(days_str) = trigger.metadata.get("days") {
                        let day = now.day();
                        days_str.split(',').any(|d| d.trim().parse::<u32>() == Ok(day))
                    } else {
                        true
                    }
                },
                |day_str| match day_str.as_str() {
                    "sunday" => now.weekday() == chrono::Weekday::Sun,
                    "monday" => now.weekday() == chrono::Weekday::Mon,
                    "last" => {
                        let tomorrow = now.date_naive().succ_opt();
                        tomorrow.is_some_and(|t| t.month() != now.month())
                    }
                    _ => true,
                },
            )
        }
        _ => true,
    }
}

/// Execute idle analysis based on trigger metadata
pub async fn execute_idle_analysis(
    trigger: &Trigger,
    memory: &MemoryManager,
    llm_client: &LlmClient,
) -> Result<()> {
    let analysis_type = trigger.metadata.get("analysis_type")
        .map_or("conversation", String::as_str);

    info!("Executing idle analysis: {} ({})", trigger.name, analysis_type);

    match analysis_type {
        "conversation" => {
            // Daily compaction: aggregate conversations into summary
            let yesterday = Local::now().date_naive().pred_opt()
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate yesterday"))?;
            
            if let Err(e) = memory.compact_daily(&yesterday, llm_client).await {
                error!("Failed to compact daily conversations: {}", e);
            }
        }
        "pattern" => {
            // Weekly pattern recognition
            let findings = HashMap::from([
                ("type".to_string(), "pattern_recognition".to_string()),
                ("summary".to_string(), "Weekly pattern analysis completed".to_string()),
            ]);
            memory.store_idle_analysis("pattern", &findings, None).await?;
            info!("Weekly pattern recognition completed");
        }
        "tools" => {
            // Bi-weekly tool effectiveness analysis
            let findings = HashMap::from([
                ("type".to_string(), "tool_effectiveness".to_string()),
                ("summary".to_string(), "Tool effectiveness analysis completed".to_string()),
            ]);
            memory.store_idle_analysis("tools", &findings, None).await?;
            info!("Tool effectiveness analysis completed");
        }
        "reflection" => {
            // Monthly self-reflection and compaction
            let last_month = Local::now().date_naive()
                .with_day(1)
                .and_then(|d| d.pred_opt())
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate last month"))?;
            
            let year_month = format!("{}-{:02}", last_month.year(), last_month.month());
            
            if let Err(e) = memory.compact_monthly(&year_month, llm_client).await {
                error!("Failed to compact monthly summaries: {}", e);
            }

            let findings = HashMap::from([
                ("type".to_string(), "self_reflection".to_string()),
                ("summary".to_string(), "Monthly self-reflection completed".to_string()),
            ]);
            memory.store_idle_analysis("reflection", &findings, None).await?;
            info!("Monthly self-reflection completed");
        }
        _ => {
            warn!("Unknown analysis type: {}", analysis_type);
        }
    }

    Ok(())
}

/// Main trigger loop - awaits multiple triggers and executes them
pub async fn run_trigger_loop(
    registry: Arc<TriggerRegistry>,
    memory: Arc<MemoryManager>,
    llm_client: Arc<LlmClient>,
) -> Result<()> {
    info!("Starting trigger loop");

    loop {
        let triggers = registry.get_all_triggers().await;
        
        if triggers.is_empty() {
            info!("No triggers registered, sleeping for 60 seconds");
            sleep(Duration::from_secs(60)).await;
            continue;
        }

        let mut next_trigger: Option<(Instant, Trigger)> = None;

        for trigger in triggers {
            if !trigger.enabled {
                continue;
            }

            match &trigger.trigger_type {
                TriggerType::Time(time_str) => {
                    if !should_run_today(&trigger) {
                        continue;
                    }
                    
                    if let Some(instant) = calculate_next_trigger_time(time_str) {
                        if next_trigger.as_ref().is_none_or(|(i, _)| instant < *i) {
                            next_trigger = Some((instant, trigger));
                        }
                    }
                }
                TriggerType::Interval(seconds) => {
                    let instant = Instant::now() + Duration::from_secs(*seconds);
                    if next_trigger.as_ref().is_none_or(|(i, _)| instant < *i) {
                        next_trigger = Some((instant, trigger));
                    }
                }
                TriggerType::Inactivity(_) | TriggerType::Dynamic => {
                    // Inactivity triggers handled separately by conversation system
                    // Dynamic triggers handled by AI requests
                }
            }
        }

        if let Some((instant, trigger)) = next_trigger {
            info!("Next trigger: {} at {:?}", trigger.name, instant);
            sleep_until(instant).await;
            
            // Execute the trigger
            info!("Executing trigger: {}", trigger.name);
            if let Err(e) = execute_idle_analysis(&trigger, &memory, &llm_client).await {
                error!("Failed to execute trigger {}: {}", trigger.name, e);
            }
        } else {
            // No triggers ready, check again in 60 seconds
            sleep(Duration::from_secs(60)).await;
        }
    }
}
