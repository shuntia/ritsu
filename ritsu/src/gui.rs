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
use futures::StreamExt;

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
    TasksLoaded(Vec<TaskInfo>),
    ToggleSidebar,
    ConnectionStatusChanged(ConnectionStatus),
    RetryConnection,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionInfo {
    session_id: String,
    started_at: String,
    last_activity: String,
    turn_count: i64,
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
    sidebar_visible: bool,
    sidebar_animation: f32, // 0.0 = hidden, 1.0 = visible
    connection_status: ConnectionStatus,
    retry_countdown: Option<u32>, // Seconds until retry
    streaming_message_index: Option<usize>, // Index of message being streamed to
}

#[derive(Debug, Clone, PartialEq)]
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
                sidebar_visible: false,
                sidebar_animation: 0.0,
                connection_status: ConnectionStatus::Disconnected,
                retry_countdown: None,
                streaming_message_index: None,
            },
            // Test connection on startup
            Task::perform(
                async {
                    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
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
                let mut needs_animation = false;
                
                // Animate spinner if loading
                if self.is_loading {
                    self.animation_frame = (self.animation_frame + 1) % 4;
                    needs_animation = true;
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
                for (_i, msg) in self.messages.iter_mut().enumerate() {
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
                    });
                    self.input.clear();
                    self.is_loading = true;
                    self.animation_frame = 0;

                    // Add empty assistant message that will be streamed to
                    self.messages.push(ChatMessage {
                        content: String::new(),
                        is_user: false,
                        opacity: 1.0,
                    });
                    self.streaming_message_index = Some(self.messages.len() - 1);

                    // Start streaming using stream-based task
                    Task::batch([
                        Task::run(
                            {
                                let content = content.clone();
                                let session_id = session_id.clone();
                                stream::channel(100, move |mut sender: futures::channel::mpsc::Sender<Message>| async move {
                                    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
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
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
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
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
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
                    _ => Task::none(),
                }
            }
            Message::LoadSession(session_id) => {
                // TODO: Load conversation history for this session
                self.session_id = session_id;
                self.messages.clear();
                self.current_view = ViewState::Chat;
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
                    });
                }
                
                // If disconnected, start retry countdown
                if matches!(self.connection_status, ConnectionStatus::Disconnected) {
                    self.retry_countdown = Some(5); // Retry in 5 seconds
                    return Task::perform(async {}, |()| Message::Tick);
                }
                
                Task::none()
            }
            Message::RetryConnection => {
                self.connection_status = ConnectionStatus::Reconnecting;
                Task::perform(
                    async {
                        let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
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
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // Connection status indicator
        let status_text = match &self.connection_status {
            ConnectionStatus::Connected => "Connected".to_string(),
            ConnectionStatus::Disconnected => {
                if let Some(countdown) = self.retry_countdown {
                    format!("Reconnecting in {}s...", countdown)
                } else {
                    "Disconnected".to_string()
                }
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

        container(main_layout)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn view_chat(&self) -> Element<'_, Message> {
        let mut messages_view = self.messages.iter().fold(
            column![].spacing(12),
            |col, msg| {
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
                
                let message_container = container(
                    text(&msg.content)
                        .size(14)
                        .color(final_text_color)
                )
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
                    let session_button = button(
                        column![
                            text(&session.session_id).size(14),
                            text(format!("Started: {}", session.started_at)).size(12),
                            text(format!("{} turns", session.turn_count)).size(12),
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
        let header = text("Task Manager").size(24);
        
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
                    
                    let task_view = row![
                        text(status_icon).size(20),
                        column![
                            text(&task.title).size(16),
                            text(format!("Priority: {} | Status: {}", task.priority, task.status))
                                .size(12)
                                .color(priority_color),
                        ]
                        .spacing(5)
                    ]
                    .spacing(10)
                    .padding(10);
                    
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
        
        let memory_content = column![
            text("Notes:").size(18),
            text("(Coming soon: View and manage your notes)").size(14),
            text("").size(10),
            text("Daily Summaries:").size(18),
            text("(Coming soon: View daily conversation summaries)").size(14),
            text("").size(10),
            text("Monthly Summaries:").size(18),
            text("(Coming soon: View monthly summaries)").size(14),
        ]
        .spacing(10);

        let content = column![
            header,
            scrollable(memory_content).height(iced::Length::Fill),
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
    // For now, no subscriptions - streaming handled via Tasks
    Subscription::none()
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
