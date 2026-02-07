//! Tool system - dynamic tool registry and execution

use anyhow::Result;
use ritsu_common::ToolResult;
use rusqlite::Connection;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

pub type ToolFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;
pub type ToolFunction = Arc<dyn Fn(HashMap<String, String>) -> ToolFuture + Send + Sync>;

/// Tool metadata and handler for AI context
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub parameters: Vec<ToolParameter>,
    pub handler: ToolFunction,
}

/// Tool parameter definition
#[derive(Clone)]
#[allow(dead_code)]
pub struct ToolParameter {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub param_type: String,
}

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Tool>>,
    db_conn: Option<Arc<Mutex<Connection>>>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            db_conn: None,
        }
    }

    /// Set database connection for usage tracking
    pub fn with_database(mut self, conn: Arc<Mutex<Connection>>) -> Self {
        self.db_conn = Some(conn);
        self
    }

    pub async fn register(&self, tool: Tool) {
        let name = tool.name.clone();
        self.tools.write().await.insert(name.clone(), tool);
        info!("Registered tool: {name}");
    }

    #[allow(dead_code)]
    pub async fn tool_count(&self) -> usize {
        self.tools.read().await.len()
    }

    #[allow(dead_code)]
    pub async fn execute(&self, name: &str, args: HashMap<String, String>) -> Result<ToolResult> {
        let start = std::time::Instant::now();
        let tools = self.tools.read().await;
        
        let result = if let Some(tool) = tools.get(name) {
            (tool.handler)(args.clone()).await
        } else {
            ToolResult::error(format!("Tool '{name}' not found"))
        };
        
        let execution_time = start.elapsed().as_millis() as i64;
        
        // Log tool usage to database
        if let Some(db) = &self.db_conn {
            let name_clone = name.to_string();
            let args_json = serde_json::to_string(&args).unwrap_or_default();
            let result_str = result.output.clone();
            let success = result.success;
            
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::log_tool_usage(
                    &db_clone,
                    &name_clone,
                    &args_json,
                    success,
                    &result_str,
                    execution_time,
                ).await {
                    warn!("Failed to log tool usage: {}", e);
                }
            });
        }
        
        Ok(result)
    }

    async fn log_tool_usage(
        db: &Arc<Mutex<Connection>>,
        tool_name: &str,
        args: &str,
        success: bool,
        result: &str,
        execution_time_ms: i64,
    ) -> Result<()> {
        let conn = db.lock().await;
        conn.execute(
            "INSERT INTO tool_usage (tool_name, arguments, success, result, execution_time_ms, triggered_by)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ai')",
            (tool_name, args, success, result, execution_time_ms),
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn list_tools(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// Get tool information for AI context
    #[allow(dead_code)]
    pub async fn get_tools_for_ai(&self) -> Vec<ToolInfo> {
        let tools = self.tools.read().await;
        tools.values()
            .map(|tool| ToolInfo {
                name: tool.name.clone(),
                description: tool.description.clone(),
                tags: tool.tags.clone(),
                parameters: tool.parameters.iter()
                    .map(|p| ToolParameterInfo {
                        name: p.name.clone(),
                        description: p.description.clone(),
                        required: p.required,
                        param_type: p.param_type.clone(),
                    })
                    .collect(),
            })
            .collect()
    }
}

/// Tool information for AI (without the handler)
#[derive(Clone)]
#[allow(dead_code)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub parameters: Vec<ToolParameterInfo>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct ToolParameterInfo {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub param_type: String,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Tool implementations
mod tool_impls {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tracing::{info, warn};
    
    use super::{Tool, ToolParameter};
    use ritsu_common::ToolResult;

    pub fn notify_client(state: Arc<super::super::state::ServerState>) -> Tool {
        Tool {
            name: "notify_client".to_string(),
            description: "Send a notification to the user's desktop/client".to_string(),
            tags: vec!["notification".to_string(), "ui".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "title".to_string(),
                    description: "Notification title".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "message".to_string(),
                    description: "Notification message body".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "urgency".to_string(),
                    description: "Urgency level: low, normal, urgent".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let state = state.clone();
                Box::pin(async move {
                    let title = args.get("title").cloned().unwrap_or_default();
                    let message = args.get("message").cloned().unwrap_or_default();
                    let urgency_str = args.get("urgency").cloned().unwrap_or_else(|| "normal".to_string());

                    let urgency = match urgency_str.as_str() {
                        "low" => ritsu_common::protocol::NotificationUrgency::Low,
                        "critical" | "urgent" => ritsu_common::protocol::NotificationUrgency::Critical,
                        _ => ritsu_common::protocol::NotificationUrgency::Normal,
                    };

                    // Send to client daemon instead of broadcast
                    let request = ritsu_common::protocol::ServerToClientRequest::NotifyUser {
                        title: title.clone(),
                        message: message.clone(),
                        urgency,
                    };

                    match state.send_to_client_daemon(request).await {
                        Ok(()) => {
                            info!("Notification sent to client daemon: [{urgency_str}] {title}: {message}");
                            ToolResult::success(format!("Notification sent: {title}"))
                        }
                        Err(e) => {
                            warn!("Failed to send notification to client daemon: {}", e);
                            // Fallback to broadcast for backwards compatibility
                            state.broadcast_push(ritsu_common::protocol::ServerPush::Notification {
                                title: title.clone(),
                                message: message.clone(),
                                urgency,
                            }).await;
                            ToolResult::success(format!("Notification sent (fallback): {title}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn create_note(memory: Arc<super::super::memory::MemoryManager>) -> Tool {
        Tool {
            name: "create_note".to_string(),
            description: "Create a persistent note for future reference".to_string(),
            tags: vec!["memory".to_string(), "note".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "content".to_string(),
                    description: "Note content".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "tags".to_string(),
                    description: "Comma-separated tags".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let memory = memory.clone();
                Box::pin(async move {
                    let content = args.get("content").cloned().unwrap_or_default();
                    let tags_str = args.get("tags").cloned().unwrap_or_default();
                    
                    // Parse tags
                    let tags: Vec<String> = tags_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();

                    // Store note in database
                    match memory.create_note(&content, &tags).await {
                        Ok(_) => {
                            info!("Note created with tags: {}", tags_str);
                            ToolResult::success(format!("Note created successfully with {} tags", tags.len()))
                        }
                        Err(e) => {
                            tracing::error!("Failed to create note: {}", e);
                            ToolResult::error(format!("Failed to create note: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn query_memory(memory: Arc<super::super::memory::MemoryManager>) -> Tool {
        Tool {
            name: "query_memory".to_string(),
            description: "Query past conversations, summaries, or notes".to_string(),
            tags: vec!["memory".to_string(), "search".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "type".to_string(),
                    description: "Query type: conversations, daily, monthly, notes".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "query".to_string(),
                    description: "Search query or filter".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "limit".to_string(),
                    description: "Maximum number of results (default: 10)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let memory = memory.clone();
                Box::pin(async move {
                    let query_type = args.get("type").cloned().unwrap_or_default();
                    let query = args.get("query").cloned().unwrap_or_default();
                    let limit = args.get("limit")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(10);

                    info!("Querying memory: type={}, query={}, limit={}", query_type, query, limit);

                    let result = match query_type.as_str() {
                        "conversations" => {
                            match memory.get_recent_conversations_days(30).await {
                                Ok(convs) => {
                                    let filtered: Vec<(String, String, String, String)> = convs.into_iter()
                                        .filter(|(_, _, content, _)| query.is_empty() || content.contains(&query))
                                        .take(limit as usize)
                                        .collect();
                                    
                                    let summary = filtered.iter()
                                        .map(|(timestamp, role, content, _)| format!("[{timestamp}] {role}: {content}"))
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    
                                    Ok(format!("Found {} conversations:\n{}", filtered.len(), summary))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        "daily" => {
                            match memory.get_summaries("daily", 30).await {
                                Ok(summaries) => {
                                    let filtered: Vec<(String, String, String)> = summaries.into_iter()
                                        .filter(|(_, summary, _)| query.is_empty() || summary.contains(&query))
                                        .take(limit as usize)
                                        .collect();
                                    
                                    let summary = filtered.iter()
                                        .map(|(date, summ, _)| format!("[{date}] {summ}"))
                                        .collect::<Vec<_>>()
                                        .join("\n\n");
                                    
                                    Ok(format!("Found {} daily summaries:\n{}", filtered.len(), summary))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        "monthly" => {
                            match memory.get_summaries("monthly", 12).await {
                                Ok(summaries) => {
                                    let filtered: Vec<(String, String, String)> = summaries.into_iter()
                                        .filter(|(_, summary, _)| query.is_empty() || summary.contains(&query))
                                        .take(limit as usize)
                                        .collect();
                                    
                                    let summary = filtered.iter()
                                        .map(|(date, summ, _)| format!("[{date}] {summ}"))
                                        .collect::<Vec<_>>()
                                        .join("\n\n");
                                    
                                    Ok(format!("Found {} monthly summaries:\n{}", filtered.len(), summary))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        "notes" => {
                            match memory.query_notes(limit as u32).await {
                                Ok(notes) => {
                                    let filtered_notes: Vec<(i64, String, String)> = notes.into_iter()
                                        .filter(|(_, content, _)| query.is_empty() || content.contains(&query))
                                        .collect();
                                    
                                    let summary = filtered_notes.iter()
                                        .map(|(id, content, tags_json)| {
                                            let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
                                            let tags_display = if tags.is_empty() {
                                                String::new()
                                            } else {
                                                format!(" [tags: {}]", tags.join(", "))
                                            };
                                            format!("#{id}{tags_display} {content}")
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n\n");
                                    
                                    Ok(format!("Found {} notes:\n{}", filtered_notes.len(), summary))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        _ => {
                            Err(anyhow::anyhow!("Unknown query type: {query_type}. Use: conversations, daily, monthly, notes"))
                        }
                    };

                    match result {
                        Ok(output) => ToolResult::success(output),
                        Err(e) => {
                            tracing::error!("Memory query failed: {}", e);
                            ToolResult::error(format!("Query failed: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn create_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "create_trigger".to_string(),
            description: "Create custom reminder/notification triggers. Use for meaningful, time-specific reminders. Examples: 'Remind me at 7 AM to review PRs', 'Check in every Monday at 9 AM'. Don't create triggers for things you can do immediately.".to_string(),
            tags: vec!["automation".to_string(), "trigger".to_string(), "reminder".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "name".to_string(),
                    description: "Unique trigger name (e.g., 'morning_pr_review')".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "schedule".to_string(),
                    description: "Schedule: 'HH:MM' for daily time, seconds for interval, or cron expression".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "type".to_string(),
                    description: "Trigger type: 'time' (daily HH:MM), 'interval' (seconds), 'cron' (cron expression), 'dynamic' (one-time)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "message".to_string(),
                    description: "Message to send when trigger fires (for custom triggers)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "open_chat".to_string(),
                    description: "'true' to open chat window when triggered, 'false' for notification only".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "urgency".to_string(),
                    description: "Notification urgency: 'low', 'normal', or 'critical'".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "tag".to_string(),
                    description: "Tag for categorization".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "description".to_string(),
                    description: "What this trigger should do".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = args.get("name").cloned().unwrap_or_default();
                    let schedule = args.get("schedule").cloned().unwrap_or_default();
                    let trigger_type = args.get("type").cloned().unwrap_or_else(|| "time".to_string());
                    let tag = args.get("tag").cloned();
                    let description = args.get("description").cloned();
                    
                    // Check if this is a custom trigger (has message)
                    let is_custom = args.contains_key("message");
                    
                    // Build metadata for custom triggers
                    let mut metadata = std::collections::HashMap::new();
                    if let Some(t) = tag {
                        metadata.insert("tag".to_string(), t);
                    }
                    if let Some(d) = description.clone() {
                        metadata.insert("description".to_string(), d);
                    }
                    
                    if is_custom {
                        // Custom trigger metadata
                        metadata.insert("analysis_type".to_string(), "custom".to_string());
                        if let Some(msg) = args.get("message") {
                            metadata.insert("message".to_string(), msg.clone());
                        }
                        if let Some(open_chat) = args.get("open_chat") {
                            metadata.insert("open_chat".to_string(), open_chat.clone());
                        }
                        if let Some(urgency) = args.get("urgency") {
                            metadata.insert("urgency".to_string(), urgency.clone());
                        }
                    }

                    info!("Creating trigger: {} ({}) at {} (custom: {})", name, trigger_type, schedule, is_custom);
                    
                    // Create trigger using database directly
                    let conn = match rusqlite::Connection::open(&trigger_registry.db_path) {
                        Ok(c) => c,
                        Err(e) => return ToolResult::error(format!("Database error: {e}")),
                    };
                    
                    let metadata_json = match serde_json::to_string(&metadata) {
                        Ok(j) => j,
                        Err(e) => return ToolResult::error(format!("Failed to serialize metadata: {e}")),
                    };
                    
                    if let Err(e) = conn.execute(
                        "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
                         VALUES (?1, ?2, ?3, 1, 'ai', ?4, datetime('now'))",
                        (&name, &trigger_type, &schedule, &metadata_json),
                    ) {
                        return ToolResult::error(format!("Failed to insert trigger: {e}"));
                    }
                    
                    if let Err(e) = trigger_registry.load_from_database().await {
                        return ToolResult::error(format!("Failed to reload triggers: {e}"));
                    }
                    
                    let desc_info = description.map(|d| format!(": {d}")).unwrap_or_default();
                    ToolResult::success(format!("Trigger '{name}' created successfully{desc_info}"))
                })
            }),
        }
    }

    pub fn analyze_now(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "analyze_now".to_string(),
            description: "Trigger an immediate analysis or compaction".to_string(),
            tags: vec!["analysis".to_string(), "immediate".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "type".to_string(),
                    description: "Analysis type: conversation, pattern, reflection, tool_effectiveness".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let analysis_type = args.get("type").cloned().unwrap_or_else(|| "conversation".to_string());

                    // Create an immediate dynamic trigger
                    let trigger_name = format!("analyze_now_{}", chrono::Utc::now().timestamp());
                    let metadata = HashMap::from([
                        ("analysis_type".to_string(), analysis_type.clone()),
                    ]);
                    
                    if let Err(e) = trigger_registry.add_dynamic_trigger(
                        trigger_name.clone(),
                        "Immediate analysis".to_string(),
                        vec!["analysis".to_string(), "immediate".to_string()],
                        metadata,
                    ).await {
                        return ToolResult::error(format!("Failed to create analysis trigger: {e}"));
                    }
                    
                    info!("Created immediate analysis trigger: {}", trigger_name);
                    ToolResult::success(format!("Analysis scheduled: {analysis_type}"))
                })
            }),
        }
    }

    pub fn open_chat(state: Arc<super::super::state::ServerState>) -> Tool {
        Tool {
            name: "open_chat".to_string(),
            description: "Open the chat GUI window and request user attention".to_string(),
            tags: vec!["ui".to_string(), "interaction".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "message".to_string(),
                    description: "Initial message to display".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "urgency".to_string(),
                    description: "Urgency level: low, normal, urgent".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let state = state.clone();
                Box::pin(async move {
                    let message = args.get("message").cloned();
                    
                    // Send to client daemon
                    let request = ritsu_common::protocol::ServerToClientRequest::OpenChat {
                        message: message.clone(),
                        session_id: None,
                    };

                    match state.send_to_client_daemon(request).await {
                        Ok(()) => {
                            info!("Chat window open requested via client daemon");
                            if let Some(msg) = &message {
                                info!("With message: {msg}");
                            }
                            ToolResult::success("Chat window opened".to_string())
                        }
                        Err(e) => {
                            warn!("Failed to open chat via client daemon: {}", e);
                            // Fallback to broadcast
                            let urgency = ritsu_common::protocol::NotificationUrgency::Normal;
                            state.broadcast_push(ritsu_common::protocol::ServerPush::OpenChat {
                                message: message.clone(),
                                urgency,
                            }).await;
                            ToolResult::success("Chat window opened (fallback)".to_string())
                        }
                    }
                })
            }),
        }
    }

    pub fn create_task(task_manager: Arc<super::super::tasks::TaskManager>) -> Tool {
        Tool {
            name: "create_task".to_string(),
            description: "Create a new task to track".to_string(),
            tags: vec!["task".to_string(), "productivity".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "title".to_string(),
                    description: "Task title".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "priority".to_string(),
                    description: "Priority: low, medium, high, urgent".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "due_date".to_string(),
                    description: "Due date (YYYY-MM-DD)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "description".to_string(),
                    description: "Task description".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let task_manager = task_manager.clone();
                Box::pin(async move {
                    let title = args.get("title").cloned().unwrap_or_default();
                    let priority_str = args.get("priority").cloned().unwrap_or_else(|| "medium".to_string());
                    let due_date = args.get("due_date").cloned();
                    let description = args.get("description").cloned();

                    // Parse priority
                    #[allow(clippy::match_same_arms)]
                    let priority = match priority_str.to_lowercase().as_str() {
                        "low" => ritsu_common::TaskPriority::Low,
                        "medium" => ritsu_common::TaskPriority::Medium,
                        "high" => ritsu_common::TaskPriority::High,
                        "urgent" => ritsu_common::TaskPriority::Urgent,
                        _ => ritsu_common::TaskPriority::Medium,
                    };

                    info!("Creating task: {} (priority: {})", title, priority_str);
                    
                    match task_manager.create_task(
                        &title,
                        description.as_deref(),
                        &priority,
                        &[], // no tags from tool
                        due_date.as_deref()
                    ).await {
                        Ok(task_id) => {
                            let due_info = due_date.map(|d| format!(" (due: {d})")).unwrap_or_default();
                            ToolResult::success(format!("Task created with ID {task_id}: {title}{due_info}"))
                        }
                        Err(e) => {
                            tracing::error!("Failed to create task: {}", e);
                            ToolResult::error(format!("Failed to create task: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn update_task(task_manager: Arc<super::super::tasks::TaskManager>) -> Tool {
        Tool {
            name: "update_task".to_string(),
            description: "Update an existing task's status or priority".to_string(),
            tags: vec!["task".to_string(), "productivity".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "id".to_string(),
                    description: "Task ID".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "status".to_string(),
                    description: "New status: pending, in_progress, completed".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "priority".to_string(),
                    description: "New priority: low, medium, high".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let task_manager = task_manager.clone();
                Box::pin(async move {
                    let id_str = args.get("id").cloned().unwrap_or_default();
                    let status = args.get("status").cloned();
                    let priority = args.get("priority").cloned();

                    // Parse task ID
                    let Ok(id) = id_str.parse::<i64>() else {
                        return ToolResult::error(format!("Invalid task ID: {id_str}"));
                    };

                    info!("Updating task {}", id);
                    
                    let mut updates = Vec::new();
                    
                    if let Some(s) = status {
                        let task_status = match s.to_lowercase().as_str() {
                            "pending" => ritsu_common::TaskStatus::Pending,
                            "in_progress" | "inprogress" | "progress" => ritsu_common::TaskStatus::InProgress,
                            "completed" | "done" => ritsu_common::TaskStatus::Completed,
                            "cancelled" | "canceled" => ritsu_common::TaskStatus::Cancelled,
                            _ => return ToolResult::error(format!("Invalid status: {s}")),
                        };
                        
                        match task_manager.update_task_status(id, &task_status).await {
                            Ok(()) => {
                                info!("Updated status to: {:?}", task_status);
                                updates.push(format!("status → {task_status:?}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to update status: {}", e);
                                return ToolResult::error(format!("Failed to update status: {e}"));
                            }
                        }
                    }
                    
                    if let Some(p) = priority {
                        let task_priority = match p.to_lowercase().as_str() {
                            "low" => ritsu_common::TaskPriority::Low,
                            "medium" => ritsu_common::TaskPriority::Medium,
                            "high" => ritsu_common::TaskPriority::High,
                            "urgent" => ritsu_common::TaskPriority::Urgent,
                            _ => return ToolResult::error(format!("Invalid priority: {p}")),
                        };
                        
                        match task_manager.update_task_priority(id, &task_priority).await {
                            Ok(()) => {
                                info!("Updated priority to: {:?}", task_priority);
                                updates.push(format!("priority → {task_priority:?}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to update priority: {}", e);
                                return ToolResult::error(format!("Failed to update priority: {e}"));
                            }
                        }
                    }

                    if updates.is_empty() {
                        ToolResult::error("No updates provided (need status or priority)".to_string())
                    } else {
                        ToolResult::success(format!("Task {} updated: {}", id, updates.join(", ")))
                    }
                })
            }),
        }
    }

    pub fn list_tasks(task_manager: Arc<super::super::tasks::TaskManager>) -> Tool {
        Tool {
            name: "list_tasks".to_string(),
            description: "List tasks with optional filters".to_string(),
            tags: vec!["task".to_string(), "productivity".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "status".to_string(),
                    description: "Filter by status: pending, in_progress, completed".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "priority".to_string(),
                    description: "Filter by priority: low, medium, high".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let task_manager = task_manager.clone();
                Box::pin(async move {
                    let status = args.get("status").cloned();
                    let priority = args.get("priority").cloned();

                    info!("Listing tasks - status: {:?}, priority: {:?}", status, priority);
                    
                    match task_manager.list_tasks(status.as_deref(), priority.as_deref()).await {
                        Ok(tasks) => {
                            if tasks.is_empty() {
                                ToolResult::success("No tasks found matching the filters".to_string())
                            } else {
                                let summary = tasks.iter()
                                    .map(|t| {
                                        let due_info = t.due_date.as_ref()
                                            .map(|d| format!(" (due: {d})"))
                                            .unwrap_or_default();
                                        format!("#{} [{:?}] [{:?}] {}{}", 
                                            t.id, t.status, t.priority, t.title, due_info)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                
                                ToolResult::success(format!("Found {} tasks:\n{}", tasks.len(), summary))
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to list tasks: {}", e);
                            ToolResult::error(format!("Failed to list tasks: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn set_preference(preferences: Arc<super::super::preferences::PreferencesManager>) -> Tool {
        Tool {
            name: "set_preference".to_string(),
            description: "Remember a user preference for future reference".to_string(),
            tags: vec!["preferences".to_string(), "memory".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "category".to_string(),
                    description: "Preference category (e.g., schedule, communication, work)".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "key".to_string(),
                    description: "Preference key (e.g., wake_time, notification_style)".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "value".to_string(),
                    description: "Preference value".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let preferences = preferences.clone();
                Box::pin(async move {
                    let category = args.get("category").cloned().unwrap_or_default();
                    let key = args.get("key").cloned().unwrap_or_default();
                    let value = args.get("value").cloned().unwrap_or_default();

                    match preferences.set_preference(&category, &key, &value, 1.0, Some("user")).await {
                        Ok(()) => {
                            info!("Set preference: {}/{} = {}", category, key, value);
                            ToolResult::success(format!("Preference saved: {category} / {key} = {value}"))
                        }
                        Err(e) => {
                            tracing::error!("Failed to set preference: {}", e);
                            ToolResult::error(format!("Failed to save preference: {e}"))
                        }
                    }
                })
            }),
        }
    }
}

use crate::memory::MemoryManager;
use crate::preferences::PreferencesManager;
use crate::state::ServerState;
use crate::tasks::TaskManager;
use crate::trigger::TriggerRegistry as TriggerReg;

pub async fn register_all_tools(
    registry: &ToolRegistry,
    memory: Arc<MemoryManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerReg>,
    state: Arc<ServerState>,
    preferences: Arc<PreferencesManager>,
) {
    registry.register(tool_impls::notify_client(state.clone())).await;
    registry.register(tool_impls::create_note(memory.clone())).await;
    registry.register(tool_impls::query_memory(memory.clone())).await;
    registry.register(tool_impls::create_trigger(trigger_registry.clone())).await;
    registry.register(tool_impls::analyze_now(trigger_registry.clone())).await;
    registry.register(tool_impls::open_chat(state.clone())).await;
    registry.register(tool_impls::create_task(task_manager.clone())).await;
    registry.register(tool_impls::update_task(task_manager.clone())).await;
    registry.register(tool_impls::list_tasks(task_manager.clone())).await;
    registry.register(tool_impls::set_preference(preferences.clone())).await;
    
    info!("Registered {} tools", 10);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_registration() {
        let registry = ToolRegistry::new();
        
        let tool = Tool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            tags: vec!["test".to_string()],
            parameters: vec![],
            handler: Arc::new(|_args| {
                Box::pin(async { ToolResult::success("test".to_string()) })
            }),
        };
        
        registry.register(tool).await;
        
        let count = registry.tool_count().await;
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_tool_execution() {
        let registry = ToolRegistry::new();
        
        let tool = Tool {
            name: "echo".to_string(),
            description: "Echo tool".to_string(),
            tags: vec![],
            parameters: vec![],
            handler: Arc::new(|args| {
                Box::pin(async move {
                    let msg = args.get("message").cloned().unwrap_or_default();
                    ToolResult::success(msg)
                })
            }),
        };
        
        registry.register(tool).await;
        
        let mut args = HashMap::new();
        args.insert("message".to_string(), "hello".to_string());
        
        let result = registry.execute("echo", args).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    #[tokio::test]
    async fn test_tool_not_found() {
        let registry = ToolRegistry::new();
        
        let result = registry.execute("nonexistent", HashMap::new()).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_tool_result_creation() {
        let success = ToolResult::success("worked".to_string());
        assert!(success.success);
        assert_eq!(success.output, "worked");
        
        let error = ToolResult::error("failed".to_string());
        assert!(!error.success);
        assert_eq!(error.error, Some("failed".to_string()));
    }
}
