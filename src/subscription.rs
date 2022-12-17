use serde::{Serialize, Deserialize, de::DeserializeOwned};
use serde_json::value::RawValue;

#[allow(unused)]
#[derive(Deserialize)]
pub struct Subscription {
    id: String,
    #[serde(rename = "type")]
    name: String,
    version: String,
    status: SubStatus,
    condition: Box<RawValue>,
    created_at: String,
}

impl Subscription {
    pub fn id(&self) -> &str { &self.id }
    pub fn status(&self) -> SubStatus { self.status }
    pub fn condition<'de, T: DeserializeOwned> (&self) -> serde_json::Result<T> {
        serde_json::from_str(self.condition.get())
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn is_ok(&self) -> bool { *self == Self::Enabled || *self == Self::VerificationPending }
}
