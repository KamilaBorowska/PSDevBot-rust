#![feature(async_await, await_macro, futures_api)]
#![recursion_limit = "128"]

mod config;
mod unbounded;
mod webhook;

use config::Config;
use log::info;
use showdown::message::{Kind, UpdateUser};
use showdown::{connect_to_url, url::Url, Receiver};
use std::error::Error;
use tokio::await;
use unbounded::UnboundedSender;
use webhook::start_server;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let config = Config::new()?;
    tokio::run_async(async move { await!(start(config)).unwrap() });
    Ok(())
}

async fn start(config: Config) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let (mut sender, mut receiver) = await!(connect_to_url(&Url::parse(&config.server)?))?;
    loop {
        let message = await!(receiver.receive())?;
        if let Kind::Challenge(ch) = message.parse().kind {
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
    let _server = start_server(config, &sender);
    loop {
        let message = await!(receiver.receive())?;
        info!("Received message: {:?}", message);
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.parse().kind {
            sender.send_global_command("away")?;
            sender.send_global_command("join bot dev")?;
        }
    }
}
