#![feature(async_await, await_macro, futures_api)]
#![recursion_limit = "128"]

mod unbounded;
mod webhook;

use futures03::prelude::*;
use showdown::message::{Kind, UpdateUser};
use showdown::{connect_to_url, url::Url, Receiver};
use std::env;
use std::error::Error;
use tokio::await;
use unbounded::UnboundedSender;
use webhook::start_server;

fn main() -> Result<(), Box<dyn Error>> {
    tokio::run_async(
        start(
            env::var("PSDEVBOT_SERVER")?,
            env::var("PSDEVBOT_USER")?,
            env::var("PSDEVBOT_PASSWORD")?,
            env::var("PSDEVBOT_SECRET")?,
            match env::var("PSDEVBOT_PORT") {
                Ok(port) => port.parse()?,
                Err(_) => 3030,
            },
        )
        .map(|e| e.unwrap()),
    );
    Ok(())
}

async fn start(
    server: String,
    login: String,
    password: String,
    secret: String,
    port: u16,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let (mut sender, mut receiver) = await!(connect_to_url(&Url::parse(&server)?))?;
    loop {
        let message = await!(receiver.receive())?;
        if let Kind::Challenge(ch) = message.parse().kind {
            await!(ch.login_with_password(&mut sender, &login, &password))?;
            break;
        }
    }
    await!(run_authenticated(
        UnboundedSender::new(sender),
        receiver,
        secret,
        port,
    ))
}

async fn run_authenticated(
    sender: UnboundedSender,
    mut receiver: Receiver,
    secret: String,
    port: u16,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let _server = start_server(secret, &sender, port);
    loop {
        let message = await!(receiver.receive())?;
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.parse().kind {
            sender.send_global_command("away")?;
            sender.send_global_command("join bot dev")?;
        }
    }
}
