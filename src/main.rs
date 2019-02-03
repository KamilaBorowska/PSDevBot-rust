#![feature(async_await, await_macro, futures_api)]
#![recursion_limit = "128"]

mod unbounded;

use futures03::prelude::*;
use htmlescape::encode_minimal as h;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde_derive::Deserialize;
use showdown::message::{Kind, UpdateUser};
use showdown::{connect, RoomId};
use std::env;
use std::error::Error;
use tokio::await;
use unbounded::UnboundedSender;
use warp::{self, path, Filter};
use warp_github_webhook::{webhook, PUSH};

fn main() -> Result<(), Box<dyn Error>> {
    tokio::run_async(
        start(
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
    login: String,
    password: String,
    secret: String,
    port: u16,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let (mut sender, mut receiver) = await!(connect("showdown"))?;
    loop {
        let message = await!(receiver.receive())?;
        if let Kind::Challenge(ch) = message.parse().kind {
            await!(ch.login_with_password(&mut sender, &login, &password))?;
            break;
        }
    }
    let mut sender = UnboundedSender::new(sender);
    let route_sender = sender.clone();
    let route = path!("github" / "callback")
        .and(webhook(PUSH, secret))
        .and_then(move |push_event| {
            handle_push_event(route_sender.clone(), push_event)
                .boxed()
                .compat()
        });
    tokio::spawn(warp::serve(route).bind(([0, 0, 0, 0], port)));
    loop {
        let message = await!(receiver.receive())?;
        if let Kind::UpdateUser(UpdateUser { named: true, .. }) = message.parse().kind {
            sender.send_global_command("away")?;
            sender.send_global_command("join bot dev")?;
        }
    }
}

async fn handle_push_event(
    mut sender: UnboundedSender,
    push_event: PushEvent,
) -> Result<&'static str, warp::Rejection> {
    sender
        .send_chat_message(RoomId("botdevelopment"), &push_event.get_message())
        .map_err(warp::reject::custom)?;
    Ok("")
}

#[derive(Debug, Deserialize)]
struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    created: bool,
    forced: bool,
    commits: Vec<Commit>,
    compare: String,
    pusher: User,
    repository: Repository,
}

impl PushEvent {
    fn get_message(&self) -> String {
        let pushed = if self.created {
            "pushed <font color='red'>in new branch</font>"
        } else if self.forced {
            "<font color='red'>force-pushed</font>"
        } else {
            "pushed"
        };
        let mut output = format!(
            concat!(
                "/addhtmlbox [<font color='FF00FF'>{repo}</font>] ",
                "<font color='909090'>{pusher}</font> {pushed} ",
                r#"<a href="{compare}"><b>{commits}</b> new commit{s}</a> "#,
                "to <font color='800080'>{branch}</font>",
            ),
            repo = h(self.get_repo_name()),
            pusher = h(&self.pusher.name),
            pushed = pushed,
            compare = self.compare,
            commits = self.commits.len(),
            s = if self.commits.len() == 1 { "" } else { "s" },
            branch = h(self.get_branch()),
        );
        for commit in &self.commits {
            output += &commit.format(&self.repository.url);
        }
        output
    }

    fn get_repo_name(&self) -> &str {
        match self.repository.name.as_str() {
            "Pokemon-Showdown" => "server",
            "Pokemon-Showdown-Client" => "client",
            "Pokemon-Showdown-Dex" => "dex",
            repo => repo,
        }
    }

    fn get_branch(&self) -> &str {
        match self.git_ref.rfind('/') {
            Some(index) => &self.git_ref[index + 1..],
            None => &self.git_ref,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Commit {
    id: String,
    message: String,
    author: User,
    url: String,
}

impl Commit {
    fn format(&self, url: &str) -> String {
        let (message, more) = match self.message.find('\n') {
            Some(index) => (&self.message[..index], true),
            None => (&self.message[..], false),
        };
        let message = h(message);
        lazy_static! {
            static ref ISSUE_PATTERN: Regex = Regex::new(r#"#(\d+)"#).unwrap();
        }
        let message = ISSUE_PATTERN.replace_all(&message, |c: &Captures<'_>| {
            format!("<a href='{}/issues/{}'>{}</a>", h(url), h(&c[1]), h(&c[0]))
        });
        format!(
            concat!(
                "<br /><a href=\"{url}\"><font color='606060'>{id}</font></a> ",
                "<font color='909090'>{author}</font>: {message}{more}",
            ),
            url = h(&self.url),
            id = &self.id[0..6],
            author = h(&self.author.name),
            message = message,
            more = if more { "..." } else { "" },
        )
    }
}

#[derive(Debug, Deserialize)]
struct User {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Repository {
    name: String,
    url: String,
}
