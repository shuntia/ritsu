//! Task management

use anyhow::Result;
use ritsu_common::{TaskPriority, TaskStatus};
use rusqlite::Connection;
use tracing::info;

pub struct TaskManager<'a> {
    conn: &'a Connection,
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
}

#[allow(dead_code)]
impl<'a> TaskManager<'a> {
    #[must_use]
    pub const fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Create a new task
    pub fn create_task(
        &self,
        title: &str,
        description: Option<&str>,
        priority: TaskPriority,
        due_date: Option<&str>,
        tags: &[String],
        created_by: &str,
    ) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        let status = "pending";
        let priority_str = format!("{priority:?}").to_lowercase();

        self.conn.execute(
            "INSERT INTO tasks (title, description, status, priority, tags, due_date, created_by) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (title, description, status, priority_str, tags_json, due_date, created_by),
        )?;

        let id = self.conn.last_insert_rowid();
        info!("Created task {id}: '{title}'");
        Ok(id)
    }

    /// Update task status
    pub fn update_task_status(&self, id: i64, status: TaskStatus) -> Result<()> {
        let status_str = format!("{status:?}").to_lowercase();
        let completed_at = if status == TaskStatus::Completed {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };

        self.conn.execute(
            "UPDATE tasks SET status = ?1, completed_at = ?2, updated_at = CURRENT_TIMESTAMP 
             WHERE id = ?3",
            (status_str, completed_at, id),
        )?;

        info!("Updated task {id} status to {status:?}");
        Ok(())
    }

    /// Update task priority
    pub fn update_task_priority(&self, id: i64, priority: TaskPriority) -> Result<()> {
        let priority_str = format!("{priority:?}").to_lowercase();

        self.conn.execute(
            "UPDATE tasks SET priority = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            (priority_str, id),
        )?;

        info!("Updated task {id} priority to {priority:?}");
        Ok(())
    }

    /// List tasks with optional filters
    pub fn list_tasks(
        &self,
        status_filter: Option<TaskStatus>,
        priority_filter: Option<TaskPriority>,
    ) -> Result<Vec<Task>> {
        use std::fmt::Write;
        
        let mut query = "SELECT id, title, description, status, priority, tags, due_date, created_by 
                         FROM tasks WHERE 1=1".to_string();
        
        if let Some(status) = status_filter {
            let status_str = format!("{status:?}").to_lowercase();
            write!(query, " AND status = '{status_str}'").ok();
        }
        
        if let Some(priority) = priority_filter {
            let priority_str = format!("{priority:?}").to_lowercase();
            write!(query, " AND priority = '{priority_str}'").ok();
        }

        query.push_str(" ORDER BY created_at DESC");

        let mut stmt = self.conn.prepare(&query)?;
        let tasks = stmt.query_map([], |row| {
            let tags_json: String = row.get(5)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            
            let status_str: String = row.get(3)?;
            let status = match status_str.as_str() {
                "in_progress" => TaskStatus::InProgress,
                "completed" => TaskStatus::Completed,
                "cancelled" => TaskStatus::Cancelled,
                _ => TaskStatus::Pending,
            };

            let priority_str: String = row.get(4)?;
            let priority = match priority_str.as_str() {
                "low" => TaskPriority::Low,
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
            })
        })?;

        Ok(tasks.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}
