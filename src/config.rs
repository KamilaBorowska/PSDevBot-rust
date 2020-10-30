use crate::github_api::GitHubApi;
use showdown::futures::lock::Mutex;
use showdown::url::Url;
use std::env;
use std::error::Error;

pub struct Config {
    pub server: Url,
    pub user: String,
    pub password: String,
    pub secret: String,
    pub port: u16,
    pub room_name: String,
    pub github_api: Option<Mutex<GitHubApi>>,
}

impl Config {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let server = Url::parse(&env::var("PSDEVBOT_SERVER")?)?;
        let user = env::var("PSDEVBOT_USER")?;
        let password = env::var("PSDEVBOT_PASSWORD")?;
        let secret = env::var("PSDEVBOT_SECRET")?;
        let port = match env::var("PSDEVBOT_PORT") {
            Ok(port) => port.parse()?,
            Err(_) => 3030,
        };
        let room_name = env::var("PSDEVBOT_ROOM")?;
        let github_api = env::var("PSDEVBOT_GITHUB_API_USER").ok().and_then(|user| {
            let password = env::var("PSDEVBOT_GITHUB_API_PASSWORD").ok()?;
            Some(Mutex::new(GitHubApi::new(user, password)))
        });
        Ok(Self {
            server,
            user,
            password,
            secret,
            port,
            room_name,
            github_api,
        })
    }
}
