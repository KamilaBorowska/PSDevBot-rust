use crate::config::UsernameAliases;
use crate::github_api::{GitHubApi, User};
use askama::Template;
use htmlescape::encode_minimal as h;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::Deserialize;
use std::borrow::Cow;

#[derive(Deserialize)]
pub struct InitialPayload<'a> {
    #[serde(borrow)]
    pub repository: InitialRepository<'a>,
}

#[derive(Deserialize)]
pub struct InitialRepository<'a> {
    #[serde(borrow)]
    pub full_name: Cow<'a, str>,
}

#[derive(Debug, Deserialize)]
pub struct PushEvent<'a> {
    #[serde(borrow, rename = "ref")]
    git_ref: Cow<'a, str>,
    #[serde(borrow)]
    commits: Vec<Commit<'a>>,
    #[serde(borrow)]
    pub repository: Repository<'a>,
}

pub struct PushEventContext<'a> {
    pub github_api: Option<&'a mut GitHubApi>,
    pub username_aliases: &'a UsernameAliases,
}

macro_rules! view_method {
    ($name:ident($s:ident, $($ex:tt)*)) => {
        pub async fn $name<'a>(&'a $s, mut ctx: PushEventContext<'a>) -> ViewPushEvent<'a> {
            let mut commits_view = Vec::new();
            for commit in &$s.commits {
                commits_view.push(
                    commit
                        .$name($($ex)* &mut ctx)
                        .await
                        .to_string(),
                );
            }
            ViewPushEvent {
                commits: commits_view,
                repository: $s.repository.to_view(),
            }
        }
    };
}

impl PushEvent<'_> {
    view_method!(to_view(self, &self.repository.html_url,));
    view_method!(to_simple_view(self,));

    pub fn branch(&self) -> &str {
        self.git_ref.rsplit('/').next().unwrap()
    }
}

#[derive(Template)]
#[template(path = "push_event.html")]
pub struct ViewPushEvent<'a> {
    commits: Vec<String>,
    repository: ViewRepository<'a>,
}

#[derive(Debug, Deserialize)]
struct Commit<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(borrow)]
    message: Cow<'a, str>,
    #[serde(borrow)]
    author: Author<'a>,
    #[serde(borrow)]
    url: Cow<'a, str>,
}

impl Commit<'_> {
    async fn to_view<'a>(&'a self, url: &str, ctx: &'a mut PushEventContext<'_>) -> ViewCommit<'a> {
        let message = self.short_message();
        ViewCommit {
            id: &self.id[..6],
            message,
            full_message: &self.message,
            formatted_message: format_title(message, url),
            author: self.author.to_view(ctx).await,
            url: &self.url,
        }
    }

    async fn to_simple_view<'a>(
        &'a self,
        ctx: &'a mut PushEventContext<'_>,
    ) -> ViewSimpleCommit<'a> {
        ViewSimpleCommit {
            message: self.short_message(),
            full_message: &self.message,
            author: self.author.to_view(ctx).await,
            url: &self.url,
        }
    }

    fn short_message(&self) -> &str {
        self.message.split('\n').next().unwrap()
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

#[derive(Template)]
#[template(path = "simple_commit.html")]
struct ViewSimpleCommit<'a> {
    message: &'a str,
    full_message: &'a str,
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
struct Author<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    username: Option<String>,
}

impl Author<'_> {
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

#[derive(Debug, Deserialize)]
pub struct Repository<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    #[serde(borrow)]
    html_url: Cow<'a, str>,
    #[serde(borrow)]
    pub default_branch: Cow<'a, str>,
}

impl Repository<'_> {
    fn to_view(&self) -> ViewRepository<'_> {
        let name = match &*self.name {
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
pub struct ViewRepository<'a> {
    name: &'a str,
    html_url: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestEvent<'a> {
    #[serde(borrow)]
    pub action: Cow<'a, str>,
    #[serde(borrow)]
    pub pull_request: PullRequest<'a>,
    #[serde(borrow)]
    pub repository: Repository<'a>,
    #[serde(borrow)]
    sender: Sender<'a>,
}

