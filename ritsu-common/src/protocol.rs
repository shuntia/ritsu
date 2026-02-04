//! IPC protocol definitions for client-server communication

use serde::{Deserialize, Serialize};

/// Client request to server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientRequest {
    /// Ping to check server status
    Ping,
    /// Send a message to the AI
    SendMessage { 
        content: String,
        session_id: Option<String>,
    },
    /// Shutdown the server
    Shutdown,
    /// List all triggers
    ListTriggers,
    /// Create a new trigger
    CreateTrigger {
        name: String,
        trigger_type: String,
        schedule: String,
        tag: Option<String>,
        description: Option<String>,
    },
    /// Delete a trigger
    DeleteTrigger { name: String },
    /// Disable a trigger
    DisableTrigger { name: String },
    /// List tasks with optional filters
    ListTasks { filter: Option<TaskFilter> },
    /// Create a new task
    CreateTask {
        title: String,
        description: Option<String>,
        priority: TaskPriority,
        due_date: Option<String>,
        tags: Vec<String>,
    },
    /// Update an existing task
    UpdateTask {
        id: i64,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    },
    /// Query memory
    QueryMemory {
        query_type: MemoryQueryType,
        date_range: Option<DateRange>,
    },
}

/// Server response to client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerResponse {
    /// Successful operation
    Ok,
    /// Pong response
    Pong,
    /// Error occurred
    Error { message: String },
    /// AI message response
    Message { content: String },
    /// List of triggers
    Triggers { triggers: Vec<TriggerInfo> },
    /// List of tasks
    Tasks { tasks: Vec<TaskInfo> },
    /// Memory query result
    Memory { content: String },
}

/// Server push notification to client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerPush {
    /// System notification
    Notification {
        title: String,
        message: String,
        urgency: NotificationUrgency,
    },
    /// Request to open chat window
    OpenChat {
        message: Option<String>,
        urgency: NotificationUrgency,
    },
    /// User response timeout
    ResponseTimeout { context: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub id: i64,
    pub name: String,
    pub trigger_type: String,
    pub schedule: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub tags: Vec<String>,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MemoryQueryType {
    Recent,
    Daily,
    Monthly,
    Notes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NotificationUrgency {
    Low,
    Normal,
    Urgent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}
