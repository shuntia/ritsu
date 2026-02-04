//! Task management
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]

use anyhow::Result;
use ritsu_common::{TaskPriority, TaskStatus};
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub struct TaskManager {
    conn: Arc<Mutex<Connection>>,
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
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
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
        let tags_json = serde_json::to_string(tags)?;
        let priority_str = match priority {
            TaskPriority::Low => "low",
            TaskPriority::Medium => "medium",
            TaskPriority::High => "high",
            TaskPriority::Urgent => "urgent",
        };

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (title, description, status, priority, tags, due_date, created_by) 
             VALUES (?1, ?2, 'pending', ?3, ?4, ?5, 'system')",
            (title, description, priority_str, tags_json, due_date),
        )?;

        let id = conn.last_insert_rowid();
        info!("Created task #{}: {}", id, title);
        Ok(id)
    }

    /// Update task status
    pub async fn update_task_status(&self, id: i64, status: &TaskStatus) -> Result<()> {
        let status_str = match status {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Cancelled => "cancelled",
        };

        let conn = self.conn.lock().await;
        let completed_at = if matches!(status, TaskStatus::Completed) {
            Some("datetime('now')")
        } else {
            None
        };

        if let Some(completed) = completed_at {
            conn.execute(
                &format!("UPDATE tasks SET status = ?1, completed_at = {} WHERE id = ?2", completed),
                (status_str, id),
            )?;
        } else {
            conn.execute(
                "UPDATE tasks SET status = ?1 WHERE id = ?2",
                (status_str, id),
            )?;
        }

        info!("Updated task #{} status to {}", id, status_str);
        Ok(())
    }

    /// Update task priority
    pub async fn update_task_priority(&self, id: i64, priority: &TaskPriority) -> Result<()> {
        let priority_str = match priority {
            TaskPriority::Low => "low",
            TaskPriority::Medium => "medium",
            TaskPriority::High => "high",
            TaskPriority::Urgent => "urgent",
        };

        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET priority = ?1 WHERE id = ?2",
            (priority_str, id),
        )?;

        info!("Updated task #{} priority to {}", id, priority_str);
        Ok(())
    }

    /// List tasks with optional filters
    pub async fn list_tasks(
        &self,
        status_filter: Option<&str>,
        priority_filter: Option<&str>,
    ) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;

        let mut query = String::from("SELECT id, title, description, status, priority, tags, due_date, created_by, created_at FROM tasks WHERE 1=1");

        if let Some(status) = status_filter {
            use std::fmt::Write;
            write!(query, " AND status = '{}'", status)?;
        }

        if let Some(priority) = priority_filter {
            use std::fmt::Write;
            write!(query, " AND priority = '{}'", priority)?;
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
    }
}
