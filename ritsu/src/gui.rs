//! GUI implementation using iced

#![allow(clippy::if_not_else)]
#![allow(clippy::unused_self)]
#![allow(clippy::enum_variant_names)]

use iced::{
    widget::{button, column, container, row, scrollable, text, text_input},
    Element, Task, Theme,
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
    ServerResponse(Result<String, String>),
    Tick,
    SwitchView(ViewState),
    LoadSession(String),
    SessionsLoaded(Vec<SessionInfo>),
    TasksLoaded(Vec<TaskInfo>),
    ToggleSidebar,
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
}

#[derive(Debug, Clone)]
struct ChatMessage {
    content: String,
    is_user: bool,
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
            },
            Task::none(),
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
                if self.is_loading {
                    self.animation_frame = (self.animation_frame + 1) % 4;
                    Task::perform(
                        async {
                            tokio::time::sleep(Duration::from_millis(250)).await;
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
                    });
                    self.input.clear();
                    self.is_loading = true;
                    self.animation_frame = 0;

                    Task::batch([
                        Task::perform(
                            async move {
                                let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
                                let request = ritsu_common::protocol::ClientRequest::SendMessage {
                                    content: content.clone(),
                                    session_id: Some(session_id),
                                };
                                client.send_request(request).await
                            },
                            |result| match result {
                                Ok(response) => match response {
                                    ritsu_common::protocol::ServerResponse::Message { content } => {
                                        Message::MessageReceived(content)
                                    }
                                    ritsu_common::protocol::ServerResponse::Ok => {
                                        Message::ServerResponse(Ok("Message sent".to_string()))
                                    }
                                    ritsu_common::protocol::ServerResponse::Error { message } => {
                                        Message::ServerResponse(Err(message))
                                    }
                                    _ => Message::ServerResponse(Err("Unexpected response".to_string())),
                                },
                                Err(e) => Message::ServerResponse(Err(format!("IPC error: {e}"))),
                            },
                        ),
                        Task::perform(async {}, |()| Message::Tick),
                    ])
                } else {
                    Task::none()
                }
            }
            Message::MessageReceived(content) => {
                self.messages.push(ChatMessage {
                    content,
                    is_user: false,
                });
                self.is_loading = false;
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
                                // TODO: Implement IPC call to get sessions
                                // For now, return mock data
                                vec![]
                            },
                            Message::SessionsLoaded,
                        )
                    }
                    ViewState::Tasks => {
                        // Fetch tasks
                        Task::perform(
                            async {
                                // TODO: Implement IPC call to get tasks
                                // For now, return mock data
                                vec![]
                            },
                            Message::TasksLoaded,
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
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
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

        // Main content based on current view
        let main_content = match self.current_view {
            ViewState::Chat => self.view_chat(),
            ViewState::Sessions => self.view_sessions(),
            ViewState::Tasks => self.view_tasks(),
            ViewState::Memory => self.view_memory(),
        };

        // If sidebar visible, show it
        let layout = if self.sidebar_visible {
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
            .width(iced::Length::Fixed(180.0));

            let sidebar_container = container(sidebar)
                .style(|_theme: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(0.12, 0.12, 0.15))),
                    border: iced::Border {
                        width: 0.0,
                        color: iced::Color::from_rgb(0.3, 0.3, 0.35),
                        radius: 0.0.into(),
                    },
                    ..container::Style::default()
                })
                .height(iced::Length::Fill);

            row![
                column![menu_button, sidebar_container].spacing(0),
                main_content
            ]
            .spacing(0)
        } else {
            // Just menu button and content
            row![
                column![menu_button].width(iced::Length::Fixed(50.0)),
                main_content
            ]
            .spacing(0)
        };

        container(layout)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn view_chat(&self) -> Element<'_, Message> {
        let messages_view = self.messages.iter().fold(
            column![].spacing(12),
            |col, msg| {
                let (bg_color, text_color, align) = if msg.is_user {
                    (iced::Color::from_rgb(0.2, 0.35, 0.6), iced::Color::WHITE, iced::alignment::Horizontal::Right)
                } else {
                    (iced::Color::from_rgb(0.18, 0.18, 0.22), iced::Color::from_rgb(0.9, 0.9, 0.95), iced::alignment::Horizontal::Left)
                };
                
                let message_container = container(
                    text(&msg.content)
                        .size(14)
                        .color(text_color)
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

pub fn run_blocking() -> anyhow::Result<()> {
    iced::application(
        RitsuGui::default,
        update,
        view
    )
    .theme(|_state: &RitsuGui| Theme::Dark)
    .run()
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}
