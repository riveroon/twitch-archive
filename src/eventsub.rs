use async_std::channel::{unbounded, Sender, Receiver};
use futures::future::join_all;
use std::net::SocketAddr;
use tide::{Request, Response};
use serde::{Serialize, Deserialize};
use serde_json::value::RawValue;
use super::{Subscription, HelixAuth};
use std::fmt;

const EVENTSUB_API: &'static str = "https://api.twitch.tv/helix/eventsub/subscriptions";

#[allow(unused)]
const MSG_ID: &'static str = "Twitch-Eventsub-Message-Id";
#[allow(unused)]
const MSG_RETRY: &'static str = "Twitch-Eventsub-Message-Retry";
const MSG_TYPE: &'static str = "Twitch-Eventsub-Message-Type";
#[allow(unused)]
const MSG_SIG: &'static str = "Twitch-Eventsub-Message-Sig";
#[allow(unused)]
const MSG_TIME: &'static str = "Twitch-Eventsub-Message-Timestamp";
#[allow(unused)]
const SUB_TYPE: &'static str = "Twitch-Eventsub-Subscription-Type";
#[allow(unused)]
const SUB_VER: &'static str = "Twitch-Eventsub-Subscription-Version";

const MSG_NOTIFICATION: &'static str = "notification";
const MSG_VERIFICATION: &'static str = "webhook_callback_verification";
const MSG_REVOCATION: &'static str = "revocation";

type Item = (Message<Box<RawValue>>, Subscription);

pub enum Message<T> {
    Notification(T),
    Revocation,
}

async fn callback(mut req: Request<Sender<Item>>) -> tide::Result {
    let body = req.body_bytes().await?;
    log::trace!("recieved webhook request: {:?}", std::str::from_utf8(&body));
    let Some(msg_type) = req.header(MSG_TYPE)
        else { return Ok(Response::builder(404).build()) };
    match msg_type.as_str() {
        MSG_NOTIFICATION => {
            #[derive(Deserialize)]
            struct RawEvent {
                subscription: Subscription,
                event: Box<RawValue>,
            }
            
            let msg: RawEvent = serde_json::from_slice(&body)?;
            return match req.state().send((
                Message::Notification(msg.event), msg.subscription
            )).await {
                Ok(_) => Ok(Response::builder(200).build()),
                Err(_) => Ok(Response::builder(404).build()),
            }
        },
        MSG_VERIFICATION => {
            #[derive(Deserialize)]
            struct ChallengeReq { challenge: String }

            let challenge: ChallengeReq = serde_json::from_slice(&body)?;

            return Ok(Response::builder(200)
                .body(challenge.challenge)
                .build());
        },
        MSG_REVOCATION => {
            #[derive(Deserialize)]
            struct RevokeReq { subscription: Subscription }
            
            let rev: RevokeReq = serde_json::from_slice(&body)?;
            return match req.state().send((
                Message::Revocation,
                rev.subscription,
            )).await {
                Ok(_) => Ok(Response::builder(200).build()),
                Err(_) => Ok(Response::builder(404).build()),
            }
        }
        _ => return Ok(Response::builder(404).build()),
    };
}

#[derive(fmt::Debug, Serialize, Deserialize)]
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
    auth: HelixAuth,
    v_addr: url::Url,
}

impl EventSub {
    pub async fn new(addr: SocketAddr, v_addr: &url::Url, auth: HelixAuth) -> (Self, Receiver<Item>) {
        let (tx, rx) = unbounded::<Item> ();
        let mut serve = tide::with_state(tx);
        serve.at("/callback").post(callback);

        async_std::task::spawn(async move {
            serve.listen(addr).await.unwrap()
        });
        log::debug!("started server at {:?}", addr);

        return ( Self { auth, v_addr: v_addr.join("callback").unwrap() }, rx );
    }

    pub fn transport(&self) -> Transport {
        return Transport::Webhook {
            callback: &self.v_addr.as_str()
        };
    }

    pub async fn online (&self, channel: &str) -> surf::Result<Subscription> {
        #[derive(Serialize)]
        struct CreateSub<'a> {
            #[serde(rename = "type")]
            name: &'static str,
            version: &'static str,
            condition: Cond<'a>,
            transport: TransportWithSecret<'a>,
        }

        #[derive(Serialize)]
        struct TransportWithSecret<'a> {
            #[serde(flatten)]
            transport: Transport<'a>,
            secret: String
        }
        
        #[derive(Serialize)]
        struct Cond<'a> {
            #[serde(rename = "broadcaster_user_id")]
            streamer_uid: &'a str
        }

        #[derive(Deserialize)]
        struct CreateSubRes {
            data: [Subscription; 1],
        }

        let body = CreateSub {
            name: "stream.online",
            version: "1",
            condition: Cond { streamer_uid: channel },
            transport: TransportWithSecret {
                transport: self.transport(),
                secret: "8a40a45fb5".to_owned()
            },
        };

        log::trace!(
            "sending POST request for creation of event 'stream.online' for channel {}:\n\
            \tAuthorization: {}, Client-Id: {}\n\
            \t{:?}",
            channel, self.auth.auth(), self.auth.client_id(), serde_json::to_string(&body)
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
            "response recieved for creation of 'stream.online' for channel {}: {}\n\t{:?}",
            channel, x.status(), std::str::from_utf8(&body)
        );

        let res: CreateSubRes = serde_json::from_slice(&body)?;

        let [s] = res.data;
        return Ok(s);
    }
}

pub async fn get(auth: &HelixAuth) -> surf::Result<Vec<Subscription>> {
    #[derive(Deserialize)]
    struct SubRetDes {
        data: Vec<Subscription>
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

pub async fn delete(auth: &HelixAuth, sub: Subscription) -> Result<(), ()> {
    async fn _delete(auth: &HelixAuth, sub: &Subscription) -> surf::Result<surf::Response> {
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
    
    return match _delete(auth, &sub).await {
        Ok(x) => {
            if x.status().is_success() { Ok(()) } else {
                log::error!("error while deleting subscription: recieved status {} for subscription {}", x.status(), sub.id());
                Err(())
            }
        },
        Err(e) => {
            log::error!("error while deleting subscription: {:?}", e);
            Err(())
        }
    }
}

pub async fn clean(auth: &HelixAuth) -> surf::Result<()> {
    log::debug!("cleaning all dangling connections...");
    join_all(
        get(auth).await?
            .into_iter()
            .filter(|x| !x.status().is_ok())
            .map(|x| delete(auth, x))
    ).await;
    return Ok(());
}

pub async fn wipe(auth: &HelixAuth) -> surf::Result<()> {
    log::debug!("wiping all leftover connections...");
    join_all(
        get(auth).await?
            .into_iter()
            .map(|x| delete(auth, x))
    ).await;
    return Ok(())
}