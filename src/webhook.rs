use crate::unbounded::UnboundedSender;
use futures::sync::oneshot;
use htmlescape::encode_minimal as h;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde_derive::Deserialize;
use showdown::RoomId;
use warp::{self, path, Filter};
use warp_github_webhook::webhook;

pub fn start_server(secret: String, sender: &UnboundedSender, port: u16) -> oneshot::Sender<()> {
    let push_sender = sender.clone();
    let pull_request_sender = sender.clone();
    let route = path!("github" / "callback").and(
        webhook(warp_github_webhook::Kind::PUSH, secret.clone())
            .and_then(move |push_event| handle_push_event(&push_sender, push_event))
            .or(
                webhook(warp_github_webhook::Kind::PULL_REQUEST, secret).and_then(
                    move |pull_request| handle_pull_request(&pull_request_sender, pull_request),
                ),
            ),
    );
    let (tx, rx) = oneshot::channel();
    tokio::spawn(
        warp::serve(route)
            .bind_with_graceful_shutdown(([0, 0, 0, 0], port), rx)
            .1,
    );
    tx
}

fn handle_push_event(
    sender: &UnboundedSender,
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
                "/addhtmlbox {repo} <font color='909090'>{pusher}</font> {pushed} ",
                r#"<a href="{compare}"><b>{commits}</b> new commit{s}</a> "#,
                "to <font color='800080'>{branch}</font>",
            ),
            repo = self.repository.format(),
            pusher = h(&self.pusher.name),
            pushed = pushed,
            compare = self.compare,
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
        format!(
            concat!(
                "<br /><a href=\"{url}\"><font color='606060'>{id}</font></a> ",
                "<font color='909090'>{author}</font>: {message}{more}",
            ),
            url = h(&self.url),
            id = &self.id[0..6],
            author = match &self.author.username {
                Some(username) => format!(
                    r#"<a href="https://github.com/{username}">{name}</a>"#,
                    username = h(username),
                    name = h(&self.author.name)
                ),
                None => h(&self.author.name),
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
        format!("[<font color='FF00FF'>{}</font>]", h(repo))
    }
}

fn handle_pull_request(
    sender: &UnboundedSender,
    pull_request: PullRequestEvent,
) -> Result<&'static str, warp::Rejection> {
    sender
        .send_chat_message(RoomId("botdevelopment"), &pull_request.get_message())
        .map_err(warp::reject::custom)?;
    Ok("")
}

#[derive(Debug, Deserialize)]
struct PullRequestEvent {
    action: String,
    pull_request: PullRequest,
    repository: Repository,
    sender: Sender,
}

impl PullRequestEvent {
    fn get_message(&self) -> String {
        format!(
            concat!(
                "/addhtmlbox {repo} <font color='909090'>{author}</font> ",
                "{action} pull request ",
                "<a href=\"{url}\">#{number}</a>: {title}",
            ),
            repo = self.repository.format(),
            author = h(&self.sender.login),
            action = h(&self.action),
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
