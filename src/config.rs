use crate::github_api::GitHubApi;
use futures::lock::Mutex;
use serde::Deserialize;
use showdown::url::Url;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::slice;

pub struct Config {
    pub server: Url,
    pub user: String,
    pub password: String,
    pub secret: String,
    pub port: u16,
    default_room_name: Option<String>,
    room_configuration: HashMap<String, RoomConfiguration>,
    pub github_api: Option<Mutex<GitHubApi>>,
}

#[derive(Deserialize)]
pub struct RoomConfiguration {
    pub rooms: Vec<String>,
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
        let default_room_name = env::var("PSDEVBOT_ROOM").ok();
        let room_configuration = env::var("PSDEVBOT_PROJECT_CONFIGURATION")
            .map(|json| {
                serde_json::from_str(&json)
                    .expect("PSDEVBOT_PROJECT_CONFIGURATION should be valid JSON")
            })
            .ok();
        if default_room_name.is_none() && room_configuration.is_none() {
            panic!("At least one of PSDEVBOT_ROOM or PSDEVBOT_PROJECT_CONFIGURATION needs to be provided");
        }
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
            default_room_name,
            room_configuration: room_configuration.unwrap_or_default(),
            github_api,
        })
    }

    pub fn all_rooms(&self) -> HashSet<&str> {
        self.room_configuration
            .values()
            .flat_map(|r| &r.rooms)
            .chain(&self.default_room_name)
            .map(String::as_str)
            .collect()
    }

    pub fn rooms_for(&self, name: &str) -> &[String] {
        if let Some(configuration) = self.room_configuration.get(name) {
            &configuration.rooms
        } else {
            self.default_room_name
                .as_ref()
                .map(slice::from_ref)
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Config, RoomConfiguration};
    use std::collections::HashMap;

    fn base_config() -> Config {
        Config {
            server: "wss://localhost/showdown/websocket".parse().unwrap(),
            user: "".into(),
            password: "".into(),
            secret: "".into(),
            port: 3030,
            default_room_name: None,
            room_configuration: HashMap::new(),
            github_api: None,
        }
    }

    #[test]
    fn test_all_rooms_default_room() {
        let mut config = base_config();
        config.default_room_name = Some("room".into());
        let mut rooms: Vec<_> = config.all_rooms().into_iter().collect();
        rooms.sort();
        assert_eq!(rooms, ["room"]);
    }

    #[test]
    fn test_all_rooms_room_configuration() {
        let mut config = base_config();
        config.room_configuration.insert(
            "Project".into(),
            RoomConfiguration {
                rooms: vec!["a".into(), "b".into()],
            },
        );
        config.room_configuration.insert(
            "AnotherProject".into(),
            RoomConfiguration {
                rooms: vec!["b".into(), "c".into()],
            },
        );
        config
            .room_configuration
            .insert("StupidProject".into(), RoomConfiguration { rooms: vec![] });
        let mut rooms: Vec<_> = config.all_rooms().into_iter().collect();
        rooms.sort();
        assert_eq!(rooms, ["a", "b", "c"]);
    }
}
