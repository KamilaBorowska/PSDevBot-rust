use futures::channel::mpsc::{self, SendError};
use futures::{Sink, SinkExt};
use log::info;
use showdown::SendMessage;
use tokio::time::Duration;
use tokio_stream::StreamExt;

#[derive(Clone, Debug)]
pub struct DelayedSender {
    sender: mpsc::UnboundedSender<SendMessage>,
}

impl DelayedSender {
    pub fn new(mut showdown_sender: impl Sink<SendMessage> + Send + Unpin + 'static) -> Self {
        let (tx, rx) = mpsc::unbounded::<SendMessage>();
        let rx = rx.throttle(Duration::from_millis(700));
        tokio::spawn(async move {
            tokio::pin!(rx);
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

#[cfg(test)]
mod test {
    use super::DelayedSender;
    use futures::channel::mpsc;
    use futures::StreamExt;
    use showdown::SendMessage;
    use std::error::Error;
    use tokio::time::{self, Duration, Instant};

    #[tokio::test]
    async fn sender_does_not_delay_on_first_message() -> Result<(), Box<dyn Error + Send + Sync>> {
        time::pause();
        // Spawning a task is necessary to workaround https://github.com/tokio-rs/tokio/issues/3108
        tokio::spawn(async {
            let (tx, mut rx) = mpsc::unbounded();
            let sender = DelayedSender::new(tx);
            let now = Instant::now();
            let message = SendMessage::global_command("test");
            sender.send(message.clone()).await?;
            assert_eq!(rx.next().await, Some(message));
            assert_eq!(now, Instant::now());
            Ok(())
        })
        .await?
    }

    #[tokio::test]
    async fn sender_does_delay_on_second_message() -> Result<(), Box<dyn Error + Send + Sync>> {
        time::pause();
        // Spawning a task is necessary to workaround https://github.com/tokio-rs/tokio/issues/3108
        tokio::spawn(async {
            let (tx, mut rx) = mpsc::unbounded();
            let sender = DelayedSender::new(tx);
            let start = Instant::now();
            let a_message = SendMessage::global_command("a");
            sender.send(a_message.clone()).await?;
            assert_eq!(rx.next().await, Some(a_message));
            assert_eq!(start, Instant::now());
            let b_message = SendMessage::global_command("b");
            sender.send(b_message.clone()).await?;
            assert_eq!(rx.next().await, Some(b_message));
            assert!(Instant::now() + Duration::from_millis(700) >= start);
            Ok(())
        })
        .await?
    }
}
