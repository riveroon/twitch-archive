use anyhow::Context;
use async_std::{channel::Sender, sync::Arc};
use atomic::{Atomic, Ordering};
use dashmap::DashMap;
use serde_json::value::RawValue;
use tide::{Request, Response};

use super::HelixAuth;
use crate::{prelude::*, rand, eventsub::event::Version};

use event::SubscriptionType;
pub use subscription::*;

pub mod event;
mod subscription;

const EVENTSUB_API: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";

#[allow(unused)]
const MSG_ID: &str = "Twitch-Eventsub-Message-Id";
#[allow(unused)]
const MSG_RETRY: &str = "Twitch-Eventsub-Message-Retry";
const MSG_TYPE: &str = "Twitch-Eventsub-Message-Type";
#[allow(unused)]
const MSG_SIG: &str = "Twitch-Eventsub-Message-Signature";
#[allow(unused)]
const MSG_TIME: &str = "Twitch-Eventsub-Message-Timestamp";
#[allow(unused)]
const SUB_TYPE: &str = "Twitch-Eventsub-Subscription-Type";
#[allow(unused)]
const SUB_VER: &str = "Twitch-Eventsub-Subscription-Version";

const MSG_NOTIFICATION: &str = "notification";
const MSG_VERIFICATION: &str = "webhook_callback_verification";
const MSG_REVOCATION: &str = "revocation";

type Secret = Box<str>;
type State = Arc<DashMap<SubUnique, (Arc<Atomic<SubStatus>>, Secret, Sender<Box<RawValue>>)>>;

async fn callback(mut req: Request<State>) -> tide::Result {
    fn err_state(state: SubStatus) -> tide::Result {
        #[derive(Serialize)]
        struct ErrMsg {
            status: &'static str,
            desc: &'static str,
            state: SubStatus,
        }

        Ok(Response::builder(409)
            .body(tide::Body::from_json(&ErrMsg {
                status: "409 Conflict",
                desc: "The subscription exists, but is not in a respondable state for the request.",
                state,
            })?)
            .build())
    }

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    fn verify_msg(secret: &str, req: &Request<State>, body: &[u8]) -> bool {
        let (Some(v1), Some(v2), Some(sig)) = (
            req.header(MSG_ID),
            req.header(MSG_TIME),
            req.header(MSG_SIG)
        ) else {
            log::warn!("required header for hmac verification was empty");
            return false;
        };

        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        else {
            log::warn!("unexpected error: could not initialize hmac!");
            return false;
        };

        mac.update(v1.as_str().as_bytes());
        mac.update(v2.as_str().as_bytes());
        mac.update(body);

        if sig.as_str().len() < 7 {
            return false;
        };
        let sig = &sig.as_str()[7..];

        if sig.len() % 2 != 0 {
            return false;
        };
        let Ok(sig): Result<Vec<u8>, _> = (0..sig.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&sig[i..i + 2], 16))
            .collect() else { return false };

        mac.verify_slice(&sig).is_ok()
    }

    log::trace!("recieved webhook request: {:?}", req);
    let body = req.body_bytes().await?;
    let Some(msg_type) = req.header(MSG_TYPE) else {
        log::warn!("received webhook request missing message type!");
        return Ok(Response::builder(400).build())
    };

    match msg_type.as_str() {
        MSG_NOTIFICATION => {
            #[derive(Deserialize)]
            struct RawEvent {
                subscription: SubUnique,
                event: Box<RawValue>,
            }

            let msg: RawEvent = serde_json::from_slice(&body)?;

            let e = req.state().get(&msg.subscription);
            let Some((status, secret, tx)) = e.as_deref() else {
                log::warn!("subscription #{} not found", msg.subscription.id());
                return Ok(Response::builder(404).build());
            };

            if !verify_msg(secret, &req, &body) {
                log::warn!("verification failed!");
                return Ok(Response::builder(401).build());
            }

            let s = status.load(Ordering::Relaxed);
            if s != SubStatus::Enabled {
                log::warn!("subscription #{} is not enabled: {s:?}", msg.subscription.id());
                return err_state(s);
            }

            match tx.send(msg.event).await {
                Ok(_) => Ok(Response::builder(200).build()),
                Err(_) => {
                    req.state().remove(&msg.subscription);
                    Ok(Response::builder(410).build())
                }
            }
        }
        MSG_VERIFICATION => {
            #[derive(Deserialize)]
            struct ChallengeReq {
                subscription: SubUnique,
                challenge: String,
            }

            let challenge: ChallengeReq = serde_json::from_slice(&body)?;

            let e = req.state().get(&challenge.subscription);
            let Some((status, secret, _)) = e.as_deref() else {
                log::warn!("subscription #{} not found", challenge.subscription.id());
                return Ok(Response::builder(404).build());
            };

            if !verify_msg(secret, &req, &body) {
                log::warn!("verification failed!");
                return Ok(Response::builder(401).build());
            }

            match status.compare_exchange(
                SubStatus::VerificationPending,
                SubStatus::Enabled,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => Ok(Response::builder(200).body(challenge.challenge).build()),
                Err(state) => {
                    log::warn!("verification request on subscription of state {state:?}");
                    err_state(state)
                }
            }
        }
        MSG_REVOCATION => {
            #[derive(Deserialize)]
            struct RevokeReq {
                subscription: SubUnqStatus,
            }

            #[derive(Deserialize)]
            struct SubUnqStatus {
                #[serde(flatten)]
                unique: SubUnique,
                status: SubStatus,
            }

            let rev: RevokeReq = serde_json::from_slice(&body)?;

            let Some((_, (status, secret, _))) = (*req.state()).remove(&rev.subscription.unique) else {
                log::warn!("subscription #{} not found", rev.subscription.unique.id());
                return Ok(Response::builder(404).build());
            };

            if !verify_msg(&secret, &req, &body) {
                log::warn!("verification failed!");
                return Ok(Response::builder(401).build());
            }

            status.swap(rev.subscription.status, Ordering::Relaxed);
            Ok(Response::builder(200).build())
        }
        unknown => {
            log::warn!("received webhook request has unknown message type: {unknown}");
            Ok(Response::builder(400).build())
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Transport<'a> {
    Webhook {
        callback: &'a str,
    },
    Websocket {
        session_id: &'a str,
        connected_at: &'a str,
    },
}

pub struct EventSub {
    map: State,
    auth: HelixAuth,
    v_addr: url::Url,
}

impl EventSub {
    pub fn new(addr: std::net::SocketAddr, v_addr: &url::Url, auth: HelixAuth) -> Self {
        let state = Arc::new(DashMap::new());
        let mut serve = tide::with_state(Arc::clone(&state));
        serve.at("/callback").post(callback);

        async_std::task::Builder::new()
            .name("callback".to_owned())
            .spawn(async move { serve.listen(addr).await.unwrap() })
            .expect("cannot spawn future");
        log::info!("started server at {addr:?}");

        Self {
            map: state,
            auth,
            v_addr: v_addr.join("callback").unwrap(),
        }
    }

    pub fn transport(&self) -> Transport {
        Transport::Webhook {
            callback: self.v_addr.as_str(),
        }
    }

    pub async fn subscribe<T: SubscriptionType>(
        &self,
        cond: impl Into<T::Cond>,
    ) -> Result<Subscription<T>> {
        #[derive(Debug, Serialize)]
        struct CreateSub<'a, T> {
            #[serde(rename = "type")]
            name: &'static str,
            version: Version,
            condition: T,
            transport: TransportWithSecret<'a>,
        }

        #[derive(Debug, Serialize)]
        struct TransportWithSecret<'a> {
            #[serde(flatten)]
            transport: Transport<'a>,
            secret: &'a str,
        }

        #[derive(Deserialize)]
        struct CreateSubRes {
            data: [SubDes; 1],
        }

        #[derive(Deserialize)]
        pub struct SubDes {
            id: Box<str>,
            status: SubStatus,
            condition: Box<RawValue>,
            created_at: Box<str>,
        }

        let cond = cond.into();
        let secret = rand::rand_hex(10);

        let body = CreateSub {
            name: T::NAME,
            version: T::VERSION,
            condition: cond,
            transport: TransportWithSecret {
                transport: self.transport(),
                secret: &secret,
            },
        };

        log::trace!(
            "sending POST request for event {:?}:\n\t{:?}",
            T::NAME,
            serde_json::to_string(&body)
        );

        let res: CreateSubRes = self
            .auth
            .send_req_json(surf::post(EVENTSUB_API).body_json(&body).unwrap().build())
            .await
            .context("failed to send subscription creation request")?;

        let [s] = res.data;

        let (tx, rx) = async_std::channel::unbounded();
        let sub = Subscription::<T>::new(
            s.id,
            s.status,
            s.condition,
            s.created_at,
            secret.clone().into(),
            rx,
        );

        self.map.insert(sub.get_unique(), (sub._status(), secret.into(), tx));

        Ok(sub)
    }
}

