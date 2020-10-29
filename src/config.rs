use showdown::url::Url;
use std::env;
use std::error::Error;

pub struct Config {
    pub server: Url,
    pub user: String,
    pub password: String,
    pub secret: String,
    pub port: u16,
    pub sentry_dsn: String,
    pub room_name: String,
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
        let sentry_dsn = env::var("SENTRY_DSN").unwrap_or_default();
        let room_name = env::var("PSDEVBOT_ROOM")?;
        Ok(Self {
            server,
            user,
            password,
            secret,
            port,
            sentry_dsn,
            room_name,
        })
    }
}
