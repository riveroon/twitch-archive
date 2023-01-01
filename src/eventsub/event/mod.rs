use serde::{de::DeserializeOwned, Serialize};

pub mod stream;

pub trait SubscriptionType {
    type Cond: Serialize;
    type Event: DeserializeOwned;

    fn name() -> &'static str;
    fn ver() -> &'static str;
}