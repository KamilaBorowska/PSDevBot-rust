use crate::config::UsernameAliases;
use crate::github_api::{GitHubApi, User};
use askama::Template;
use htmlescape::encode_minimal as h;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct InitialPayload {
    pub repository: InitialRepository,
}

#[derive(Deserialize)]
pub struct InitialRepository {
    pub full_name: String,
}

#[derive(Debug, Deserialize)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    forced: bool,
    commits: Vec<Commit>,
    compare: String,
    pusher: Pusher,
    pub repository: Repository,
}

pub struct PushEventContext<'a> {
    pub github_api: Option<&'a mut GitHubApi>,
    pub username_aliases: &'a UsernameAliases,
}

impl PushEvent {
    pub async fn get_message(&self, mut ctx: PushEventContext<'_>) -> String {
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
            let commit_view = commit.to_view(&self.repository.html_url, &mut ctx).await;
            output += &format!("<br>{}", commit_view);
        }
        output
    }

    pub fn branch(&self) -> &str {
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
    async fn to_view<'a>(&'a self, url: &str, ctx: &'a mut PushEventContext<'_>) -> ViewCommit<'a> {
        let message = self.message.split('\n').next().unwrap();
        ViewCommit {
            id: &self.id[..6],
            message,
            full_message: &self.message,
            formatted_message: format_title(message, url),
            author: self.author.to_view(ctx).await,
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
    async fn to_view<'a>(&'a self, ctx: &'a mut PushEventContext<'_>) -> ViewAuthor<'a> {
        let username = if let Some(username) = &self.username {
            let github_metadata = if let Some(github_api) = &mut ctx.github_api {
                github_api.fetch_user(username).await
            } else {
                None
            };
            Some(Username {
                username: ctx.username_aliases.get(username),
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
pub struct Repository {
    name: String,
    html_url: String,
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestEvent {
    pub action: String,
    pub pull_request: PullRequest,
    pub repository: Repository,
    sender: Sender,
}

impl PullRequestEvent {
    pub fn to_view<'a>(
        &'a self,
        username_aliases: &'a UsernameAliases,
    ) -> ViewPullRequestEvent<'a> {
        ViewPullRequestEvent {
            action: match self.action.as_str() {
                "synchronize" => "updated",
                "review_requested" => "requested a review for",
                action => action,
            },
            pull_request: &self.pull_request,
            repository: &self.repository,
            sender: self.sender.to_view(username_aliases),
        }
    }
}

#[derive(Template)]
#[template(path = "pull_request_event.html")]
pub struct ViewPullRequestEvent<'a> {
    action: &'a str,
    pull_request: &'a PullRequest,
    repository: &'a Repository,
    sender: ViewSender<'a>,
}

#[derive(Debug, Deserialize, Template)]
#[template(path = "pull_request.html")]
pub struct PullRequest {
    pub number: u32,
    html_url: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct Sender {
    login: String,
}

impl Sender {
    fn to_view<'a>(&'a self, username_aliases: &'a UsernameAliases) -> ViewSender<'a> {
        ViewSender {
            login: &self.login,
            renamed_login: username_aliases.get(&self.login),
        }
    }
}

struct ViewSender<'a> {
    login: &'a str,
    renamed_login: &'a str,
}

#[cfg(test)]
mod test {
    use super::{
        Author, Commit, PullRequest, PullRequestEvent, PushEventContext, Repository, Sender,
    };
    use crate::config::UsernameAliases;

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
            commit
                .to_view(
                    "shouldn't be used",
                    &mut PushEventContext {
                        github_api: None,
                        username_aliases: &UsernameAliases::default(),
                    }
                )
                .await
                .to_string(),
            concat!(
                "<a href='http:&#x2f;&#x2f;example.com'>",
                "<font color=606060><kbd>0da259</kbd></font></a>\n",
                r#"<span title="Konrad Borowski"><font color=909090>xfix</font></span>: "#,
                "<span title='Hello, world!'>Hello, world!</span>",
            ),
        );
    }

    fn sample_pull_request() -> PullRequestEvent {
        PullRequestEvent {
            action: "created".into(),
            pull_request: PullRequest {
                number: 1,
                html_url: "http://example.com/pr/1".into(),
                title: "Hello, world".into(),
            },
            repository: Repository {
                name: "ExampleCom".into(),
                html_url: "http://example.com/".into(),
                default_branch: "master".into(),
            },
            sender: Sender { login: "Me".into() },
        }
    }

    #[test]
    fn test_pull_request() {
        assert_eq!(
            sample_pull_request()
                .to_view(&UsernameAliases::default())
                .to_string(),
            concat!(
                "[<a href='http:&#x2f;&#x2f;example.com&#x2f;'><font color=FF00FF>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Me</font></a> created pull request ",
                "<a href='http:&#x2f;&#x2f;example.com&#x2f;pr&#x2f;1'>#1</a>: Hello, world",
            ),
        );
    }

    #[test]
    fn test_pull_request_with_an_alias() {
        let mut aliases = UsernameAliases::default();
        aliases.insert("mE".into(), "Not me".into());
        assert_eq!(
            sample_pull_request().to_view(&aliases).to_string(),
            concat!(
                "[<a href='http:&#x2f;&#x2f;example.com&#x2f;'><font color=FF00FF>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Not me</font></a> created pull request ",
                "<a href='http:&#x2f;&#x2f;example.com&#x2f;pr&#x2f;1'>#1</a>: Hello, world",
            ),
        );
    }
}
