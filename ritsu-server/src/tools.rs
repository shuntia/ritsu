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

    pub fn create_note() -> Tool {
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
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let content = args.get("content").cloned().unwrap_or_default();
                    let tags = args.get("tags").cloned().unwrap_or_default();

                    // TODO: Store note in database
                    info!("Creating note with tags: {tags}");
                    
                    ToolResult::success(format!("Note created: {content}"))
                })
            }),
        }
    }

    pub fn query_memory() -> Tool {
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
            ],
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let query_type = args.get("type").cloned().unwrap_or_default();
                    let query = args.get("query").cloned().unwrap_or_default();

                    // TODO: Query memory from database
                    info!("Querying memory: type={query_type}, query={query}");
                    
                    ToolResult::success("Memory query results (placeholder)".to_string())
                })
            }),
        }
    }

    pub fn create_trigger() -> Tool {
        Tool {
            name: "create_trigger".to_string(),
            description: "Create a new time-based or event-based trigger".to_string(),
            tags: vec!["automation".to_string(), "trigger".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "name".to_string(),
                    description: "Trigger name".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "schedule".to_string(),
                    description: "Cron-like schedule or event type".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "type".to_string(),
                    description: "Trigger type: time, event, idle_analysis".to_string(),
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
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let name = args.get("name").cloned().unwrap_or_default();
                    let schedule = args.get("schedule").cloned().unwrap_or_default();
                    let trigger_type = args.get("type").cloned().unwrap_or_else(|| "time".to_string());

                    // TODO: Create trigger in database and register with trigger system
                    info!("Creating trigger: {name} ({trigger_type}) at {schedule}");
                    
                    ToolResult::success(format!("Trigger created: {name}"))
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

    pub fn create_task() -> Tool {
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
                    description: "Priority: low, medium, high".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "due_date".to_string(),
                    description: "Due date (YYYY-MM-DD)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let title = args.get("title").cloned().unwrap_or_default();
                    let priority = args.get("priority").cloned().unwrap_or_else(|| "medium".to_string());
                    let due_date = args.get("due_date").cloned();

                    // TODO: Create task in database
                    info!("Creating task: {title} (priority: {priority})");
                    if let Some(due) = due_date {
                        info!("Due date: {due}");
                    }
                    
                    ToolResult::success(format!("Task created: {title}"))
                })
            }),
        }
    }

    pub fn update_task() -> Tool {
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
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let id = args.get("id").cloned().unwrap_or_default();
                    let status = args.get("status").cloned();
                    let priority = args.get("priority").cloned();

                    // TODO: Update task in database
                    info!("Updating task {id}");
                    if let Some(s) = status {
                        info!("New status: {s}");
                    }
                    if let Some(p) = priority {
                        info!("New priority: {p}");
                    }
                    
                    ToolResult::success(format!("Task {id} updated"))
                })
            }),
        }
    }

    pub fn list_tasks() -> Tool {
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
            handler: Arc::new(|args: HashMap<String, String>| {
                Box::pin(async move {
                    let status = args.get("status").cloned();
                    let priority = args.get("priority").cloned();

                    // TODO: Query tasks from database
                    info!("Listing tasks with filters - status: {status:?}, priority: {priority:?}");
                    
                    ToolResult::success("Task list (placeholder)".to_string())
                })
            }),
        }
    }
}

pub async fn register_all_tools(registry: &ToolRegistry) {
    registry.register(tool_impls::notify_client()).await;
    registry.register(tool_impls::create_note()).await;
    registry.register(tool_impls::query_memory()).await;
    registry.register(tool_impls::create_trigger()).await;
    registry.register(tool_impls::analyze_now()).await;
    registry.register(tool_impls::open_chat()).await;
    registry.register(tool_impls::create_task()).await;
    registry.register(tool_impls::update_task()).await;
    registry.register(tool_impls::list_tasks()).await;
    
    info!("Registered {} tools", 9);
}
