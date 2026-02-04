//! Tool system - dynamic tool registry and execution

use anyhow::Result;
use ritsu_common::ToolResult;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

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
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, tool: Tool) {
        let name = tool.name.clone();
        self.tools.write().await.insert(name.clone(), tool);
        info!("Registered tool: {name}");
    }

    #[allow(dead_code)]
    pub async fn execute(&self, name: &str, args: HashMap<String, String>) -> Result<ToolResult> {
        let tools = self.tools.read().await;
        
        if let Some(tool) = tools.get(name) {
            let result = (tool.handler)(args).await;
            Ok(result)
        } else {
            Ok(ToolResult::error(format!("Tool '{name}' not found")))
        }
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
    use tracing::info;
    
    use super::{Tool, ToolParameter};
    use ritsu_common::ToolResult;

    pub fn notify_client() -> Tool {
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
                    description: "Urgency level: low, normal, high".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let title = args.get("title").cloned().unwrap_or_default();
                    let message = args.get("message").cloned().unwrap_or_default();
                    let urgency = args.get("urgency").cloned().unwrap_or_else(|| "normal".to_string());

                    // TODO: Send notification via IPC to connected clients
                    info!("Notification: [{urgency}] {title}: {message}");
                    
                    ToolResult::success(format!("Notification sent: {title}"))
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
                            ToolResult::error(format!("Failed to create note: {}", e))
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
                                        .map(|(timestamp, role, content, _)| format!("[{}] {}: {}", timestamp, role, content))
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
                                        .map(|(date, summ, _)| format!("[{}] {}", date, summ))
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
                                        .map(|(date, summ, _)| format!("[{}] {}", date, summ))
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
                                            let tags_display = if !tags.is_empty() {
                                                format!(" [tags: {}]", tags.join(", "))
                                            } else {
                                                String::new()
                                            };
                                            format!("#{}{} {}", id, tags_display, content)
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n\n");
                                    
                                    Ok(format!("Found {} notes:\n{}", filtered_notes.len(), summary))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        _ => {
                            Err(anyhow::anyhow!("Unknown query type: {}. Use: conversations, daily, monthly, notes", query_type))
                        }
                    };

                    match result {
                        Ok(output) => ToolResult::success(output),
                        Err(e) => {
                            tracing::error!("Memory query failed: {}", e);
                            ToolResult::error(format!("Query failed: {}", e))
                        }
                    }
                })
            }),
        }
    }

    pub fn create_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "create_trigger".to_string(),
            description: "Create a new time-based or event-based trigger for automation".to_string(),
            tags: vec!["automation".to_string(), "trigger".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "name".to_string(),
                    description: "Unique trigger name".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "schedule".to_string(),
                    description: "Schedule: 'HH:MM' for time, seconds for interval/inactivity".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "type".to_string(),
                    description: "Trigger type: time, interval, inactivity, dynamic".to_string(),
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

                    info!("Creating trigger: {} ({}) at {}", name, trigger_type, schedule);
                    
                    match trigger_registry.create_trigger(
                        &name,
                        &trigger_type,
                        &schedule,
                        tag.as_deref(),
                        description.as_deref(),
                    ).await {
                        Ok(_) => {
                            let desc_info = description.map(|d| format!(": {}", d)).unwrap_or_default();
                            ToolResult::success(format!("Trigger '{}' created successfully{}", name, desc_info))
                        }
                        Err(e) => {
                            tracing::error!("Failed to create trigger: {}", e);
                            ToolResult::error(format!("Failed to create trigger: {}", e))
                        }
                    }
                })
            }),
        }
    }

    pub fn analyze_now() -> Tool {
        Tool {
            name: "analyze_now".to_string(),
            description: "Trigger an immediate analysis or compaction".to_string(),
            tags: vec!["analysis".to_string(), "immediate".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "type".to_string(),
                    description: "Analysis type: daily, pattern, reflection".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let analysis_type = args.get("type").cloned().unwrap_or_else(|| "dynamic".to_string());

                    // TODO: Trigger idle analysis
                    info!("Triggering idle analysis: {analysis_type}");
                    
                    ToolResult::success(format!("Analysis triggered: {analysis_type}"))
                })
            }),
        }
    }

    pub fn open_chat() -> Tool {
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
                    description: "Urgency level: low, normal, high".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let message = args.get("message").cloned();
                    let urgency = args.get("urgency").cloned().unwrap_or_else(|| "normal".to_string());

                    // TODO: Launch GUI via `ritsu chat` command
                    info!("Opening chat with urgency: {urgency}");
                    if let Some(msg) = message {
                        info!("With message: {msg}");
                    }
                    
                    ToolResult::success("Chat window opened".to_string())
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
                            let due_info = due_date.map(|d| format!(" (due: {})", d)).unwrap_or_default();
                            ToolResult::success(format!("Task created with ID {}: {}{}", task_id, title, due_info))
                        }
                        Err(e) => {
                            tracing::error!("Failed to create task: {}", e);
                            ToolResult::error(format!("Failed to create task: {}", e))
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
                    let id = match id_str.parse::<i64>() {
                        Ok(id) => id,
                        Err(_) => return ToolResult::error(format!("Invalid task ID: {}", id_str)),
                    };

                    info!("Updating task {}", id);
                    
                    let mut updates = Vec::new();
                    
                    if let Some(s) = status {
                        let task_status = match s.to_lowercase().as_str() {
                            "pending" => ritsu_common::TaskStatus::Pending,
                            "in_progress" | "inprogress" | "progress" => ritsu_common::TaskStatus::InProgress,
                            "completed" | "done" => ritsu_common::TaskStatus::Completed,
                            "cancelled" | "canceled" => ritsu_common::TaskStatus::Cancelled,
                            _ => return ToolResult::error(format!("Invalid status: {}", s)),
                        };
                        
                        match task_manager.update_task_status(id, &task_status).await {
                            Ok(_) => {
                                info!("Updated status to: {:?}", task_status);
                                updates.push(format!("status → {:?}", task_status));
                            }
                            Err(e) => {
                                tracing::error!("Failed to update status: {}", e);
                                return ToolResult::error(format!("Failed to update status: {}", e));
                            }
                        }
                    }
                    
                    if let Some(p) = priority {
                        let task_priority = match p.to_lowercase().as_str() {
                            "low" => ritsu_common::TaskPriority::Low,
                            "medium" => ritsu_common::TaskPriority::Medium,
                            "high" => ritsu_common::TaskPriority::High,
                            "urgent" => ritsu_common::TaskPriority::Urgent,
                            _ => return ToolResult::error(format!("Invalid priority: {}", p)),
                        };
                        
                        match task_manager.update_task_priority(id, &task_priority).await {
                            Ok(_) => {
                                info!("Updated priority to: {:?}", task_priority);
                                updates.push(format!("priority → {:?}", task_priority));
                            }
                            Err(e) => {
                                tracing::error!("Failed to update priority: {}", e);
                                return ToolResult::error(format!("Failed to update priority: {}", e));
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
                                            .map(|d| format!(" (due: {})", d))
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
                            ToolResult::error(format!("Failed to list tasks: {}", e))
                        }
                    }
                })
            }),
        }
    }
}

use crate::memory::MemoryManager;
use crate::tasks::TaskManager;
use crate::trigger::TriggerRegistry as TriggerReg;

pub async fn register_all_tools(
    registry: &ToolRegistry,
    memory: Arc<MemoryManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerReg>,
) {
    registry.register(tool_impls::notify_client()).await;
    registry.register(tool_impls::create_note(memory.clone())).await;
    registry.register(tool_impls::query_memory(memory.clone())).await;
    registry.register(tool_impls::create_trigger(trigger_registry.clone())).await;
    registry.register(tool_impls::analyze_now()).await;
    registry.register(tool_impls::open_chat()).await;
    registry.register(tool_impls::create_task(task_manager.clone())).await;
    registry.register(tool_impls::update_task(task_manager.clone())).await;
    registry.register(tool_impls::list_tasks(task_manager.clone())).await;
    
    info!("Registered {} tools", 9);
}
