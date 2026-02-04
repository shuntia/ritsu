//! IPC server - Unix domain socket communication
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::format_push_string)]

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use crate::memory::MemoryManager;
use crate::tasks::TaskManager;
use crate::trigger::TriggerRegistry;

pub struct IpcServer {
    socket_path: String,
    memory: Arc<MemoryManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerRegistry>,
}

impl IpcServer {
    pub fn new(
        socket_path: String,
        memory: Arc<MemoryManager>,
        task_manager: Arc<TaskManager>,
        trigger_registry: Arc<TriggerRegistry>,
    ) -> Self {
        Self {
            socket_path,
            memory,
            task_manager,
            trigger_registry,
        }
    }

    pub async fn run(&self) -> Result<()> {
        // Remove existing socket if present
        if std::path::Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!("IPC server listening on {}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let memory = self.memory.clone();
                    let task_manager = self.task_manager.clone();
                    let trigger_registry = self.trigger_registry.clone();
                    
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, memory, task_manager, trigger_registry).await {
                            error!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

async fn handle_client(
    mut stream: UnixStream,
    memory: Arc<MemoryManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerRegistry>,
) -> Result<()> {
    loop {
        // Read message length (4 bytes)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Client disconnected
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10 * 1024 * 1024 {
            // 10MB limit
            warn!("Message too large: {} bytes", len);
            return Ok(());
        }

        // Read message data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        // Deserialize request
        let request: ClientRequest = bincode::deserialize(&data)?;
        info!("Received request: {:?}", request);

        // Handle request
        let response = handle_request(request, &memory, &task_manager, &trigger_registry).await;

        // Serialize response
        let response_data = bincode::serialize(&response)?;
        #[allow(clippy::cast_possible_truncation)]
        let response_len = (response_data.len() as u32).to_be_bytes();

        // Send response
        stream.write_all(&response_len).await?;
        stream.write_all(&response_data).await?;
        stream.flush().await?;
    }
}

#[allow(clippy::too_many_lines)]
async fn handle_request(
    request: ClientRequest,
    memory: &MemoryManager,
    task_manager: &TaskManager,
    trigger_registry: &TriggerRegistry,
) -> ServerResponse {
    match request {
        ClientRequest::Ping => ServerResponse::Pong,

        ClientRequest::SendMessage { content } => {
            match memory.store_conversation("user", &content).await {
                Ok(()) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to store message: {}", e),
                },
            }
        }

        ClientRequest::Shutdown => {
            info!("Shutdown requested");
            // In a real implementation, we'd signal the main loop to exit
            ServerResponse::Ok
        }

        ClientRequest::ListTriggers => {
            let triggers = trigger_registry.get_all_triggers().await;
            let trigger_infos = triggers
                .into_iter()
                .map(|t| {
                    let (trigger_type_str, schedule_str) = match &t.trigger_type {
                        crate::trigger::TriggerType::Time(time) => ("time".to_string(), time.clone()),
                        crate::trigger::TriggerType::Interval(secs) => ("interval".to_string(), secs.to_string()),
                        crate::trigger::TriggerType::Inactivity(secs) => ("inactivity".to_string(), secs.to_string()),
                        crate::trigger::TriggerType::Dynamic => ("dynamic".to_string(), String::new()),
                    };
                    ritsu_common::protocol::TriggerInfo {
                        id: t.id,
                        name: t.name,
                        trigger_type: trigger_type_str,
                        schedule: schedule_str,
                        enabled: t.enabled,
                    }
                })
                .collect();
            ServerResponse::Triggers {
                triggers: trigger_infos,
            }
        }

        ClientRequest::CreateTrigger {
            name,
            trigger_type,
            schedule,
        } => {
            match trigger_registry.create_trigger(&name, &trigger_type, &schedule).await {
                Ok(()) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to create trigger: {}", e),
                },
            }
        }

        ClientRequest::DeleteTrigger { name } => {
            match trigger_registry.delete_trigger(&name).await {
                Ok(()) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to delete trigger: {}", e),
                },
            }
        }

        ClientRequest::DisableTrigger { name } => {
            match trigger_registry.disable_trigger(&name).await {
                Ok(()) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to disable trigger: {}", e),
                },
            }
        }

