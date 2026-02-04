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
        tag: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        
        // Build metadata JSON with tag and description
        let mut metadata = HashMap::new();
        if let Some(t) = tag {
            metadata.insert("tag".to_string(), t.to_string());
        }
        if let Some(d) = description {
            metadata.insert("description".to_string(), d.to_string());
        }
        let metadata_json = serde_json::to_string(&metadata)?;
        
        conn.execute(
            "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
             VALUES (?1, ?2, ?3, 1, 'user', ?4, datetime('now'))",
            (name, trigger_type, schedule, metadata_json),
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
#[allow(clippy::too_many_lines)]
pub async fn execute_idle_analysis(
    trigger: &Trigger,
    memory: &MemoryManager,
    task_manager: &crate::tasks::TaskManager,
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
            
            // Get task context
            let task_context = task_manager.get_task_summary().await.ok();
            
            if let Err(e) = memory.compact_daily(&yesterday, llm_client, task_context.as_deref()).await {
                error!("Failed to compact daily conversations: {}", e);
            }
        }
        "pattern" => {
            // Weekly pattern recognition
            info!("Starting weekly pattern recognition");
            
            // Get past week's daily summaries
            let past_summaries = memory.query_daily_summaries(7).await?;
            
            if past_summaries.is_empty() {
                info!("No summaries to analyze for pattern recognition");
                return Ok(());
            }
            
            let system_prompt = memory.build_effective_prompt().await?;
            let summaries_text = past_summaries.iter()
                .map(|(date, summary)| format!("{date}: {summary}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            
            // Get task context
            let task_summary = task_manager.get_active_tasks_summary().await.unwrap_or_default();
            
            let mut prompt = format!(
                "Analyze patterns from the past week's activity:\n\n{}\n\nIdentify:\n1. Recurring themes and topics\n2. Time-based patterns\n3. User preferences and habits\n4. Areas of focus",
                summaries_text
            );
            
            if !task_summary.is_empty() {
                prompt = format!("{}\n\n📋 Current Tasks:\n{}\n\nInclude task progress and workflow patterns in analysis.", prompt, task_summary);
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
                .unwrap_or_else(|| {
                    warn!("Pattern analysis LLM error");
                    crate::llm::LlmResponse {
                        content: format!("Pattern analysis for past 7 days ({} summaries)", past_summaries.len()),
                        tool_calls: Vec::new(),
                    }
                });
            
            let findings = HashMap::from([
                ("type".to_string(), "pattern_recognition".to_string()),
                ("summary".to_string(), response.content.clone()),
                ("period".to_string(), "7_days".to_string()),
            ]);
            memory.store_idle_analysis("pattern", &findings, None).await?;
            info!("Weekly pattern recognition completed: {}", response.content.chars().take(100).collect::<String>());
        }
        "tools" => {
            // Bi-weekly tool effectiveness analysis
            info!("Starting tool effectiveness analysis");
            
            // Get tool usage statistics
            let tool_stats = memory.get_tool_effectiveness_summary(14).await
                .unwrap_or_else(|_| "Unable to retrieve tool usage statistics".to_string());
            
            // Get past 14 days of activity
            let past_summaries = memory.query_daily_summaries(14).await?;
            
            if past_summaries.is_empty() && tool_stats.contains("No tool usage") {
                info!("No activity to analyze for tool effectiveness");
                return Ok(());
            }
            
            let system_prompt = memory.build_effective_prompt().await?;
            let summaries_text = if !past_summaries.is_empty() {
                past_summaries.iter()
                    .map(|(date, summary)| format!("{date}: {summary}"))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else {
                "No daily summaries available".to_string()
            };
            
            let messages = vec![
                crate::llm::Message {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                crate::llm::Message {
                    role: "user".to_string(),
                    content: format!(
                        "Analyze tool effectiveness over the past 14 days:\n\n{}\n\n📊 Tool Usage Statistics:\n{}\n\nEvaluate:\n1. Which tools are most/least used\n2. Tool success rates and performance\n3. User interaction patterns\n4. Suggestions for improvement",
                        summaries_text, tool_stats
                    ),
                }
            ];
            
            let response = llm_client.generate(&messages, None).await
                .ok()
                .unwrap_or_else(|| {
                    warn!("Effectiveness analysis LLM error");
                    crate::llm::LlmResponse {
                        content: format!("Tool effectiveness analysis for past 14 days\n{}", tool_stats),
                        tool_calls: Vec::new(),
                    }
                });
            
            let findings = HashMap::from([
                ("type".to_string(), "tool_effectiveness".to_string()),
                ("summary".to_string(), response.content.clone()),
                ("period".to_string(), "14_days".to_string()),
                ("stats".to_string(), tool_stats),
            ]);
            memory.store_idle_analysis("tools", &findings, None).await?;
            info!("Tool effectiveness analysis completed: {}", response.content.chars().take(100).collect::<String>());
        }
        "reflection" => {
            // Monthly self-reflection and compaction
            let last_month = Local::now().date_naive()
                .with_day(1)
                .and_then(|d| d.pred_opt())
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate last month"))?;
            
            let year_month = format!("{}-{:02}", last_month.year(), last_month.month());
            
            // Get task context for the month
            let task_summary = task_manager.get_task_summary().await.ok();
            
            if let Err(e) = memory.compact_monthly(&year_month, llm_client, task_summary.as_deref()).await {
                error!("Failed to compact monthly summaries: {}", e);
            }

            let findings = HashMap::from([
                ("type".to_string(), "self_reflection".to_string()),
                ("summary".to_string(), "Monthly self-reflection completed".to_string()),
            ]);
            memory.store_idle_analysis("reflection", &findings, None).await?;
            info!("Monthly self-reflection completed");
        }
        "morning_briefing" | "daily_briefing" => {
            // Morning/daily briefing trigger
            info!("Generating daily briefing");
            
            // Get yesterday's summary
            let yesterday = Local::now().date_naive().pred_opt()
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate yesterday"))?;
            
            let yesterday_summary = memory.get_daily_summary(&yesterday.to_string()).await?;
            
            // Get today's tasks
            let task_summary = task_manager.get_active_tasks_summary().await.unwrap_or_default();
            
            let system_prompt = memory.build_effective_prompt().await?;
            let today = Local::now().format("%A, %B %d, %Y").to_string();
            
            let mut prompt = format!("Good morning! Today is {}.", today);
            if let Some(summary) = yesterday_summary {
                prompt.push_str("\n\nYesterday: ");
                prompt.push_str(&summary);
            }
            if !task_summary.is_empty() {
                prompt.push_str("\n\n📋 Today's Tasks:\n");
                prompt.push_str(&task_summary);
            }
            prompt.push_str("\n\nProvide a brief morning briefing and motivation for the day ahead.");
            
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
                .unwrap_or_else(|| {
                    warn!("Daily briefing LLM error");
                    crate::llm::LlmResponse {
                        content: "Good morning! Have a great day ahead!".to_string(),
                        tool_calls: Vec::new(),
                    }
                });
            
            // Store as a note for the user to see
            memory.create_note(&format!("Daily Briefing - {}: {}", today, response.content), &["briefing".to_string()]).await?;
            info!("Daily briefing generated and stored as note");
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
    task_manager: Arc<crate::tasks::TaskManager>,
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
            if let Err(e) = execute_idle_analysis(&trigger, &memory, &task_manager, &llm_client).await {
                error!("Failed to execute trigger {}: {}", trigger.name, e);
            }
        } else {
            // No triggers ready, check again in 60 seconds
            sleep(Duration::from_secs(60)).await;
        }
    }
}
