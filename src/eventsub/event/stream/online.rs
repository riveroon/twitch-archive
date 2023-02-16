use chrono::{DateTime, Local};
use futures::TryStreamExt;

use super::super::SubscriptionType;
use crate::{
    helix::{get_streams, Stream, StreamFilter, User},
    prelude::*,
    HelixAuth,
};

pub struct Online;

impl SubscriptionType for Online {
    type Cond = OnlineCond;
    type Event = OnlineEvent;

    fn name() -> &'static str {
        "stream.online"
    }
    fn ver() -> &'static str {
        "1"
    }
}

#[derive(Serialize)]
pub struct OnlineCond {
    #[serde(rename = "broadcaster_user_id")]
    user_id: Box<str>,
}

impl OnlineCond {
    pub fn from_id(id: impl ToString) -> Self {
        OnlineCond {
            user_id: id.to_string().into(),
        }
    }
}

impl From<&User> for OnlineCond {
    fn from(value: &User) -> Self {
        Self::from_id(value.id())
    }
}

#[derive(Deserialize)]
#[serde(try_from = "OnlineEventDes")]
pub struct OnlineEvent {
    id: Box<str>,
    user: User,
    started_at: DateTime<Local>,
}

impl OnlineEvent {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn user(&self) -> &User {
        &self.user
    }
    pub fn started_at(&self) -> &DateTime<Local> {
        &self.started_at
    }

    pub async fn to_stream(&self, auth: HelixAuth) -> Result<Option<Stream>> {
        get_streams(auth, std::iter::once(StreamFilter::User(&self.user)))
            .try_next()
            .await
    }
}

#[derive(Deserialize)]
struct OnlineEventDes {
    id: Box<str>,
    #[serde(rename = "broadcaster_user_id")]
    user_id: Box<str>,
    #[serde(rename = "broadcaster_user_login")]
    user_login: Box<str>,
    #[serde(rename = "broadcaster_user_name")]
    user_name: Box<str>,
    started_at: Box<str>,
}

impl TryFrom<OnlineEventDes> for OnlineEvent {
    type Error = chrono::ParseError;

    fn try_from(value: OnlineEventDes) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            user: User::new(value.user_id, value.user_login, value.user_name),
            started_at: DateTime::parse_from_rfc3339(&value.started_at)?.with_timezone(&Local),
        })
    }
}
