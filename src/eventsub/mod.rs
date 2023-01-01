use async_std::channel::Sender;
use atomic::{Atomic, Ordering};
use futures::lock::Mutex;
use std::{collections::HashMap, sync::Arc};
use tide::{Request, Response};
use serde::{Serialize, Deserialize};
use serde_json::value::RawValue;
use super::HelixAuth;

pub mod event;
use event::SubscriptionType;

mod subscription;
pub use subscription::*;

const EVENTSUB_API: &'static str = "https://api.twitch.tv/helix/eventsub/subscriptions";

#[allow(unused)]
const MSG_ID: &'static str = "Twitch-Eventsub-Message-Id";
#[allow(unused)]
const MSG_RETRY: &'static str = "Twitch-Eventsub-Message-Retry";
const MSG_TYPE: &'static str = "Twitch-Eventsub-Message-Type";
#[allow(unused)]
const MSG_SIG: &'static str = "Twitch-Eventsub-Message-Signature";
#[allow(unused)]
const MSG_TIME: &'static str = "Twitch-Eventsub-Message-Timestamp";
#[allow(unused)]
const SUB_TYPE: &'static str = "Twitch-Eventsub-Subscription-Type";
#[allow(unused)]
const SUB_VER: &'static str = "Twitch-Eventsub-Subscription-Version";

const MSG_NOTIFICATION: &'static str = "notification";
const MSG_VERIFICATION: &'static str = "webhook_callback_verification";
const MSG_REVOCATION: &'static str = "revocation";


type Secret = Box<str>;
type State = Arc<Mutex<HashMap<SubUnique, (Arc<Atomic<SubStatus>>, Secret, Sender<Box<RawValue>>)>>>;

async fn callback(mut req: Request<State>) -> tide::Result {
    fn err_state(state: SubStatus) -> tide::Result {
        #[derive(Serialize)]
        struct ErrMsg {
            status: &'static str,
            desc: &'static str,
            state: SubStatus
        }

        return Ok(Response::builder(409)
            .body( tide::Body::from_json(&ErrMsg {
                status: "409 Conflict",
                desc: "The subscription exists, but is not in a respondable state for the request.",
                state,
            })? ).build());
    }

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    fn verify_msg(secret: &str, req: &Request<State>, body: &[u8]) -> bool {
        let (Some(v1), Some(v2), Some(sig)) = (
            req.header(MSG_ID),
            req.header(MSG_TIME),
            req.header(MSG_SIG)
        ) else {
            log::debug!("required header for hmac verification was empty");
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

        if sig.as_str().len() < 7 { return false };
        let sig = &sig.as_str()[7..];

        if sig.len() % 2 != 0 { return false };
        let Ok(sig): Result<Vec<u8>, _> = (0..sig.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&sig[i..i + 2], 16))
            .collect() else { return false };

        return mac.verify_slice(&sig).is_ok();
    }

    let body = req.body_bytes().await?;
    log::trace!("recieved webhook request: {:?}", std::str::from_utf8(&body));
    let Some(msg_type) = req.header(MSG_TYPE)
        else { return Ok(Response::builder(400).build()) };
    
    match msg_type.as_str() {
        MSG_NOTIFICATION => {
            #[derive(Deserialize)]
            struct RawEvent {
                subscription: SubUnique,
                event: Box<RawValue>,
            }
            
            let msg: RawEvent = serde_json::from_slice(&body)?;
            
            let map = &mut *req.state().lock().await;
            let (status, secret, tx) =
                if let Some(x) = map.get(&msg.subscription) {x}
                else { return Ok(Response::builder(404).build()) };
            
            if !verify_msg(secret, &req, &body) { return Ok(Response::builder(401).build()); }
            
            let s = status.load(Ordering::Relaxed);
            if s != SubStatus::Enabled { return err_state(s); }

            return match tx.send(msg.event).await {
                Ok(_) => Ok(Response::builder(200).build()),
                Err(_) => {
                    map.remove(&msg.subscription);
                    Ok(Response::builder(410).build())
                },
            }
        },
        MSG_VERIFICATION => {
            #[derive(Deserialize)]
            struct ChallengeReq {
                subscription: SubUnique,
                challenge: String
            }

            let challenge: ChallengeReq = serde_json::from_slice(&body)?;
            
            let map = &*req.state().lock().await;
            let (status, secret, tx) =
                if let Some(x) = map.get(&challenge.subscription) {x}
                else { return Ok(Response::builder(404).build()) };
            
            if !verify_msg(secret, &req, &body) { return Ok(Response::builder(401).build()); }

            match status.compare_exchange(
                SubStatus::VerificationPending,
                SubStatus::Enabled,
                Ordering::Relaxed, Ordering::Relaxed
            ) {
                Ok(_) => return Ok(Response::builder(200)
                    .body(challenge.challenge).build()),
                Err(state) => return err_state(state),
            }
        },
        MSG_REVOCATION => {
            #[derive(Deserialize)]
            struct RevokeReq { subscription: SubUnqStatus }

            #[derive(Deserialize)]
            struct SubUnqStatus {
                #[serde(flatten)]
                unique: SubUnique,
                status: SubStatus,
            }
            
            let rev: RevokeReq = serde_json::from_slice(&body)?;

            let (status, secret, tx) =
                if let Some(x) = (*req.state().lock().await).remove(&rev.subscription.unique) {x}
                else { return Ok(Response::builder(404).build()) };
            
            if !verify_msg(&secret, &req, &body) { return Ok(Response::builder(401).build()); }
            
            let s = status.swap(rev.subscription.status, Ordering::Relaxed);
            return Ok(Response::builder(200).build());
        }
        _ => return Ok(Response::builder(400).build()),
    };
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
    }
}

