mod schema;

use crate::config::Config;
use crate::unbounded::UnboundedSender;
use dashmap::DashSet;
use futures::channel::oneshot;
use futures::FutureExt;
use schema::{PullRequestEvent, PushEvent};
use showdown::{RoomId, SendMessage};
use std::time::Duration;
use tokio::time;
use warp::reject::Reject;
use warp::{path, Filter, Rejection};
use warp_github_webhook::webhook;

pub fn start_server(
    config: &'static Config,
    sender: &'static UnboundedSender,
) -> oneshot::Sender<()> {
    let (tx, rx) = oneshot::channel();
    let port = config.port;
    tokio::spawn(
        warp::serve(get_route(config, sender).with(warp::log("webhook")))
            .bind_with_graceful_shutdown(([0, 0, 0, 0], port), rx.map(|_| ()))
            .1,
    );
    tx
}

fn get_route(
    config: &'static Config,
    sender: &'static UnboundedSender,
) -> impl Clone + Filter<Extract = (&'static str,), Error = Rejection> {
    let skip_pull_requests = &*Box::leak(Box::new(DashSet::new()));
    path!("github" / "callback").and(
        webhook(warp_github_webhook::Kind::PUSH, SecretGetter(config))
            .and_then(move |push_event| handle_push_event(config, sender, push_event))
            .or(webhook(
                warp_github_webhook::Kind::PULL_REQUEST,
                SecretGetter(config),
            )
            .and_then(move |pull_request| {
                handle_pull_request(config, skip_pull_requests, sender, pull_request)
            }))
            .unify(),
    )
}

#[derive(Clone)]
struct SecretGetter(&'static Config);

impl AsRef<str> for SecretGetter {
    fn as_ref(&self) -> &str {
        &self.0.secret
    }
}

async fn handle_push_event(
    config: &'static Config,
    sender: &'static UnboundedSender,
    push_event: PushEvent,
) -> Result<&'static str, Rejection> {
    let mut github_api = match &config.github_api {
        Some(github_api) => Some(github_api.lock().await),
        None => None,
    };
    if push_event.repository.default_branch == push_event.get_branch() {
        for room in config.rooms_for(&push_event.repository.full_name) {
            let message = html_command(
                room,
                &push_event.get_message(github_api.as_deref_mut()).await,
            );
            sender
                .send(message)
                .await
                .map_err(|_| warp::reject::custom(ChannelError))?;
        }
    }
    Ok("")
}

const IGNORE_ACTIONS: &[&str] = &[
    "ready_for_review",
    "labeled",
    "unlabeled",
    "converted_to_draft",
];

async fn handle_pull_request(
    config: &'static Config,
    skip_pull_requests: &'static DashSet<u32>,
    sender: &'static UnboundedSender,
    pull_request: PullRequestEvent,
) -> Result<&'static str, Rejection> {
    let number = pull_request.pull_request.number;
    if !IGNORE_ACTIONS.contains(&&pull_request.action[..]) && skip_pull_requests.insert(number) {
        tokio::spawn(async move {
            time::delay_for(Duration::from_secs(10 * 60)).await;
            skip_pull_requests.remove(&number);
        });
        for room in config.rooms_for(&pull_request.repository.full_name) {
            let message = html_command(room, &format!("addhtmlbox {}", pull_request.to_view()));
            sender
                .send(message)
                .await
                .map_err(|_| warp::reject::custom(ChannelError))?;
        }
    }
    Ok("")
}

#[derive(Debug)]
struct ChannelError;

impl Reject for ChannelError {}

fn html_command(room_id: &str, input: &str) -> SendMessage {
    // Workaround for https://github.com/smogon/pokemon-showdown/pull/7611
    SendMessage::chat_command(RoomId(room_id), input.replace("here", "her&#101;"))
}
