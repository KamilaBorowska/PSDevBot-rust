use crate::config::Config;
use crate::github_api::{GitHubApi, User};
use crate::unbounded::UnboundedSender;
use askama::Template;
use dashmap::DashSet;
use futures::channel::oneshot;
use futures::FutureExt;
use htmlescape::encode_minimal as h;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::Deserialize;
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

#[derive(Debug, Deserialize)]
struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    forced: bool,
    commits: Vec<Commit>,
    compare: String,
    pusher: Pusher,
    repository: Repository,
}

impl PushEvent {
    async fn get_message(&self, mut github_api: Option<&mut GitHubApi>) -> String {
        let pushed = if self.forced {
            "<font color='red'>force-pushed</font>"
        } else {
            "pushed"
        };
        let mut output = format!(
            concat!(
                "addhtmlbox {repo} <a href='https://github.com/{pusher}'>",
                "<font color='909090'>{pusher}</font></a> ",
                "{pushed} <a href='{compare}'><b>{commits}</b> new ",
                "commit{s}</a>",
            ),
            repo = self.repository,
            pusher = h(&self.pusher.name),
            pushed = pushed,
            compare = h(&self.compare),
            commits = self.commits.len(),
            s = if self.commits.len() == 1 { "" } else { "s" },
        );
        for commit in &self.commits {
            let commit_view = commit
                .to_view(&self.repository.html_url, github_api.as_deref_mut())
                .await;
            output += &format!("<br>{}", commit_view);
        }
        output
    }

    fn get_branch(&self) -> &str {
        self.git_ref.rsplit('/').next().unwrap()
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
    async fn to_view<'a>(
        &'a self,
        url: &str,
        github_api: Option<&'a mut GitHubApi>,
    ) -> ViewCommit<'a> {
        let message = self.message.split('\n').next().unwrap();
        ViewCommit {
            id: &self.id[..6],
            message,
            full_message: &self.message,
            formatted_message: format_title(message, url),
            author: self.author.to_view(github_api).await,
            url: &self.url,
        }
    }
}

#[derive(Template)]
#[template(path = "commit.html")]
struct ViewCommit<'a> {
    id: &'a str,
    message: &'a str,
    full_message: &'a str,
    formatted_message: String,
    author: ViewAuthor<'a>,
    url: &'a str,
}

fn format_title(message: &str, url: &str) -> String {
    static ISSUE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r#"#([0-9]+)"#).unwrap());
    ISSUE_PATTERN
        .replace_all(&h(message), |c: &Captures| {
            format!("<a href='{}/issues/{}'>{}</a>", h(url), h(&c[1]), &c[0])
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

impl Author {
    async fn to_view<'a>(&'a self, github_api: Option<&'a mut GitHubApi>) -> ViewAuthor<'a> {
        let username = if let Some(username) = &self.username {
            let github_metadata = if let Some(github_api) = github_api {
                github_api.fetch_user(username).await
            } else {
                None
            };
            Some(Username {
                username,
                github_metadata,
            })
        } else {
            None
        };
        ViewAuthor {
            name: &self.name,
            username,
        }
    }
}

#[derive(Template)]
#[template(path = "author.html")]
struct ViewAuthor<'a> {
    name: &'a str,
    username: Option<Username<'a>>,
}

#[derive(Template)]
#[template(path = "username.html")]
struct Username<'a> {
    username: &'a str,
    github_metadata: Option<&'a User>,
}

#[derive(Debug, Deserialize, Template)]
#[template(path = "repository.html")]
struct Repository {
    full_name: String,
    name: String,
    html_url: String,
    default_branch: String,
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

#[derive(Debug, Deserialize)]
struct PullRequestEvent {
    action: String,
    pull_request: PullRequest,
    repository: Repository,
    sender: Sender,
}

impl PullRequestEvent {
    fn to_view(&self) -> ViewPullRequestEvent<'_> {
        ViewPullRequestEvent {
            action: match self.action.as_str() {
                "synchronize" => "updated",
                "review_requested" => "requested a review for",
                action => action,
            },
            pull_request: &self.pull_request,
            repository: &self.repository,
            sender: &self.sender,
        }
    }
}

#[derive(Template)]
#[template(path = "pull_request_event.html")]
struct ViewPullRequestEvent<'a> {
    action: &'a str,
    pull_request: &'a PullRequest,
    repository: &'a Repository,
    sender: &'a Sender,
}

#[derive(Debug, Deserialize, Template)]
#[template(path = "pull_request.html")]
struct PullRequest {
    number: u32,
    html_url: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct Sender {
    login: String,
}

fn html_command(room_id: &str, input: &str) -> SendMessage {
    // Workaround for https://github.com/smogon/pokemon-showdown/pull/7611
    SendMessage::chat_command(RoomId(room_id), input.replace("here", "her&#101;"))
}

#[cfg(test)]
mod test {
    use super::{Author, Commit, PullRequest, PullRequestEvent, Repository, Sender};

    #[tokio::test]
    async fn test_commit() {
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
            commit.to_view("shouldn't be used", None).await.to_string(),
            concat!(
                "<a href='http:&#x2f;&#x2f;example.com'>",
                "<font color=606060><kbd>0da259</kbd></font></a>\n",
                r#"<span title="Konrad Borowski"><font color=909090>xfix</font></span>: "#,
                "<span title='Hello, world!'>Hello, world!</span>",
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
                full_name: "Super/ExampleCom".into(),
                html_url: "http://example.com/".into(),
                default_branch: "master".into(),
            },
            sender: Sender { login: "Me".into() },
        };
        assert_eq!(
            event.to_view().to_string(),
            concat!(
                "[<a href='http:&#x2f;&#x2f;example.com&#x2f;'><font color=FF00FF>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Me</font></a> created pull request ",
                "<a href='http:&#x2f;&#x2f;example.com&#x2f;pr&#x2f;1'>#1</a>: Hello, world",
            ),
        );
    }
}