pub struct EventSub {
    map: State,
    auth: HelixAuth,
    v_addr: url::Url,
}

impl EventSub {
    pub async fn new(addr: std::net::SocketAddr, v_addr: &url::Url, auth: HelixAuth) -> Self {
        let state = Arc::new(Mutex::new(HashMap::new()));
        let mut serve = tide::with_state(state.clone());
        serve.at("/callback").post(callback);

        async_std::task::spawn(async move {
            serve.listen(addr).await.unwrap()
        });
        log::info!("started server at {:?}", addr);

        return Self { map: state, auth, v_addr: v_addr.join("callback").unwrap() };
    }

    pub fn transport(&self) -> Transport {
        return Transport::Webhook {
            callback: &self.v_addr.as_str()
        };
    }

    pub async fn subscribe<T: SubscriptionType> (&self, cond: impl Into<T::Cond>) -> surf::Result<Subscription<T>> {
        #[derive(Debug, Serialize)]
        struct CreateSub<'a, T> {
            #[serde(rename = "type")]
            name: &'static str,
            version: &'static str,
            condition: T,
            transport: TransportWithSecret<'a>,
        }

        #[derive(Debug, Serialize)]
        struct TransportWithSecret<'a> {
            #[serde(flatten)]
            transport: Transport<'a>,
            secret: &'a str
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

        let body = CreateSub {
            name: T::name(),
            version: T::ver(),
            condition: cond,
            transport: TransportWithSecret {
                transport: self.transport(),
                secret: "8a40a45fb5"
            },
        };

        log::trace!(
            "sending POST request for event {:?}:\n\t{:?}",
            T::name(), serde_json::to_string(&body)
        );

        let mut x = surf::post(EVENTSUB_API)
            .header("Authorization", self.auth.auth())
            .header("Client-Id", self.auth.client_id())
            .body_json(&body)
            .unwrap()
            .send()
            .await?;

        let body = x.body_bytes().await?;

        log::trace!(
            "response of POST request for event {:?}: {}\n\t{:?}",
            T::name(), x.status(), std::str::from_utf8(&body)
        );

        let res: CreateSubRes = serde_json::from_slice(&body)?;

        let [s] = res.data;

        let (tx, rx) = async_std::channel::unbounded();
        let sub = Subscription::<T>::new(
            s.id,
            s.status,
            s.condition,
            s.created_at,
            "8a40a45fb5".into(),
            rx,
        );

        (*self.map.lock().await).insert(
            sub.get_unique(),
            (sub._status(), "8a40a45fb5".into(), tx)
        );

        return Ok(sub);
    }
}


pub async fn get(auth: &HelixAuth) -> surf::Result<Vec<SubInner>> {
    #[derive(Deserialize)]
    struct SubRetDes {
        data: Vec<SubInner>
    }

    let sub: SubRetDes = surf::get(EVENTSUB_API)
        .header("Authorization", auth.auth())
        .header("Client-Id", auth.client_id())
        .send()
        .await?
        .body_json()
        .await?;

    log::debug!("retrieved {} subscriptions", sub.data.len());

    return Ok(sub.data);
}

pub async fn delete(auth: &HelixAuth, sub: SubInner) -> surf::Result<()> {
    async fn _delete(auth: &HelixAuth, sub: &SubInner) -> surf::Result<surf::Response> {
        #[derive(Serialize)]
        struct Id<'a> {
            id: &'a str
        }
    
        return Ok( surf::delete(EVENTSUB_API)
            .query( &Id { id: sub.id() } )?
            .header("Authorization", auth.auth())
            .header("Client-Id", auth.client_id())
            .send()
            .await? );
    }
    
    let mut res =_delete(auth, &sub).await?;
    if res.status().is_success() { Ok(()) } else {
        log::error!("error while deleting subscription: recieved status {} for subscription {}", res.status(), sub.id());
        Err(surf::Error::from_str(res.status(), format!("failed to delete subscription {}: {:?}", sub.id(), res.body_string().await?)))
    }
}

pub async fn clean(auth: &HelixAuth) -> surf::Result<()> {
    use futures::future::join_all;

    log::info!("cleaning all dangling connections...");
    join_all(
        get(auth).await?
            .into_iter()
            .filter(|x| !x.status().is_ok())
            .map(|x| delete(auth, x))
    ).await;
    return Ok(());
}

pub async fn wipe(auth: &HelixAuth) -> surf::Result<()> {
    use futures::future::join_all;

    log::info!("wiping all leftover connections...");
    join_all(
        get(auth).await?
            .into_iter()
            .map(|x| delete(auth, x))
    ).await;
    return Ok(())
}
