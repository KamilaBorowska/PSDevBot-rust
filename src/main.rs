mod config;
mod github_api;
mod unbounded;
mod webhook;

use config::Config;
use futures::stream::{SplitStream, StreamExt};
use log::info;
use showdown::message::{Kind, UpdateUser};
use showdown::{connect_to_url, SendMessage, Stream};
use std::error::Error;
use std::time::Duration;
use tokio::time::timeout;
use unbounded::DelayedSender;
use webhook::start_server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let _ = dotenv::dotenv();
    let config = Config::new()?;
    env_logger::init();
    start(config).await
}

async fn start(config: Config) -> Result<(), Box<dyn Error + Send + Sync>> {
    let stream = timeout(Duration::from_secs(30), authenticate(&config)).await??;
    let (sender, receiver) = stream.split();
    run_authenticated(DelayedSender::new(sender), receiver, config).await
}

async fn authenticate(config: &Config) -> Result<Stream, Box<dyn Error + Send + Sync>> {
    let mut stream = connect_to_url(&config.server).await?;
    while let Some(message) = stream.next().await {
        if let Kind::Challenge(ch) = message?.kind() {
            ch.login_with_password(&mut stream, &config.user, &config.password)
                .await?;
            return Ok(stream);
        }
    }
    Err("Server disconnected before authenticating".into())
}

async fn run_authenticated(
    sender: DelayedSender,
    mut receiver: SplitStream<Stream>,
    config: Config,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let config = Box::leak(Box::new(config));
    let sender = Box::leak(Box::new(sender));
    let _server = start_server(config, sender);
    while let Some(message) = receiver.next().await {
        let message = message?;
        info!("Received message: {:?}", message);
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.kind() {
            for room in config.all_rooms() {
                let command = SendMessage::global_command(format_args!("join {}", room));
                sender.send(command).await?;
            }
        }
    }
    Ok(())
}
