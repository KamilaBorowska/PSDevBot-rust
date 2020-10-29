use crate::config::Config;
use crate::unbounded::UnboundedSender;
use htmlescape::encode_minimal as h;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde::Deserialize;
use showdown::futures::channel::oneshot;
use showdown::futures::{Future, FutureExt};
use showdown::{RoomId, SendMessage};
use std::sync::Arc;
use warp::reject::Reject;
use warp::{path, Filter, Rejection};
use warp_github_webhook::webhook;

pub fn start_server(config: Arc<Config>, sender: &UnboundedSender) -> oneshot::Sender<()> {
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
    config: Arc<Config>,
    sender: &UnboundedSender,
) -> impl Clone + Filter<Extract = (&'static str,), Error = Rejection> {
    let push_sender = sender.clone();
    let pull_request_sender = sender.clone();
    let config_clone = config.clone();
    path!("github" / "callback").and(
        webhook(
            warp_github_webhook::Kind::PUSH,
            SecretGetter(config.clone()),
        )
        .and_then(move |push_event| handle_push_event(&config_clone, &push_sender, push_event))
        .or(webhook(
            warp_github_webhook::Kind::PULL_REQUEST,
            SecretGetter(config.clone()),
        )
        .and_then(move |pull_request| {
            handle_pull_request(&config, &pull_request_sender, pull_request)
        }))
        .unify(),
    )
}

#[derive(Clone)]
struct SecretGetter(Arc<Config>);

impl AsRef<str> for SecretGetter {
    fn as_ref(&self) -> &str {
        &self.0.room_name
    }
}

fn handle_push_event(
    config: &Config,
    sender: &UnboundedSender,
    push_event: PushEvent,
) -> impl Future<Output = Result<&'static str, Rejection>> {
    let message = SendMessage::chat_message(RoomId(&config.room_name), &push_event.get_message());
    let sender = sender.clone();
    async move {
        sender
            .send(message)
            .await
            .map_err(|_| warp::reject::custom(ChannelError))?;
        Ok("")
    }
}

#[derive(Debug, Deserialize)]
struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    created: bool,
    forced: bool,
    commits: Vec<Commit>,
    compare: String,
    pusher: Pusher,
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
                "/addhtmlbox {repo} <a href='https://github.com/{pusher}'>",
                "<font color='909090'>{pusher}</font></a> ",
                "{pushed} <a href='{compare}'><b>{commits}</b> new ",
                "commit{s}</a> to <font color='800080'>{branch}</font>",
            ),
            repo = self.repository.format(),
            pusher = h(&self.pusher.name),
            pushed = pushed,
            compare = h(&self.compare),
            commits = self.commits.len(),
            s = if self.commits.len() == 1 { "" } else { "s" },
            branch = h(self.get_branch()),
        );
        for commit in &self.commits {
            output += &commit.format(&self.repository.html_url);
        }
        output
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
    author: Author,
    url: String,
}

impl Commit {
    fn format(&self, url: &str) -> String {
        let (message, more) = match self.message.find('\n') {
            Some(index) => (&self.message[..index], true),
            None => (&self.message[..], false),
        };
        let formatted_name = format!("<font color='909090'>{}</font>", h(&self.author.name));
        format!(
            concat!(
                "<br /><a href='{url}'><font color='606060'>{id}</font></a> ",
                "{author}: {message}{more}",
            ),
            url = h(&self.url),
            id = &self.id[0..6],
            author = match &self.author.username {
                Some(username) => format!(
                    "<a href='https://github.com/{username}'>{formatted_name}</a>",
                    username = h(username),
                    formatted_name = formatted_name,
                ),
                None => formatted_name,
            },
            message = format_title(message, url),
            more = if more { "\u{2026}" } else { "" },
        )
    }
}

