//! IPC server - Unix domain socket communication
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::format_push_string)]

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientRequest, ServerResponse, ServerPush};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::conversations::ConversationManager;
use crate::llm::LlmClient;
use crate::memory::MemoryManager;
use crate::state::ServerState;
use crate::tasks::TaskManager;
use crate::trigger::TriggerRegistry;

pub struct IpcServer {
    socket_path: String,
    memory: Arc<MemoryManager>,
    conversation_manager: Arc<ConversationManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerRegistry>,
    llm_client: Arc<LlmClient>,
    state: Arc<ServerState>,
}

impl IpcServer {
    pub fn new(
        socket_path: String,
        memory: Arc<MemoryManager>,
        conversation_manager: Arc<ConversationManager>,
        task_manager: Arc<TaskManager>,
        trigger_registry: Arc<TriggerRegistry>,
        llm_client: Arc<LlmClient>,
        state: Arc<ServerState>,
    ) -> Self {
        Self {
            socket_path,
            memory,
            conversation_manager,
            task_manager,
            trigger_registry,
            llm_client,
            state,
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
                    let conversation_manager = self.conversation_manager.clone();
                    let task_manager = self.task_manager.clone();
                    let trigger_registry = self.trigger_registry.clone();
                    let llm_client = self.llm_client.clone();
                    let state = self.state.clone();
                    
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, memory, conversation_manager, task_manager, trigger_registry, llm_client, state).await {
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
    conversation_manager: Arc<ConversationManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerRegistry>,
    llm_client: Arc<LlmClient>,
    state: Arc<ServerState>,
) -> Result<()> {
    // Register this client for push notifications
    let (push_tx, mut push_rx) = mpsc::unbounded_channel();
    state.register_client(push_tx).await;
    
    loop {
        tokio::select! {
            // Handle incoming requests from client
            result = read_request(&mut stream) => {
                match result {
                    Ok(Some(request)) => {
                        state.mark_activity().await;
                        let response = handle_request(
                            request,
                            &memory,
                            &conversation_manager,
                            &task_manager,
                            &trigger_registry,
                            &llm_client,
                        ).await;
                        send_response(&mut stream, response).await?;
                    }
                    Ok(None) => return Ok(()), // Client disconnected
                    Err(e) => return Err(e),
                }
            }
            // Handle outgoing push notifications to client
            Some(push) = push_rx.recv() => {
                send_push(&mut stream, push).await?;
            }
        }
    }
}

async fn read_request(stream: &mut UnixStream) -> Result<Option<ClientRequest>> {
    // Read message length (4 bytes)
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(None); // Client disconnected
        }
        Err(e) => return Err(e.into()),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    debug!("Reading request of {} bytes", len);
    
    if len > 10_000_000 {
        anyhow::bail!("Message too large: {} bytes", len);
    }

    // Read message body
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    debug!("Deserializing {} bytes: {:?}", buf.len(), &buf[..buf.len().min(100)]);
    
    let request: ClientRequest = postcard::from_bytes(&buf)
        .with_context(|| format!("Failed to deserialize {} bytes", buf.len()))?;
    
    debug!("Successfully deserialized request: {:?}", request);
    Ok(Some(request))
}

async fn send_response(stream: &mut UnixStream, response: ServerResponse) -> Result<()> {
    let bytes = postcard::to_allocvec(&response)?;
    let len = u32::try_from(bytes.len()).context("Response too large")?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn send_push(stream: &mut UnixStream, push: ServerPush) -> Result<()> {
    let bytes = postcard::to_allocvec(&push)?;
    let len = u32::try_from(bytes.len()).context("Push message too large")?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn handle_request(
    request: ClientRequest,
    memory: &MemoryManager,
    conversation_manager: &ConversationManager,
    task_manager: &TaskManager,
    trigger_registry: &TriggerRegistry,
    llm_client: &LlmClient,
) -> ServerResponse {
    match request {
        ClientRequest::Ping => ServerResponse::Pong,

        ClientRequest::SendMessage { content, session_id } => {
            // Get or create conversation session
            let session_id = session_id.unwrap_or_else(|| format!("session_{}", chrono::Utc::now().timestamp()));
            let session = match conversation_manager.get_or_create_session(&session_id).await {
                Ok(s) => s,
                Err(e) => {
                    return ServerResponse::Error {
                        message: format!("Failed to create session: {}", e),
                    };
                }
            };

            // Store user message in session
            if let Err(e) = conversation_manager.add_turn(&session_id, "user", &content, None, None).await {
                warn!("Failed to store user turn: {}", e);
            }

            // Also store in legacy conversations table for compaction
            if let Err(e) = memory.store_conversation("user", &content).await {
                warn!("Failed to store in conversations: {}", e);
            }

            // Get system prompt
            let system_prompt = match memory.build_effective_prompt().await {
                Ok(prompt) => Some(prompt),
                Err(e) => {
                    warn!("Failed to build system prompt: {}", e);
                    None
                }
            };

            // Get conversation history from session (last 20 turns)
            let history = match conversation_manager.get_history(&session_id, 20).await {
                Ok(turns) => turns.into_iter()
                    .filter(|turn| turn.turn_number < session.turn_count) // Exclude current turn
                    .map(|turn| crate::llm::Message {
                        role: turn.role,
                        content: turn.content,
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    warn!("Failed to get session history: {}", e);
                    Vec::new()
                }
            };

            // Build enhanced user message with context
            let now = chrono::Local::now();
            let time_str = now.format("%A, %B %d, %Y at %I:%M %p").to_string();
            
            // Get task count
            let task_summary = task_manager.get_task_summary().await
                .unwrap_or_else(|_| "Unable to retrieve task summary".to_string());
            
            let enhanced_content = format!(
                "[Current Time: {}]\n[{}]\n\n{}",
                time_str,
                task_summary,
                content
            );

            // Add current message with context
            let mut messages = history;
            messages.push(crate::llm::Message {
                role: "user".to_string(),
                content: enhanced_content,
            });

            // Generate response with tool execution
            match llm_client.generate_with_tool_execution(
                &messages,
                system_prompt.as_deref(),
                5, // max 5 iterations
            ).await {
                Ok(response) => {
                    // Store assistant response in session
                    if let Err(e) = conversation_manager.add_turn(&session_id, "assistant", &response.content, None, None).await {
                        warn!("Failed to store assistant turn: {}", e);
                    }

                    // Also store in legacy conversations table
                    if let Err(e) = memory.store_conversation("assistant", &response.content).await {
                        warn!("Failed to store assistant response: {}", e);
                    }

                    ServerResponse::Message {
                        content: response.content,
                    }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to generate response: {}", e),
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
            tag,
            description,
        } => {
            match trigger_registry.create_trigger(&name, &trigger_type, &schedule, tag.as_deref(), description.as_deref()).await {
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
        
        ClientRequest::Subscribe => {
            // Client is subscribing for push notifications only
            // Just acknowledge the subscription - pushes are handled via the channel
            info!("Client subscribed for push notifications");
            ServerResponse::Ok
        }
    }
}
