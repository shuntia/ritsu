//! Task management commands

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, TaskFilter};
use crate::TaskCommands;

pub async fn handle(cmd: TaskCommands) -> Result<()> {
    match cmd {
        TaskCommands::List { status } => {
            let status_enum = status.as_deref().and_then(|s| match s {
                "pending" => Some(ritsu_common::TaskStatus::Pending),
                "in_progress" => Some(ritsu_common::TaskStatus::InProgress),
                "completed" => Some(ritsu_common::TaskStatus::Completed),
                "cancelled" => Some(ritsu_common::TaskStatus::Cancelled),
                _ => None,
            });
            list_tasks(status_enum).await
        }
        TaskCommands::Add { title, priority, due } => {
            let priority_enum = match priority.as_str() {
                "low" => ritsu_common::TaskPriority::Low,
                "high" => ritsu_common::TaskPriority::High,
                "urgent" => ritsu_common::TaskPriority::Urgent,
                _ => ritsu_common::TaskPriority::Medium,
            };
            create_task(&title, None, priority_enum, Vec::new(), due).await
        }
        TaskCommands::Update { id, status } => {
            let status_enum = match status.as_str() {
                "pending" => ritsu_common::TaskStatus::Pending,
                "in_progress" => ritsu_common::TaskStatus::InProgress,
                "cancelled" => ritsu_common::TaskStatus::Cancelled,
                _ => ritsu_common::TaskStatus::Completed,
            };
            update_task(id, Some(status_enum), None).await
        }
    }
}

async fn list_tasks(status: Option<ritsu_common::TaskStatus>) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ClientRequest::ListTasks {
            filter: Some(TaskFilter { status, priority: None, tags: None }),
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Tasks { tasks } => {
            if tasks.is_empty() {
                println!("No tasks found");
            } else {
                for task in tasks {
                    println!("#{}: {} [{:?}] ({:?})", task.id, task.title, task.status, task.priority);
                    if let Some(desc) = &task.description {
                        println!("  {}", desc);
                    }
                }
            }
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn create_task(
    title: &str,
    description: Option<String>,
    priority: ritsu_common::TaskPriority,
    tags: Vec<String>,
    due_date: Option<String>,
) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ClientRequest::CreateTask {
            title: title.to_string(),
            description,
            priority,
            tags,
            due_date,
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("✓ Task created");
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn update_task(
    id: i64,
    status: Option<ritsu_common::TaskStatus>,
    priority: Option<ritsu_common::TaskPriority>,
) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ClientRequest::UpdateTask { id, status, priority })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("✓ Task updated");
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}
