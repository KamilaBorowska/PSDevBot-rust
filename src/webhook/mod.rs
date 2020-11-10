mod schema;

use crate::config::Config;
use crate::unbounded::UnboundedSender;
use bytes::Bytes;
use dashmap::DashSet;
use futures::channel::oneshot;
use futures::FutureExt;
use hmac::{Hmac, Mac, NewMac};
use log::info;
use schema::{InitialPayload, PullRequestEvent, PushEvent};
use serde::de::DeserializeOwned;
use sha2::Sha256;
use showdown::{RoomId, SendMessage};
use std::fmt::{self, Debug, Display, Formatter};
use std::time::Duration;
use tokio::time;
use warp::reject::Reject;
use warp::{path, Filter, Rejection};

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
    path!("github" / "callback")
        .and(warp::header::optional("X-Hub-Signature-256"))
        .and(warp::header("X-GitHub-Event"))
        .and(warp::body::bytes())
        .and_then(move |signature, event: String, bytes: Bytes| async move {
            info!("Got event {}", event);
            let rooms = get_rooms(config, signature, &bytes)?;
            match event.as_str() {
                "push" => handle_push_event(config, sender, rooms, json(&bytes)?).await?,
                "pull_request" => {
                    handle_pull_request(skip_pull_requests, sender, rooms, json(&bytes)?).await?
                }
                _ => {}
            }
            Ok::<_, Rejection>("")
        })
}

fn get_rooms<'a>(
    config: &'a Config,
    signature: Option<String>,
    bytes: &[u8],
) -> Result<&'a [String], Rejection> {
    let payload: InitialPayload = json(bytes)?;
    let (rooms, secret) = config.rooms_for(&payload.repository.full_name);
    verify_signature(secret, signature, bytes)?;
    Ok(rooms)
}

fn verify_signature(
    secret: &str,
    signature: Option<String>,
    bytes: &[u8],
) -> Result<(), Rejection> {
    if !secret.is_empty() {
        let signature = signature.ok_or_else(|| reject("Missing signature"))?;
        let signature = signature
            .strip_prefix("sha256=")
            .ok_or_else(|| reject("Signature doesn't start with sha256="))?;
        let signature = hex::decode(signature).map_err(reject)?;
        let mut mac =
            Hmac::<Sha256>::new_varkey(secret.as_bytes()).expect("HMAC can take a key of any size");
        mac.update(bytes);
        mac.verify(&signature).map_err(reject)?;
    }
    Ok(())
}

fn json<T: DeserializeOwned>(input: &[u8]) -> Result<T, Rejection> {
    serde_json::from_slice(input).map_err(reject)
}

async fn handle_push_event(
    config: &'static Config,
    sender: &'static UnboundedSender,
    rooms: &[String],
    push_event: PushEvent,
) -> Result<(), Rejection> {
    let mut github_api = match &config.github_api {
        Some(github_api) => Some(github_api.lock().await),
        None => None,
    };
    if push_event.repository.default_branch == push_event.branch() {
        for room in rooms {
            let message = html_command(
                room,
                &push_event.get_message(github_api.as_deref_mut()).await,
            );
            sender.send(message).await.map_err(reject)?;
        }
    }
    Ok(())
}

const IGNORE_ACTIONS: &[&str] = &[
    "ready_for_review",
    "labeled",
    "unlabeled",
    "converted_to_draft",
];

async fn handle_pull_request(
    skip_pull_requests: &'static DashSet<u32>,
    sender: &'static UnboundedSender,
    rooms: &[String],
    pull_request: PullRequestEvent,
) -> Result<(), Rejection> {
    let number = pull_request.pull_request.number;
    if !IGNORE_ACTIONS.contains(&&pull_request.action[..]) && skip_pull_requests.insert(number) {
        tokio::spawn(async move {
            time::delay_for(Duration::from_secs(10 * 60)).await;
            skip_pull_requests.remove(&number);
        });
        for room in rooms {
            let message = html_command(room, &format!("addhtmlbox {}", pull_request.to_view()));
            sender.send(message).await.map_err(reject)?;
        }
    }
    Ok(())
}

fn reject<T: Display + Send + Sync + 'static>(error: T) -> Rejection {
    warp::reject::custom(ErrorRejection(error))
}

struct ErrorRejection<T>(T);

impl<T: Display> Debug for ErrorRejection<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T: Display + Send + Sync + 'static> Reject for ErrorRejection<T> {}

fn html_command(room_id: &str, input: &str) -> SendMessage {
    // Workaround for https://github.com/smogon/pokemon-showdown/pull/7611
    SendMessage::chat_command(RoomId(room_id), input.replace("here", "her&#101;"))
}