        ClientRequest::ListTasks { filter } => {
            let (status, priority) = if let Some(f) = filter {
                (
                    f.status.as_ref().map(|s| match s {
                        ritsu_common::TaskStatus::Pending => "pending",
                        ritsu_common::TaskStatus::InProgress => "in_progress",
                        ritsu_common::TaskStatus::Completed => "completed",
                        ritsu_common::TaskStatus::Cancelled => "cancelled",
                    }),
                    f.priority.as_ref().map(|p| match p {
                        ritsu_common::TaskPriority::Low => "low",
                        ritsu_common::TaskPriority::Medium => "medium",
                        ritsu_common::TaskPriority::High => "high",
                        ritsu_common::TaskPriority::Urgent => "urgent",
                    }),
                )
            } else {
                (None, None)
            };

            match task_manager.list_tasks(status, priority).await {
                Ok(tasks) => {
                    let task_infos = tasks
                        .into_iter()
                        .map(|t| ritsu_common::protocol::TaskInfo {
                            id: t.id,
                            title: t.title,
                            description: t.description,
                            status: t.status,
                            priority: t.priority,
                            tags: t.tags,
                            due_date: t.due_date,
                        })
                        .collect();
                    ServerResponse::Tasks { tasks: task_infos }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to list tasks: {}", e),
                },
            }
        }

        ClientRequest::CreateTask {
            title,
            description,
            priority,
            tags,
            due_date,
        } => {
            match task_manager
                .create_task(&title, description.as_deref(), &priority, &tags, due_date.as_deref())
                .await
            {
                Ok(_id) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to create task: {}", e),
                },
            }
        }

        ClientRequest::UpdateTask { id, status, priority } => {
            if let Some(s) = status {
                match task_manager.update_task_status(id, &s).await {
                    Ok(()) => {}
                    Err(e) => {
                        return ServerResponse::Error {
                            message: format!("Failed to update task status: {}", e),
                        };
                    }
                }
            }
            
            if let Some(p) = priority {
                match task_manager.update_task_priority(id, &p).await {
                    Ok(()) => {}
                    Err(e) => {
                        return ServerResponse::Error {
                            message: format!("Failed to update task priority: {}", e),
                        };
                    }
                }
            }
            
            ServerResponse::Ok
        }

        ClientRequest::QueryMemory { query_type, date_range: _ } => {
            use ritsu_common::protocol::MemoryQueryType;
            
            // Default to 7 days for queries (date_range could be used for more specific filtering in future)
            let days = 7;
            
            let result = match query_type {
                MemoryQueryType::Recent => {
                    match memory.query_recent_conversations(days).await {
                        Ok(conversations) => {
                            if conversations.is_empty() {
                                "No recent conversations found.".to_string()
                            } else {
                                let mut output = format!("Recent conversations (last {days} days):\n\n");
                                for (date, role, content) in conversations {
                                    output.push_str(&format!("[{date}] {role}: {content}\n"));
                                }
                                output
                            }
                        }
                        Err(e) => format!("Error querying conversations: {e}"),
                    }
                }
                MemoryQueryType::Daily => {
                    match memory.query_daily_summaries(days).await {
                        Ok(summaries) => {
                            if summaries.is_empty() {
                                "No daily summaries found.".to_string()
                            } else {
                                let mut output = format!("Daily summaries (last {days} days):\n\n");
                                for (date, summary) in summaries {
                                    output.push_str(&format!("[{date}]\n{summary}\n\n"));
                                }
                                output
                            }
                        }
                        Err(e) => format!("Error querying daily summaries: {e}"),
                    }
                }
                MemoryQueryType::Monthly => {
                    match memory.query_monthly_summaries(12).await {
                        Ok(summaries) => {
                            if summaries.is_empty() {
                                "No monthly summaries found.".to_string()
                            } else {
                                let mut output = "Monthly summaries:\n\n".to_string();
                                for (year_month, summary, days_count) in summaries {
                                    output.push_str(&format!("[{year_month}] ({days_count} days)\n{summary}\n\n"));
                                }
                                output
                            }
                        }
                        Err(e) => format!("Error querying monthly summaries: {e}"),
                    }
                }
                MemoryQueryType::Notes => {
                    match memory.query_notes(50).await {
                        Ok(notes) => {
                            if notes.is_empty() {
                                "No notes found.".to_string()
                            } else {
                                let mut output = "Notes:\n\n".to_string();
                                for (id, content, _tags) in notes {
                                    output.push_str(&format!("#{id}: {content}\n\n"));
                                }
                                output
                            }
                        }
                        Err(e) => format!("Error querying notes: {e}"),
                    }
                }
            };
            
            ServerResponse::Memory { content: result }
        }
    }
}
