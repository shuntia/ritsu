//! GUI implementation using iced

#![allow(clippy::if_not_else)]
#![allow(clippy::unused_self)]
#![allow(clippy::enum_variant_names)]

use iced::{
    widget::{button, column, container, row, scrollable, text, text_input},
    Element, Task, Theme,
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    SendMessage,
    MessageReceived(String),
    ServerResponse(Result<String, String>),
}

pub struct RitsuGui {
    input: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    is_loading: bool,
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
                    )
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
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let messages_view = self.messages.iter().fold(
            column![].spacing(10),
            |col, msg| {
                let message_text = if msg.is_user {
                    text(format!("You: {}", msg.content))
                } else {
                    text(format!("Ritsu: {}", msg.content))
                };
                col.push(message_text)
            }
        );

        let mut input_field = text_input("Type your message...", &self.input)
            .on_input(Message::InputChanged)
            .padding(10);
        
        if !self.is_loading {
            input_field = input_field.on_submit(Message::SendMessage);
        }

        let input_area = row![
            input_field,
            if !self.is_loading {
                button("Send")
                    .on_press(Message::SendMessage)
                    .padding(10)
            } else {
                button(text("⏳"))
                    .padding(10)
                    .style(|theme: &iced::Theme, _status| {
                        button::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgb(0.5, 0.5, 0.5))),
                            text_color: theme.palette().text,
                            ..button::Style::default()
                        }
                    })
            }
        ]
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