pub async fn get(auth: &HelixAuth) -> Result<Vec<SubInner>> {
    #[derive(Deserialize)]
    struct SubRetDes {
        data: Vec<SubInner>,
    }

    let sub: SubRetDes = auth.send_req_json(surf::get(EVENTSUB_API).build()).await?;

    log::debug!("retrieved {} subscriptions", sub.data.len());

    Ok(sub.data)
}

pub async fn delete(auth: &HelixAuth, sub: SubInner) -> Result<()> {
    #[derive(Serialize)]
    struct Id<'a> {
        id: &'a str,
    }

    let mut res = auth
        .send_req(
            surf::delete(EVENTSUB_API)
                .query(&Id { id: sub.id() })
                .map_err(|e| e.into_inner())?
                .build(),
        )
        .await?;

    if !res.status().is_success() {
        let body = res.body_string().await.map_err(|e| {
            e.into_inner()
                .context("error while trying to receive unsuccessful request")
        })?;

        return Err(anyhow!(
            "error while deleting subscription {} (status {}): {}",
            sub.id(),
            res.status(),
            body
        ));
    }

    Ok(())
}

pub async fn clean(auth: &HelixAuth) -> Result<()> {
    use futures::future::join_all;

    log::info!("cleaning all dangling connections...");
    join_all(
        get(auth)
            .await?
            .into_iter()
            .filter(|x| !x.status().is_ok())
            .map(|x| delete(auth, x)),
    )
    .await;
    Ok(())
}

pub async fn wipe(auth: &HelixAuth) -> Result<()> {
    use futures::future::join_all;

    log::info!("wiping all leftover connections...");
    join_all(get(auth).await?.into_iter().map(|x| delete(auth, x))).await;
    Ok(())
}
