//! IPC server - Unix domain socket communication
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::format_push_string)]

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientRequest, ServerPush, ServerResponse};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_rusqlite::rusqlite;
use tracing::{debug, error, info, warn};

use crate::config::Config;
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
    config: Arc<Config>,
}

impl IpcServer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        socket_path: String,
        memory: Arc<MemoryManager>,
        conversation_manager: Arc<ConversationManager>,
        task_manager: Arc<TaskManager>,
        trigger_registry: Arc<TriggerRegistry>,
        llm_client: Arc<LlmClient>,
        state: Arc<ServerState>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            socket_path,
            memory,
            conversation_manager,
            task_manager,
            trigger_registry,
            llm_client,
            state,
            config,
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
                    let config = self.config.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_client(
                            stream,
                            memory,
                            conversation_manager,
                            task_manager,
                            trigger_registry,
                            llm_client,
                            state,
                            config,
                        )
                        .await
                        {
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

#[allow(clippy::too_many_arguments)]
async fn handle_client(
    mut stream: UnixStream,
    memory: Arc<MemoryManager>,
    conversation_manager: Arc<ConversationManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerRegistry>,
    llm_client: Arc<LlmClient>,
    state: Arc<ServerState>,
    config: Arc<Config>,
) -> Result<()> {
    // Register this client for push notifications (bounded channel with backpressure)
    let (push_tx, mut push_rx) = mpsc::channel(crate::state::PUSH_CHANNEL_CAPACITY);
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
                            match handle_send_message_streaming(
                                &mut stream,
                                content,
                                session_id,
                                &memory,
                                &conversation_manager,
                                &task_manager,
                                llm_client.clone(),
                                config.llm.disable_streaming,
                                config.llm.disable_tools,
                            ).await {
                                Ok(()) => {
                                    // Streaming handler has already sent the final ServerResponse::Ok
                                }
                                Err(e) => {
                                    error!("Error handling streaming message: {}", e);
                                    let error_response = ServerResponse::Error {
                                        message: format!("Error: {}", e),
                                    };
                                    send_response(&mut stream, error_response).await?;
                                }
                            }
                        } else {
                            let response = handle_request(
                                request,
                                &memory,
                                &conversation_manager,
                                &task_manager,
                                &trigger_registry,
                                &llm_client,
                                &config,
                                state.clone(),
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

    debug!(
        "Deserializing {} bytes: {:?}",
        buf.len(),
        &buf[..buf.len().min(100)]
    );

    let request: ClientRequest = postcard::from_bytes(&buf)
        .with_context(|| format!("Failed to deserialize {} bytes", buf.len()))?;

    debug!("Successfully deserialized request: {:?}", request);
    Ok(Some(request))
}

#[allow(clippy::too_many_arguments)]
async fn handle_send_message_streaming(
    stream: &mut UnixStream,
    content: String,
    session_id: Option<String>,
    memory: &MemoryManager,
    conversation_manager: &ConversationManager,
    task_manager: &TaskManager,
    llm_client: Arc<LlmClient>,
    disable_streaming: bool,
    disable_tools: bool,
) -> Result<()> {
    // Get or create conversation session
    let session_id =
        session_id.unwrap_or_else(|| format!("session_{}", chrono::Utc::now().timestamp()));
    let session = conversation_manager
        .get_or_create_session(&session_id)
        .await?;

    // Store user message (fatal on failure)
    if let Err(e) = conversation_manager
        .add_turn(&session_id, "user", &content, None, None, None)
        .await
    {
        error!("Failed to store user turn: {}", e);
        let push = ServerPush::MessageChunk {
            content: format!(
                "{} Failed to store user message: {}\n",
                nerd_font::categories::Fa::Cross,
                e
            ),
            is_final: true,
        };
        // Inform client and stop processing
        send_push(stream, push).await?;
        return Ok(());
    }
    if let Err(e) = memory.store_conversation("user", &content).await {
        error!("Failed to store in conversations: {}", e);
        let push = ServerPush::MessageChunk {
            content: format!(
                "{} Failed to store message in memory: {}\n",
                nerd_font::categories::Fa::Cross,
                e
            ),
            is_final: true,
        };
        send_push(stream, push).await?;
        return Ok(());
    }

    // Build system prompt and messages via PromptBuilder
    let history = conversation_manager
        .get_history(&session_id, 20)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|turn| turn.turn_number < session.turn_count)
        .map(|turn| crate::llm::Message {
            role: turn.role,
            content: turn.content,
        })
        .collect::<Vec<_>>();

    let (system_prompt, messages) = match crate::prompt::PromptBuilder::build_chat(
        memory,
        task_manager,
        history,
        &content,
    )
    .await
    {
        Ok((sp, msgs)) => (sp, msgs),
        Err(e) => {
            warn!("Failed to build chat prompt: {}", e);
            (Some("You are Ritsu, a helpful AI assistant. You are in chat mode - respond directly to the user.".to_string()), vec![crate::llm::Message { role: "user".to_string(), content }])
        }
    };

    // Check if streaming is disabled in config
    if disable_streaming {
        info!(
            "Streaming disabled in config, using non-streaming mode (tools: {})",
            !disable_tools
        );

        // For non-streaming mode, if tools are enabled, run the tool execution loop which will
        // call tools and synthesize a follow-up response. Otherwise, just do a simple request.
        let response_result = if !disable_tools {
            // Run up to 3 iterations of tool execution
            llm_client
                .generate_with_tool_execution(&messages, system_prompt.as_deref(), 3)
                .await
        } else {
            llm_client
                .generate_with_tools(&messages, system_prompt.as_deref(), false)
                .await
        };

        match response_result {
            Ok(response) => {
                debug!(
                    "Non-streaming response ready (tool_calls: {})",
                    response.tool_calls.len()
                );
                if !response.tool_calls.is_empty() {
                    debug!(tool_calls = ?response.tool_calls.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), "Tool calls executed in non-streaming flow");
                }

                info!("Non-streaming request succeeded");
                let push = ServerPush::MessageChunk {
                    content: response.content.clone(),
                    is_final: true,
                };
                send_push(stream, push).await?;

                // Store assistant response
                if let Err(e) = conversation_manager
                    .add_turn(
                        &session_id,
                        "assistant",
                        &response.content,
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    warn!("Failed to store assistant turn: {}", e);
                }
                if let Err(e) = memory
                    .store_conversation("assistant", &response.content)
                    .await
                {
                    warn!("Failed to store in conversations: {}", e);
                }

                return Ok(());
            }
            Err(e) => {
                error!("Non-streaming request failed: {}", e);
                let push = ServerPush::MessageChunk {
                    content: format!("\n\n{} Error: {}\n\nPlease check if Ollama is running and the model is available.", nerd_font::categories::Fa::Cross, e),
                    is_final: true,
                };
                send_push(stream, push).await?;
                return Err(e);
            }
        }
    }

    // Start streaming
    info!("Starting streaming response");
    let stream_rx_result =
        LlmClient::generate_streaming(Arc::clone(&llm_client), &messages, system_prompt.as_deref())
            .await;

    let mut stream_rx = match stream_rx_result {
        Ok(rx) => rx,
        Err(e) => {
            error!("Failed to start streaming: {}", e);
            // Send error as a message chunk so GUI knows what happened
            let push = ServerPush::MessageChunk {
                content: format!("{} Failed to connect to LLM: {}\n\nPlease check that Ollama is running and the model is available.", nerd_font::categories::Fa::Cross, e),
                is_final: true,
            };
            send_push(stream, push).await?;
            return Err(e);
        }
    };

    let mut full_response = String::new();
    let mut chunk_count = 0;

    // Stream chunks to client
    while let Some(chunk_result) = stream_rx.recv().await {
        match chunk_result {
            Ok(chunk) => {
                chunk_count += 1;
                debug!("Received chunk #{}: {} bytes", chunk_count, chunk.len());

                if chunk.is_empty() {
                    debug!("Skipping empty chunk");
                } else {
                    full_response.push_str(&chunk);

                    // Send chunk as push notification
                    let push = ServerPush::MessageChunk {
                        content: chunk,
                        is_final: false,
                    };
                    send_push(stream, push).await?;
                }
            }
            Err(e) => {
                error!("Streaming error during LLM response: {}", e);
                let push = ServerPush::MessageChunk {
                    content: format!("\n\n{} Error during streaming: {}\n\nThe connection to the LLM may have been interrupted.", nerd_font::categories::Fa::Cross, e),
                    is_final: true,
                };
                send_push(stream, push).await?;
                // Don't propagate error to caller; we've already informed the client
                return Ok(());
            }
        }
    }

    info!(
        "Streaming complete: {} chunks received, {} bytes total",
        chunk_count,
        full_response.len()
    );

    // Send final marker
    let push = ServerPush::MessageChunk {
        content: String::new(),
        is_final: true,
    };
    send_push(stream, push).await?;

    info!(
        "Streaming complete: {} chunks received, {} bytes total",
        chunk_count,
        full_response.len()
    );

    // Store assistant response
    if let Err(e) = conversation_manager
        .add_turn(&session_id, "assistant", &full_response, None, None, None)
        .await
    {
        warn!("Failed to store assistant turn: {}", e);
    }
    if let Err(e) = memory.store_conversation("assistant", &full_response).await {
        warn!("Failed to store assistant response: {}", e);
    }

    // Check if we need to generate a title for this session
    if let Ok(needs_title) = conversation_manager
        .needs_title_generation(&session_id)
        .await
    {
        if needs_title {
            info!("Generating title for session {}", session_id);
            if let Err(e) = conversation_manager
                .generate_title(&session_id, &llm_client)
                .await
            {
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
    config: &crate::config::Config,
    state: Arc<crate::state::ServerState>,
) -> ServerResponse {
    match request {
        ClientRequest::Ping => ServerResponse::Pong,

        ClientRequest::SendMessage {
            content,
            session_id,
        } => {
            // Get or create conversation session
            let session_id =
                session_id.unwrap_or_else(|| format!("session_{}", chrono::Utc::now().timestamp()));
            let session = match conversation_manager
                .get_or_create_session(&session_id)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    return ServerResponse::Error {
                        message: format!("Failed to create session: {}", e),
                    };
                }
            };

            // Store user message in session
            if let Err(e) = conversation_manager
                .add_turn(&session_id, "user", &content, None, None, None)
                .await
            {
                warn!("Failed to store user turn: {}", e);
            }

            // Also store in legacy conversations table for compaction
            if let Err(e) = memory.store_conversation("user", &content).await {
                warn!("Failed to store in conversations: {}", e);
            }

            // Build system prompt and messages via PromptBuilder
            let history = match conversation_manager.get_history(&session_id, 20).await {
                Ok(turns) => turns
                    .into_iter()
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

            let (system_prompt, messages) = match crate::prompt::PromptBuilder::build_chat(
                memory,
                task_manager,
                history,
                &content,
            )
            .await
            {
                Ok((sp, msgs)) => (sp, msgs),
                Err(e) => {
                    warn!("Failed to build chat prompt: {}", e);
                    (Some("You are Ritsu, a helpful AI assistant. You are in chat mode - respond directly to the user.".to_string()), vec![crate::llm::Message { role: "user".to_string(), content }])
                }
            };

            // Generate response with tool execution
            match llm_client
                .generate_with_tool_execution(
                    &messages,
                    system_prompt.as_deref(),
                    5, // max 5 iterations
                )
                .await
            {
                Ok(response) => {
                    // Store assistant response in session
                    if let Err(e) = conversation_manager
                        .add_turn(
                            &session_id,
                            "assistant",
                            &response.content,
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        warn!("Failed to store assistant turn: {}", e);
                    }

                    // Also store in legacy conversations table
                    if let Err(e) = memory
                        .store_conversation("assistant", &response.content)
                        .await
                    {
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
                        crate::trigger::TriggerType::Cron(expr) => {
                            ("cron".to_string(), expr.clone())
                        }
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
            match trigger_registry
                .create_trigger(
                    &name,
                    &trigger_type,
                    &schedule,
                    tag.as_deref(),
                    description.as_deref(),
                )
                .await
            {
                Ok(()) => {
                    // Notify connected clients about the new trigger
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "Trigger Created".to_string(),
                            message: format!("Trigger '{}' created", name),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Ok
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to create trigger: {}", e),
                },
            }
        }

        ClientRequest::DeleteTrigger { name } => {
            match trigger_registry.delete_trigger(&name).await {
                Ok(()) => {
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "Trigger Deleted".to_string(),
                            message: format!("Trigger '{}' deleted", name),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Ok
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to delete trigger: {}", e),
                },
            }
        }

        ClientRequest::DisableTrigger { name } => {
            match trigger_registry.disable_trigger(&name).await {
                Ok(()) => {
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "Trigger Disabled".to_string(),
                            message: format!("Trigger '{}' disabled", name),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Ok
                }
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
                .create_task(
                    &title,
                    description.as_deref(),
                    &priority,
                    &tags,
                    due_date.as_deref(),
                )
                .await
            {
                Ok(id) => {
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "Task Created".to_string(),
                            message: format!("Task '{}' created (id: {})", title, id),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Ok
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to create task: {}", e),
                },
            }
        }

        ClientRequest::UpdateTask {
            id,
            status,
            priority,
        } => {
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

            // Notify clients about task update
            let _ = state
                .broadcast_push(ServerPush::Notification {
                    title: "Task Updated".to_string(),
                    message: format!("Task #{} updated", id),
                    urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                })
                .await;

            ServerResponse::Ok
        }

        ClientRequest::DeleteTask { id } => match task_manager.delete_task(id).await {
            Ok(()) => {
                let _ = state
                    .broadcast_push(ServerPush::Notification {
                        title: "Task Deleted".to_string(),
                        message: format!("Task #{} deleted", id),
                        urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                    })
                    .await;
                ServerResponse::Ok
            }
            Err(e) => ServerResponse::Error {
                message: format!("Failed to delete task: {}", e),
            },
        },

        ClientRequest::QueryMemory {
            query_type,
            date_range: _,
        } => {
            use ritsu_common::protocol::MemoryQueryType;

            // Default to 7 days for queries (date_range could be used for more specific filtering in future)
            let days = 7;

            let result = match query_type {
                MemoryQueryType::Recent => match memory.query_recent_conversations(days).await {
                    Ok(conversations) => {
                        if conversations.is_empty() {
                            "No recent conversations found.".to_string()
                        } else {
                            let mut output =
                                format!("Recent conversations (last {days} days):\n\n");
                            for (date, role, content) in conversations {
                                output.push_str(&format!("[{date}] {role}: {content}\n"));
                            }
                            output
                        }
                    }
                    Err(e) => format!("Error querying conversations: {e}"),
                },
                MemoryQueryType::Daily => match memory.query_daily_summaries(days).await {
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
                },
                MemoryQueryType::Monthly => match memory.query_monthly_summaries(12).await {
                    Ok(summaries) => {
                        if summaries.is_empty() {
                            "No monthly summaries found.".to_string()
                        } else {
                            let mut output = "Monthly summaries:\n\n".to_string();
                            for (year_month, summary, days_count) in summaries {
                                output.push_str(&format!(
                                    "[{year_month}] ({days_count} days)\n{summary}\n\n"
                                ));
                            }
                            output
                        }
                    }
                    Err(e) => format!("Error querying monthly summaries: {e}"),
                },
                MemoryQueryType::Notes => match memory.query_notes(50).await {
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
                },
            };

            ServerResponse::Memory { content: result }
        }
        ClientRequest::CreateNote { content, tags } => {
            match memory.create_note(&content, &tags).await {
                Ok(id) => {
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "Note Created".to_string(),
                            message: format!("Note #{} created", id),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Success {
                        message: format!("Created note id: {}", id),
                    }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to create note: {}", e),
                },
            }
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

                    ServerResponse::Sessions {
                        sessions: session_infos,
                    }
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
                    message: "ClearMemory requires confirm=true to prevent accidental deletion"
                        .to_string(),
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
            let _ = state
                .broadcast_push(ServerPush::Notification {
                    title: "Memory Cleared".to_string(),
                    message: "All memory has been cleared by client request".to_string(),
                    urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                })
                .await;
            ServerResponse::Ok
        }

        ClientRequest::Subscribe => {
            // Client is subscribing for push notifications only
            // Just acknowledge the subscription - pushes are handled via the channel
            info!("Client subscribed for push notifications");
            ServerResponse::Ok
        }

        ClientRequest::GetSystemPrompt => match memory.build_effective_prompt().await {
            Ok(prompt) => ServerResponse::SystemPrompt { content: prompt },
            Err(e) => ServerResponse::Error {
                message: format!("Failed to get system prompt: {}", e),
            },
        },

        ClientRequest::SetSystemPrompt { content } => {
            match memory.store_system_prompt("base", &content).await {
                Ok(()) => {
                    let _ = state
                        .broadcast_push(ServerPush::Notification {
                            title: "System Prompt Updated".to_string(),
                            message: "Base system prompt updated".to_string(),
                            urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                        })
                        .await;
                    ServerResponse::Success {
                        message: "System prompt updated successfully".to_string(),
                    }
                }
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to set system prompt: {}", e),
                },
            }
        }

        ClientRequest::GetModelInfo => {
            // Pick the configured default backend if present, otherwise show the first configured backend
            let backend = config
                .llm
                .backends
                .iter()
                .find(|b| b.name == config.llm.default_backend)
                .or_else(|| config.llm.backends.first());

            let model_info = backend.map_or_else(
                || "No LLM backend configured".to_string(),
                |b| {
                    format!(
                        "Model Information:\n\
                     Backend: {}\n\
                     Endpoint: {}\n\
                     Model: {}\n\
                     Streaming: {}\n\
                     Tools: {}",
                        b.name,
                        b.endpoint,
                        b.model,
                        !config.llm.disable_streaming,
                        !config.llm.disable_tools,
                    )
                },
            );
            ServerResponse::Success {
                message: model_info,
            }
        }

        ClientRequest::GetDatabaseStats => match memory.get_database_stats().await {
            Ok(stats) => ServerResponse::Success { message: stats },
            Err(e) => ServerResponse::Error {
                message: format!("Failed to get database stats: {}", e),
            },
        },

        ClientRequest::ExportDatabase => match memory.export_database().await {
            Ok(json) => ServerResponse::Success { message: json },
            Err(e) => ServerResponse::Error {
                message: format!("Failed to export database: {}", e),
            },
        },

        ClientRequest::InspectSession { session_id } => {
            match conversation_manager.inspect_session(&session_id).await {
                Ok(info) => ServerResponse::Success { message: info },
                Err(e) => ServerResponse::Error {
                    message: format!("Failed to inspect session: {}", e),
                },
            }
        }

        ClientRequest::GetToolStats => {
            // For now, return a placeholder message
            // Full implementation would track tool usage in database
            ServerResponse::Success {
                message: "Tool statistics tracking not yet implemented".to_string(),
            }
        }

        ClientRequest::GetMemoryStatus => match memory.get_compaction_status().await {
            Ok(status) => ServerResponse::Success { message: status },
            Err(e) => ServerResponse::Error {
                message: format!("Failed to get memory status: {}", e),
            },
        },

        ClientRequest::ForceCompact => match memory.force_compact().await {
            Ok(()) => {
                let _ = state
                    .broadcast_push(ServerPush::Notification {
                        title: "Memory Compaction".to_string(),
                        message: "Memory compaction completed successfully".to_string(),
                        urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                    })
                    .await;
                ServerResponse::Success {
                    message: "Memory compaction completed successfully".to_string(),
                }
            }
            Err(e) => ServerResponse::Error {
                message: format!("Failed to force compact: {}", e),
            },
        },

        ClientRequest::ReindexDatabase => match memory.reindex_database().await {
            Ok(()) => {
                let _ = state
                    .broadcast_push(ServerPush::Notification {
                        title: "Database Reindexed".to_string(),
                        message: "Database reindexed successfully".to_string(),
                        urgency: ritsu_common::protocol::NotificationUrgency::Normal,
                    })
                    .await;
                ServerResponse::Success {
                    message: "Database reindexed successfully".to_string(),
                }
            }
            Err(e) => ServerResponse::Error {
                message: format!("Failed to reindex database: {}", e),
            },
        },
        ClientRequest::ResetDatabase { confirm } => {
            if !confirm {
                return ServerResponse::Error {
                    message: "ResetDatabase requires confirm=true to prevent accidental deletion"
                        .to_string(),
                };
            }

            info!("Resetting full database (destructive operation requested by client)");

            // Execute deletion of key tables in a blocking context with backup and transaction
            let db_path = config.server.database_path.clone();
            let exec_result = crate::database::Database::execute_blocking(db_path.clone(), move |conn: &rusqlite::Connection| -> anyhow::Result<()> {
                // Create a filesystem-level backup before destructive reset
                let backup_path = format!("{}.reset_backup.{}", db_path, chrono::Utc::now().timestamp());
                std::fs::copy(&db_path, &backup_path).map_err(|e| anyhow::anyhow!(e))?;

                // Perform deletes inside a transaction to ensure atomicity
                conn.execute_batch(
                    "BEGIN IMMEDIATE;\nDELETE FROM conversation_turns;\nDELETE FROM conversations;\nDELETE FROM notes;\nDELETE FROM daily_conversations;\nDELETE FROM daily_summaries;\nDELETE FROM monthly_summaries;\nDELETE FROM idle_analyses;\nDELETE FROM tool_usage;\nDELETE FROM tasks;\nDELETE FROM triggers;\nDELETE FROM system_prompts;\nDELETE FROM preferences;\nCOMMIT;",
                ).map_err(|e| anyhow::anyhow!(e))?;

                // Reset AUTOINCREMENT counters stored in sqlite_sequence
                conn.execute("DELETE FROM sqlite_sequence", []).map_err(|e| anyhow::anyhow!(e))?;

                // VACUUM must be run outside of an active transaction; this will rebuild the database file
                conn.execute("VACUUM", []).map_err(|e| anyhow::anyhow!(e))?;

                Ok(())
            }).await;

            if let Err(e) = exec_result {
                return ServerResponse::Error {
                    message: format!("Failed to reset database: {}", e),
                };
            }

            // Reload triggers
            if let Err(e) = trigger_registry.load_from_database().await {
                warn!("Failed to reload triggers after reset: {}", e);
            }

            // Ensure managers clear in-memory caches
            if let Err(e) = conversation_manager.clear_all().await {
                warn!("Failed to clear conversations after reset: {}", e);
            }
            if let Err(e) = memory.clear_all().await {
                warn!("Failed to clear memory after reset: {}", e);
            }

            // Reindex database
            if let Err(e) = memory.reindex_database().await {
                warn!("Failed to reindex database after reset: {}", e);
            }

            let _ = state
                .broadcast_push(ServerPush::Notification {
                    title: "Database Reset".to_string(),
                    message: "Database was reset by client request".to_string(),
                    urgency: ritsu_common::protocol::NotificationUrgency::Critical,
                })
                .await;

            ServerResponse::Success {
                message: "Database reset successfully".to_string(),
            }
        }
    }
}
