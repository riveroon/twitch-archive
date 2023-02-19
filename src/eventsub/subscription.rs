use async_std::channel::Receiver;
use atomic::{Atomic, Ordering};
use serde_json::value::RawValue;
use std::{marker::PhantomData, sync::Arc};

use super::SubscriptionType;
use crate::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct SubUnique {
    id: Box<str>,
}

impl SubUnique {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Deserialize)]
#[serde(from = "SubInnerDes")]
pub struct SubInner {
    unique: SubUnique,
    status: Arc<Atomic<SubStatus>>,
    condition: Box<RawValue>,
    created_at: Box<str>,
}

impl SubInner {
    pub fn new(
        id: Box<str>,
        status: SubStatus,
        condition: Box<RawValue>,
        created_at: Box<str>,
    ) -> Self {
        Self {
            unique: SubUnique { id },
            status: Arc::new(Atomic::new(status)),
            condition,
            created_at,
        }
    }

    pub fn id(&self) -> &str {
        self.unique.id()
    }
    pub fn status(&self) -> SubStatus {
        self.status.load(Ordering::Relaxed)
    }
    pub(crate) fn _status(&self) -> Arc<Atomic<SubStatus>> {
        self.status.clone()
    }
    pub fn get_unique(&self) -> SubUnique {
        self.unique.clone()
    }
}

#[derive(Deserialize)]
struct SubInnerDes {
    id: Box<str>,
    status: SubStatus,
    condition: Box<RawValue>,
    created_at: Box<str>,
}

impl From<SubInnerDes> for SubInner {
    fn from(value: SubInnerDes) -> Self {
        Self::new(value.id, value.status, value.condition, value.created_at)
    }
}

pub struct Subscription<T> {
    inner: SubInner,
    secret: Box<str>,
    rx: Receiver<Box<RawValue>>,
    phantom: PhantomData<T>,
}

impl<T> Subscription<T> {
    pub fn get_unique(&self) -> SubUnique {
        self.inner.get_unique()
    }
    pub fn id(&self) -> &str {
        self.inner.id()
    }
    pub fn status(&self) -> SubStatus {
        self.inner.status()
    }
    pub(crate) fn _status(&self) -> Arc<Atomic<SubStatus>> {
        self.inner._status()
    }
}

impl<T: SubscriptionType> Subscription<T> {
    pub fn new(
        id: Box<str>,
        status: SubStatus,
        condition: Box<RawValue>,
        created_at: Box<str>,
        secret: Box<str>,
        rx: Receiver<Box<RawValue>>,
    ) -> Self {
        Self {
            inner: SubInner::new(id, status, condition, created_at),
            secret,
            rx,
            phantom: PhantomData,
        }
    }

    pub async fn recv(&self) -> Result<Option<T::Event>, RecvError> {
        if !self.status().is_ok() {
            return Ok(None);
        }

        let event = self.rx.recv().await.map_err(RecvError::ChannelClosed)?;

        match serde_json::from_str(event.get()) {
            Ok(x) => Ok(Some(x)),
            Err(e) => Err(RecvError::ParseError(e)),
        }
    }
}

#[derive(Debug)]
pub enum RecvError {
    ChannelClosed(async_std::channel::RecvError),
    ParseError(serde_json::Error),
}

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelClosed(e) => write!(f, "subscription failed to receive event: {e}"),
            Self::ParseError(e) => write!(f, "subscription failed to parse event: {e}"),
        }
    }
}

impl std::error::Error for RecvError {}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    Enabled,
    #[serde(rename = "webhook_callback_verification_pending")]
    VerificationPending,
    #[serde(rename = "webhook_callback_verification_failed")]
    VerificationFailed,
    NotificationFailuresExceeded,
    AuthorizationRevoked,
    ModeratorRemoved,
    UserRemoved,
    VersionRemoved,
}

impl SubStatus {
    pub fn is_ok(&self) -> bool {
        *self == Self::Enabled || *self == Self::VerificationPending
    }
}
