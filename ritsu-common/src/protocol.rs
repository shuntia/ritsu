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
    /// Delete a task
    DeleteTask {
        id: i64,
    },
    /// Query memory
    QueryMemory {
        query_type: MemoryQueryType,
        date_range: Option<DateRange>,
    },
    /// List conversation sessions
    ListSessions {
        limit: Option<usize>,
    },
    /// Clear all memory (for testing)
    ClearMemory {
        /// Confirm flag to prevent accidental deletion
        confirm: bool,
    },
    /// Subscribe to server push notifications
    Subscribe,
    /// Get conversation history for a session
    GetConversationHistory {
        session_id: String,
        limit: i64,
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
    /// List of conversation sessions
    Sessions { sessions: Vec<SessionInfo> },
    /// Conversation history (list of turns)
    ConversationHistory { turns: Vec<ConversationTurn> },
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
    /// Streaming message chunk
    MessageChunk { 
        content: String,
        is_final: bool,
    },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub started_at: String,
    pub last_activity: String,
    pub turn_count: i64,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub turn_number: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_results: Option<String>,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_client_request_creation() {
        let requests = [
            ClientRequest::Ping,
            ClientRequest::SendMessage {
                content: "test".to_string(),
                session_id: None,
            },
            ClientRequest::Shutdown,
            ClientRequest::ListTriggers,
        ];

        for request in &requests {
            // Just check they can be created and matched
            #[allow(clippy::wildcard_in_or_patterns)]
            match request {
                ClientRequest::Ping
                | ClientRequest::SendMessage { .. }
                | ClientRequest::Shutdown
                | ClientRequest::ListTriggers
                | _ => {}
            }
        }
    }

    #[test]
    fn test_server_response_creation() {
        let _responses = [
            ServerResponse::Ok,
            ServerResponse::Pong,
            ServerResponse::Error {
                message: "test error".to_string(),
            },
            ServerResponse::Message {
                content: "test message".to_string(),
            },
        ];
        // Just check they compile
    }

    #[test]
    fn test_server_push_creation() {
        let _pushes = [
            ServerPush::Notification {
                title: "Test".to_string(),
                message: "Message".to_string(),
                urgency: NotificationUrgency::Normal,
            },
            ServerPush::OpenChat {
                message: Some("Open".to_string()),
                urgency: NotificationUrgency::Urgent,
            },
        ];
        // Just check they compile
    }

    #[test]
    fn test_task_status_enum() {
        assert_eq!(TaskStatus::Pending, TaskStatus::Pending);
        assert_ne!(TaskStatus::Pending, TaskStatus::Completed);
    }

    #[test]
    fn test_task_priority_enum() {
        assert_eq!(TaskPriority::High, TaskPriority::High);
        assert_ne!(TaskPriority::Low, TaskPriority::Urgent);
    }
}
