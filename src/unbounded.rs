use log::info;
use futures::channel::mpsc;
use futures::channel::mpsc::SendError;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use showdown::{SendMessage, Stream};
use tokio::time;

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
                time::delay_for(time::Duration::from_millis(700)).await;
            }
        });
        Self { sender: tx }
    }

    pub async fn send(&self, message: SendMessage) -> Result<(), SendError> {
        (&self.sender).send(message).await
    }
}
