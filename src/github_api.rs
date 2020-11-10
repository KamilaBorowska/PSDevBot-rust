use log::info;
use lru::LruCache;
use reqwest::{header, Client};
use serde::Deserialize;
use std::time::Duration;

pub struct GitHubApi {
    user: String,
    password: String,
    cache: LruCache<String, User>,
    client: Client,
}

impl GitHubApi {
    pub fn new(user: String, password: String) -> Self {
        Self {
            user,
            password,
            cache: LruCache::new(100),
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .user_agent("psdevbot-rust")
                .build()
                .unwrap(),
        }
    }

    pub async fn fetch_user(
        &mut self,
        #[allow(clippy::ptr_arg)] // due to LruCache limitations accepting &String is necessary.
        user_name: &String,
    ) -> Option<&User> {
        if !self.cache.contains(user_name) {
            info!("Fetching user `{}` from GitHub", user_name);
            let user = self
                .client
                .get(&format!("https://api.github.com/users/{}", user_name))
                .header(header::ACCEPT, "application/vnd.github.v3+json")
                .basic_auth(&self.user, Some(&self.password))
                .send()
                .await
                .ok()?
                .json()
                .await
                .ok()?;
            self.cache.put(user_name.clone(), user);
        }
        self.cache.get(user_name)
    }
}

#[derive(Deserialize)]
pub struct User {
    pub html_url: String,
}
