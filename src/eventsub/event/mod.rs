use serde::{de::DeserializeOwned, Serialize};

pub mod stream;

pub trait SubscriptionType {
    type Cond: Serialize;
    type Event: DeserializeOwned;

    const NAME: &'static str;
    const VERSION: Version;
}


#[derive(Debug, Serialize)]
pub struct Version {
    #[serde(flatten)]
    inner: &'static str
}

impl Version {
    pub(crate) const fn new(inner: &'static str) -> Self {
        Self { inner }
    }
}