//! Trigger system - self-scheduling and idle analysis
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::future_not_send)]
#![allow(clippy::struct_field_names)]
#![allow(dead_code)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::uninlined_format_args)]

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, sleep_until, Duration, Instant};
use tokio_rusqlite::rusqlite;
use tracing::{error, info, warn};

use crate::llm::LlmClient;
use crate::memory::MemoryManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerType {
    /// Cron: cron expression for complex scheduling
    Cron(String),
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
    pub db_path: String,
    trigger_changed: Arc<tokio::sync::Notify>,
}

impl TriggerRegistry {
    pub fn new(db_path: String) -> Self {
        Self {
            triggers: Arc::new(RwLock::new(Vec::new())),
            db_path,
            trigger_changed: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub async fn load_from_database(&self) -> Result<()> {
        let db_path = self.db_path.clone();

        let loaded_triggers = crate::database::Database::execute_blocking(db_path, |conn| {
            let mut stmt = conn.prepare("SELECT id, name, trigger_type, schedule, enabled, created_by, metadata FROM triggers WHERE enabled = 1")?;

            let triggers_iter = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let trigger_type_str: String = row.get(2)?;
                let schedule: String = row.get(3)?;
                let enabled: bool = row.get(4)?;
                let created_by: String = row.get(5)?;
                let metadata_json: String = row.get(6)?;

                // Map legacy types to cron where possible
                let cron_expr = match trigger_type_str.as_str() {
                    "cron" => schedule.clone(),
                    "time" => {
                        // Convert HH:MM to a five-field cron "m H * * *" (minute hour day month day-of-week).
                        // Note: the code accepts both five-field and six-field cron expressions; canonical one-shot cron strings include a leading seconds field ("s m H D M *").
                        let parts: Vec<&str> = schedule.split(':').collect();
                        if parts.len() == 2 {
                            let hour = parts[0].parse::<u32>().unwrap_or(0);
                            let minute = parts[1].parse::<u32>().unwrap_or(0);
                            // Use six-field cron expressions with leading seconds field for compatibility with the cron crate
                            format!("0 {minute} {hour} * * *")
                        } else {
                            schedule.clone()
                        }
                    }
                    "interval" => {
                        // Convert interval seconds to minute-based cron where possible
                        if let Ok(secs) = schedule.parse::<u64>() {
                            if secs >= 60 && secs % 60 == 0 {
                                let mins = secs / 60;
                                // Use six-field cron expressions with leading seconds
                                format!("0 */{} * * * *", mins)
                            } else {
                                // Fallback to every minute (at second 0)
                                "0 */1 * * * *".to_string()
                            }
                        } else {
                            schedule.clone()
                        }
                    }
                    "dynamic" => {
                        // Schedule near-future one-shot via cron (next minute)
                        let now = Local::now();
                        let next = now + chrono::Duration::minutes(1);
                        // Use six-field cron with leading seconds for dynamic one-shot triggers
                        format!("0 {} {} * * *", next.minute(), next.hour())
                    }
                    // inactivity and unknown types - fallback to the stored schedule as cron
                    _ => schedule.clone(),
                };

                let mut metadata: HashMap<String, String> = serde_json::from_str(&metadata_json)
                    .unwrap_or_default();

                // Preserve dynamic semantics as one_shot for legacy dynamic triggers
                if trigger_type_str.as_str() == "dynamic" {
                    metadata.insert("one_shot".to_string(), "true".to_string());
                }

                Ok(Trigger {
                    id,
                    name,
                    trigger_type: TriggerType::Cron(cron_expr),
                    enabled,
                    created_by,
                    metadata,
                })
            })?;

            Ok(triggers_iter.filter_map(Result::ok).collect::<Vec<_>>())
        }).await?;

        let mut triggers = self.triggers.write().await;
        triggers.clear();
        triggers.extend(loaded_triggers);

        info!("Loaded {} triggers from database", triggers.len());
        Ok(())
    }

