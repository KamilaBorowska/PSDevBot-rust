mod config;
mod unbounded;
mod webhook;

use config::Config;
use env_logger::Logger;
use log::info;
use sentry::internals::ClientInitGuard;
use sentry::ClientOptions;
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
    let _sentry = initialize_sentry(&config);
    start(config).await
}

fn initialize_sentry(config: &Config) -> ClientInitGuard {
    sentry::init((
        config.sentry_dsn.as_str(),
        ClientOptions {
            release: option_env!("CI_COMMIT_SHA").map(<&str>::into),
            ..ClientOptions::default()
        }
        .add_integration(
            sentry::integrations::log::LogIntegration::default()
                .with_env_logger_dest(Some(Logger::from_default_env())),
        )
        .add_integration(sentry::integrations::panic::PanicIntegration::new()),
    ))
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
            sender.send(SendMessage::global_command("away")).await?;
            sender
                .send(SendMessage::global_command(format_args!(
                    "join {}",
                    config.room_name
                )))
                .await?;
        }
    }
}
