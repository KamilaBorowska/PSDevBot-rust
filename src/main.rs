mod config;
mod unbounded;
mod webhook;

use config::Config;
use log::info;
use showdown::futures::stream::{SplitStream, StreamExt};
use showdown::message::{Kind, UpdateUser};
use showdown::{connect_to_url, ReceiveExt, SendMessage, Stream};
use std::error::Error;
use std::sync::Arc;
use unbounded::UnboundedSender;
use webhook::start_server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::new()?;
    env_logger::init();
    start(config).await
}

async fn start(config: Config) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let mut stream = connect_to_url(&config.server).await?;
    loop {
        if let Kind::Challenge(ch) = stream.receive().await?.kind() {
            ch.login_with_password(&mut stream, &config.user, &config.password)
                .await?;
            break;
        }
    }
    let (sender, receiver) = stream.split();
    run_authenticated(UnboundedSender::new(sender), receiver, config).await
}

async fn run_authenticated(
    sender: UnboundedSender,
    mut receiver: SplitStream<Stream>,
    config: Config,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let config = Arc::new(config);
    let _server = start_server(config.clone(), &sender);
    loop {
        let message = receiver.receive().await?;
        info!("Received message: {:?}", message);
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.kind() {
            sender
                .send(SendMessage::global_command(format_args!(
                    "join {}",
                    config.room_name
                )))
                .await?;
        }
    }
}