    pub async fn register_builtin_triggers(&self) -> Result<()> {
        let db_path = self.db_path.clone();

        crate::database::Database::execute_blocking(db_path, |conn| {
            // Daily conversation compaction at 2:00 AM
            Self::insert_trigger_if_not_exists(
                conn,
                "daily_compaction",
                "time",
                "02:00",
                "system",
                r#"{"analysis_type":"conversation"}"#,
            )?;

            // Weekly pattern recognition on Sunday at 3:00 AM
            Self::insert_trigger_if_not_exists(
                conn,
                "weekly_pattern",
                "time",
                "03:00",
                "system",
                r#"{"analysis_type":"pattern","day":"sunday"}"#,
            )?;

            // Monthly self-reflection on last day at 5:00 AM
            Self::insert_trigger_if_not_exists(
                conn,
                "monthly_reflection",
                "time",
                "05:00",
                "system",
                r#"{"analysis_type":"reflection","day":"last"}"#,
            )?;

            Ok(())
        })
        .await?;

        self.load_from_database().await?;
        Ok(())
    }

    fn insert_trigger_if_not_exists(
        conn: &rusqlite::Connection,
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

    pub async fn add_dynamic_trigger(
        &self,
        name: String,
        description: String,
        tags: Vec<String>,
        metadata: HashMap<String, String>,
    ) -> Result<()> {
        let db_path = self.db_path.clone();

        // Build full metadata with tags and description and mark as one_shot
        let mut full_metadata = metadata;
        full_metadata.insert("tags".to_string(), tags.join(","));
        full_metadata.insert("description".to_string(), description);
        full_metadata.insert("one_shot".to_string(), "true".to_string());
        let metadata_json = serde_json::to_string(&full_metadata)?;

        // Schedule to the next minute via cron expression (minute hour * * *)
        let now = Local::now();
        let next = now + chrono::Duration::minutes(1);
        let cron_schedule = format!("{} {} * * *", next.minute(), next.hour());

        crate::database::Database::execute_blocking(db_path, move |conn| {
            conn.execute(
                "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
                 VALUES (?1, 'cron', ?2, 1, 'ai', ?3, datetime('now'))",
                (&name, &cron_schedule, metadata_json),
            )?;
            Ok(())
        }).await?;

        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    /// Create a one-shot trigger scheduled at an exact ISO8601 datetime.
    /// Validates that the corresponding cron expression will trigger at the exact requested minute.
    pub async fn create_one_shot_trigger(
        &self,
        name: &str,
        datetime_iso: &str,
        tags: Vec<String>,
        description: Option<&str>,
    ) -> Result<()> {
        let db_path = self.db_path.clone();
        let name_owned = name.to_string();

        // Parse ISO8601 datetime string
        let parsed = chrono::DateTime::parse_from_rfc3339(datetime_iso)
            .map_err(|e| anyhow::anyhow!("Invalid datetime format: {}", e))?;
        let dt_utc = parsed.with_timezone(&Utc);

        // Ensure datetime is in the future
        let now_utc = Utc::now();
        if dt_utc <= now_utc {
            anyhow::bail!("Provided datetime must be in the future");
        }

        // Build cron expression including seconds, minute, hour, day and month for exact date-match: "0 M H D M *"
        let cron_expr = format!(
            "0 {} {} {} {} *",
            dt_utc.minute(),
            dt_utc.hour(),
            dt_utc.day(),
            dt_utc.month()
        );

        // Validate that the cron schedule will fire exactly at the requested minute
        // Use tolerant cron parsing (accept 5-field or 6-field cron expressions)
        let schedule = parse_cron_schedule(&cron_expr)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse cron expression: {}", cron_expr))?;

        // Compute the next occurrence from 'now' and compare to desired datetime truncated to minute
        let next_occurrence = schedule
            .after(&now_utc)
            .next()
            .ok_or_else(|| anyhow::anyhow!("Cron schedule produced no next occurrence"))?;

        // Truncate desired datetime to minute precision
        let desired_minute = Utc
            .ymd(dt_utc.year(), dt_utc.month(), dt_utc.day())
            .and_hms(dt_utc.hour(), dt_utc.minute(), 0);

        if next_occurrence != desired_minute {
            anyhow::bail!("Cron expression does not match the requested datetime (next cron occurrence: {} vs requested: {})", next_occurrence, desired_minute);
        }

        // Build metadata
        let mut metadata_map: HashMap<String, String> = HashMap::new();
        metadata_map.insert("one_shot".to_string(), "true".to_string());
        metadata_map.insert("one_shot_time".to_string(), dt_utc.to_rfc3339());
        if let Some(desc) = description {
            metadata_map.insert("description".to_string(), desc.to_string());
        }
        metadata_map.insert("tags".to_string(), tags.join(","));

        let metadata_json = serde_json::to_string(&metadata_map)?;

        // Insert into DB as cron trigger
        crate::database::Database::execute_blocking(db_path, move |conn| {
            conn.execute(
                "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
                 VALUES (?1, 'cron', ?2, 1, 'user', ?3, datetime('now'))",
                (&name_owned, &cron_expr, metadata_json),
            )?;
            Ok(())
        }).await?;

        // Reload triggers and notify
        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    pub async fn remove_trigger(&self, name: &str) -> Result<()> {
        let db_path = self.db_path.clone();
        let name = name.to_string();

        crate::database::Database::execute_blocking(db_path, move |conn| {
            conn.execute("DELETE FROM triggers WHERE name = ?1", [&name])?;
            Ok(())
        })
        .await?;

        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    pub fn notify_changed(&self) {
        self.trigger_changed.notify_one();
    }

    pub fn notifier(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.trigger_changed)
    }

    pub async fn create_trigger(
        &self,
        name: &str,
        trigger_type: &str,
        schedule: &str,
        tag: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let db_path = self.db_path.clone();
        let name = name.to_string();
        let trigger_type = trigger_type.to_string();
        let schedule = schedule.to_string();

        // Build metadata JSON with tag and description
        let mut metadata = HashMap::new();
        if let Some(t) = tag {
            metadata.insert("tag".to_string(), t.to_string());
        }
        if let Some(d) = description {
            metadata.insert("description".to_string(), d.to_string());
        }
        let metadata_json = serde_json::to_string(&metadata)?;

        crate::database::Database::execute_blocking(db_path, move |conn| {
            conn.execute(
                "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
                 VALUES (?1, ?2, ?3, 1, 'user', ?4, datetime('now'))",
                (&name, &trigger_type, &schedule, &metadata_json),
            )?;
            Ok(())
        }).await?;

        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    /// Update an existing trigger's fields (name, type, schedule, enabled, metadata fields)
    pub async fn update_trigger(&self, name: &str, updates: HashMap<String, String>) -> Result<()> {
        let db_path = self.db_path.clone();
        let name = name.to_string();
        let updates_owned = updates.clone();

        crate::database::Database::execute_blocking(db_path, move |conn| {
            // Fetch existing trigger row
            let mut stmt = conn.prepare(
                "SELECT id, name, trigger_type, schedule, enabled, metadata FROM triggers WHERE name = ?1",
            )?;

            let (id, old_name, old_type, old_schedule, old_enabled_int, old_metadata_json) = stmt.query_row([&name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?;

            // Parse existing metadata JSON
            let mut metadata_map: HashMap<String, String> = match old_metadata_json {
                Some(j) => serde_json::from_str(&j).unwrap_or_default(),
                None => HashMap::new(),
            };

            // Apply updates
            let new_name = updates_owned.get("new_name").cloned().unwrap_or(old_name);
            let new_type = updates_owned.get("type").cloned().unwrap_or(old_type);
            let new_schedule = updates_owned.get("schedule").cloned().unwrap_or(old_schedule);
            let new_enabled = updates_owned.get("enabled").map(|s| matches!(s.as_str(), "true" | "1")).unwrap_or(old_enabled_int != 0);

            if let Some(tag) = updates_owned.get("tag") {
                metadata_map.insert("tag".to_string(), tag.clone());
            }
            if let Some(description) = updates_owned.get("description") {
                metadata_map.insert("description".to_string(), description.clone());
            }

            // Merge any other provided metadata fields
            for (k, v) in updates_owned.iter() {
                if ["new_name", "type", "schedule", "enabled", "tag", "description"].contains(&k.as_str()) {
                    continue;
                }
                metadata_map.insert(k.clone(), v.clone());
            }

            let metadata_json = serde_json::to_string(&metadata_map)?;
            let enabled_int = if new_enabled { 1 } else { 0 };

            conn.execute(
                "UPDATE triggers SET name = ?1, trigger_type = ?2, schedule = ?3, enabled = ?4, metadata = ?5 WHERE id = ?6",
                (&new_name, &new_type, &new_schedule, &enabled_int, &metadata_json, &id),
            )?;

            Ok(())
        }).await?;

        // Reload and notify
        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    pub async fn delete_trigger(&self, name: &str) -> Result<()> {
        let db_path = self.db_path.clone();
        let name = name.to_string();
        let name_for_error = name.clone();

        let rows = crate::database::Database::execute_blocking(db_path, move |conn| {
            Ok(conn.execute("DELETE FROM triggers WHERE name = ?1", [&name])?)
        })
        .await?;

        if rows == 0 {
            anyhow::bail!("Trigger not found: {}", name_for_error);
        }

        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    pub async fn disable_trigger(&self, name: &str) -> Result<()> {
        let db_path = self.db_path.clone();
        let name = name.to_string();
        let name_for_error = name.clone();

        let rows = crate::database::Database::execute_blocking(db_path, move |conn| {
            Ok(conn.execute("UPDATE triggers SET enabled = 0 WHERE name = ?1", [&name])?)
        })
        .await?;

        if rows == 0 {
            anyhow::bail!("Trigger not found: {}", name_for_error);
        }

        self.load_from_database().await?;
        self.trigger_changed.notify_one();
        Ok(())
    }

    /// Cancel a specific cron occurrence by trigger name and ISO8601 datetime.
    /// Stores the cancelled occurrence in the cron_exceptions table and notifies the trigger loop.
    pub async fn cancel_cron(&self, name: &str, occurrence_iso: &str) -> Result<()> {
        use cron::Schedule;
        use std::str::FromStr;

        // Parse occurrence datetime (accept any offset and convert to UTC)
        let parsed = chrono::DateTime::parse_from_rfc3339(occurrence_iso)
            .map_err(|e| anyhow::anyhow!("Invalid datetime format: {}", e))?;
        let occ_utc = parsed.with_timezone(&Utc);

        // Truncate to minute precision
        let desired_minute = Utc
            .ymd(occ_utc.year(), occ_utc.month(), occ_utc.day())
            .and_hms(occ_utc.hour(), occ_utc.minute(), 0);

        // Find trigger
        let triggers = self.triggers.read().await;
        let trigger = triggers
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| anyhow::anyhow!("Trigger not found: {}", name))?;

        // Only cron triggers supported
        let cron_expr = match &trigger.trigger_type {
            TriggerType::Cron(s) => s.clone(),
        };

        // Validate that cron produces the requested occurrence (tolerant parse)
        let schedule = parse_cron_schedule(&cron_expr)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse cron expression: {}", cron_expr))?;

        let before = desired_minute - chrono::Duration::seconds(1);
        let next_occ = schedule.after(&before).next().ok_or_else(|| {
            anyhow::anyhow!("Cron schedule produced no occurrence near requested time")
        })?;

        let next_min = Utc
            .ymd(next_occ.year(), next_occ.month(), next_occ.day())
            .and_hms(next_occ.hour(), next_occ.minute(), 0);

        if next_min != desired_minute {
            anyhow::bail!("Cron schedule does not trigger at the requested time (next cron occurrence: {} vs requested: {})", next_min, desired_minute);
        }

        // Check if already cancelled
        let trig_name = name.to_string();
        let occ_str = desired_minute.to_rfc3339();
        let db_path = self.db_path.clone();
        let occ_str_for_check = occ_str.clone();

        let already = crate::database::Database::execute_blocking(db_path.clone(), move |conn| {
            let mut stmt = conn.prepare(
                "SELECT COUNT(1) FROM cron_exceptions WHERE trigger_name = ?1 AND occurrence = ?2",
            )?;
            let count: i64 = stmt.query_row((&trig_name, &occ_str_for_check), |r| r.get(0))?;
            Ok(count > 0)
        })
        .await?;

        if already {
            anyhow::bail!("Occurrence already cancelled: {} at {}", name, occ_str);
        }

        // Insert cancellation
        let trig_name2 = name.to_string();
        let occ_str2 = occ_str.clone();
        crate::database::Database::execute_blocking(db_path, move |conn| {
            conn.execute(
                "INSERT INTO cron_exceptions (trigger_name, occurrence) VALUES (?1, ?2)",
                (&trig_name2, &occ_str2),
            )?;
            Ok(())
        })
        .await?;

        // Notify trigger loop to reschedule
        self.trigger_changed.notify_one();
        Ok(())
    }
}

impl std::fmt::Display for Trigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("{} (id={})", self.name, self.id));
        parts.push(if self.enabled {
            "enabled".to_string()
        } else {
            "disabled".to_string()
        });
        parts.push(format!("created_by={}", self.created_by));

        if let Some(desc) = self.metadata.get("description") {
            parts.push(format!("desc={}", desc));
        }

        match &self.trigger_type {
            TriggerType::Cron(expr) => {
                parts.push(format!("cron=\"{}\"", expr));

                // If this trigger was stored as a one-shot, prefer showing the explicit time
                if let Some(one_shot_time) = self.metadata.get("one_shot_time") {
                    parts.push(format!("one_shot_time={}", one_shot_time));
                } else {
                    // Attempt to compute next occurrence for human-friendly display
                    if let Some(schedule) = parse_cron_schedule(expr) {
                        if let Some(next) = schedule.after(&chrono::Utc::now()).next() {
                            let next_local = next.with_timezone(&chrono::Local);
                            parts.push(format!(
                                "next={}",
                                next_local.format("%Y-%m-%d %H:%M:%S %:z")
                            ));
                        }
                    }
                }
            }
        }

        write!(f, "{}", parts.join(" | "))
    }
}

/// Try parsing a cron expression, accepting either five-field (no seconds) or six-field (with seconds) formats.
fn parse_cron_schedule(expr: &str) -> Option<cron::Schedule> {
    use cron::Schedule;
    use std::str::FromStr;

    // Try directly, then try prepending a leading seconds field ("0 ")
    Schedule::from_str(expr).or_else(|_| Schedule::from_str(&format!("0 {}", expr))).ok()
}

/// Calculate next trigger time for cron-based triggers
fn calculate_next_cron_time(cron_expr: &str) -> Option<Instant> {
    let schedule = parse_cron_schedule(cron_expr)?;
    let now = Utc::now();
    let next = schedule.after(&now).next()?;

    let duration = (next - now).to_std().ok()?;
    Some(Instant::now() + duration)
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

    // If a specific day filter is present in metadata, honor it
    if let Some(day_str) = trigger.metadata.get("day") {
        return match day_str.as_str() {
            "sunday" => now.weekday() == chrono::Weekday::Sun,
            "monday" => now.weekday() == chrono::Weekday::Mon,
            "tuesday" => now.weekday() == chrono::Weekday::Tue,
            "wednesday" => now.weekday() == chrono::Weekday::Wed,
            "thursday" => now.weekday() == chrono::Weekday::Thu,
            "friday" => now.weekday() == chrono::Weekday::Fri,
            "saturday" => now.weekday() == chrono::Weekday::Sat,
            "last" => {
                let tomorrow = now.date_naive().succ_opt();
                tomorrow.is_some_and(|t| t.month() != now.month())
            }
            _ => true,
        };
    }

    // If days numeric list provided (e.g., "1,15" for month days), check against day of month
    if let Some(days_str) = trigger.metadata.get("days") {
        return days_str
            .split(',')
            .any(|d| d.trim().parse::<u32>() == Ok(now.day()));
    }

    true
}

/// Execute idle analysis based on trigger metadata
#[allow(clippy::too_many_lines)]
pub async fn execute_idle_analysis(
    trigger: &Trigger,
    memory: &MemoryManager,
    task_manager: &crate::tasks::TaskManager,
    llm_client: &LlmClient,
) -> Result<()> {
    let analysis_type = trigger
        .metadata
        .get("analysis_type")
        .map_or("conversation", String::as_str);

    info!(
        "Executing idle analysis: {} ({})",
        trigger.name, analysis_type
    );

    match analysis_type {
        "conversation" => {
            // Daily compaction: aggregate conversations into summary
            let yesterday = Local::now()
                .date_naive()
                .pred_opt()
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate yesterday"))?;

            // Get task context
            let task_context = task_manager.get_task_summary().await.ok();

            // Load per-trigger prompt override if present
            let per_trigger_prompt = memory
                .load_trigger_prompt(&trigger.name)
                .await
                .ok()
                .flatten();

            // Use compact-specific prompt for daily compaction, allowing override
            if let Err(e) = memory
                .compact_daily(
                    &yesterday,
                    llm_client,
                    task_context.as_deref(),
                    per_trigger_prompt.as_deref(),
                )
                .await
            {
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

            let summaries_text = past_summaries
                .iter()
                .map(|(date, summary)| format!("{date}: {summary}"))
                .collect::<Vec<_>>()
                .join("\n\n");

            let prompt_text = format!(
                "Analyze patterns from the past week's activity:\n\n{}\n\nIdentify:\n1. Recurring themes and topics\n2. Time-based patterns\n3. User preferences and habits\n4. Areas of focus",
                summaries_text
            );

            let (mut system_prompt, messages) =
                match crate::prompt::PromptBuilder::build_background(
                    &memory,
                    task_manager,
                    Some("pattern"),
                    Some(&prompt_text),
                )
                .await
                {
                    Ok((sp, msgs)) => (sp, msgs),
                    Err(e) => {
                        warn!(
                            "Failed to build background prompt for pattern analysis: {}",
                            e
                        );
                        (
                            None,
                            vec![crate::llm::Message {
                                role: "user".to_string(),
                                content: prompt_text.clone(),
                            }],
                        )
                    }
                };

            // Override with per-trigger prompt file if present
            if let Ok(Some(per)) = memory.load_trigger_prompt(&trigger.name).await {
                system_prompt = Some(per);
            }

            let response = llm_client
                .generate(&messages, system_prompt.as_deref())
                .await
                .ok()
                .unwrap_or_else(|| {
                    warn!("Pattern analysis LLM error");
                    crate::llm::LlmResponse {
                        content: format!(
                            "Pattern analysis for past 7 days ({} summaries)",
                            past_summaries.len()
                        ),
                        tool_calls: Vec::new(),
                    }
                });

            let findings = HashMap::from([
                ("type".to_string(), "pattern_recognition".to_string()),
                ("summary".to_string(), response.content.clone()),
                ("period".to_string(), "7_days".to_string()),
            ]);
            memory
                .store_idle_analysis("pattern", &findings, None)
                .await?;
            info!(
                "Weekly pattern recognition completed: {}",
                response.content.chars().take(100).collect::<String>()
            );
        }
        "reflection" => {
            // Monthly self-reflection and compaction
            let last_month = Local::now()
                .date_naive()
                .with_day(1)
                .and_then(|d| d.pred_opt())
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate last month"))?;

            let year_month = format!("{}-{:02}", last_month.year(), last_month.month());

            // Get task context for the month
            let task_summary = task_manager.get_task_summary().await.ok();

            let per_trigger_prompt = memory
                .load_trigger_prompt(&trigger.name)
                .await
                .ok()
                .flatten();
            if let Err(e) = memory
                .compact_monthly(
                    &year_month,
                    llm_client,
                    task_summary.as_deref(),
                    per_trigger_prompt.as_deref(),
                )
                .await
            {
                error!("Failed to compact monthly summaries: {}", e);
            }

            let findings = HashMap::from([
                ("type".to_string(), "self_reflection".to_string()),
                (
                    "summary".to_string(),
                    "Monthly self-reflection completed".to_string(),
                ),
            ]);
            memory
                .store_idle_analysis("reflection", &findings, None)
                .await?;
            info!("Monthly self-reflection completed");
        }
        "morning_briefing" | "daily_briefing" => {
            // Morning/daily briefing trigger
            info!("Generating daily briefing");

            // Get yesterday's summary
            let yesterday = Local::now()
                .date_naive()
                .pred_opt()
                .ok_or_else(|| anyhow::anyhow!("Failed to calculate yesterday"))?;

            let yesterday_summary = memory.get_daily_summary(&yesterday.to_string()).await?;

            let today = Local::now().format("%A, %B %d, %Y").to_string();

            let mut prompt_text = format!("Good morning! Today is {}.", today);
            if let Some(summary) = yesterday_summary {
                prompt_text.push_str("\n\nYesterday: ");
                prompt_text.push_str(&summary);
            }
            prompt_text
                .push_str("\n\nProvide a brief morning briefing and motivation for the day ahead.");

            let (mut system_prompt, messages) =
                match crate::prompt::PromptBuilder::build_background(
                    &memory,
                    task_manager,
                    Some("morning_briefing"),
                    Some(&prompt_text),
                )
                .await
                {
                    Ok((sp, msgs)) => (sp, msgs),
                    Err(e) => {
                        warn!("Failed to build background prompt for briefing: {}", e);
                        (
                            None,
                            vec![crate::llm::Message {
                                role: "user".to_string(),
                                content: prompt_text.clone(),
                            }],
                        )
                    }
                };

            if let Ok(Some(per)) = memory.load_trigger_prompt(&trigger.name).await {
                system_prompt = Some(per);
            }

            let response = llm_client
                .generate(&messages, system_prompt.as_deref())
                .await
                .ok()
                .unwrap_or_else(|| {
                    warn!("Daily briefing LLM error");
                    crate::llm::LlmResponse {
                        content: "Good morning! Have a great day ahead!".to_string(),
                        tool_calls: Vec::new(),
                    }
                });

            // Store as a note for the user to see
            memory
                .create_note(
                    &format!("Daily Briefing - {}: {}", today, response.content),
                    &["briefing".to_string()],
                )
                .await?;
            info!("Daily briefing generated and stored as note");
        }
        "custom" | "reminder" => {
            // Custom AI-created trigger - send notification/note and start LLM with the note as instructions
            info!("Executing custom trigger: {}", trigger.name);

            let note = trigger
                .metadata
                .get("note")
                .map_or_else(|| format!("Reminder: {}", trigger.name), String::clone);

            let _urgency = trigger
                .metadata
                .get("urgency")
                .map_or("normal", String::as_str);

            let open_chat = trigger
                .metadata
                .get("open_chat")
                .is_some_and(|v| v == "true");

            // Log the intended action
            info!(
                "Custom trigger would notify: {} (open_chat: {})",
                note, open_chat
            );

            // Store as note so user can see it later
            let note_id = memory
                .create_note(
                    &format!("Trigger '{}': {}", trigger.name, note),
                    &["trigger".to_string(), "reminder".to_string()],
                )
                .await?;
            info!("Stored trigger note id: {}", note_id);

            // Build a background system prompt for the LLM and invoke it with the note as instructions.
            let (mut system_prompt, messages) =
                match crate::prompt::PromptBuilder::build_background(
                    &memory,
                    task_manager,
                    Some("background"),
                    Some(&note),
                )
                .await
                {
                    Ok((sp, msgs)) => (sp, msgs),
                    Err(e) => {
                        warn!(
                            "Failed to build background prompt for trigger {}: {}",
                            trigger.name, e
                        );
                        (
                            None,
                            vec![crate::llm::Message {
                                role: "user".to_string(),
                                content: note.clone(),
                            }],
                        )
                    }
                };

            if let Ok(Some(per)) = memory.load_trigger_prompt(&trigger.name).await {
                system_prompt = Some(per);
            }

            info!(
                "Invoking LLM for trigger '{}' with note instructions (may execute tools)",
                trigger.name
            );

            match llm_client
                .generate_with_tool_execution(&messages, system_prompt.as_deref(), 3)
                .await
            {
                Ok(resp) => {
                    info!(
                        "LLM responded for trigger '{}', content length {}",
                        trigger.name,
                        resp.content.len()
                    );
                    if !resp.content.is_empty() {
                        // Store LLM response for user visibility
                        let _ = memory
                            .create_note(
                                &format!(
                                    "Trigger '{}' LLM response: {}",
                                    trigger.name, resp.content
                                ),
                                &["trigger_response".to_string()],
                            )
                            .await;
                    }
                }
                Err(e) => {
                    warn!("LLM execution for trigger '{}' failed: {}", trigger.name, e);
                }
            }
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
    state: Arc<crate::state::ServerState>,
) -> Result<()> {
    info!("Starting trigger loop");
    let notifier = registry.notifier();

    loop {
        let triggers = registry.get_all_triggers().await;

        if triggers.is_empty() {
            info!("No triggers registered, waiting for changes or 60 seconds");
            tokio::select! {
                () = sleep(Duration::from_secs(60)) => {},
                () = notifier.notified() => {
                    info!("Trigger registry changed, reloading");
                }
            }
            continue;
        }

        let mut next_trigger: Option<(Instant, Trigger)> = None;

        for trigger in triggers {
            if !trigger.enabled {
                continue;
            }

            match &trigger.trigger_type {
                TriggerType::Cron(cron_expr) => {
                    if !should_run_today(&trigger) {
                        continue;
                    }
                    if let Some(instant) = calculate_next_cron_time(cron_expr) {
                        // Before considering this trigger, ensure the next occurrence isn't cancelled
                        let next_dt = {
                            // compute the next occurrence time in UTC minute precision
                            if let Some(schedule) = parse_cron_schedule(cron_expr) {
                                if let Some(next) = schedule.after(&Utc::now()).next() {
                                    let next_min = Utc
                                        .ymd(next.year(), next.month(), next.day())
                                        .and_hms(next.hour(), next.minute(), 0);
                                    Some(next_min)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        let mut is_cancelled = false;
                        if let Some(next_min) = next_dt {
                            // check cron_exceptions table for this trigger and occurrence
                            let trig_name = trigger.name.clone();
                            let occ_str = next_min.to_rfc3339();
                            // perform DB check (synchronously via execute_blocking)
                            let db_path_clone = registry.db_path.clone();
                            let trig_name_check = trig_name.clone();
                            let occ_check = occ_str.clone();
                            let cancelled = crate::database::Database::execute_blocking(db_path_clone, move |conn| {
                                let mut stmt = conn.prepare("SELECT COUNT(1) FROM cron_exceptions WHERE trigger_name = ?1 AND occurrence = ?2")?;
                                let count: i64 = stmt.query_row((&trig_name_check, &occ_check), |r| r.get(0))?;
                                Ok(count > 0)
                            }).await.unwrap_or(false);
                            is_cancelled = cancelled;
                        }

                        if is_cancelled {
                            // Skip this occurrence and look for the next one (we'll let reschedule logic handle future runs)
                            continue;
                        }

                        if next_trigger.as_ref().is_none_or(|(i, _)| instant < *i) {
                            next_trigger = Some((instant, trigger));
                        }
                    } else {
                        warn!(
                            "Invalid cron expression for trigger {}: {}",
                            trigger.name, cron_expr
                        );
                    }
                }
            }
        }

        if let Some((instant, trigger)) = next_trigger {
            // Calculate human-readable time from Instant
            let now = std::time::SystemTime::now();
            let duration_until = instant.duration_since(tokio::time::Instant::now());
            let trigger_time = now + duration_until;

            let datetime: chrono::DateTime<chrono::Local> = trigger_time.into();
            let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");

            info!(
                "Next trigger: {} at {} (in {:.1}s)",
                trigger.name,
                formatted_time,
                duration_until.as_secs_f64()
            );

            // Wait for trigger time or registry change
            tokio::select! {
                () = sleep_until(instant) => {
                    // Trigger time reached - execute it
                    info!("Executing trigger: {}", trigger.name);
                    if let Err(e) = execute_idle_analysis(&trigger, &memory, &task_manager, &llm_client).await {
                        error!("Failed to execute trigger {}: {}", trigger.name, e);
                    }

                    // Remove one-shot triggers after execution if requested
                    if trigger.metadata.get("one_shot").map(|v| v == "true").unwrap_or(false) {
                        if let Err(e) = registry.remove_trigger(&trigger.name).await {
                            error!("Failed to remove one-shot trigger {}: {}", trigger.name, e);
                        }
                    }
                }
                () = notifier.notified() => {
                    // Trigger registry changed - reschedule
                    info!("Trigger registry changed, rescheduling");
                }
            }
        } else {
            // No triggers ready, wait for registry change or check again in 60 seconds
            tokio::select! {
                () = sleep(Duration::from_secs(60)) => {},
                () = notifier.notified() => {
                    info!("Trigger registry changed, rescheduling");
                }
            }
        }
    }
}
