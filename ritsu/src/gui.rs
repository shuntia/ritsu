//! GUI implementation using iced

#![allow(clippy::if_not_else)]
#![allow(clippy::unused_self)]
#![allow(clippy::enum_variant_names)]

use iced::{
    widget::{button, column, container, row, scrollable, text, text_input},
    Element, Subscription, Task, Theme,
    futures,
    stream,
};
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewState {
    Chat,
    Sessions,
    Tasks,
    Memory,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    SendMessage,
    MessageReceived(String),
    MessageChunk(String, bool), // content, is_final
    StreamingStarted,
    ServerResponse(Result<String, String>),
    Tick,
    SwitchView(ViewState),
    LoadSession(String),
    SessionsLoaded(Vec<SessionInfo>),
    ConversationHistoryLoaded(Result<Vec<ritsu_common::protocol::ConversationTurn>, String>),
    TasksLoaded(Vec<TaskInfo>),
    TaskStatusChanged(i64, String), // task_id, new_status
    TaskDeleted(i64), // task_id
    TaskOperationComplete(Result<(), String>),
    MemoryLoaded(Result<String, String>), // Memory query result
    // Task creation dialog
    ShowTaskCreateDialog,
    TaskTitleChanged(String),
    TaskDescriptionChanged(String),
    TaskPrioritySelected(String),
    CreateTaskSubmit,
    CancelTaskCreate,
    CopyMessage(usize),
    ToggleThinking(usize),
    ToggleSidebar,
    ConnectionStatusChanged(ConnectionStatus),
    RetryConnection,
    KeyPressed(iced::keyboard::Key, iced::keyboard::Modifiers),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionInfo {
    session_id: String,
    started_at: String,
    last_activity: String,
    turn_count: i64,
    title: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TaskInfo {
    id: i64,
    title: String,
    status: String,
    priority: String,
}

pub struct RitsuGui {
    input: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    is_loading: bool,
    animation_frame: usize,
    current_view: ViewState,
    sessions: Vec<SessionInfo>,
    tasks: Vec<TaskInfo>,
    memory_content: String, // Loaded memory/notes content
    // Task creation dialog state
    show_task_dialog: bool,
    task_title_input: String,
    task_description_input: String,
    task_priority_input: String,
    sidebar_visible: bool,
    sidebar_animation: f32, // 0.0 = hidden, 1.0 = visible
    connection_status: ConnectionStatus,
    retry_countdown: Option<u32>, // Seconds until retry
    streaming_message_index: Option<usize>, // Index of message being streamed to
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Reconnecting,
    Error(String),
}

#[derive(Debug, Clone)]
struct ChatMessage {
    content: String,
    is_user: bool,
    opacity: f32, // For fade-in animation
    timestamp: chrono::DateTime<chrono::Utc>,
    thinking: Option<String>, // For models that expose thinking/reasoning
    show_thinking: bool, // Whether to show thinking section (toggle)
}

impl RitsuGui {
    #[allow(dead_code)]
    fn new() -> (Self, Task<Message>) {
        let session_id = format!("gui_session_{}", chrono::Utc::now().timestamp());
        (
            Self {
                input: String::new(),
                messages: Vec::new(),
                session_id,
                is_loading: false,
                animation_frame: 0,
                current_view: ViewState::Chat,
                sessions: Vec::new(),
                tasks: Vec::new(),
                memory_content: String::new(),
                show_task_dialog: false,
                task_title_input: String::new(),
                task_description_input: String::new(),
                task_priority_input: "medium".to_string(),
                sidebar_visible: false,
                sidebar_animation: 0.0,
                connection_status: ConnectionStatus::Disconnected,
                retry_countdown: None,
                streaming_message_index: None,
            },
            // Test connection on startup
            Task::perform(
                async {
                    let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                    client.ping().await
                },
                |result| {
                    if result.is_ok() {
                        Message::ConnectionStatusChanged(ConnectionStatus::Connected)
                    } else {
                        Message::ConnectionStatusChanged(ConnectionStatus::Disconnected)
                    }
                },
            ),
        )
    }
}

impl Default for RitsuGui {
    fn default() -> Self {
        Self {
            input: String::new(),
            messages: Vec::new(),
            session_id: format!("gui_session_{}", chrono::Utc::now().timestamp()),
            is_loading: false,
            animation_frame: 0,
            current_view: ViewState::Chat,
            sessions: Vec::new(),
            tasks: Vec::new(),
            memory_content: String::new(),
            show_task_dialog: false,
            task_title_input: String::new(),
            task_description_input: String::new(),
            task_priority_input: "medium".to_string(),
            sidebar_visible: false,
            sidebar_animation: 0.0,
            connection_status: ConnectionStatus::Disconnected,
            retry_countdown: None,
            streaming_message_index: None,
        }
    }
}

impl RitsuGui {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(value) => {
                self.input = value;
                Task::none()
            }
            Message::Tick => {
                // Animate spinner if loading
                let mut needs_animation = self.is_loading;
                
                if self.is_loading {
                    self.animation_frame = (self.animation_frame + 1) % 4;
                }
                
                // Animate sidebar transition
                if self.sidebar_visible && self.sidebar_animation < 1.0 {
                    self.sidebar_animation = (self.sidebar_animation + 0.15).min(1.0);
                    needs_animation = true;
                } else if !self.sidebar_visible && self.sidebar_animation > 0.0 {
                    self.sidebar_animation = (self.sidebar_animation - 0.15).max(0.0);
                    needs_animation = true;
                }
                
                // Animate message fade-ins
                for msg in &mut self.messages {
                    if msg.opacity < 1.0 {
                        msg.opacity = (msg.opacity + 0.1).min(1.0);
                        needs_animation = true;
                    }
                }
                
                // Countdown retry timer
                if let Some(countdown) = self.retry_countdown.as_mut() {
                    if *countdown > 0 {
                        *countdown -= 1;
                        needs_animation = true;
                    } else {
                        self.retry_countdown = None;
                        return Task::perform(async {}, |()| Message::RetryConnection);
                    }
                }
                
                if needs_animation {
                    Task::perform(
                        async {
                            tokio::time::sleep(Duration::from_millis(1000)).await; // 1 FPS for countdown
                        },
                        |()| Message::Tick,
                    )
                } else {
                    Task::none()
                }
            }
            Message::SendMessage => {
                if !self.input.is_empty() && !self.is_loading {
                    let content = self.input.clone();
                    let session_id = self.session_id.clone();
                    self.messages.push(ChatMessage {
                        content: content.clone(),
                        is_user: true,
                        opacity: 0.0, // Start invisible for fade-in
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                        show_thinking: false,
                    });
                    self.input.clear();
                    self.is_loading = true;
                    self.animation_frame = 0;

                    // Add empty assistant message that will be streamed to
                    self.messages.push(ChatMessage {
                        content: String::new(),
                        is_user: false,
                        opacity: 1.0,
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                        show_thinking: false,
                    });
                    self.streaming_message_index = Some(self.messages.len() - 1);

                    // Start streaming using stream-based task
                    Task::batch([
                        Task::run(
                            {
                                stream::channel(100, move |mut sender: futures::channel::mpsc::Sender<Message>| async move {
                                    let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                                    match client.send_message_streaming(content, Some(session_id)).await {
                                        Ok(mut rx) => {
                                            while let Some(push) = rx.recv().await {
                                                if let ritsu_common::protocol::ServerPush::MessageChunk { content, is_final } = push {
                                                    let _ = sender.try_send(Message::MessageChunk(content.clone(), is_final));
                                                    if is_final {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let _ = sender.try_send(Message::ConnectionStatusChanged(
                                                ConnectionStatus::Error(format!("Failed to start streaming: {}", e))
                                            ));
                                        }
                                    }
                                })
                            },
                            |stream| stream,
                        ),
                        Task::perform(async {}, |()| Message::Tick),
                    ])
                } else {
                    Task::none()
                }
            }
            Message::StreamingStarted => {
                // Streaming channel is now active via subscription
                Task::none()
            }
            Message::MessageReceived(_content) => {
                // Legacy handler - no longer used with streaming
                self.is_loading = false;
                Task::none()
            }
            Message::MessageChunk(chunk, is_final) => {
                // Append chunk to the streaming message
                if let Some(idx) = self.streaming_message_index {
                    if let Some(msg) = self.messages.get_mut(idx) {
                        msg.content.push_str(&chunk);
                    }
                }
                
                if is_final {
                    // Streaming complete
                    self.is_loading = false;
                    self.streaming_message_index = None;
                }
                
                Task::none()
            }
            Message::ServerResponse(result) => {
                match result {
                    Ok(msg) => println!("✓ {msg}"),
                    Err(e) => eprintln!("✗ Error: {e}"),
                }
                self.is_loading = false;
                Task::none()
            }
            Message::SwitchView(view) => {
                self.current_view = view.clone();
                match view {
                    ViewState::Sessions => {
                        // Fetch today's sessions
                        Task::perform(
                            async {
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                                let request = ritsu_common::protocol::ClientRequest::ListSessions {
                                    limit: Some(20),
                                };
                                client.send_request(request).await
                            },
                            |result| match result {
                                Ok(ritsu_common::protocol::ServerResponse::Sessions { sessions }) => {
                                    Message::SessionsLoaded(
                                        sessions
                                            .into_iter()
                                            .map(|s| SessionInfo {
                                                session_id: s.session_id,
                                                started_at: s.started_at,
                                                last_activity: s.last_activity,
                                                turn_count: s.turn_count,
                                                title: s.title,
                                            })
                                            .collect(),
                                    )
                                }
                                Ok(ritsu_common::protocol::ServerResponse::Error { message }) => {
                                    eprintln!("Error loading sessions: {}", message);
                                    Message::SessionsLoaded(vec![])
                                }
                                Err(e) => {
                                    eprintln!("IPC error loading sessions: {}", e);
                                    Message::SessionsLoaded(vec![])
                                }
                                _ => Message::SessionsLoaded(vec![]),
                            },
                        )
                    }
                    ViewState::Tasks => {
                        // Fetch tasks
                        Task::perform(
                            async {
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                                let request = ritsu_common::protocol::ClientRequest::ListTasks {
                                    filter: None,
                                };
                                client.send_request(request).await
                            },
                            |result| match result {
                                Ok(ritsu_common::protocol::ServerResponse::Tasks { tasks }) => {
                                    Message::TasksLoaded(
                                        tasks
                                            .into_iter()
                                            .map(|t| TaskInfo {
                                                id: t.id,
                                                title: t.title,
                                                status: format!("{:?}", t.status),
                                                priority: format!("{:?}", t.priority),
                                            })
                                            .collect(),
                                    )
                                }
                                _ => Message::TasksLoaded(vec![]),
                            },
                        )
                    }
                    ViewState::Memory => {
                        // Fetch memory (notes and recent summaries)
                        Task::perform(
                            async {
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                                let request = ritsu_common::protocol::ClientRequest::QueryMemory {
                                    query_type: ritsu_common::protocol::MemoryQueryType::Notes,
                                    date_range: None,
                                };
                                client.send_request(request).await
                            },
                            |result| match result {
                                Ok(ritsu_common::protocol::ServerResponse::Memory { content }) => {
                                    Message::MemoryLoaded(Ok(content))
                                }
                                Ok(ritsu_common::protocol::ServerResponse::Error { message }) => {
                                    Message::MemoryLoaded(Err(message))
                                }
                                Ok(_) | Err(_) => Message::MemoryLoaded(Err("Unexpected response".to_string())),
                            },
                        )
                    }
                    ViewState::Chat => Task::none(),
                }
            }
            Message::LoadSession(session_id) => {
                // Load conversation history for this session
                let sid = session_id.clone();
                self.session_id = session_id;
                self.messages.clear();
                self.current_view = ViewState::Chat;
                self.is_loading = true;
                
                Task::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                        client.get_conversation_history(sid, 100).await
                    },
                    |result| match result {
                        Ok(turns) => Message::ConversationHistoryLoaded(Ok(turns)),
                        Err(e) => Message::ConversationHistoryLoaded(Err(e.to_string())),
                    },
                )
            }
            Message::ConversationHistoryLoaded(result) => {
                self.is_loading = false;
                match result {
                    Ok(turns) => {
                        // Convert conversation turns to chat messages
                        for turn in turns {
                            let is_user = turn.role == "user";
                            self.messages.push(ChatMessage {
                                content: turn.content,
                                is_user,
                                opacity: 1.0, // No fade-in for loaded messages
                                timestamp: chrono::Utc::now(), // Use current time as fallback
                                thinking: turn.thinking,
                                show_thinking: false,
                            });
                        }
                    }
                    Err(e) => {
                        // Show error message in chat
                        self.messages.push(ChatMessage {
                            content: format!("Error loading conversation history: {}", e),
                            is_user: false,
                            opacity: 1.0,
                            timestamp: chrono::Utc::now(),
                        thinking: None,
                        show_thinking: false,
                        });
                    }
                }
                Task::none()
            }
            Message::SessionsLoaded(sessions) => {
                self.sessions = sessions;
                Task::none()
            }
            Message::TasksLoaded(tasks) => {
                self.tasks = tasks;
                Task::none()
            }
            Message::TaskStatusChanged(task_id, new_status) => {
                // Send update request to server
                Task::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                        let status = match new_status.as_str() {
                            "in_progress" => ritsu_common::protocol::TaskStatus::InProgress,
                            "completed" => ritsu_common::protocol::TaskStatus::Completed,
                            "cancelled" => ritsu_common::protocol::TaskStatus::Cancelled,
                            _ => ritsu_common::protocol::TaskStatus::Pending,
                        };
                        
                        let request = ritsu_common::protocol::ClientRequest::UpdateTask {
                            id: task_id,
                            status: Some(status),
                            priority: None,
                        };
                        
                        match client.send_request(request).await {
                            Ok(ritsu_common::protocol::ServerResponse::Ok) => Ok(()),
                            Ok(ritsu_common::protocol::ServerResponse::Error { message }) => Err(message),
                            Err(e) => Err(e.to_string()),
                            _ => Err("Unexpected response".to_string()),
                        }
                    },
                    Message::TaskOperationComplete,
                )
            }
            Message::TaskDeleted(task_id) => {
                // Send delete request to server
                Task::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                        let request = ritsu_common::protocol::ClientRequest::DeleteTask { id: task_id };
                        
                        match client.send_request(request).await {
                            Ok(ritsu_common::protocol::ServerResponse::Ok) => Ok(()),
                            Ok(ritsu_common::protocol::ServerResponse::Error { message }) => Err(message),
                            Err(e) => Err(e.to_string()),
                            _ => Err("Unexpected response".to_string()),
                        }
                    },
                    Message::TaskOperationComplete,
                )
            }
            Message::TaskOperationComplete(result) => {
                match result {
                    Ok(()) => {
                        // Reload tasks to reflect changes
                        Task::perform(
                            async {
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                                let request = ritsu_common::protocol::ClientRequest::ListTasks { filter: None };
                                client.send_request(request).await
                            },
                            |result| match result {
                                Ok(ritsu_common::protocol::ServerResponse::Tasks { tasks }) => {
                                    Message::TasksLoaded(
                                        tasks
                                            .into_iter()
                                            .map(|t| TaskInfo {
                                                id: t.id,
                                                title: t.title,
                                                status: format!("{:?}", t.status).to_lowercase(),
                                                priority: format!("{:?}", t.priority).to_lowercase(),
                                            })
                                            .collect(),
                                    )
                                }
                                _ => Message::TasksLoaded(Vec::new()),
                            },
                        )
                    }
                    Err(e) => {
                        tracing::error!("Task operation failed: {}", e);
                        Task::none()
                    }
                }
            }
            Message::MemoryLoaded(result) => {
                match result {
                    Ok(content) => {
                        self.memory_content = content;
                    }
                    Err(e) => {
                        self.memory_content = format!("Error loading memory: {}", e);
                    }
                }
                Task::none()
            }
            Message::ShowTaskCreateDialog => {
                self.show_task_dialog = true;
                // Reset form fields
                self.task_title_input.clear();
                self.task_description_input.clear();
                self.task_priority_input = "medium".to_string();
                Task::none()
            }
            Message::TaskTitleChanged(title) => {
                self.task_title_input = title;
                Task::none()
            }
            Message::TaskDescriptionChanged(desc) => {
                self.task_description_input = desc;
                Task::none()
            }
            Message::TaskPrioritySelected(priority) => {
                self.task_priority_input = priority;
                Task::none()
            }
            Message::CreateTaskSubmit => {
                if self.task_title_input.trim().is_empty() {
                    // Don't create empty tasks
                    return Task::none();
                }
                
                // Close dialog
                self.show_task_dialog = false;
                
                // Send create request
                let title = self.task_title_input.clone();
                let description = if self.task_description_input.trim().is_empty() {
                    None
                } else {
                    Some(self.task_description_input.clone())
                };
                let priority = match self.task_priority_input.as_str() {
                    "low" => ritsu_common::protocol::TaskPriority::Low,
                    "high" => ritsu_common::protocol::TaskPriority::High,
                    "urgent" => ritsu_common::protocol::TaskPriority::Urgent,
                    _ => ritsu_common::protocol::TaskPriority::Medium,
                };
                
                Task::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                        let request = ritsu_common::protocol::ClientRequest::CreateTask {
                            title,
                            description,
                            priority,
                            due_date: None,
                            tags: Vec::new(),
                        };
                        
                        match client.send_request(request).await {
                            Ok(ritsu_common::protocol::ServerResponse::Ok) => Ok(()),
                            Ok(ritsu_common::protocol::ServerResponse::Error { message }) => Err(message),
                            Err(e) => Err(e.to_string()),
                            _ => Err("Unexpected response".to_string()),
                        }
                    },
                    Message::TaskOperationComplete,
                )
            }
            Message::CancelTaskCreate => {
                self.show_task_dialog = false;
                Task::none()
            }
            Message::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                // Start animation
                Task::perform(async {}, |()| Message::Tick)
            }
            Message::ConnectionStatusChanged(status) => {
                let was_disconnected = matches!(self.connection_status, ConnectionStatus::Disconnected);
                let now_connected = matches!(status, ConnectionStatus::Connected);
                
                self.connection_status = status;
                
                // If we just reconnected, show success message
                if was_disconnected && now_connected {
                    self.messages.push(ChatMessage {
                        content: "✓ Connected to server".to_string(),
                        is_user: false,
                        opacity: 0.0,
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                        show_thinking: false,
                    });
                }
                
                // If disconnected, start retry countdown
                if matches!(self.connection_status, ConnectionStatus::Disconnected) {
                    self.retry_countdown = Some(5); // Retry in 5 seconds
                    return Task::perform(async {}, |()| Message::Tick);
                }
                
                Task::none()
            }
            Message::CopyMessage(idx) => {
                if let Some(msg) = self.messages.get(idx) {
                    // Use iced clipboard
                    return iced::clipboard::write(msg.content.clone());
                }
                Task::none()
            }
            Message::ToggleThinking(idx) => {
                if let Some(msg) = self.messages.get_mut(idx) {
                    msg.show_thinking = !msg.show_thinking;
                }
                Task::none()
            }
            Message::RetryConnection => {
                self.connection_status = ConnectionStatus::Reconnecting;
                Task::perform(
                    async {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
                        client.ping().await
                    },
                    |result| {
                        if result.is_ok() {
                            Message::ConnectionStatusChanged(ConnectionStatus::Connected)
                        } else {
                            Message::ConnectionStatusChanged(ConnectionStatus::Disconnected)
                        }
                    },
                )
            }
            Message::KeyPressed(key, modifiers) => {
                use iced::keyboard::{Key, key::Named};
                
                // Ctrl/Cmd + Enter to send message
                if modifiers.command() && matches!(key, Key::Named(Named::Enter)) {
                    return self.update(Message::SendMessage);
                }
                
                // Ctrl/Cmd + B to toggle sidebar
                if modifiers.command() && matches!(key, Key::Character(ref c) if c.as_str() == "b") {
                    return self.update(Message::ToggleSidebar);
                }
                
                // Ctrl/Cmd + 1/2/3/4 to switch views
                if modifiers.command() {
                    match key {
                        Key::Character(ref c) if c.as_str() == "1" => {
                            return self.update(Message::SwitchView(ViewState::Chat));
                        }
                        Key::Character(ref c) if c.as_str() == "2" => {
                            return self.update(Message::SwitchView(ViewState::Sessions));
                        }
                        Key::Character(ref c) if c.as_str() == "3" => {
                            return self.update(Message::SwitchView(ViewState::Tasks));
                        }
                        Key::Character(ref c) if c.as_str() == "4" => {
                            return self.update(Message::SwitchView(ViewState::Memory));
                        }
                        _ => {}
                    }
                }
                
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // Connection status indicator
        let status_text = match &self.connection_status {
            ConnectionStatus::Connected => "Connected".to_string(),
            ConnectionStatus::Disconnected => {
                self.retry_countdown.map_or_else(
                    || "Disconnected".to_string(),
                    |countdown| format!("Reconnecting in {countdown}s...")
                )
            }
            ConnectionStatus::Reconnecting => "Connecting...".to_string(),
            ConnectionStatus::Error(msg) => format!("Error: {}", msg),
        };
        
        let status_color = match &self.connection_status {
            ConnectionStatus::Connected => iced::Color::from_rgb(0.2, 0.8, 0.2),
            ConnectionStatus::Disconnected | ConnectionStatus::Reconnecting => iced::Color::from_rgb(0.8, 0.6, 0.2),
            ConnectionStatus::Error(_) => iced::Color::from_rgb(0.8, 0.2, 0.2),
        };
        
        let status_indicator = row![
            text("●").size(12).color(status_color),
            text(status_text).size(12).color(iced::Color::from_rgb(0.7, 0.7, 0.7)),
        ]
        .spacing(5)
        .align_y(iced::alignment::Vertical::Center);
        
        // Hamburger menu button
        let menu_button = button(text("☰").size(24))
            .on_press(Message::ToggleSidebar)
            .padding(10)
            .style(|theme: &iced::Theme, status| button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.2, 0.25))),
                text_color: theme.palette().text,
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..button::primary(theme, status)
            });
        
        // Top bar with menu and status
        let top_bar = row![
            menu_button,
            container(status_indicator)
                .width(iced::Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .padding(10),
        ]
        .spacing(10);

        // Main content based on current view
        let main_content = match self.current_view {
            ViewState::Chat => self.view_chat(),
            ViewState::Sessions => self.view_sessions(),
            ViewState::Tasks => self.view_tasks(),
            ViewState::Memory => self.view_memory(),
        };

        // Animate sidebar width
        let sidebar_width = 180.0 * self.sidebar_animation;
        
        // If sidebar has any visibility, show it
        let layout = if self.sidebar_animation > 0.01 {
            // Sidebar with view switcher
            let sidebar = column![
                button("💬 Chat")
                    .on_press(Message::SwitchView(ViewState::Chat))
                    .width(iced::Length::Fill)
                    .padding(10)
                    .style(if self.current_view == ViewState::Chat {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.25, 0.45, 0.75))),
                            text_color: theme.palette().text,
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::primary(theme, status)
                        }
                    } else {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.15, 0.15, 0.2))),
                            text_color: iced::Color::from_rgb(0.8, 0.8, 0.85),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::secondary(theme, status)
                        }
                    }),
                button("📜 Sessions")
                    .on_press(Message::SwitchView(ViewState::Sessions))
                    .width(iced::Length::Fill)
                    .padding(10)
                    .style(if self.current_view == ViewState::Sessions {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.25, 0.45, 0.75))),
                            text_color: theme.palette().text,
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::primary(theme, status)
                        }
                    } else {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.15, 0.15, 0.2))),
                            text_color: iced::Color::from_rgb(0.8, 0.8, 0.85),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::secondary(theme, status)
                        }
                    }),
                button("✓ Tasks")
                    .on_press(Message::SwitchView(ViewState::Tasks))
                    .width(iced::Length::Fill)
                    .padding(10)
                    .style(if self.current_view == ViewState::Tasks {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.25, 0.45, 0.75))),
                            text_color: theme.palette().text,
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::primary(theme, status)
                        }
                    } else {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.15, 0.15, 0.2))),
                            text_color: iced::Color::from_rgb(0.8, 0.8, 0.85),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::secondary(theme, status)
                        }
                    }),
                button("🧠 Memory")
                    .on_press(Message::SwitchView(ViewState::Memory))
                    .width(iced::Length::Fill)
                    .padding(10)
                    .style(if self.current_view == ViewState::Memory {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.25, 0.45, 0.75))),
                            text_color: theme.palette().text,
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::primary(theme, status)
                        }
                    } else {
                        |theme: &iced::Theme, status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.15, 0.15, 0.2))),
                            text_color: iced::Color::from_rgb(0.8, 0.8, 0.85),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..button::secondary(theme, status)
                        }
                    }),
            ]
            .spacing(8)
            .padding(15)
            .width(iced::Length::Fixed(sidebar_width));

            let sidebar_animation = self.sidebar_animation;
            let sidebar_container = container(sidebar)
                .style(move |_theme: &iced::Theme| {
                    let mut color = iced::Color::from_rgb(0.12, 0.12, 0.15);
                    color.a = sidebar_animation;
                    container::Style {
                        background: Some(iced::Background::Color(color)),
                        border: iced::Border {
                            width: 0.0,
                            color: iced::Color::from_rgb(0.3, 0.3, 0.35),
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }
                })
                .height(iced::Length::Fill);

            row![
                column![sidebar_container].spacing(0),
                main_content
            ]
            .spacing(0)
        } else {
            // Just content (menu is in top bar)
            row![main_content]
            .spacing(0)
        };

        let main_layout = column![top_bar, layout]
            .spacing(0);

        // Add task creation dialog overlay if visible
        if self.show_task_dialog {
            let dialog = self.view_task_create_dialog();
            iced::widget::stack![
                container(main_layout)
                    .width(iced::Length::Fill)
                    .height(iced::Length::Fill),
                dialog,
            ]
            .into()
        } else {
            container(main_layout)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        }
    }
    
    fn view_task_create_dialog(&self) -> Element<'_, Message> {
        let dialog_content = column![
            text("Create New Task").size(20),
            text("Title:").size(14),
            text_input("Task title...", &self.task_title_input)
                .on_input(Message::TaskTitleChanged)
                .padding(10),
            text("Description (optional):").size(14),
            text_input("Task description...", &self.task_description_input)
                .on_input(Message::TaskDescriptionChanged)
                .padding(10),
            text("Priority:").size(14),
            row![
                button(text("Low"))
                    .on_press(Message::TaskPrioritySelected("low".to_string()))
                    .padding(8)
                    .style(if self.task_priority_input == "low" {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.6, 0.9))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    } else {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.3, 0.3, 0.35))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    }),
                button(text("Medium"))
                    .on_press(Message::TaskPrioritySelected("medium".to_string()))
                    .padding(8)
                    .style(if self.task_priority_input == "medium" {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.6, 0.9))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    } else {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.3, 0.3, 0.35))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    }),
                button(text("High"))
                    .on_press(Message::TaskPrioritySelected("high".to_string()))
                    .padding(8)
                    .style(if self.task_priority_input == "high" {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.6, 0.2))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    } else {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.3, 0.3, 0.35))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    }),
                button(text("Urgent"))
                    .on_press(Message::TaskPrioritySelected("urgent".to_string()))
                    .padding(8)
                    .style(if self.task_priority_input == "urgent" {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.2, 0.2))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    } else {
                        |_theme: &iced::Theme, _status| button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.3, 0.3, 0.35))),
                            text_color: iced::Color::WHITE,
                            ..button::Style::default()
                        }
                    }),
            ]
            .spacing(10),
            row![
                button(text("Cancel"))
                    .on_press(Message::CancelTaskCreate)
                    .padding(10)
                    .style(|_theme: &iced::Theme, _status| button::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(0.4, 0.4, 0.45))),
                        text_color: iced::Color::WHITE,
                        ..button::Style::default()
                    }),
                button(text("Create"))
                    .on_press(Message::CreateTaskSubmit)
                    .padding(10)
                    .style(|_theme: &iced::Theme, _status| button::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.7, 0.3))),
                        text_color: iced::Color::WHITE,
                        ..button::Style::default()
                    }),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(15)
        .padding(30);
        
        let dialog_box = container(dialog_content)
            .width(500)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.15, 0.15, 0.2))),
                border: iced::Border {
                    color: iced::Color::from_rgb(0.3, 0.3, 0.4),
                    width: 2.0,
                    radius: 12.0.into(),
                },
                ..container::Style::default()
            });
        
        // Center the dialog with a semi-transparent backdrop
        container(dialog_box)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .center(iced::Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.7))),
                ..container::Style::default()
            })
            .into()
    }

    fn view_chat(&self) -> Element<'_, Message> {
        let mut messages_view = self.messages.iter().enumerate().fold(
            column![].spacing(12),
            |col, (idx, msg)| {
                let opacity = msg.opacity;
                let (mut bg_color, text_color, align) = if msg.is_user {
                    (iced::Color::from_rgb(0.2, 0.35, 0.6), iced::Color::WHITE, iced::alignment::Horizontal::Right)
                } else {
                    (iced::Color::from_rgb(0.18, 0.18, 0.22), iced::Color::from_rgb(0.9, 0.9, 0.95), iced::alignment::Horizontal::Left)
                };
                
                // Apply opacity to background
                bg_color.a = opacity;
                
                let mut final_text_color = text_color;
                final_text_color.a = opacity;
                
                // Format timestamp
                let local_time: chrono::DateTime<chrono::Local> = msg.timestamp.into();
                let time_str = local_time.format("%H:%M").to_string();
                
                // Create timestamp color
                let mut timestamp_color = text_color;
                timestamp_color.a = opacity * 0.6;
                
                // Add copy button for assistant messages
                let message_with_copy: Element<'_, Message> = if !msg.is_user {
                    let button_text_color = text_color; // Copy for closure
                    let copy_btn = button(text("📋").size(12))
                        .on_press(Message::CopyMessage(idx))
                        .padding(4)
                        .style(move |_theme: &iced::Theme, _status| {
                            button::Style {
                                background: Some(iced::Background::Color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
                                text_color: button_text_color,
                                border: iced::Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                ..button::Style::default()
                            }
                        });
                    
                    // Build message column with optional thinking section
                    let mut message_col = column![
                        text(&msg.content)
                            .size(14)
                            .color(final_text_color),
                        text(time_str)
                            .size(11)
                            .color(timestamp_color)
                    ]
                    .spacing(4);
                    
                    // Add thinking section if present
                    if let Some(thinking_text) = &msg.thinking {
                        let thinking_button_color = text_color; // Copy for closure
                        let thinking_toggle_btn = button(text(if msg.show_thinking { "🧠 Hide thinking" } else { "🧠 Show thinking" }).size(11))
                            .on_press(Message::ToggleThinking(idx))
                            .padding([2, 6])
                            .style(move |_theme: &iced::Theme, _status| {
                                button::Style {
                                    background: Some(iced::Background::Color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.05))),
                                    text_color: thinking_button_color,
                                    border: iced::Border {
                                        radius: 4.0.into(),
                                        ..Default::default()
                                    },
                                    ..button::Style::default()
                                }
                            });
                        
                        message_col = message_col.push(thinking_toggle_btn);
                        
                        if msg.show_thinking {
                            let mut thinking_text_color = text_color;
                            thinking_text_color.a = opacity * 0.7;
                            
                            message_col = message_col.push(
                                container(
                                    text(thinking_text)
                                        .size(13)
                                        .color(thinking_text_color)
                                        .font(iced::Font::MONOSPACE)
                                )
                                .padding(8)
                                .style(move |_theme: &iced::Theme| container::Style {
                                    background: Some(iced::Background::Color(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.2))),
                                    border: iced::Border {
                                        radius: 6.0.into(),
                                        ..Default::default()
                                    },
                                    ..container::Style::default()
                                })
                            );
                        }
                    }
                    
                    row![
                        message_col.width(iced::Length::Fill),
                        copy_btn,
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Start)
                    .into()
                } else {
                    column![
                        text(&msg.content)
                            .size(14)
                            .color(final_text_color),
                        text(time_str)
                            .size(11)
                            .color(timestamp_color)
                    ]
                    .spacing(4)
                    .into()
                };
                
                let message_container = container(message_with_copy)
                    .padding(12)
                    .style(move |_theme: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(bg_color)),
                        border: iced::Border {
                            radius: 12.0.into(),
                            ..Default::default()
                        },
                        ..container::Style::default()
                    })
                    .max_width(600);
                
                let row_content = row![container(message_container).width(iced::Length::Fill).align_x(align)];
                
                col.push(row_content)
            }
        );

        // Add typing indicator when loading
        if self.is_loading {
            let dots_frames = ["   ", ".  ", ".. ", "..."];
            let dots = dots_frames[self.animation_frame % dots_frames.len()];
            
            let typing_indicator = container(
                text(format!("Ritsu is typing{}", dots))
                    .size(14)
                    .color(iced::Color::from_rgb(0.6, 0.6, 0.6))
            )
            .padding(12)
            .style(|_theme: &iced::Theme| {
                container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.2, 0.25))),
                    text_color: Some(iced::Color::from_rgb(0.8, 0.8, 0.85)),
                    border: iced::Border {
                        radius: 12.0.into(),
                        ..Default::default()
                    },
                    ..container::Style::default()
                }
            })
            .max_width(200);
            
            messages_view = messages_view.push(
                row![container(typing_indicator).width(iced::Length::Fill).align_x(iced::alignment::Horizontal::Left)]
            );
        }

        let mut input_field = text_input("Type your message...", &self.input)
            .on_input(Message::InputChanged)
            .padding(12)
            .size(14);
        
        if !self.is_loading {
            input_field = input_field.on_submit(Message::SendMessage);
        }

        let spinner_frames = ["⠋", "⠙", "⠹", "⠸"];
        let spinner = spinner_frames[self.animation_frame % spinner_frames.len()];
        
        let send_button = if !self.is_loading {
            button("Send")
                .on_press(Message::SendMessage)
                .padding(12)
                .style(|theme: &iced::Theme, status| button::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(0.25, 0.45, 0.75))),
                    text_color: theme.palette().text,
                    border: iced::Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..button::primary(theme, status)
                })
        } else {
            button(text(spinner).size(16))
                .padding(12)
                .style(|_theme: &iced::Theme, _status| {
                    button::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(0.4, 0.4, 0.45))),
                        text_color: iced::Color::WHITE,
                        border: iced::Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        ..button::Style::default()
                    }
                })
        };
        
        let input_area = row![input_field, send_button]
            .spacing(10);

        let content = column![
            scrollable(messages_view).height(iced::Length::Fill),
            input_area,
        ]
        .spacing(20)
        .padding(20);

        container(content)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn view_sessions(&self) -> Element<'_, Message> {
        let header = text("Today's Sessions").size(24);
        
        let sessions_list = if self.sessions.is_empty() {
            column![text("No sessions today. Start a new chat!")]
        } else {
            self.sessions.iter().fold(
                column![].spacing(10),
                |col, session| {
                    let title = session.title.as_ref()
                        .unwrap_or(&session.session_id);
                    
                    let session_button = button(
                        column![
                            text(title).size(16),
                            text(format!("Started: {}", session.started_at)).size(11),
                            text(format!("{} turns", session.turn_count)).size(11),
                        ]
                        .spacing(5)
                    )
                    .on_press(Message::LoadSession(session.session_id.clone()))
                    .width(iced::Length::Fill)
                    .padding(10);
                    
                    col.push(session_button)
                }
            )
        };

        let content = column![
            header,
            scrollable(sessions_list).height(iced::Length::Fill),
        ]
        .spacing(20)
        .padding(20);

        container(content)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn view_tasks(&self) -> Element<'_, Message> {
        let header = row![
            text("Task Manager").size(24),
            button(text("+ New Task").size(14))
                .on_press(Message::ShowTaskCreateDialog)
                .padding(8)
                .style(|_theme: &iced::Theme, _status| {
                    button::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(0.2, 0.6, 0.9))),
                        text_color: iced::Color::WHITE,
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..button::Style::default()
                    }
                }),
        ]
        .spacing(20)
        .align_y(iced::Alignment::Center);
        
        let tasks_list = if self.tasks.is_empty() {
            column![text("No tasks found. Create one from chat or memory view.")]
        } else {
            self.tasks.iter().fold(
                column![].spacing(10),
                |col, task| {
                    let priority_color = match task.priority.as_str() {
                        "urgent" => iced::Color::from_rgb(0.9, 0.2, 0.2),
                        "high" => iced::Color::from_rgb(0.9, 0.6, 0.2),
                        "medium" => iced::Color::from_rgb(0.2, 0.6, 0.9),
                        _ => iced::Color::from_rgb(0.5, 0.5, 0.5),
                    };
                    
                    let status_icon = match task.status.as_str() {
                        "completed" => "✓",
                        "in_progress" => "⟳",
                        _ => "○",
                    };
                    
                    // Status cycle buttons
                    let next_status = match task.status.as_str() {
                        "pending" => ("Start", "in_progress"),
                        "in_progress" => ("Complete", "completed"),
                        "completed" => ("Reopen", "pending"),
                        _ => ("Start", "pending"),
                    };
                    
                    let task_id = task.id;
                    let status_button = button(text(next_status.0).size(12))
                        .on_press(Message::TaskStatusChanged(task_id, next_status.1.to_string()))
                        .padding(5);
                    
                    let delete_button = button(text("×").size(16))
                        .on_press(Message::TaskDeleted(task_id))
                        .padding(5)
                        .style(|_theme: &iced::Theme, _status| {
                            button::Style {
                                background: Some(iced::Background::Color(iced::Color::from_rgb(0.8, 0.2, 0.2))),
                                text_color: iced::Color::WHITE,
                                ..button::Style::default()
                            }
                        });
                    
                    let task_view = row![
                        text(status_icon).size(20),
                        column![
                            text(&task.title).size(16),
                            text(format!("Priority: {} | Status: {}", task.priority, task.status))
                                .size(12)
                                .color(priority_color),
                        ]
                        .spacing(5),
                        row![status_button, delete_button].spacing(5),
                    ]
                    .spacing(10)
                    .padding(10)
                    .align_y(iced::Alignment::Center);
                    
                    col.push(container(task_view).style(move |_theme: &iced::Theme| {
                        container::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgba(0.2, 0.2, 0.3, 0.5))),
                            border: iced::Border {
                                color: priority_color,
                                width: 2.0,
                                radius: 5.0.into(),
                            },
                            ..container::Style::default()
                        }
                    }))
                }
            )
        };

        let content = column![
            header,
            scrollable(tasks_list).height(iced::Length::Fill),
        ]
        .spacing(20)
        .padding(20);

        container(content)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn view_memory(&self) -> Element<'_, Message> {
        let header = text("Memory & Notes").size(24);
        
        let memory_content = if self.memory_content.is_empty() {
            column![
                text("Loading memory...").size(14),
            ]
        } else {
            column![
                scrollable(
                    text(&self.memory_content)
                        .size(14)
                        .width(iced::Length::Fill)
                )
                .height(iced::Length::Fill)
            ]
        };

        let content = column![
            header,
            memory_content,
        ]
        .spacing(20)
        .padding(20);

        container(content)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }
}

fn update(state: &mut RitsuGui, message: Message) -> Task<Message> {
    state.update(message)
}

fn view(state: &RitsuGui) -> Element<'_, Message> {
    state.view()
}

fn subscription(_state: &RitsuGui) -> Subscription<Message> {
    use iced::keyboard;
    use iced::Event;
    
    iced::event::listen_with(|event, _status, _id| {
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event {
            Some(Message::KeyPressed(key, modifiers))
        } else {
            None
        }
    })
}

pub fn run_blocking() -> anyhow::Result<()> {
    iced::application(
        RitsuGui::default,
        update,
        view
    )
    .subscription(subscription)
    .theme(|_state: &RitsuGui| Theme::Dark)
    .run()
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}