impl PullRequestEvent<'_> {
    pub fn to_view<'a>(
        &'a self,
        username_aliases: &'a UsernameAliases,
    ) -> ViewPullRequestEvent<'a> {
        ViewPullRequestEvent {
            action: match &*self.action {
                "synchronize" => "updated",
                "review_requested" => "requested a review for",
                action => action,
            },
            pull_request: &self.pull_request,
            repository: self.repository.to_view(),
            sender: self.sender.to_view(username_aliases),
        }
    }
}

#[derive(Template)]
#[template(path = "pull_request_event.html")]
pub struct ViewPullRequestEvent<'a> {
    action: &'a str,
    pull_request: &'a PullRequest<'a>,
    repository: ViewRepository<'a>,
    sender: ViewSender<'a>,
}

#[derive(Debug, Deserialize, Template)]
#[template(path = "pull_request.html")]
pub struct PullRequest<'a> {
    pub number: u32,
    #[serde(borrow)]
    html_url: Cow<'a, str>,
    #[serde(borrow)]
    title: Cow<'a, str>,
}

#[derive(Debug, Deserialize)]
struct Sender<'a> {
    #[serde(borrow)]
    login: Cow<'a, str>,
}

impl Sender<'_> {
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
        Author, Commit, PullRequest, PullRequestEvent, PushEvent, PushEventContext, Repository,
        Sender,
    };
    use crate::config::UsernameAliases;

    fn sample_commit() -> Commit<'static> {
        Commit {
            id: "0da2590a700d054fc2ce39ddc9c95f360329d9be".into(),
            message: "Hello, world!".into(),
            author: Author {
                name: "Konrad Borowski".into(),
                username: Some("xfix".into()),
            },
            url: "http://example.com".into(),
        }
    }

    #[tokio::test]
    async fn test_push_event() {
        let commit = concat!(
            "[<a href='https://github.com/smogon/pokemon-showdown'>",
            "<font color=FF00FF>server</font></a>] ",
            "<a href='http://example.com'><font color=606060><kbd>0da259</kbd></font></a>\n",
            "<span title='Hello, world!'>Hello, world!</span> ",
            r#"<font color=909090 title="Konrad Borowski">(xfix)</font>"#,
        );
        assert_eq!(
            PushEvent {
                git_ref: "refs/head/master".into(),
                commits: vec![sample_commit(), sample_commit()],
                repository: Repository {
                    name: "pokemon-showdown".into(),
                    html_url: "https://github.com/smogon/pokemon-showdown".into(),
                    default_branch: "master".into(),
                }
            }
            .to_view(PushEventContext {
                github_api: None,
                username_aliases: &UsernameAliases::default(),
            })
            .await
            .to_string(),
            format!("{0}<br>{0}", commit)
        );
    }

    #[tokio::test]
    async fn test_commit() {
        assert_eq!(
            sample_commit()
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
                "<a href='http://example.com'>",
                "<font color=606060><kbd>0da259</kbd></font></a>\n",
                "<span title='Hello, world!'>Hello, world!</span> ",
                r#"<font color=909090 title="Konrad Borowski">(xfix)</font>"#,
            ),
        );
    }

    fn sample_pull_request() -> PullRequestEvent<'static> {
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
                "[<a href='http://example.com/'><font color=FF00FF>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Me</font></a> created ",
                "<a href='http://example.com/pr/1'>PR#1</a>: Hello, world",
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
                "[<a href='http://example.com/'><font color=FF00FF>",
                "ExampleCom</font></a>] <a href='https://github.com/Me'><font ",
                "color='909090'>Not me</font></a> created ",
                "<a href='http://example.com/pr/1'>PR#1</a>: Hello, world",
            ),
        );
    }
}
