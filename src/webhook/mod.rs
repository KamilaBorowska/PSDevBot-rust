mod schema;

use crate::config::{Config, RoomConfigurationRef, UsernameAliases};
use crate::unbounded::DelayedSender;
use futures::channel::oneshot;
use futures::FutureExt;
use hmac::{Hmac, Mac, NewMac};
use log::info;
use schema::{InitialPayload, PullRequestEvent, PushEvent, PushEventContext};
use serde::Deserialize;
use sha2::Sha256;
use showdown::{RoomId, SendMessage};
use std::collections::HashSet;
use std::fmt::{self, Debug, Display, Formatter};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time;
use warp::hyper::body::Bytes;
use warp::reject::Reject;
use warp::{path, Filter, Rejection};

pub fn start_server(config: &'static Config, sender: Arc<DelayedSender>) -> oneshot::Sender<()> {
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
    sender: Arc<DelayedSender>,
) -> impl Clone + Filter<Extract = (&'static str,), Error = Rejection> {
    let skip_pull_requests = &*Box::leak(Box::new(Mutex::new(HashSet::new())));
    path!("github" / "callback")
        .and(warp::header::optional("X-Hub-Signature-256"))
        .and(warp::header("X-GitHub-Event"))
        .and(warp::body::bytes())
        .and_then(move |signature, event: String, bytes: Bytes| {
            let sender = Arc::clone(&sender);
            async move {
                info!("Got event {}", event);
                let room_configuration = get_rooms(config, signature, &bytes)?;
                match event.as_str() {
                    "push" => {
                        handle_push_event(config, sender, room_configuration, json(&bytes)?).await?
                    }
                    "pull_request" => {
                        handle_pull_request(
                            &config.username_aliases,
                            skip_pull_requests,
                            sender,
                            room_configuration.rooms,
                            json(&bytes)?,
                        )
                        .await?
                    }
                    _ => {}
                }
                Ok::<_, Rejection>("")
            }
        })
}

fn get_rooms<'a>(
    config: &'a Config,
    signature: Option<String>,
    bytes: &[u8],
) -> Result<RoomConfigurationRef<'a>, Rejection> {
    let payload: InitialPayload = json(bytes)?;
    let room_configuration = config.rooms_for(&payload.repository.full_name);
    verify_signature(room_configuration.secret, signature, bytes)?;
    Ok(room_configuration)
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

fn json<'de, T: Deserialize<'de>>(input: &'de [u8]) -> Result<T, Rejection> {
    serde_json::from_slice(input).map_err(reject)
}

async fn handle_push_event<'a>(
    config: &'static Config,
    sender: Arc<DelayedSender>,
    room_configuration: RoomConfigurationRef<'a>,
    push_event: PushEvent<'a>,
) -> Result<(), Rejection> {
    let mut github_api = match &config.github_api {
        Some(github_api) => Some(github_api.lock().await),
        None => None,
    };
    if push_event.repository.default_branch == push_event.branch() {
        for room in room_configuration.rooms {
            let message = html_command(
                room,
                &format!(
                    "addhtmlbox {}",
                    push_event
                        .to_view(PushEventContext {
                            github_api: github_api.as_deref_mut(),
                            username_aliases: &config.username_aliases,
                        })
                        .await
                ),
            );
            sender.send(message).await.map_err(reject)?;
        }
        for room in room_configuration.simple_rooms {
            let message = html_command(
                room,
                &format!(
                    "addhtmlbox {}",
                    push_event
                        .to_simple_view(PushEventContext {
                            github_api: github_api.as_deref_mut(),
                            username_aliases: &config.username_aliases,
                        })
                        .await
                ),
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

async fn handle_pull_request<'a>(
    username_aliases: &'static UsernameAliases,
    skip_pull_requests: &'static Mutex<HashSet<u32>>,
    sender: Arc<DelayedSender>,
    rooms: &'a [String],
    pull_request: PullRequestEvent<'a>,
) -> Result<(), Rejection> {
    let number = pull_request.pull_request.number;
    if !IGNORE_ACTIONS.contains(&&pull_request.action[..])
        && skip_pull_requests.lock().unwrap().insert(number)
    {
        tokio::spawn(async move {
            time::delay_for(Duration::from_secs(10 * 60)).await;
            skip_pull_requests.lock().unwrap().remove(&number);
        });
        for room in rooms {
            let message = html_command(
                room,
                &format!("addhtmlbox {}", pull_request.to_view(username_aliases)),
            );
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
