//! Task management
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]

use anyhow::Result;
use ritsu_common::{TaskPriority, TaskStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

pub struct TaskManager {
    conn: Arc<tokio_rusqlite::Connection>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub tags: Vec<String>,
    pub due_date: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

#[allow(dead_code)]
impl TaskManager {
    #[must_use]
    pub fn new(conn: Arc<tokio_rusqlite::Connection>) -> Self {
        Self { conn }
    }

    /// Create a new task
    pub async fn create_task(
        &self,
        title: &str,
        description: Option<&str>,
        priority: &TaskPriority,
        tags: &[String],
        due_date: Option<&str>,
    ) -> Result<i64> {
        let title = title.to_string();
        let title_for_log = title.clone();
        let description = description.map(|s| s.to_string());
        let tags = tags.to_vec();
        let due_date = due_date.map(|s| s.to_string());
        let priority = priority.clone();
        
        let id = self.conn.call(move |conn| -> rusqlite::Result<i64> {
            let tags_json = serde_json::to_string(&tags)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let priority_str = match priority {
                TaskPriority::Low => "low",
                TaskPriority::Medium => "medium",
                TaskPriority::High => "high",
                TaskPriority::Urgent => "urgent",
            };

            conn.execute(
                "INSERT INTO tasks (title, description, status, priority, tags, due_date, created_by) 
                 VALUES (?1, ?2, 'pending', ?3, ?4, ?5, 'system')",
                (&title, &description, priority_str, &tags_json, &due_date),
            )?;

            Ok(conn.last_insert_rowid())
        }).await?;
        
        info!("Created task #{}: {}", id, title_for_log);
        Ok(id)
    }

    /// Update task status
    pub async fn update_task_status(&self, id: i64, status: &TaskStatus) -> Result<()> {
        let status = status.clone();
        
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            let status_str = match status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Completed => "completed",
                TaskStatus::Cancelled => "cancelled",
            };

            let completed_at = matches!(status, TaskStatus::Completed);

            if completed_at {
                conn.execute(
                    "UPDATE tasks SET status = ?1, completed_at = datetime('now') WHERE id = ?2",
                    (status_str, &id),
                )?;
            } else {
                conn.execute(
                    "UPDATE tasks SET status = ?1 WHERE id = ?2",
                    (status_str, &id),
                )?;
            }

            info!("Updated task #{} status to {}", id, status_str);
            Ok(())
        }).await.map_err(|e| anyhow::anyhow!("DB error: {}", e))
    }

    /// Update task priority
    pub async fn update_task_priority(&self, id: i64, priority: &TaskPriority) -> Result<()> {
        let priority = priority.clone();
        
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            let priority_str = match priority {
                TaskPriority::Low => "low",
                TaskPriority::Medium => "medium",
                TaskPriority::High => "high",
                TaskPriority::Urgent => "urgent",
            };

            conn.execute(
                "UPDATE tasks SET priority = ?1 WHERE id = ?2",
                (priority_str, &id),
            )?;

            info!("Updated task #{} priority to {}", id, priority_str);
            Ok(())
        }).await.map_err(|e| anyhow::anyhow!("DB error: {}", e))
    }

    /// Delete a task
    pub async fn delete_task(&self, id: i64) -> Result<()> {
        self.conn.call(move |conn| -> rusqlite::Result<()> {
            let rows_affected = conn.execute(
                "DELETE FROM tasks WHERE id = ?1",
                [&id],
            )?;
            
            if rows_affected > 0 {
                info!("Deleted task #{}", id);
                Ok(())
            } else {
                Err(rusqlite::Error::QueryReturnedNoRows)
            }
        }).await.map_err(|e| anyhow::anyhow!("DB error: {}", e))
    }

    /// List tasks with optional filters
    pub async fn list_tasks(
        &self,
        status_filter: Option<&str>,
        priority_filter: Option<&str>,
    ) -> Result<Vec<Task>> {
        let status_filter = status_filter.map(|s| s.to_string());
        let priority_filter = priority_filter.map(|s| s.to_string());
        
        self.conn.call(move |conn| -> rusqlite::Result<Vec<Task>> {
            let mut query = String::from("SELECT id, title, description, status, priority, tags, due_date, created_by, created_at FROM tasks WHERE 1=1");

            if let Some(status) = &status_filter {
                query.push_str(&format!(" AND status = '{}'", status));
            }

            if let Some(priority) = &priority_filter {
                query.push_str(&format!(" AND priority = '{}'", priority));
            }

            query.push_str(" ORDER BY created_at DESC");

            let mut stmt = conn.prepare(&query)?;
            let tasks = stmt
                .query_map([], |row| {
                    let tags_json: String = row.get(5)?;
                    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

                    let status_str: String = row.get(3)?;
                    let status = match status_str.as_str() {
                        "pending" => TaskStatus::Pending,
                        "in_progress" => TaskStatus::InProgress,
                        "completed" => TaskStatus::Completed,
                        "cancelled" => TaskStatus::Cancelled,
                        _ => TaskStatus::Pending,
                    };

                    let priority_str: String = row.get(4)?;
                    let priority = match priority_str.as_str() {
                        "low" => TaskPriority::Low,
                        "medium" => TaskPriority::Medium,
                        "high" => TaskPriority::High,
                        "urgent" => TaskPriority::Urgent,
                        _ => TaskPriority::Medium,
                    };

                    Ok(Task {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        description: row.get(2)?,
                        status,
                        priority,
                        tags,
                        due_date: row.get(6)?,
                        created_by: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                })?
                .filter_map(Result::ok)
                .collect();

            Ok(tasks)
        }).await.map_err(Into::into)
    }

    /// Get task summary statistics for AI context
    pub async fn get_task_summary(&self) -> Result<String> {
        self.conn.call(|conn| -> rusqlite::Result<String> {
            // Count by status
            let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
            let status_counts: HashMap<String, i64> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(Result::ok)
                .collect();
            
            // Count by priority
            let mut stmt = conn.prepare("SELECT priority, COUNT(*) FROM tasks WHERE status != 'completed' AND status != 'cancelled' GROUP BY priority")?;
            let priority_counts: HashMap<String, i64> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(Result::ok)
                .collect();
            
            // Get overdue tasks
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM tasks WHERE status != 'completed' AND status != 'cancelled' AND due_date < date('now')")?;
            let overdue: i64 = stmt.query_row([], |row| row.get(0))?;
            
            // Get tasks due soon (within 3 days)
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM tasks WHERE status != 'completed' AND status != 'cancelled' AND due_date BETWEEN date('now') AND date('now', '+3 days')")?;
            let due_soon: i64 = stmt.query_row([], |row| row.get(0))?;
            
            // Build summary
            let pending = status_counts.get("pending").unwrap_or(&0);
            let in_progress = status_counts.get("in_progress").unwrap_or(&0);
            let completed = status_counts.get("completed").unwrap_or(&0);
            
            let high = priority_counts.get("high").unwrap_or(&0);
            let urgent = priority_counts.get("urgent").unwrap_or(&0);
            
            let mut summary = format!(
                "Task Status: {} pending, {} in progress, {} completed today",
                pending, in_progress, completed
            );
            
            if *urgent > 0 || *high > 0 {
                summary = format!("{summary}\nHigh Priority: {urgent} urgent, {high} high");
            }
            
            if overdue > 0 {
                summary = format!("{summary}\n⚠️  {overdue} tasks overdue");
            }
            
            if due_soon > 0 {
                summary = format!("{summary}\n📅 {due_soon} tasks due within 3 days");
            }
            
            Ok(summary)
        }).await.map_err(|e| anyhow::anyhow!("DB error: {}", e))
    }

    /// Get detailed list of active tasks for AI context
    pub async fn get_active_tasks_summary(&self) -> Result<String> {
        let tasks = self.list_tasks(None, None).await?;
        
        let active: Vec<_> = tasks.into_iter()
            .filter(|t| !matches!(t.status, TaskStatus::Completed | TaskStatus::Cancelled))
            .collect();
        
        if active.is_empty() {
            return Ok("No active tasks".to_string());
        }
        
        let summary = active.iter()
            .take(10) // Limit to top 10
            .map(|t| {
                let priority_emoji = match t.priority {
                    TaskPriority::Urgent => "🔴",
                    TaskPriority::High => "🟠",
                    TaskPriority::Medium => "🟡",
                    TaskPriority::Low => "🟢",
                };
                
                let status_str = match t.status {
                    TaskStatus::Pending => "PENDING",
                    TaskStatus::InProgress => "IN_PROGRESS",
                    TaskStatus::Completed => "COMPLETED",
                    TaskStatus::Cancelled => "CANCELLED",
                };
                
                let due_info = t.due_date.as_ref()
                    .map(|d| format!(" (due: {})", d))
                    .unwrap_or_default();
                
                format!("{} [{}] {}{}", priority_emoji, status_str, t.title, due_info)
            })
            .collect::<Vec<_>>()
            .join("\n");
        
        let total = active.len();
        if total > 10 {
            Ok(format!("{}\n... and {} more", summary, total - 10))
        } else {
            Ok(summary)
        }
    }
}
