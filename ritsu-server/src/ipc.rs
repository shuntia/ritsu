//! IPC server - Unix domain socket communication
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use crate::memory::MemoryManager;
use crate::tasks::TaskManager;

pub struct IpcServer {
    socket_path: String,
    memory: Arc<MemoryManager>,
    task_manager: Arc<TaskManager>,
}

impl IpcServer {
    pub fn new(
        socket_path: String,
        memory: Arc<MemoryManager>,
        task_manager: Arc<TaskManager>,
    ) -> Self {
        Self {
            socket_path,
            memory,
            task_manager,
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
                    
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, memory, task_manager).await {
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
        let response = handle_request(request, &memory, &task_manager).await;

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
            // TODO: Get triggers from TriggerRegistry
            ServerResponse::Triggers {
                triggers: Vec::new(),
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
            // TODO: Implement actual memory queries based on query_type
            let content = format!("Memory query result for {:?}", query_type);
            ServerResponse::Memory { content }
        }
    }
}
