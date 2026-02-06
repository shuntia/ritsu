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
                        
                        // Special handling for SendMessage to support streaming
                        if let ClientRequest::SendMessage { content, session_id } = request {
                            if let Err(e) = handle_send_message_streaming(
                                &mut stream,
                                content,
                                session_id,
                                &memory,
                                &conversation_manager,
                                &task_manager,
                                &llm_client,
                            ).await {
                                error!("Error handling streaming message: {}", e);
                                let error_response = ServerResponse::Error {
                                    message: format!("Error: {}", e),
                                };
                                send_response(&mut stream, error_response).await?;
                            }
                        } else {
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
                    }
                    Ok(None) => {
                        debug!("Client disconnected gracefully");
                        return Ok(());
                    }
                    Err(e) => {
                        error!("Error reading client request: {}", e);
                        return Err(e);
                    }
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

async fn handle_send_message_streaming(
    stream: &mut UnixStream,
    content: String,
    session_id: Option<String>,
    memory: &MemoryManager,
    conversation_manager: &ConversationManager,
    task_manager: &TaskManager,
    llm_client: &LlmClient,
) -> Result<()> {
    // Get or create conversation session
    let session_id = session_id.unwrap_or_else(|| format!("session_{}", chrono::Utc::now().timestamp()));
    let session = conversation_manager.get_or_create_session(&session_id).await?;

    // Store user message
    if let Err(e) = conversation_manager.add_turn(&session_id, "user", &content, None, None, None).await {
        warn!("Failed to store user turn: {}", e);
    }
    if let Err(e) = memory.store_conversation("user", &content).await {
        warn!("Failed to store in conversations: {}", e);
    }

    // Get system prompt
    let base_prompt = memory.build_effective_prompt().await
        .unwrap_or_else(|_| "You are Ritsu, a helpful AI assistant.".to_string());
    
    let system_prompt = Some(format!(
        "{}\n\n---\n\n**CHAT SESSION MODE**\n\
        You are currently in an active chat conversation with the user.\n\
        - Prioritize responding directly to the user's message\n\
        - Be conversational and helpful\n\
        - Avoid doing background tasks like memory compaction or analysis\n\
        - Only use tools if directly relevant to answering the user's question\n\
        - Don't create triggers or tasks unless explicitly asked\n\
        - Focus on the conversation, not on system maintenance",
        base_prompt
    ));

    // Get conversation history
    let history = conversation_manager.get_history(&session_id, 20).await
        .unwrap_or_default()
        .into_iter()
        .filter(|turn| turn.turn_number < session.turn_count)
        .map(|turn| crate::llm::Message {
            role: turn.role,
            content: turn.content,
        })
        .collect::<Vec<_>>();

    // Build enhanced message
    let now = chrono::Local::now();
    let time_str = now.format("%A, %B %d, %Y at %I:%M %p").to_string();
    let task_summary = task_manager.get_task_summary().await
        .unwrap_or_else(|_| "Unable to retrieve task summary".to_string());
    
    let enhanced_content = format!(
        "[Current Time: {}]\n[{}]\n\n{}",
        time_str,
        task_summary,
        content
    );

    let mut messages = history;
    messages.push(crate::llm::Message {
        role: "user".to_string(),
        content: enhanced_content,
    });

    // Start streaming
    info!("Starting streaming response");
    let stream_rx_result = llm_client.generate_streaming(&messages, system_prompt.as_deref()).await;
    
    let mut stream_rx = match stream_rx_result {
        Ok(rx) => rx,
        Err(e) => {
            error!("Failed to start streaming: {}", e);
            // Send error as a message chunk so GUI knows what happened
            let push = ServerPush::MessageChunk {
                content: format!("❌ Failed to connect to LLM: {}\n\nPlease check that Ollama is running and the model is available.", e),
                is_final: true,
            };
            send_push(stream, push).await?;
            return Err(e);
        }
    };

    let mut full_response = String::new();

    // Stream chunks to client
    while let Some(chunk_result) = stream_rx.recv().await {
        match chunk_result {
            Ok(chunk) => {
                full_response.push_str(&chunk);
                
                // Send chunk as push notification
                let push = ServerPush::MessageChunk {
                    content: chunk,
                    is_final: false,
                };
                send_push(stream, push).await?;
            }
            Err(e) => {
                error!("Streaming error during LLM response: {}", e);
                let push = ServerPush::MessageChunk {
                    content: format!("\n\n❌ Error during streaming: {}\n\nThe connection to the LLM may have been interrupted.", e),
                    is_final: true,
                };
                send_push(stream, push).await?;
                return Err(e);
            }
        }
    }

    // Send final marker
    let push = ServerPush::MessageChunk {
        content: String::new(),
        is_final: true,
    };
    send_push(stream, push).await?;

    info!("Streaming complete, {} bytes total", full_response.len());

    // Store assistant response
    if let Err(e) = conversation_manager.add_turn(&session_id, "assistant", &full_response, None, None, None).await {
        warn!("Failed to store assistant turn: {}", e);
    }
    if let Err(e) = memory.store_conversation("assistant", &full_response).await {
        warn!("Failed to store assistant response: {}", e);
    }

    // Check if we need to generate a title for this session
    if let Ok(needs_title) = conversation_manager.needs_title_generation(&session_id).await {
        if needs_title {
            info!("Generating title for session {}", session_id);
            if let Err(e) = conversation_manager.generate_title(&session_id, llm_client).await {
                warn!("Failed to generate session title: {}", e);
            }
        }
    }

    Ok(())
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
            if let Err(e) = conversation_manager.add_turn(&session_id, "user", &content, None, None, None).await {
                warn!("Failed to store user turn: {}", e);
            }

            // Also store in legacy conversations table for compaction
            if let Err(e) = memory.store_conversation("user", &content).await {
                warn!("Failed to store in conversations: {}", e);
            }

            // Get system prompt with chat session context
            let base_prompt = match memory.build_effective_prompt().await {
                Ok(prompt) => prompt,
                Err(e) => {
                    warn!("Failed to build system prompt: {}", e);
                    "You are Ritsu, a helpful AI assistant.".to_string()
                }
            };
            
            // Add chat mode instructions
            let system_prompt = Some(format!(
                "{}\n\n---\n\n**CHAT SESSION MODE**\n\
                You are currently in an active chat conversation with the user.\n\
                - Prioritize responding directly to the user's message\n\
                - Be conversational and helpful\n\
                - Avoid doing background tasks like memory compaction or analysis\n\
                - Only use tools if directly relevant to answering the user's question\n\
                - Don't create triggers or tasks unless explicitly asked\n\
                - Focus on the conversation, not on system maintenance",
                base_prompt
            ));

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
                    if let Err(e) = conversation_manager.add_turn(&session_id, "assistant", &response.content, None, None, None).await {
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
        
        ClientRequest::DeleteTask { id } => {
            match task_manager.delete_task(id).await {
                Ok(()) => ServerResponse::Ok,
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to delete task: {}", e),
                },
            }
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
        
        ClientRequest::ListSessions { limit } => {
            match conversation_manager.get_active_sessions().await {
                Ok(sessions) => {
                    let mut sessions = sessions;
                    if let Some(limit) = limit {
                        sessions.truncate(limit);
                    }
                    
                    let session_infos = sessions
                        .into_iter()
                        .map(|s| ritsu_common::protocol::SessionInfo {
                            session_id: s.session_id,
                            started_at: s.started_at,
                            last_activity: s.last_activity,
                            turn_count: s.turn_count,
                            title: s.title,
                        })
                        .collect();
                    
                    ServerResponse::Sessions { sessions: session_infos }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to list sessions: {}", e),
                },
            }
        }
        
        ClientRequest::GetConversationHistory { session_id, limit } => {
            match conversation_manager.get_history(&session_id, limit).await {
                Ok(turns) => {
                    let turn_infos = turns
                        .into_iter()
                        .map(|t| ritsu_common::protocol::ConversationTurn {
                            turn_number: t.turn_number,
                            role: t.role,
                            content: t.content,
                            tool_calls: t.tool_calls,
                            tool_results: t.tool_results,
                            thinking: t.thinking,
                        })
                        .collect();
                    
                    ServerResponse::ConversationHistory { turns: turn_infos }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to get conversation history: {}", e),
                },
            }
        }
        
        ClientRequest::ClearMemory { confirm } => {
            if !confirm {
                return ServerResponse::Error {
                    message: "ClearMemory requires confirm=true to prevent accidental deletion".to_string(),
                };
            }
            
            info!("Clearing all memory (requested by client)");
            
            // Clear conversations
            if let Err(e) = conversation_manager.clear_all().await {
                return ServerResponse::Error {
                    message: format!("Failed to clear conversations: {}", e),
                };
            }
            
            // Clear memory tables
            if let Err(e) = memory.clear_all().await {
                return ServerResponse::Error {
                    message: format!("Failed to clear memory: {}", e),
                };
            }
            
            info!("All memory cleared successfully");
            ServerResponse::Ok
        }
        
        ClientRequest::Subscribe => {
            // Client is subscribing for push notifications only
            // Just acknowledge the subscription - pushes are handled via the channel
            info!("Client subscribed for push notifications");
            ServerResponse::Ok
        }

        ClientRequest::GetSystemPrompt => {
            match memory.build_effective_prompt().await {
                Ok(prompt) => ServerResponse::SystemPrompt { content: prompt },
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to get system prompt: {}", e),
                },
            }
        }

        ClientRequest::SetSystemPrompt { content } => {
            match memory.store_system_prompt("base", &content).await {
                Ok(()) => ServerResponse::Success {
                    message: "System prompt updated successfully".to_string(),
                },
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to set system prompt: {}", e),
                },
            }
        }
    }
}
