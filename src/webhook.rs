use crate::config::Config;
use crate::github_api::{GitHubApi, User};
use crate::unbounded::UnboundedSender;
use askama::Template;
use dashmap::DashSet;
use htmlescape::encode_minimal as h;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde::Deserialize;
use showdown::futures::channel::oneshot;
use showdown::futures::{Future, FutureExt};
use showdown::{RoomId, SendMessage};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
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
    let skip_pull_requests = Arc::new(DashSet::new());
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
            handle_pull_request(
                &config,
                &skip_pull_requests,
                &pull_request_sender,
                pull_request,
            )
        }))
        .unify(),
    )
}

#[derive(Clone)]
struct SecretGetter(Arc<Config>);

impl AsRef<str> for SecretGetter {
    fn as_ref(&self) -> &str {
        &self.0.secret
    }
}

fn handle_push_event(
    config: &Arc<Config>,
    sender: &UnboundedSender,
    push_event: PushEvent,
) -> impl Future<Output = Result<&'static str, Rejection>> {
    let config = config.clone();
    let sender = sender.clone();
    async move {
        let mut github_api = match &config.github_api {
            Some(github_api) => Some(github_api.lock().await),
            None => None,
        };
        let message = html_command(
            &config.room_name,
            &push_event.get_message(github_api.as_deref_mut()).await,
        );
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
    async fn get_message(&self, mut github_api: Option<&mut GitHubApi>) -> String {
        let pushed = if self.created {
            "pushed <font color='red'>in new branch</font>"
        } else if self.forced {
            "<font color='red'>force-pushed</font>"
        } else {
            "pushed"
        };
        let mut output = format!(
            concat!(
                "addhtmlbox {repo} <a href='https://github.com/{pusher}'>",
                "<font color='909090'>{pusher}</font></a> ",
                "{pushed} <a href='{compare}'><b>{commits}</b> new ",
                "commit{s}</a> to <font color='800080'>{branch}</font>",
            ),
            repo = self.repository.to_view(),
            pusher = h(&self.pusher.name),
            pushed = pushed,
            compare = h(&self.compare),
            commits = self.commits.len(),
            s = if self.commits.len() == 1 { "" } else { "s" },
            branch = h(self.get_branch()),
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
    async fn to_view<'a>(
        &'a self,
        url: &str,
        github_api: Option<&'a mut GitHubApi>,
    ) -> ViewCommit<'a> {
        let message = match self.message.find('\n') {
            Some(index) => &self.message[..index],
            None => &self.message[..],
        };
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

#[derive(Debug, Deserialize)]
struct Repository {
    name: String,
    html_url: String,
}

impl Repository {
    fn to_view(&self) -> ViewRepository<'_> {
        let name = match self.name.as_str() {
            "pokemon-showdown" => "server",
            "pokemon-showdown-client" => "client",
            name => name,
        };
        ViewRepository {
            name,
            html_url: &self.html_url,
        }
    }
}

#[derive(Template)]
#[template(path = "repository.html")]
struct ViewRepository<'a> {
    name: &'a str,
    html_url: &'a str,
}

fn handle_pull_request(
    config: &Config,
    skip_pull_requests: &Arc<DashSet<u32>>,
    sender: &UnboundedSender,
    pull_request: PullRequestEvent,
) -> impl Future<Output = Result<&'static str, Rejection>> {
    let number = pull_request.pull_request.number;
    if skip_pull_requests.insert(number) {
        let message = html_command(
            &config.room_name,
            &format!("addhtmlbox {}", pull_request.to_view()),
        );
        let skip_pull_requests = skip_pull_requests.clone();
        let sender = sender.clone();
        async move {
            tokio::spawn(async move {
                time::delay_for(Duration::from_secs(10 * 60)).await;
                skip_pull_requests.remove(&number);
            });
            sender
                .send(message)
                .await
                .map_err(|_| warp::reject::custom(ChannelError))?;
            Ok("")
        }
        .left_future()
    } else {
        async { Ok("") }.right_future()
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
    fn to_view(&self) -> ViewPullRequestEvent<'_> {
        ViewPullRequestEvent {
            action: match self.action.as_str() {
                "synchronize" => "updated",
                action => action,
            },
            pull_request: &self.pull_request,
            repository: self.repository.to_view(),
            sender: &self.sender,
        }
    }
}

#[derive(Template)]
#[template(path = "pull_request_event.html")]
struct ViewPullRequestEvent<'a> {
    action: &'a str,
    pull_request: &'a PullRequest,
    repository: ViewRepository<'a>,
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
                "<font color=909090 title='Konrad Borowski'>xfix</font>: ",
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
                html_url: "http://example.com/".into(),
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
