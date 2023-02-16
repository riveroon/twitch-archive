use async_std::sync::Mutex;
use futures::Future;
use serde::de::DeserializeOwned;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use surf::http::mime;

use crate::prelude::*;

const AUTH_API: &str = "https://id.twitch.tv/oauth2/token";

#[derive(PartialEq, Eq, Clone, Debug)]
struct Inner {
    auth: Box<str>,
    client_id: Box<str>,
    expires: Instant,
}

impl Inner {
    async fn _get(client_id: &str, secret: &str) -> Result<(Box<str>, Instant)> {
        #[derive(Deserialize)]
        struct AuthRes {
            access_token: String,
            expires_in: u64,
        }

        let res: AuthRes = {
            let mut res = surf::post(AUTH_API)
                .body_string(format!(
                    "client_id={}\
                &client_secret={}\
                &grant_type={}",
                    client_id, secret, "client_credentials"
                ))
                .content_type(mime::FORM)
                .send()
                .await
                .map_err(|e| e.into_inner())?;

            if !res.status().is_success() {
                return Err(anyhow!("authorization returned status {}", res.status()));
            }

            res.body_json().await.map_err(|e| e.into_inner())?
        };

        log::debug!("retrieved auth: expires in {}", res.expires_in);

        Ok((
            format!("Bearer {}", &res.access_token).into_boxed_str(),
            Instant::now() + Duration::from_secs(res.expires_in),
        ))
    }

    async fn get(client_id: String, secret: &str) -> Result<Self> {
        let (auth, expires) = Self::_get(&client_id, secret).await?;

        Ok(Self {
            auth,
            client_id: client_id.into_boxed_str(),
            expires,
        })
    }

    fn has_expired(&self) -> bool {
        Instant::now()
            .saturating_duration_since(self.expires)
            .as_secs()
            > 0
    }

    async fn refresh(&mut self, secret: &str) -> Result<()> {
        (self.auth, self.expires) = Self::_get(&self.client_id, secret).await?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct HelixAuth {
    inner: Arc<Mutex<(Inner, Box<str>)>>,
}

impl HelixAuth {
    pub async fn new(client_id: String, secret: String) -> Result<Self> {
        Inner::get(client_id, &secret).await.map(|x| Self {
            inner: Arc::new(Mutex::new((x, secret.into_boxed_str()))),
        })
    }

    async fn has_expired(&self) -> bool {
        (*self.inner.lock().await).0.has_expired()
    }

    pub async fn refresh(&mut self) -> Result<()> {
        let (inner, secret) = &mut *self.inner.lock().await;
        inner.refresh(secret).await?;
        Ok(())
    }

    pub async fn auth(&self) -> String {
        (*self.inner.lock().await).0.auth.clone().into()
    }
    pub async fn with_auth<F, T, Fut>(&self, mut f: F) -> T
    where
        F: FnMut(&str) -> Fut,
        Fut: Future<Output = T>,
    {
        let auth = &*(*self.inner.lock().await).0.auth;
        f(auth).await
    }

    pub async fn client_id(&self) -> String {
        (*self.inner.lock().await).0.client_id.clone().into()
    }
    pub async fn with_client_id<F, T, Fut>(&self, mut f: F) -> T
    where
        F: FnMut(&str) -> Fut,
        Fut: Future<Output = T>,
    {
        let client_id = &*(*self.inner.lock().await).0.client_id;
        f(client_id).await
    }

    pub async fn send_req(&self, req: surf::Request) -> Result<surf::Response> {
        async fn _send(
            auth: &HelixAuth,
            mut req: surf::Request,
            refresh: bool,
        ) -> Result<surf::Response> {
            let mut lock = auth.inner.lock().await;

            let (inner, secret) = &mut *lock;
            if refresh {
                inner.refresh(secret).await?
            }
            req.insert_header("Authorization", &*lock.0.auth);
            req.insert_header("Client-Id", &*lock.0.client_id);

            drop(lock);

            log::trace!("sending request: {:?}", req);
            surf::client().send(req).await.map_err(|e| e.into_inner())
        }

        use surf::StatusCode;
        let b = req.clone();
        let res = _send(self, req, false).await?;

        match res.status() {
            StatusCode::Unauthorized => (),
            x if x.is_success() => return Ok(res),
            x => return Err(anyhow!("request returned status {}", x)),
        }

        log::info!("received status code 401; refreshing auth");
        _send(self, b, true).await
    }

    pub async fn send_req_json<T: DeserializeOwned>(&self, req: surf::Request) -> Result<T> {
        self.send_req(req)
            .await?
            .body_json()
            .await
            .map_err(|e| e.into_inner())
    }
}
