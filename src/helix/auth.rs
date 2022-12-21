use std::{sync::Arc, time::{Instant, Duration}};

use async_std::sync::Mutex;
use serde::Deserialize;
use surf::{http::mime};

const AUTH_API: &'static str = "https://id.twitch.tv/oauth2/token";

#[derive(PartialEq, Eq, Clone, Debug)]
struct Inner {
    auth: Box<str>,
    client_id: Box<str>,
    expires: Instant,
}

impl Inner {
    async fn _get(client_id: &str, secret: &str) -> surf::Result<(Box<str>, Instant)> {
        #[derive(Deserialize)]
        struct AuthRes {
            access_token: String,
            expires_in: u64,
        }

        let res: AuthRes = surf::post(AUTH_API)
            .body_string(format!(
                "client_id={}\
                &client_secret={}\
                &grant_type={}",
                client_id, secret, "client_credentials"
            )).content_type(mime::FORM)
            .recv_json().await?;
        
        log::debug!("retrieved auth: expires in {}", res.expires_in);

        return Ok((
            format!("Bearer {}", &res.access_token).into_boxed_str(),
            Instant::now() + Duration::from_secs(res.expires_in)))
    }

    async fn get(client_id: String, secret: &str) -> surf::Result<Self> {
        let (auth, expires) = Self::_get(&client_id, &secret).await?;

        Ok( Self { auth, client_id: client_id.into_boxed_str(), expires } )
    }

    fn has_expired(&self) -> bool {
        return Instant::now().saturating_duration_since(self.expires).as_secs() > 0;
    }

    async fn refresh(&mut self, secret: &str) -> surf::Result<()> {
        (self.auth, self.expires) = Self::_get(&self.client_id, secret).await?;
        return Ok(());
    }
}

#[derive(Clone, Debug)]
pub struct HelixAuth {
    inner: Arc<Mutex<(Inner, Box<str>)>>,
    cache: Inner,
}

impl HelixAuth {
    pub async fn new(client_id: String, secret: String) -> surf::Result<Self> {
        return Inner::get(client_id, &secret).await.map(|x| 
            Self{ 
                inner: Arc::new(Mutex::new(
                    (x.clone(), secret.into_boxed_str())
                )),
                cache: x
            }
        );
    }

    fn has_expired(&self) -> bool { self.cache.has_expired() }

    pub async fn refresh(&mut self) -> surf::Result<()> {
        let (inner, secret) = &mut *self.inner.lock().await;
        if *inner == self.cache { inner.refresh(secret).await? };
        self.cache = inner.clone();
        return Ok(());
    }

    pub fn auth(&self) -> &str { &self.cache.auth }
    pub fn client_id(&self) -> &str { &self.cache.client_id }
}