use log::info;
use showdown::futures::channel::mpsc;
use showdown::futures::channel::mpsc::SendError;
use showdown::futures::stream::SplitSink;
use showdown::futures::{SinkExt, StreamExt};
use showdown::{SendMessage, Stream};

#[derive(Clone, Debug)]
pub struct UnboundedSender {
    sender: mpsc::UnboundedSender<SendMessage>,
}

impl UnboundedSender {
    pub fn new(mut showdown_sender: SplitSink<Stream, SendMessage>) -> Self {
        let (tx, mut rx) = mpsc::unbounded();
        tokio::spawn(async move {
            while let Some(message) = rx.next().await {
                info!("Sent message: {:?}", message);
                if showdown_sender.send(message).await.is_err() {
                    return;
                }
            }
        });
        Self { sender: tx }
    }

    pub async fn send(&self, message: SendMessage) -> Result<(), SendError> {
        (&self.sender).send(message).await
    }
}