fn format_title(message: &str, url: &str) -> String {
    let message = h(message);
    lazy_static! {
        static ref ISSUE_PATTERN: Regex = Regex::new(r#"#([0-9]+)"#).unwrap();
    }
    ISSUE_PATTERN
        .replace_all(&message, |c: &Captures<'_>| {
            format!("<a href='{}/issues/{}'>{}</a>", h(url), h(&c[1]), h(&c[0]))
        })
        .to_string()
}

#[derive(Debug, Deserialize)]
struct Pusher {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Author {
    name: String,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Repository {
    name: String,
    html_url: String,
}

impl Repository {
    fn format(&self) -> String {
        let repo = match self.name.as_str() {
            "Pokemon-Showdown" => "server",
            "Pokemon-Showdown-Client" => "client",
            "Pokemon-Showdown-Dex" => "dex",
            repo => repo,
        };
        format!(
            "[<a href='{url}'><font color='FF00FF'>{name}</font></a>]",
            url = h(&self.html_url),
            name = h(repo),
        )
    }
}

fn handle_pull_request(
    config: &Config,
    sender: &UnboundedSender,
    pull_request: PullRequestEvent,
) -> impl Future<Output = Result<&'static str, Rejection>> {
    let message = SendMessage::chat_message(RoomId(&config.room_name), &pull_request.get_message());
    let sender = sender.clone();
    async move {
        sender
            .send(message)
            .await
            .map_err(|_| warp::reject::custom(ChannelError))?;
        Ok("")
    }
}

#[derive(Debug)]
struct ChannelError;

impl Reject for ChannelError {}

#[derive(Debug, Deserialize)]
struct PullRequestEvent {
    action: String,
    pull_request: PullRequest,
    repository: Repository,
    sender: Sender,
}

impl PullRequestEvent {
    fn get_message(&self) -> String {
        let escaped_action;
        format!(
            concat!(
                "/addhtmlbox {repo} <a href='https://github.com/{author}'>",
                "<font color='909090'>{author}</font></a> {action} pull request ",
                "<a href='{url}'>#{number}</a>: {title}",
            ),
            repo = self.repository.format(),
            author = h(&self.sender.login),
            action = match self.action.as_str() {
                "synchronize" => "updated",
                action => {
                    escaped_action = h(action);
                    &escaped_action
                }
            },
            url = h(&self.pull_request.html_url),
            number = self.pull_request.number,
            title = format_title(&self.pull_request.title, &self.repository.html_url),
        )
    }
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u32,
    html_url: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct Sender {
    login: String,
}

#[cfg(test)]
mod test {
    use super::{Author, Commit, PullRequest, PullRequestEvent, Repository, Sender};

    #[test]
    fn test_commit() {
        let commit = Commit {
            id: "0da2590a700d054fc2ce39ddc9c95f360329d9be".into(),
            message: "Hello, world!".into(),
            author: Author {
                name: "Konrad Borowski".into(),
                username: Some("xfix".into()),
            },
            url: "http://example.com".into(),
        };
        assert_eq!(
            commit.format("shouldn't be used"),
            concat!(
                "<br /><a href='http://example.com'>",
                "<font color='606060'>0da259</font></a> ",
                "<a href='https://github.com/xfix'>",
                "<font color='909090'>Konrad Borowski</font></a>: ",
                "Hello, world!",
            ),
        );
    }

    #[test]
    fn test_pull_request() {
        let event = PullRequestEvent {
            action: "created".into(),
            pull_request: PullRequest {
                number: 1,
                html_url: "http://example.com/pr/1".into(),
                title: "Hello, world".into(),
            },
            repository: Repository {
                name: "ExampleCom".into(),
                html_url: "http://example.com/".into(),
            },
            sender: Sender { login: "Me".into() },
        };
        assert_eq!(
            event.get_message(),
            concat!(
                "/addhtmlbox [<a href='http://example.com/'><font color='FF00FF'>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Me</font></a> created pull request ",
                "<a href='http://example.com/pr/1'>#1</a>: Hello, world",
            ),
        );
    }
}
