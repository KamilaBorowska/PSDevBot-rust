#![feature(async_await, await_macro, futures_api)]
#![recursion_limit = "128"]

mod config;
mod unbounded;
mod webhook;

use config::Config;
use log::info;
use sentry::internals::ClientInitGuard;
use sentry::ClientOptions;
use showdown::message::{Kind, UpdateUser};
use showdown::{connect_to_url, Receiver};
use std::error::Error;
use std::sync::Arc;
use tokio::await;
use unbounded::UnboundedSender;
use webhook::start_server;

fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::new()?;
    let _sentry = initialize_sentry(&config);
    tokio::run_async(async move { await!(start(config)).unwrap() });
    Ok(())
}

fn initialize_sentry(config: &Config) -> ClientInitGuard {
    let sentry = sentry::init((
        config.sentry_dsn.as_str(),
        ClientOptions {
            release: option_env!("CI_COMMIT_SHA").map(<&str>::into),
            ..ClientOptions::default()
        },
    ));
    sentry::integrations::env_logger::init(None, Default::default());
    sentry::integrations::panic::register_panic_handler();
    sentry
}

async fn start(config: Config) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let (mut sender, mut receiver) = await!(connect_to_url(&config.server))?;
    loop {
        if let Kind::Challenge(ch) = await!(receiver.receive())?.kind() {
            await!(ch.login_with_password(&mut sender, &config.user, &config.password))?;
            break;
        }
    }
    await!(run_authenticated(
        UnboundedSender::new(sender),
        receiver,
        config,
    ))
}

async fn run_authenticated(
    sender: UnboundedSender,
    mut receiver: Receiver,
    config: Config,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let config = Arc::new(config);
    let _server = start_server(config.clone(), &sender);
    loop {
        let message = await!(receiver.receive())?;
        info!("Received message: {:?}", message);
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.kind() {
            sender.send_global_command("away")?;
            sender.send_global_command(&format!("join {}", config.room_name))?;
        }
    }
}
