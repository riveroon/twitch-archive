use serde::{de::DeserializeOwned, Serialize};

pub mod stream;

pub trait SubscriptionType {
    type Cond: Serialize;
    type Event: DeserializeOwned;

    const NAME: &'static str;
    const VERSION: Version;
}


#[derive(Copy, Clone, Debug)]
pub struct Version {
    inner: &'static str
}

impl Version {
    pub(crate) const fn new(inner: &'static str) -> Self {
        Self { inner }
    }
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer
    {
        serializer.serialize_str(self.inner)    
    }
}