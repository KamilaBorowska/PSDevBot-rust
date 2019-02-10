use futures::sync::mpsc;
use showdown::{RoomId, Sender};
use std::error::Error;
use tokio::await;
use tokio::prelude::*;

#[derive(Clone, Debug)]
pub struct UnboundedSender {
    sender: mpsc::UnboundedSender<Message>,
}

impl UnboundedSender {
    pub fn new(mut showdown_sender: Sender) -> Self {
        let (sender, mut receiver) = mpsc::unbounded();
        tokio::spawn_async(
            async move {
                while let Some(message) = await!(receiver.next()) {
                    (match message.unwrap() {
                        Message::GlobalCommand(c) => {
                            await!(showdown_sender.send_global_command(&c))
                        }
                        Message::ChatMessage(r, c) => {
                            await!(showdown_sender.send_chat_message(RoomId(&r), &c))
                        }
                    })
                    .unwrap()
                }
            },
        );
        Self { sender }
    }

    pub fn send_chat_message(
        &self,
        room_id: RoomId<'_>,
        message: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.sender.unbounded_send(Message::ChatMessage(
            room_id.0.to_string(),
            message.to_string(),
        ))?;
        Ok(())
    }

    pub fn send_global_command(&self, command: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.sender
            .unbounded_send(Message::GlobalCommand(command.to_string()))?;
        Ok(())
    }
}

#[derive(Debug)]
enum Message {
    GlobalCommand(String),
    ChatMessage(String, String),
}
