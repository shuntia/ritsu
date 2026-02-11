//! Tool system - dynamic tool registry and execution

use anyhow::Result;
use ritsu_common::ToolResult;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_rusqlite::rusqlite;
use tracing::{debug, info, warn};

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
    db_conn: Option<Arc<tokio_rusqlite::Connection>>,
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
    pub fn with_database(mut self, conn: Arc<tokio_rusqlite::Connection>) -> Self {
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
        // Summarize args: keys and lengths to avoid logging sensitive values
        let args_summary: Vec<(String, usize)> =
            args.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        debug!(tool = %name, args_keys = ?args.keys().cloned().collect::<Vec<_>>(), args_summary = ?args_summary, "ToolRegistry.execute called");

        // Clone the handler to avoid holding lock across await
        let handler = {
            let tools = self.tools.read().await;
            let h = tools.get(name).map(|tool| tool.handler.clone());
            debug!(tool = %name, has_handler = %h.is_some(), "Handler lookup result");
            h
        };

        let result = if let Some(handler) = handler {
            handler(args.clone()).await
        } else {
            ToolResult::error(format!("Tool '{name}' not found"))
        };

        let execution_time = start.elapsed().as_millis() as i64;
        debug!(tool = %name, execution_time_ms = %execution_time, success = ?result.success, output_len = %result.output.len(), "Tool handler completed");

        // Log tool usage to database
        if let Some(db) = &self.db_conn {
            let name_clone = name.to_string();
            let args_json = match serde_json::to_string(&args) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Failed to serialize tool args for DB logging");
                    String::new()
                }
            };
            let result_str = result.output.clone();
            let success = result.success;

            debug!(tool = %name, "Scheduling background DB log for tool usage");

            let db_clone = db.clone();
            tokio::spawn(async move {
                debug!(tool = %name_clone, "Performing DB log for tool usage");
                if let Err(e) = Self::log_tool_usage(
                    &db_clone,
                    &name_clone,
                    &args_json,
                    success,
                    &result_str,
                    execution_time,
                )
                .await
                {
                    warn!("Failed to log tool usage: {}", e);
                } else {
                    debug!(tool = %name_clone, "DB log completed for tool usage");
                }
            });
        }

        debug!(tool = %name, "Returning tool result");
        Ok(result)
    }

    async fn log_tool_usage(
        db: &Arc<tokio_rusqlite::Connection>,
        tool_name: &str,
        args: &str,
        success: bool,
        result: &str,
        execution_time_ms: i64,
    ) -> Result<()> {
        let tool_name = tool_name.to_string();
        let args = args.to_string();
        let result = result.to_string();

        db.call(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO tool_usage (tool_name, arguments, success, result, execution_time_ms, triggered_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ai')",
                (&tool_name, &args, success, &result, execution_time_ms),
            )?;
            Ok(())
        }).await.map_err(|e| anyhow::anyhow!("DB error: {e}"))?;
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
        tools
            .values()
            .map(|tool| ToolInfo {
                name: tool.name.clone(),
                description: tool.description.clone(),
                tags: tool.tags.clone(),
                parameters: tool
                    .parameters
                    .iter()
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
            description: r#"Send a notification to the user's desktop/client (preferred: client daemon). Falls back to broadcasting a push notification if the client daemon is unavailable.

Parameters:
- title (string, required): Notification title.
- message (string, required): Notification message body.
- urgency (string, optional): 'low', 'normal', 'urgent'/'critical' (defaults to 'normal').

Behavior:
Validates required parameters, maps urgency to NotificationUrgency, sends ServerToClientRequest::NotifyUser to the client daemon via state.send_to_client_daemon. On failure, falls back to state.broadcast_push(ServerPush::Notification).

Return:
ToolResult::success("Notification sent: <title>") on success; ToolResult::error on missing params or unrecoverable errors.

Example args: { "title": "Reminder", "message": "Stand up break", "urgency": "low" }"#.to_string(),
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
                    let title = match args.get("title") {
                        Some(v) if !v.trim().is_empty() => v.clone(),
                        _ => return ToolResult::error("Missing required parameter: title".to_string()),
                    };
                    let message = match args.get("message") {
                        Some(v) if !v.trim().is_empty() => v.clone(),
                        _ => return ToolResult::error("Missing required parameter: message".to_string()),
                    };
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
            description: r#"Create a persistent note for future reference (stored via MemoryManager).

Parameters:
- content (string, required): Note content.
- tags (string, optional): Comma-separated tags (e.g., "work,meeting").

Behavior:
Validates content, parses tags into Vec<String>, and calls memory.create_note(&content, &tags).await to persist the note.

Return:
ToolResult::success("Note created successfully with N tags") on success; ToolResult::error on failure.

Example args: { "content": "Met with Alice about roadmap", "tags": "meeting,roadmap" }"#.to_string(),
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
                    let content = match args.get("content") {
                        Some(c) if !c.trim().is_empty() => c.clone(),
                        _ => return ToolResult::error("Missing required parameter: content".to_string()),
                    };
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

    pub fn edit_note(memory: Arc<super::super::memory::MemoryManager>) -> Tool {
        Tool {
            name: "edit_note".to_string(),
            description: r#"Edit an existing note by id.

Parameters:
- id (string, required): Note ID to edit.
- content (string, optional): New content.
- tags (string, optional): Comma-separated tags to replace existing tags.

Behavior:
At least one of content or tags must be provided. Calls MemoryManager.update_note(id, content_opt, tags_opt) to persist changes.

Return:
ToolResult::success("Note <id> updated") or ToolResult::error on failure."#.to_string(),
            tags: vec!["memory".to_string(), "note".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "id".to_string(),
                    description: "Note ID".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "content".to_string(),
                    description: "New content".to_string(),
                    required: false,
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
                    let id_str = match args.get("id") {
                        Some(i) if !i.trim().is_empty() => i.clone(),
                        _ => return ToolResult::error("Missing required parameter: id".to_string()),
                    };
                    let id = match id_str.parse::<i64>() {
                        Ok(v) => v,
                        Err(_) => return ToolResult::error(format!("Invalid id: {}", id_str)),
                    };

                    let content_opt = args.get("content").cloned();
                    let tags_opt = args.get("tags").cloned().map(|s| {
                        s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>()
                    });

                    if content_opt.is_none() && tags_opt.is_none() {
                        return ToolResult::error("No updates provided (need content or tags)".to_string());
                    }

                    match memory.update_note(id, content_opt, tags_opt).await {
                        Ok(()) => ToolResult::success(format!("Note {} updated", id)),
                        Err(e) => {
                            tracing::error!("Failed to update note: {}", e);
                            ToolResult::error(format!("Failed to update note: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn delete_note(memory: Arc<super::super::memory::MemoryManager>) -> Tool {
        Tool {
            name: "delete_note".to_string(),
            description: r#"Delete a note by ID."#.to_string(),
            tags: vec!["memory".to_string(), "note".to_string()],
            parameters: vec![ToolParameter {
                name: "id".to_string(),
                description: "Note ID".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let memory = memory.clone();
                Box::pin(async move {
                    let id_str = match args.get("id") {
                        Some(i) if !i.trim().is_empty() => i.clone(),
                        _ => {
                            return ToolResult::error("Missing required parameter: id".to_string())
                        }
                    };
                    let id = match id_str.parse::<i64>() {
                        Ok(v) => v,
                        Err(_) => return ToolResult::error(format!("Invalid id: {}", id_str)),
                    };

                    match memory.delete_note(id).await {
                        Ok(()) => ToolResult::success(format!("Note {} deleted", id)),
                        Err(e) => {
                            tracing::error!("Failed to delete note: {}", e);
                            ToolResult::error(format!("Failed to delete note: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn query_memory(memory: Arc<super::super::memory::MemoryManager>) -> Tool {
        Tool {
            name: "query_memory".to_string(),
            description: r#"Query past conversations, daily/monthly summaries, or notes from MemoryManager.

Parameters:
- type (string, required): One of `conversations`, `daily`, `monthly`, `notes`.
- query (string, optional): Substring filter applied to content/summaries.
- limit (string, optional): Maximum number of results (defaults to 10).

Behavior:
Dispatches to appropriate MemoryManager methods depending on `type` and returns a formatted textual response listing matched items.

Return:
ToolResult::success(formatted_text) or ToolResult::error on unknown type or other failures.

Example args: { "type": "notes", "query": "roadmap", "limit": "5" }"#.to_string(),
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
                    let query_type = match args.get("type") {
                        Some(t) if !t.trim().is_empty() => t.clone(),
                        _ => return ToolResult::error("Missing required parameter: type".to_string()),
                    };
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
                                            let tags: Vec<String> = match serde_json::from_str(tags_json) {
                                                Ok(t) => t,
                                                Err(e) => {
                                                    warn!(error = %e, "Failed to parse tags JSON for note id {} - using empty tags", id);
                                                    Vec::new()
                                                }
                                            };
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
            description: r#"Create custom reminder/notification triggers. Intended for meaningful, time-specific reminders; avoid creating triggers for trivial immediate actions.

Parameters:
- name (string, required): Unique trigger name (e.g., 'morning_pr_review').
- schedule (string, required): Schedule format - 'HH:MM' for daily, a number of seconds (interval), or a cron expression. Cron expressions are expected in the canonical six-field form with a leading seconds field: 's m H D M *' (e.g. '0 30 08 10 2 *' for 2026-02-10 08:30 UTC). Five-field cron (minute hour day month day-of-week) is still accepted by the parser but canonical one-shot triggers use the six-field form.
- type (string, optional): 'time', 'interval', 'cron', 'dynamic' (default: 'time').
- note (string, optional): Instructional note describing what AI should do when the trigger fires.

- urgency (string, optional): 'low', 'normal', 'critical'.
- tag (string, optional): Tag for categorization.
- description (string, optional): Short description of the trigger.

Behavior:
Validates 'name' and 'schedule', builds JSON metadata from optional parameters (note, urgency, tag, description), inserts a row into the 'triggers' database table as created_by='ai', reloads triggers, and notifies the trigger loop to reschedule (trigger_registry.notify_changed()).

Return:
ToolResult::success on success or ToolResult::error on DB/migration errors.

Example args: { "name": "standup_reminder", "schedule": "09:00", "type": "time", "note": "Send notification and open chat if no response" }"#.to_string(),
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
                    name: "note".to_string(),
                    description: "Instructional note describing what the AI should do when the trigger fires (e.g. 'Send a desktop notification; if no user response within 60s, open chat and tell them to get it together')".to_string(),
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
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => return ToolResult::error("Missing required parameter: name".to_string()),
                    };
                    let schedule = match args.get("schedule") {
                        Some(s) if !s.trim().is_empty() => s.clone(),
                        _ => return ToolResult::error("Missing required parameter: schedule".to_string()),
                    };
                    let trigger_type = args.get("type").cloned().unwrap_or_else(|| "time".to_string());
                    let tag = args.get("tag").cloned();
                    let description = args.get("description").cloned();

                    // Check if this is a custom trigger (has note)
                    let is_custom = args.contains_key("note");

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
                        if let Some(note) = args.get("note") {
                            metadata.insert("note".to_string(), note.clone());
                        }

                        if let Some(urgency) = args.get("urgency") {
                            metadata.insert("urgency".to_string(), urgency.clone());
                        }
                    }

                    info!("Creating trigger: {} ({}) at {} (custom: {})", name, trigger_type, schedule, is_custom);

                    let metadata_json = match serde_json::to_string(&metadata) {
                        Ok(j) => j,
                        Err(e) => return ToolResult::error(format!("Failed to serialize metadata: {e}")),
                    };

                    // Create trigger using spawn_blocking
                    let db_path = trigger_registry.db_path.clone();
                    let name_clone = name.clone();
                    let trigger_type_clone = trigger_type.clone();
                    let schedule_clone = schedule.clone();
                    let metadata_json_clone = metadata_json.clone();

                    if let Err(e) = crate::database::Database::execute_blocking(db_path, move |conn| {
                        conn.execute(
                            "INSERT INTO triggers (name, trigger_type, schedule, enabled, created_by, metadata, created_at)
                             VALUES (?1, ?2, ?3, 1, 'ai', ?4, datetime('now'))",
                            (&name_clone, &trigger_type_clone, &schedule_clone, &metadata_json_clone),
                        )?;
                        Ok(())
                    }).await {
                        return ToolResult::error(format!("Failed to insert trigger: {e}"));
                    }

                    if let Err(e) = trigger_registry.load_from_database().await {
                        return ToolResult::error(format!("Failed to reload triggers: {e}"));
                    }

                    // Notify trigger loop to wake up and reschedule
                    trigger_registry.notify_changed();

                    let desc_info = description.map(|d| format!(": {d}")).unwrap_or_default();
                    ToolResult::success(format!("Trigger '{name}' created successfully{desc_info}"))
                })
            }),
        }
    }

    pub fn create_one_shot(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "create_one_shot".to_string(),
            description: r#"Create a one-shot trigger at an exact ISO8601 datetime (e.g. 2026-02-10T08:30:00Z). The tool validates that the generated cron expression will fire exactly at the requested minute; otherwise it fails. The internal (canonical) cron format used for one-shot triggers includes a seconds field: 's m H D M *' (e.g. '0 30 08 10 2 *').

Parameters:
- name (string, required): Unique trigger name.
- datetime (string, required): ISO8601 datetime when the trigger should fire (UTC-aware, e.g., 2026-02-10T08:30:00Z).
- tags (string, optional): Comma-separated tags.
- description (string, optional): Description.

Behavior:
Parses the provided datetime, builds a cron expression including day and month, validates the cron's next occurrence matches the requested minute, and inserts a one-shot cron trigger into the DB.
"#.to_string(),
            tags: vec!["trigger".to_string(), "one_shot".to_string()],
            parameters: vec![
                ToolParameter { name: "name".to_string(), description: "Trigger name".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "datetime".to_string(), description: "ISO8601 datetime (UTC-aware)".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "tags".to_string(), description: "Comma-separated tags".to_string(), required: false, param_type: "string".to_string() },
                ToolParameter { name: "description".to_string(), description: "Description".to_string(), required: false, param_type: "string".to_string() },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => return ToolResult::error("Missing required parameter: name".to_string()),
                    };
                    let datetime = match args.get("datetime") {
                        Some(d) if !d.trim().is_empty() => d.clone(),
                        _ => return ToolResult::error("Missing required parameter: datetime".to_string()),
                    };
                    let tags_str = args.get("tags").cloned().unwrap_or_default();
                    let tags: Vec<String> = tags_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    let description = args.get("description").cloned();

                    match trigger_registry.create_one_shot_trigger(&name, &datetime, tags, description.as_deref()).await {
                        Ok(()) => ToolResult::success(format!("One-shot trigger '{}' created for {}", name, datetime)),
                        Err(e) => {
                            tracing::error!("Failed to create one-shot trigger: {}", e);
                            ToolResult::error(format!("Failed to create one-shot trigger: {}", e))
                        }
                    }
                })
            }),
        }
    }

    pub fn edit_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "edit_trigger".to_string(),
            description: r#"Edit an existing trigger's properties.

Parameters:
- name (string, required): Current trigger name.
- new_name (string, optional): New name.
- schedule (string, optional): New schedule.
- type (string, optional): New type.
- tag (string, optional): Tag.
- description (string, optional): Description.
- enabled (string, optional): 'true' or 'false'.
- any other metadata keys will be merged into the trigger's metadata.

Behavior:
Applies provided updates to the trigger and reloads triggers.

Return:
ToolResult::success or ToolResult::error."#
                .to_string(),
            tags: vec!["trigger".to_string(), "automation".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "name".to_string(),
                    description: "Current trigger name".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "new_name".to_string(),
                    description: "New trigger name".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "schedule".to_string(),
                    description: "New schedule (HH:MM, seconds, or cron)".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "type".to_string(),
                    description: "Trigger type: time, interval, cron, dynamic".to_string(),
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
                    description: "Description of the trigger".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "enabled".to_string(),
                    description: "Enable or disable: true/false".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: name".to_string(),
                            )
                        }
                    };

                    // Build updates map excluding the 'name' key
                    let mut updates: HashMap<String, String> = HashMap::new();
                    for (k, v) in args.iter() {
                        if k == "name" {
                            continue;
                        }
                        updates.insert(k.clone(), v.clone());
                    }

                    if updates.is_empty() {
                        return ToolResult::error("No updates provided for trigger".to_string());
                    }

                    match trigger_registry.update_trigger(&name, updates).await {
                        Ok(()) => ToolResult::success(format!("Trigger '{}' updated", name)),
                        Err(e) => {
                            tracing::error!("Failed to update trigger: {}", e);
                            ToolResult::error(format!("Failed to update trigger: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn list_triggers(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "list_triggers".to_string(),
            description: r#"List all registered triggers with basic metadata."#.to_string(),
            tags: vec!["trigger".to_string(), "automation".to_string()],
            parameters: vec![],
            handler: Arc::new(move |_args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let triggers = trigger_registry.get_all_triggers().await;
                    if triggers.is_empty() {
                        return ToolResult::success("No triggers registered".to_string());
                    }
                    let summary = triggers
                        .iter()
                        .map(|t| {
                            let ttype = match &t.trigger_type {
                                crate::trigger::TriggerType::Cron(s) => format!("cron({})", s),
                            };
                            format!("#{}: {} ({}) [enabled: {}]", t.id, t.name, ttype, t.enabled)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult::success(format!("Found {} triggers:\n{}", triggers.len(), summary))
                })
            }),
        }
    }

    pub fn delete_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "delete_trigger".to_string(),
            description: r#"Delete a trigger by name."#.to_string(),
            tags: vec!["trigger".to_string()],
            parameters: vec![ToolParameter {
                name: "name".to_string(),
                description: "Trigger name".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: name".to_string(),
                            )
                        }
                    };
                    match trigger_registry.delete_trigger(&name).await {
                        Ok(()) => ToolResult::success(format!("Trigger '{}' deleted", name)),
                        Err(e) => {
                            tracing::error!("Failed to delete trigger: {}", e);
                            ToolResult::error(format!("Failed to delete trigger: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn cancel_cron(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "cancel_cron".to_string(),
            description: r#"Cancel a specific scheduled occurrence of a cron trigger.

Parameters:
- name (string, required): Trigger name.
- occurrence (string, required): ISO8601 datetime (UTC-aware) of the occurrence to cancel.

Behavior:
Validates that the trigger exists and that its cron expression would fire at the provided minute (the occurrence is matched to minute precision), checks that the occurrence isn't already cancelled, and stores a cancellation in the cron_exceptions table. Note: cron expressions are interpreted by the parser; canonical stored one-shot cron strings include a leading seconds field ('s m H D M *').
"#.to_string(),
            tags: vec!["trigger".to_string(), "cron".to_string()],
            parameters: vec![
                ToolParameter { name: "name".to_string(), description: "Trigger name".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "occurrence".to_string(), description: "ISO8601 datetime of occurrence to cancel".to_string(), required: true, param_type: "string".to_string() },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => return ToolResult::error("Missing required parameter: name".to_string()),
                    };
                    let occurrence = match args.get("occurrence") {
                        Some(o) if !o.trim().is_empty() => o.clone(),
                        _ => return ToolResult::error("Missing required parameter: occurrence".to_string()),
                    };

                    match trigger_registry.cancel_cron(&name, &occurrence).await {
                        Ok(()) => ToolResult::success(format!("Cancelled occurrence {} for trigger {}", occurrence, name)),
                        Err(e) => { tracing::error!("Failed to cancel cron occurrence: {}", e); ToolResult::error(format!("Failed to cancel cron occurrence: {}", e)) }
                    }
                })
            }),
        }
    }

    pub fn enable_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "enable_trigger".to_string(),
            description: r#"Enable a trigger by name."#.to_string(),
            tags: vec!["trigger".to_string()],
            parameters: vec![ToolParameter {
                name: "name".to_string(),
                description: "Trigger name".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: name".to_string(),
                            )
                        }
                    };
                    let mut updates = std::collections::HashMap::new();
                    updates.insert("enabled".to_string(), "true".to_string());
                    match trigger_registry.update_trigger(&name, updates).await {
                        Ok(()) => ToolResult::success(format!("Trigger '{}' enabled", name)),
                        Err(e) => {
                            tracing::error!("Failed to enable trigger: {}", e);
                            ToolResult::error(format!("Failed to enable trigger: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn disable_trigger(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "disable_trigger".to_string(),
            description: r#"Disable a trigger by name."#.to_string(),
            tags: vec!["trigger".to_string()],
            parameters: vec![ToolParameter {
                name: "name".to_string(),
                description: "Trigger name".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let trigger_registry = trigger_registry.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: name".to_string(),
                            )
                        }
                    };
                    match trigger_registry.disable_trigger(&name).await {
                        Ok(()) => ToolResult::success(format!("Trigger '{}' disabled", name)),
                        Err(e) => {
                            tracing::error!("Failed to disable trigger: {}", e);
                            ToolResult::error(format!("Failed to disable trigger: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn analyze_now(trigger_registry: Arc<super::super::trigger::TriggerRegistry>) -> Tool {
        Tool {
            name: "analyze_now".to_string(),
            description: r#"Schedule an immediate analysis/compaction by creating a dynamic trigger (e.g., conversation analysis, pattern recognition).

Parameters:
- type (string, required): Analysis type: 'conversation', 'pattern', 'reflection', 'tool_effectiveness' (default: 'conversation').

Behavior:
Creates a dynamic trigger via trigger_registry.add_dynamic_trigger(...) with metadata { analysis_type } and returns success once scheduled.

Return:
ToolResult::success("Analysis scheduled: <type>") or ToolResult::error on failure.

Example args: { "type": "conversation" }"#.to_string(),
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

    pub fn open_chat(state: Arc<super::super::state::ServerState>, conversation_manager: Arc<super::super::conversations::ConversationManager>) -> Tool {
        Tool {
            name: "open_chat".to_string(),
            description: r#"Request that the client open the chat GUI, optionally display an initial message, and select a conversation session.

Parameters:
- message (string, optional): Initial message to display in the chat.
- session_id (string, optional): Session ID to select. If omitted, the most recently active session will be selected or a new session will be created.
- urgency (string, optional): Urgency level (unused by send_to_client_daemon path but included for parity).

Behavior:
Selects an appropriate conversation session (provided session_id, or most recent active session, or creates a new session) and sends ServerToClientRequest::OpenChat with that session_id. On failure, falls back to broadcasting ServerPush::OpenChat (which may not include session selection).

Return:
ToolResult::success("Chat window opened (session <id>)") or ToolResult::success("Chat window opened (fallback)") on fallback; ToolResult::error for other failures.

Example args: { "message": "Time to review PRs", "session_id": "abcd" }"#.to_string(),
            tags: vec!["ui".to_string(), "interaction".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "message".to_string(),
                    description: "Initial message to display".to_string(),
                    required: false,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "session_id".to_string(),
                    description: "Session ID to select (optional)".to_string(),
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
                let conv = conversation_manager.clone();
                Box::pin(async move {
                    let message = args.get("message").cloned();
                    let session_arg = args.get("session_id").cloned();

                    // Determine session id: use provided, else pick most recent active, else create a new auto session
                    let session_id = if let Some(s) = session_arg {
                        s
                    } else {
                        match conv.get_active_sessions().await {
                            Ok(sessions) if !sessions.is_empty() => sessions[0].session_id.clone(),
                            _ => format!("auto-{}", chrono::Utc::now().timestamp_nanos()),
                        }
                    };

                    // Ensure session exists (best-effort)
                    if let Err(e) = conv.get_or_create_session(&session_id).await {
                        warn!("Failed to ensure session exists {}: {}", session_id, e);
                    }

                    // Send to client daemon with session selection
                    let request = ritsu_common::protocol::ServerToClientRequest::OpenChat {
                        message: message.clone(),
                        session_id: Some(session_id.clone()),
                    };

                    match state.send_to_client_daemon(request).await {
                        Ok(()) => {
                            info!("Chat window open requested via client daemon (session={})", session_id);
                            if let Some(msg) = &message {
                                info!("With message: {msg}");
                            }
                            ToolResult::success(format!("Chat window opened (session {})", session_id))
                        }
                        Err(e) => {
                            warn!("Failed to open chat via client daemon: {}", e);
                            // Fallback to broadcast (no session selection available)
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

    pub fn set_title(
        conversation_manager: Arc<super::super::conversations::ConversationManager>,
    ) -> Tool {
        Tool {
            name: "set_title".to_string(),
            description: r#"Set the title of a conversation session.
Parameters:
- session_id (string, required): Session ID.
- title (string, required): New title.
Behavior:
Updates conversations.title via ConversationManager.set_title(session_id, title).
Return:
ToolResult::success on success or ToolResult::error on failure."#
                .to_string(),
            tags: vec!["conversation".to_string(), "meta".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "session_id".to_string(),
                    description: "Session ID".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
                ToolParameter {
                    name: "title".to_string(),
                    description: "New title for conversation session".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let conversation_manager = conversation_manager.clone();
                Box::pin(async move {
                    let session_id = match args.get("session_id") {
                        Some(s) if !s.trim().is_empty() => s.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: session_id".to_string(),
                            )
                        }
                    };
                    let title = match args.get("title") {
                        Some(t) if !t.trim().is_empty() => t.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: title".to_string(),
                            )
                        }
                    };
                    match conversation_manager.set_title(&session_id, &title).await {
                        Ok(()) => {
                            info!("Set title for session {}: {}", session_id, title);
                            ToolResult::success(format!(
                                "Title set for session {}: {}",
                                session_id, title
                            ))
                        }
                        Err(e) => {
                            tracing::error!("Failed to set title: {}", e);
                            ToolResult::error(format!("Failed to set title: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn create_task(task_manager: Arc<super::super::tasks::TaskManager>) -> Tool {
        Tool {
            name: "create_task".to_string(),
            description: r#"Create a new task via TaskManager.

Parameters:
- title (string, required): Task title.
- priority (string, optional): 'low', 'medium', 'high', 'urgent' (default: 'medium').
- due_date (string, optional): Due date in 'YYYY-MM-DD' format.
- description (string, optional): Task description.

Behavior:
Validates 'title' and converts 'priority' into ritsu_common::TaskPriority, calls task_manager.create_task(...) and returns the created task ID in the success message.

Return:
ToolResult::success("Task created with ID <id>: <title>") or ToolResult::error on failure.

Example args: { "title": "Write release notes", "priority": "high", "due_date": "2026-02-10" }"#.to_string(),
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
                    let title = match args.get("title") {
                        Some(t) if !t.trim().is_empty() => t.clone(),
                        _ => return ToolResult::error("Missing required parameter: title".to_string()),
                    };
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
            description: r#"Update an existing task's status or priority via TaskManager.

Parameters:
- id (string, required): Task ID (parsable to integer).
- status (string, optional): 'pending', 'in_progress', 'completed', 'cancelled'.
- priority (string, optional): 'low', 'medium', 'high', 'urgent'.

Behavior:
Parses 'id' as i64 and applies updates via TaskManager; returns an error if neither status nor priority provided.

Return:
ToolResult::success("Task <id> updated: <updates>") or ToolResult::error on parse/validation/manager errors.

Example args: { "id": "42", "status": "completed" }"#.to_string(),
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
                    let id_str = match args.get("id") {
                        Some(i) if !i.trim().is_empty() => i.clone(),
                        _ => return ToolResult::error("Missing required parameter: id".to_string()),
                    };
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
            description: r#"List tasks with optional status/priority filters.

Parameters:
- status (string, optional): Filter by status.
- priority (string, optional): Filter by priority.

Behavior:
Calls task_manager.list_tasks(status, priority) and returns a human-readable summary for each found task.

Return:
ToolResult::success("Found N tasks:\n<list>") or ToolResult::success("No tasks found matching the filters") or ToolResult::error on failure.

Example args: { "status": "pending" }"#.to_string(),
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

    pub fn delete_task(task_manager: Arc<super::super::tasks::TaskManager>) -> Tool {
        Tool {
            name: "delete_task".to_string(),
            description: r#"Delete a task by ID via TaskManager."#.to_string(),
            tags: vec!["task".to_string()],
            parameters: vec![ToolParameter {
                name: "id".to_string(),
                description: "Task ID".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let tm = task_manager.clone();
                Box::pin(async move {
                    let id_str = match args.get("id") {
                        Some(i) if !i.trim().is_empty() => i.clone(),
                        _ => {
                            return ToolResult::error("Missing required parameter: id".to_string())
                        }
                    };
                    let id = match id_str.parse::<i64>() {
                        Ok(v) => v,
                        Err(_) => return ToolResult::error(format!("Invalid task ID: {id_str}")),
                    };
                    match tm.delete_task(id).await {
                        Ok(()) => ToolResult::success(format!("Deleted task {}", id)),
                        Err(e) => {
                            tracing::error!("Failed to delete task: {}", e);
                            ToolResult::error(format!("Failed to delete task: {e}"))
                        }
                    }
                })
            }),
        }
    }

    pub fn set_preference(preferences: Arc<super::super::preferences::PreferencesManager>) -> Tool {
        Tool {
            name: "set_preference".to_string(),
            description: r#"Persist a user preference using PreferencesManager.

Parameters:
- category (string, required): Preference category (e.g., schedule, communication).
- key (string, required): Preference key (e.g., wake_time).
- value (string, required): Preference value.

Behavior:
Calls preferences.set_preference(&category, &key, &value, 1.0, Some("user")).

Return:
ToolResult::success("Preference saved: category / key = value") or ToolResult::error on failure.

Example args: { "category": "schedule", "key": "wake_time", "value": "07:00" }"#
                .to_string(),
            tags: vec!["preferences".to_string(), "memory".to_string()],
            parameters: vec![
                ToolParameter {
                    name: "category".to_string(),
                    description: "Preference category (e.g., schedule, communication, work)"
                        .to_string(),
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
                    let category = match args.get("category") {
                        Some(c) if !c.trim().is_empty() => c.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: category".to_string(),
                            )
                        }
                    };
                    let key = match args.get("key") {
                        Some(k) if !k.trim().is_empty() => k.clone(),
                        _ => {
                            return ToolResult::error("Missing required parameter: key".to_string())
                        }
                    };
                    let value = match args.get("value") {
                        Some(v) if !v.trim().is_empty() => v.clone(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: value".to_string(),
                            )
                        }
                    };

                    match preferences
                        .set_preference(&category, &key, &value, 1.0, Some("user"))
                        .await
                    {
                        Ok(()) => {
                            info!("Set preference: {}/{} = {}", category, key, value);
                            ToolResult::success(format!(
                                "Preference saved: {category} / {key} = {value}"
                            ))
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

    pub fn update_system_prompt(memory: Arc<super::super::memory::MemoryManager>, config: Arc<crate::config::Config>) -> Tool {
        Tool {
            name: "update_system_prompt".to_string(),
            description: "Safely update or propose changes to system prompts.\n\nParameters:\n- old (string, required): Exact substring to replace.\n- new (string, required): Replacement text or new prompt content.\n- apply (string, optional): 'true' to force apply immediately (overrides require_user_approval).\n\nBehavior:\nAttempts to replace `old` with `new` in the following order: user's base prompt file (~/.config/ritsu/prompts/system_base.md), DB-stored `base` prompt, DB-stored `ai_generated` prompt. If `old` is not found and `apply=true`, inserts `new` as a new `ai_generated` prompt. If require_user_approval is enabled in config, the change will be recorded as a proposal (idle_analysis) and audit-logged instead of applying automatically.".to_string(),
            tags: vec!["prompt".to_string(), "config".to_string()],
            parameters: vec![
                ToolParameter { name: "old".to_string(), description: "Old substring to replace".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "new".to_string(), description: "New replacement text".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "apply".to_string(), description: "Set to 'true' to force apply immediately".to_string(), required: false, param_type: "string".to_string() },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let memory = memory.clone();
                let config = config.clone();
                Box::pin(async move {
                    // Validate params
                    let old = match args.get("old") {
                        Some(s) if !s.trim().is_empty() => s.clone(),
                        _ => return ToolResult::error("Missing required parameter: old".to_string()),
                    };
                    let new = match args.get("new") {
                        Some(s) if !s.trim().is_empty() => s.clone(),
                        _ => return ToolResult::error("Missing required parameter: new".to_string()),
                    };
                    let apply_override = args.get("apply").map(|v| v == "true" || v == "1").unwrap_or(false);

                    // Read tool config (optional)
                    let mut require_user_approval = true;
                    let mut max_prompt_length: usize = 800;
                    let mut audit_log_path: Option<String> = None;
                    let mut allow_background_updates = false;

                    if let Ok(cfg_contents) = crate::database::read_file_async(crate::config::Config::config_file_path()).await {
                        if let Ok(cfg_val) = toml::from_str::<toml::Value>(&cfg_contents) {
                            if let Some(tools_tbl) = cfg_val.get("tools").and_then(|v| v.as_table()) {
                                if let Some(usp) = tools_tbl.get("update_system_prompt").and_then(|v| v.as_table()) {
                                    if let Some(b) = usp.get("require_user_approval").and_then(|v| v.as_bool()) { require_user_approval = b; }
                                    if let Some(i) = usp.get("max_prompt_length").and_then(|v| v.as_integer()) { max_prompt_length = i as usize; }
                                    if let Some(s) = usp.get("audit_log").and_then(|v| v.as_str()) { audit_log_path = Some(s.to_string()); }
                                    if let Some(b) = usp.get("allow_background_updates").and_then(|v| v.as_bool()) { allow_background_updates = b; }
                                }
                            }
                        }
                    }

                    if new.len() > max_prompt_length {
                        return ToolResult::error(format!("New prompt exceeds max length ({} > {})", new.len(), max_prompt_length));
                    }

                    // Helper: replace first occurrence only
                    let replace_first = |text: &str, needle: &str, replacement: &str| -> Option<String> {
                        if let Some(pos) = text.find(needle) {
                            let mut s = String::with_capacity(text.len() - needle.len() + replacement.len());
                            s.push_str(&text[..pos]);
                            s.push_str(replacement);
                            s.push_str(&text[pos + needle.len()..]);
                            Some(s)
                        } else { None }
                    };

                    // Helper: append audit log (best-effort)
                    let append_audit = |path_opt: Option<String>, location: &str, applied: bool, old_snip: &str, new_snip: &str| {
                        let mut path = path_opt.unwrap_or_else(|| {
                            if let Some(h) = dirs::home_dir() { h.join(".local/share/ritsu/ai_prompt_changes.log").to_string_lossy().to_string() } else { "/tmp/ritsu_ai_prompt_changes.log".to_string() }
                        });
                        if path.starts_with("~/") {
                            if let Some(h) = dirs::home_dir() { path = path.replacen("~", &h.to_string_lossy(), 1); }
                        }
                        let entry = format!("{} | update_system_prompt | location={} | applied={}\nOLD:\n{}\n---\nNEW:\n{}\n\n", chrono::Utc::now().to_rfc3339(), location, applied, old_snip, new_snip);
                        let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| std::io::Write::write_all(&mut f, entry.as_bytes()));
                    };

                    // 1) Try user's base prompt file
                    if let Some(home) = dirs::home_dir() {
                        let base_path = home.join(".config/ritsu/prompts/system_base.md");
                        if let Ok(content) = crate::database::read_file_async(base_path.clone()).await {
                            if content.contains(&old) {
                                if require_user_approval && !apply_override {
                                    let mut findings = std::collections::HashMap::new();
                                    findings.insert("old_snippet".to_string(), old.clone());
                                    findings.insert("new_snippet".to_string(), new.clone());
                                    let _ = memory.store_idle_analysis("prompt_update", &findings, Some(&new)).await;
                                    append_audit(audit_log_path.clone(), "file:system_base.md", false, &old, &new);
                                    return ToolResult::success("Proposed prompt update recorded; requires user approval".to_string());
                                }
                                // Apply change to file
                                if let Some(updated) = replace_first(&content, &old, &new) {
                                    let write_path = base_path.clone();
                                    let updated_clone = updated.clone();
                                    let res = tokio::task::spawn_blocking(move || std::fs::write(write_path, updated_clone)).await;
                                    if let Err(e) = res {
                                        return ToolResult::error(format!("Failed to write updated prompt file: {}", e));
                                    }
                                    let _ = memory.store_idle_analysis("prompt_update", &std::collections::HashMap::from([("applied".to_string(), "true".to_string())]), Some(&new)).await;
                                    append_audit(audit_log_path.clone(), "file:system_base.md", true, &old, &new);
                                    return ToolResult::success("System base prompt updated".to_string());
                                }
                            }
                        }
                    }

                    // 2) Try DB-stored base prompt
                    if let Ok(Some(base)) = memory.get_system_prompt("base").await {
                        if base.contains(&old) {
                            if require_user_approval && !apply_override {
                                let mut findings = std::collections::HashMap::new();
                                findings.insert("old_snippet".to_string(), old.clone());
                                findings.insert("new_snippet".to_string(), new.clone());
                                let _ = memory.store_idle_analysis("prompt_update", &findings, Some(&new)).await;
                                append_audit(audit_log_path.clone(), "db:base", false, &old, &new);
                                return ToolResult::success("Proposed prompt update recorded; requires user approval".to_string());
                            }
                            if let Some(updated) = replace_first(&base, &old, &new) {
                                if let Err(e) = memory.store_system_prompt("base", &updated).await {
                                    return ToolResult::error(format!("Failed to store updated base prompt: {}", e));
                                }
                                append_audit(audit_log_path.clone(), "db:base", true, &old, &new);
                                return ToolResult::success("DB base prompt updated".to_string());
                            }
                        }
                    }

                    // 3) Try DB-stored ai_generated prompt
                    if let Ok(Some(ai)) = memory.get_system_prompt("ai_generated").await {
                        if ai.contains(&old) {
                            if require_user_approval && !apply_override {
                                let mut findings = std::collections::HashMap::new();
                                findings.insert("old_snippet".to_string(), old.clone());
                                findings.insert("new_snippet".to_string(), new.clone());
                                let _ = memory.store_idle_analysis("prompt_update", &findings, Some(&new)).await;
                                append_audit(audit_log_path.clone(), "db:ai_generated", false, &old, &new);
                                return ToolResult::success("Proposed prompt update recorded; requires user approval".to_string());
                            }
                            if let Some(updated) = replace_first(&ai, &old, &new) {
                                if let Err(e) = memory.store_system_prompt("ai_generated", &updated).await {
                                    return ToolResult::error(format!("Failed to store updated ai_generated prompt: {}", e));
                                }
                                append_audit(audit_log_path.clone(), "db:ai_generated", true, &old, &new);
                                return ToolResult::success("AI-generated prompt updated".to_string());
                            }
                        }
                    }

                    // Not found - if apply_override and approval not required, insert as new ai_generated prompt
                    if apply_override && !require_user_approval {
                        if let Err(e) = memory.store_system_prompt("ai_generated", &new).await {
                            return ToolResult::error(format!("Failed to store new ai_generated prompt: {}", e));
                        }
                        append_audit(audit_log_path.clone(), "db:ai_generated:new", true, "", &new);
                        return ToolResult::success("New ai_generated prompt stored".to_string());
                    }

                    // Otherwise record proposal
                    let mut findings = std::collections::HashMap::new();
                    findings.insert("old_snippet".to_string(), old.clone());
                    findings.insert("new_snippet".to_string(), new.clone());
                    let _ = memory.store_idle_analysis("prompt_update", &findings, Some(&new)).await;
                    append_audit(audit_log_path.clone(), "proposal", false, &old, &new);
                    ToolResult::success("No matching prompt found; proposal recorded for review".to_string())
                })
            }),
        }
    }

    pub fn wait() -> Tool {
        Tool {
            name: "wait".to_string(),
            description: r#"Pause asynchronously for a specified duration (seconds).

Parameters:
- seconds (string, required): Float or integer number of seconds to wait (e.g., "0.5", "2").

Behavior:
Parses 'seconds' as f64 and performs tokio::time::sleep(dur).await.

Return:
ToolResult::success("Waited X seconds") or ToolResult::error on invalid parameter.

Example args: { "seconds": "1.5" }"#
                .to_string(),
            tags: vec!["utility".to_string(), "time".to_string()],
            parameters: vec![ToolParameter {
                name: "seconds".to_string(),
                description: "Number of seconds to wait (integer or float)".to_string(),
                required: true,
                param_type: "string".to_string(),
            }],
            handler: Arc::new(move |args: HashMap<String, String>| {
                Box::pin(async move {
                    let secs_str = match args.get("seconds") {
                        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
                        _ => {
                            return ToolResult::error(
                                "Missing required parameter: seconds".to_string(),
                            )
                        }
                    };
                    let secs = match secs_str.parse::<f64>() {
                        Ok(v) if v >= 0.0 => v,
                        _ => {
                            return ToolResult::error(format!(
                                "Invalid seconds value: {}",
                                secs_str
                            ))
                        }
                    };
                    let dur = std::time::Duration::from_secs_f64(secs);
                    info!("Tool 'wait' sleeping for {}s", secs);
                    tokio::time::sleep(dur).await;
                    ToolResult::success(format!("Waited {} seconds", secs))
                })
            }),
        }
    }

    pub fn get(config: Arc<crate::config::Config>) -> Tool {
        Tool {
            name: "get".to_string(),
            description: r#"Perform an HTTP GET request to a whitelisted endpoint.
Allowed endpoints are configured via the [network].allowed_http_hosts config key. Each entry may be a host (e.g., "example.com") or include an optional path prefix (e.g., "example.com/foo") — in which case requests to "example.com/foo/bar?baz=..." are allowed.

Parameters:
- url (string, required): Full URL to fetch (must match one of the configured allowed hosts/prefixes).
- timeout_seconds (string, optional): Request timeout in seconds (default: 10).

Behavior:
Validates the URL's host and optional path prefix against configuration, requires https scheme, performs GET request with a timeout, and returns HTTP status and body (truncated to 8192 chars) on success or ToolResult::error on failure.

Return:
ToolResult::success("Status: 200\\n\\n<body...>") or ToolResult::error(...)

Example args: { "url": "https://api.ipify.org?format=json" }"#.to_string(),
            tags: vec!["http".to_string(), "network".to_string()],
            parameters: vec![
                ToolParameter { name: "url".to_string(), description: "URL to fetch (must be whitelisted)".to_string(), required: true, param_type: "string".to_string() },
                ToolParameter { name: "timeout_seconds".to_string(), description: "Request timeout in seconds (default: 10)".to_string(), required: false, param_type: "string".to_string() },
            ],
            handler: Arc::new(move |args: HashMap<String, String>| {
                let cfg = config.clone();
                Box::pin(async move {
                    let url_str = match args.get("url") {
                        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
                        _ => return ToolResult::error("Missing required parameter: url".to_string()),
                    };
                    // Parse URL
                    let url = match reqwest::Url::parse(&url_str) {
                        Ok(u) => u,
                        Err(e) => return ToolResult::error(format!("Invalid URL: {}", e)),
                    };
                    if url.scheme() != "https" {
                        return ToolResult::error("Only https:// URLs are allowed".to_string());
                    }
                    let host = match url.host_str() {
                        Some(h) => h,
                        None => return ToolResult::error("URL missing host".to_string()),
                    };
                    let allowed_entries = &cfg.network.allowed_http_hosts;
                    if allowed_entries.is_empty() {
                        return ToolResult::error("No allowed hosts configured for 'get' tool".to_string());
                    }
                    // Each entry may be "host" or "host/path" and may optionally be a full URL with scheme.
                    let mut permitted = false;
                    for entry in allowed_entries.iter() {
                        // Parse entry into host and optional path prefix
                        let (entry_host, entry_path_opt) = if entry.starts_with("http://") || entry.starts_with("https://") {
                            match reqwest::Url::parse(entry) {
                                Ok(u) => (u.host_str().map(|s| s.to_string()), Some(u.path().to_string())),
                                Err(_) => {
                                    // fallback: treat entire entry as host-like
                                    let h = entry.split('/').next().map(|s| s.split(':').next().unwrap_or(s).to_string());
                                    (h, None)
                                }
                            }
                        } else {
                            if let Some(pos) = entry.find('/') {
                                let host_part = &entry[..pos];
                                let path_part = &entry[pos..]; // includes '/'
                                (Some(host_part.split(':').next().unwrap_or(host_part).to_string()), Some(path_part.to_string()))
                            } else {
                                (Some(entry.split(':').next().unwrap_or(entry).to_string()), None)
                            }
                        };

                        let entry_host = match entry_host {
                            Some(h) => h,
                            None => continue,
                        };

                        // Host match: exact or subdomain
                        if !(host == entry_host || host.ends_with(&format!(".{}", entry_host))) {
                            continue;
                        }

                        // Path match if provided
                        if let Some(ref prefix) = entry_path_opt {
                            let target_path = url.path();
                            if target_path.starts_with(prefix) {
                                permitted = true;
                                break;
                            } else {
                                continue;
                            }
                        } else {
                            permitted = true;
                            break;
                        }
                    }

                    if !permitted {
                        return ToolResult::error(format!("Host or path not allowed: {}{}", host, url.path()));
                    }

                    let timeout_secs = args.get("timeout_seconds").and_then(|s| s.parse::<u64>().ok()).unwrap_or(10);
                    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)).build() {
                        Ok(c) => c,
                        Err(e) => return ToolResult::error(format!("Failed to build HTTP client: {}", e)),
                    };
                    // Perform GET
                    let resp = match client.get(url.clone()).send().await {
                        Ok(r) => r,
                        Err(e) => return ToolResult::error(format!("HTTP request failed: {}", e)),
                    };
                    let status = resp.status();
                    let body = match resp.text().await {
                        Ok(t) => t,
                        Err(e) => return ToolResult::error(format!("Failed to read response body: {}", e)),
                    };
                    let max = 8192usize;
                    let truncated = if body.len() > max { format!("{}...[truncated {} bytes]", &body[..max], body.len() - max) } else { body.clone() };
                    ToolResult::success(format!("Status: {}\\n\\n{}", status, truncated))
                })
            }),
        }
    }

    pub fn parallel(registry: std::sync::Arc<super::ToolRegistry>) -> Tool {
        Tool {
            name: "parallel".to_string(),
            description: r#"Execute multiple registered tools in parallel.

Parameters:
- calls (string, required): JSON array of calls. Each call may be either a string (tool name) or an object {"name":"tool_name", "args": {"k":"v"}}.

Return:
A JSON array string with per-call results: [{"name": "tool", "success": bool, "output": "...", "error": null}, ...]."#.to_string(),
            tags: vec!["parallel".to_string(), "utility".to_string()],
            parameters: vec![ToolParameter { name: "calls".to_string(), description: "JSON array of tool calls (string or {name,args})".to_string(), required: true, param_type: "string".to_string() }],
            handler: std::sync::Arc::new(move |args: std::collections::HashMap<String, String>| {
                let registry = registry.clone();
                Box::pin(async move {
                    let calls_str = match args.get("calls") {
                        Some(s) if !s.trim().is_empty() => s.clone(),
                        _ => return ToolResult::error("Missing required parameter: calls".to_string()),
                    };

                    let calls_val: serde_json::Value = match serde_json::from_str(&calls_str) {
                        Ok(v) => v,
                        Err(e) => return ToolResult::error(format!("Invalid JSON for 'calls': {}", e)),
                    };

                    let calls_arr = match calls_val.as_array() {
                        Some(a) => a.clone(),
                        None => return ToolResult::error("'calls' must be a JSON array".to_string()),
                    };

                    let mut handles = Vec::new();

                    for item in calls_arr.into_iter() {
                        // Normalize to (name, args_map)
                        let (name, arg_map) = if item.is_string() {
                            match item.as_str() {
                                Some(s) => (s.to_string(), std::collections::HashMap::new()),
                                None => continue,
                            }
                        } else if let Some(obj) = item.as_object() {
                            let name = match obj.get("name").and_then(|v| v.as_str()) {
                                Some(n) if !n.is_empty() => n.to_string(),
                                _ => continue,
                            };
                            let mut hm = std::collections::HashMap::new();
                            if let Some(a) = obj.get("args").and_then(|v| v.as_object()) {
                                for (k, v) in a.iter() {
                                    let val_str = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
                                    hm.insert(k.clone(), val_str);
                                }
                            }
                            (name, hm)
                        } else {
                            continue;
                        };

                        // Lookup handler without holding lock across await
                        let handler_opt = {
                            let tools_map = registry.tools.read().await;
                            tools_map.get(&name).map(|t| t.handler.clone())
                        };

                        if let Some(handler) = handler_opt {
                            let args_clone = arg_map.clone();
                            let name_clone = name.clone();
                            // Spawn per-call task
                            handles.push(tokio::spawn(async move {
                                let res = (handler)(args_clone).await;
                                (name_clone, res)
                            }));
                        } else {
                            // Tool not found: immediate error result
                            let name_clone = name.clone();
                            handles.push(tokio::spawn(async move {
                                let msg = format!("Tool '{}' not found", name_clone.clone());
                                (name_clone, ToolResult::error(msg))
                            }));
                        }
                    }

                    // Await all
                    let mut results = Vec::new();
                    for h in handles {
                        match h.await {
                            Ok((name, res)) => {
                                results.push(serde_json::json!({"name": name, "success": res.success, "output": res.output, "error": res.error }));
                            }
                            Err(e) => {
                                results.push(serde_json::json!({"name": "<join_error>", "success": false, "output": "", "error": format!("Join error: {}", e) }));
                            }
                        }
                    }

                    let out = match serde_json::to_string(&results) {
                        Ok(s) => s,
                        Err(e) => format!("Failed to serialize results: {}", e),
                    };

                    ToolResult::success(out)
                })
            }),
        }
    }
}

use crate::conversations::ConversationManager;
use crate::memory::MemoryManager;
use crate::preferences::PreferencesManager;
use crate::state::ServerState;
use crate::tasks::TaskManager;
use crate::trigger::TriggerRegistry as TriggerReg;

pub async fn register_all_tools(
    registry: std::sync::Arc<ToolRegistry>,
    memory: Arc<MemoryManager>,
    conversation_manager: Arc<crate::conversations::ConversationManager>,
    task_manager: Arc<TaskManager>,
    trigger_registry: Arc<TriggerReg>,
    state: Arc<ServerState>,
    preferences: Arc<PreferencesManager>,
    config: Arc<crate::config::Config>,
) {
    registry
        .register(tool_impls::notify_client(state.clone()))
        .await;
    registry
        .register(tool_impls::create_note(memory.clone()))
        .await;
    registry
        .register(tool_impls::edit_note(memory.clone()))
        .await;
    registry
        .register(tool_impls::delete_note(memory.clone()))
        .await;
    registry
        .register(tool_impls::query_memory(memory.clone()))
        .await;
    registry
        .register(tool_impls::create_trigger(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::create_one_shot(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::cancel_cron(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::edit_trigger(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::list_triggers(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::delete_trigger(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::enable_trigger(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::disable_trigger(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::analyze_now(trigger_registry.clone()))
        .await;
    registry
        .register(tool_impls::open_chat(state.clone(), conversation_manager.clone()))
        .await;
    registry
        .register(tool_impls::set_title(conversation_manager.clone()))
        .await;
    registry
        .register(tool_impls::create_task(task_manager.clone()))
        .await;
    registry
        .register(tool_impls::update_task(task_manager.clone()))
        .await;
    registry
        .register(tool_impls::delete_task(task_manager.clone()))
        .await;
    registry
        .register(tool_impls::list_tasks(task_manager.clone()))
        .await;
    registry
        .register(tool_impls::set_preference(preferences.clone()))
        .await;
    registry
        .register(tool_impls::update_system_prompt(memory.clone(), config.clone()))
        .await;
    registry.register(tool_impls::wait()).await;
    registry.register(tool_impls::get(config.clone())).await;
    registry.register(tool_impls::parallel(registry.clone())).await;

    info!("Registered {} tools", 22);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::conversations::ConversationManager;
    use crate::database::Database;
    use crate::memory::MemoryManager;
    use crate::tasks::TaskManager;
    use crate::trigger::TriggerRegistry;
    use chrono::{Datelike, Timelike};
    use ritsu_common::TaskPriority;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_tool_registration() {
        let registry = ToolRegistry::new();

        let tool = Tool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            tags: vec!["test".to_string()],
            parameters: vec![],
            handler: Arc::new(|_args| Box::pin(async { ToolResult::success("test".to_string()) })),
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

        let result = registry
            .execute("nonexistent", HashMap::new())
            .await
            .unwrap();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_delete_note_tool() {
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_delete_note_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let db = Database::new(&db_path).await.expect("DB init");
        let memory = Arc::new(MemoryManager::new(
            db.connection.clone(),
            db_path_str.clone(),
            false,
        ));

        // create a note
        let note_id = memory
            .create_note("Test note content", &vec!["test".to_string()])
            .await
            .expect("create_note");

        // Call delete_note tool
        let tool = tool_impls::delete_note(memory.clone());
        let mut args = HashMap::new();
        args.insert("id".to_string(), note_id.to_string());
        let res = (tool.handler)(args).await;
        assert!(res.success, "delete_note failed: {:?}", res.error);

        // Verify note removed
        let notes = memory.query_notes(10).await.expect("query_notes");
        assert!(!notes.iter().any(|(id, _, _)| *id == note_id));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_trigger_crud_tools() {
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_trigger_crud_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let _db = Database::new(&db_path).await.expect("DB init");
        let trig_reg = Arc::new(TriggerRegistry::new(db_path_str.clone()));

        // Create a trigger via the registry helper
        trig_reg
            .create_trigger("ttest", "time", "12:34", Some("tag1"), Some("desc"))
            .await
            .expect("create_trigger");

        // List triggers tool
        let list_tool = tool_impls::list_triggers(trig_reg.clone());
        let res = (list_tool.handler)(HashMap::new()).await;
        assert!(res.success);
        assert!(
            res.output.contains("ttest"),
            "list did not include trigger: {}",
            res.output
        );

        // Disable trigger
        let disable_tool = tool_impls::disable_trigger(trig_reg.clone());
        let mut args = HashMap::new();
        args.insert("name".to_string(), "ttest".to_string());
        let res2 = (disable_tool.handler)(args.clone()).await;
        assert!(res2.success, "disable failed: {:?}", res2.error);

        // After disable, listing should not contain the trigger
        let res_list_after = (list_tool.handler)(HashMap::new()).await;
        assert!(res_list_after.success);
        assert!(
            res_list_after.output.contains("No triggers registered")
                || !res_list_after.output.contains("ttest")
        );

        // Enable trigger
        let enable_tool = tool_impls::enable_trigger(trig_reg.clone());
        let res3 = (enable_tool.handler)(args.clone()).await;
        assert!(res3.success, "enable failed: {:?}", res3.error);

        let res_list_after_enable = (list_tool.handler)(HashMap::new()).await;
        assert!(res_list_after_enable.success);
        assert!(res_list_after_enable.output.contains("ttest"));

        // Delete trigger
        let delete_tool = tool_impls::delete_trigger(trig_reg.clone());
        let res4 = (delete_tool.handler)(args.clone()).await;
        assert!(res4.success, "delete failed: {:?}", res4.error);

        let res_list_after_delete = (list_tool.handler)(HashMap::new()).await;
        assert!(res_list_after_delete.success);
        assert!(
            res_list_after_delete
                .output
                .contains("No triggers registered")
                || !res_list_after_delete.output.contains("ttest")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_one_shot_trigger_tool() {
        use chrono::Utc;
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_create_one_shot_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let _db = Database::new(&db_path).await.expect("DB init");
        let trig_reg = Arc::new(TriggerRegistry::new(db_path_str.clone()));

        // Use a near-future time (UTC) to avoid timezone surprises
        let dt = Utc::now() + chrono::Duration::minutes(2);
        let dt_str = dt.to_rfc3339();

        let tool = tool_impls::create_one_shot(trig_reg.clone());
        let mut args = HashMap::new();
        args.insert("name".to_string(), "oneshot_test".to_string());
        args.insert("datetime".to_string(), dt_str.clone());
        args.insert("tags".to_string(), "test".to_string());
        args.insert(
            "description".to_string(),
            "one-shot trigger test".to_string(),
        );

        let res = (tool.handler)(args).await;
        assert!(res.success, "create_one_shot failed: {:?}", res.error);

        // Ensure trigger exists and its cron schedule matches expected
        let triggers = trig_reg.get_all_triggers().await;
        let expected_cron = format!(
            "0 {} {} {} {} *",
            dt.minute(),
            dt.hour(),
            dt.day(),
            dt.month()
        );
        let found = triggers.iter().any(|t| t.name == "oneshot_test" && matches!(t.trigger_type, crate::trigger::TriggerType::Cron(ref s) if s == &expected_cron));
        assert!(
            found,
            "One-shot trigger not found with expected cron: {}",
            expected_cron
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cancel_cron_tool() {
        use chrono::Utc;
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_cancel_cron_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let _db = Database::new(&db_path).await.expect("DB init");
        let trig_reg = Arc::new(TriggerRegistry::new(db_path_str.clone()));

        // Create a one-shot trigger for near future
        let dt = Utc::now() + chrono::Duration::minutes(3);
        let dt_str = dt.to_rfc3339();
        trig_reg
            .create_one_shot_trigger(
                "cancel_test",
                &dt_str,
                vec!["test".to_string()],
                Some("cancel test"),
            )
            .await
            .expect("create_one_shot_trigger");

        // Cancel the exact occurrence via tool
        let tool = tool_impls::cancel_cron(trig_reg.clone());
        let mut args = HashMap::new();
        args.insert("name".to_string(), "cancel_test".to_string());
        args.insert("occurrence".to_string(), dt_str.clone());
        let res = (tool.handler)(args.clone()).await;
        assert!(res.success, "cancel_cron failed: {:?}", res.error);

        // Attempt to cancel again should fail with already cancelled
        let res2 = (tool.handler)(args.clone()).await;
        assert!(!res2.success);
        assert!(res2.error.unwrap_or_default().contains("already cancelled"));

        // Attempt to cancel a non-matching time should error
        let bad_dt = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
        let mut bad_args = HashMap::new();
        bad_args.insert("name".to_string(), "cancel_test".to_string());
        bad_args.insert("occurrence".to_string(), bad_dt.clone());
        let res3 = (tool.handler)(bad_args).await;
        assert!(!res3.success);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_delete_task_tool() {
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_delete_task_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let db = Database::new(&db_path).await.expect("DB init");
        let tm = Arc::new(TaskManager::new(db.connection.clone()));

        // Create a task
        let task_id = tm
            .create_task("Do unit tests", None, &TaskPriority::Medium, &[], None)
            .await
            .expect("create_task");

        // Delete via tool
        let del_tool = tool_impls::delete_task(tm.clone());
        let mut args = HashMap::new();
        args.insert("id".to_string(), task_id.to_string());
        let res = (del_tool.handler)(args).await;
        assert!(res.success, "delete_task failed: {:?}", res.error);

        // Ensure task removed
        let tasks = tm.list_tasks(None, None).await.expect("list_tasks");
        assert!(!tasks.iter().any(|t| t.id == task_id));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_title_tool() {
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "ritsu_test_set_title_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_path.to_string_lossy().to_string();

        let db = Database::new(&db_path).await.expect("DB init");
        let conv = Arc::new(ConversationManager::new(db.connection.clone()));

        // Create or get session
        let session = conv
            .get_or_create_session("sess1")
            .await
            .expect("get_or_create");
        assert!(session.title.is_none());

        // Set title via tool
        let tool = tool_impls::set_title(conv.clone());
        let mut args = HashMap::new();
        args.insert("session_id".to_string(), "sess1".to_string());
        args.insert("title".to_string(), "My Session Title".to_string());
        let res = (tool.handler)(args).await;
        assert!(res.success, "set_title failed: {:?}", res.error);

        // Re-read session
        let session2 = conv
            .get_or_create_session("sess1")
            .await
            .expect("get_or_create2");
        assert_eq!(session2.title, Some("My Session Title".to_string()));

        let _ = std::fs::remove_file(&db_path);
    }
}
