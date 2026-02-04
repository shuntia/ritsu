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
}

#[derive(Debug, Clone)]
struct ChatMessage {
    content: String,
    is_user: bool,
}

impl RitsuGui {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                input: String::new(),
                messages: Vec::new(),
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        String::from("Ritsu - AI Agent")
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(value) => {
                self.input = value;
                Task::none()
            }
            Message::SendMessage => {
                if !self.input.is_empty() {
                    let content = self.input.clone();
                    self.messages.push(ChatMessage {
                        content: content.clone(),
                        is_user: true,
                    });
                    self.input.clear();

                    Task::perform(
                        async move {
                            let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
                            let request = ritsu_common::protocol::ClientRequest::SendMessage {
                                content: content.clone(),
                            };
                            client.send_request(request).await
                        },
                        |result| match result {
                            Ok(response) => match response {
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
                Task::none()
            }
            Message::ServerResponse(result) => {
                match result {
                    Ok(msg) => println!("✓ {msg}"),
                    Err(e) => eprintln!("✗ Error: {e}"),
                }
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

        let input_area = row![
            text_input("Type your message...", &self.input)
                .on_input(Message::InputChanged)
                .on_submit(Message::SendMessage)
                .padding(10),
            button("Send")
                .on_press(Message::SendMessage)
                .padding(10),
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

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

pub async fn run() -> anyhow::Result<()> {
    iced::application(RitsuGui::title, RitsuGui::update, RitsuGui::view)
        .theme(RitsuGui::theme)
        .run_with(RitsuGui::new)
        .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}
