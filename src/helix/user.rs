use super::HelixAuth;
use async_once_cell::OnceCell;
use serde::{Deserialize, Serialize};
use url::Url;

const USER_API: &str = "https://api.twitch.tv/helix/users";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    Admin,
    GlobalMod,
    Staff,
    #[serde(rename = "")]
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcasterType {
    Affiliate,
    Partner,
    #[serde(rename = "")]
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Details {
    #[serde(rename = "type")]
    user_type: UserType,
    broadcaster_type: BroadcasterType,
    description: Box<str>,
    #[serde(deserialize_with = "empty_to_none")]
    profile_image_url: Option<Url>,
    #[serde(deserialize_with = "empty_to_none")]
    offline_image_url: Option<Url>,
    created_at: Box<str>,
}

fn empty_to_none<'de, D>(deserializer: D) -> Result<Option<Url>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    // Due to a bug with #[serde(flatten)] and RawValue,
    // deserializing to RawValue doesn't work.
    let raw: Box<str> = Box::deserialize(deserializer)?;
    if raw.is_empty() {
        Ok(None)
    } else {
        raw.parse().map(Some).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Credentials {
    id: Box<str>,
    login: Box<str>,
    #[serde(rename(deserialize = "display_name"))]
    name: Box<str>,
}

#[derive(Serialize, Debug)]
#[serde(into = "UserSer")]
pub struct User {
    credentials: Credentials,
    details: OnceCell<Box<Details>>,
}

impl User {
    pub fn new(id: impl ToString, login: impl ToString, name: impl ToString) -> Self {
        Self {
            credentials: Credentials {
                id: id.to_string().into(),
                login: login.to_string().into(),
                name: name.to_string().into(),
            },
            details: OnceCell::new(),
        }
    }

    pub async fn from_id(id: &str, auth: &HelixAuth) -> surf::Result<Self> {
        get_user(auth, UserCredentials::Id(id)).await
    }

    pub async fn from_login(login: &str, auth: &HelixAuth) -> surf::Result<Self> {
        get_user(auth, UserCredentials::Login(login)).await
    }

    pub fn id(&self) -> &str {
        &self.credentials.id
    }
    pub fn login(&self) -> &str {
        &self.credentials.login
    }
    pub fn name(&self) -> &str {
        &self.credentials.name
    }

    /// Fetches the user details from the twitch server.
    /// This creates a local cache of the response, and therefore
    /// returns the details of a user sometime in the past.
    async fn get_details(&self, auth: &HelixAuth) -> surf::Result<&Details> {
        self.details
            .get_or_try_init(async {
                Ok(get_user(auth, UserCredentials::Id(&self.credentials.id))
                    .await?
                    .details
                    .into_inner()
                    .unwrap())
            })
            .await
            .map(|x| &**x)
    }
}

impl Clone for User {
    fn clone(&self) -> Self {
        Self {
            credentials: self.credentials.clone(),
            details: OnceCell::new_with(self.details.get().cloned()),
        }
    }
}

impl fmt::Display for User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{} ({})", self.id(), self.login())
    }
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.credentials == other.credentials
    }
}

impl Eq for User {}

use core::{
    fmt,
    hash::{Hash, Hasher},
};
impl Hash for User {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.credentials.hash(state);
    }
}

#[derive(Serialize)]
struct UserSer {
    #[serde(flatten)]
    credentials: Credentials,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    details: Option<Details>,
}

impl From<User> for UserSer {
    fn from(value: User) -> Self {
        Self {
            credentials: value.credentials,
            details: value.details.into_inner().map(|x| *x),
        }
    }
}

#[derive(PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserCredentials<'a> {
    Id(&'a str),
    Login(&'a str),
}

// users should have a maximum length of 100.
async fn _get_user(
    auth: &HelixAuth,
    users: &[UserCredentials<'_>],
) -> surf::Result<impl Iterator<Item = User>> {
    #[derive(Deserialize)]
    struct GetUserRes {
        data: Vec<UserDes>,
    }

    #[derive(Deserialize)]
    struct UserDes {
        #[serde(flatten)]
        credentials: Credentials,
        #[serde(flatten)]
        details: Details,
    }

    let mut url: Url = USER_API.parse().unwrap();
    url.query_pairs_mut()
        .extend_pairs(users.iter().map(|user| match user {
            UserCredentials::Id(id) => ("id", *id),
            UserCredentials::Login(login) => ("login", *login),
        }));

    let res: GetUserRes = auth.send(surf::get(url).build()).await?.body_json().await?;

    Ok(res.data.into_iter().map(
        |UserDes {
             credentials,
             details,
         }| User {
            credentials,
            details: OnceCell::new_with(Some(Box::new(details))),
        },
    ))
}

pub(crate) async fn get_user(auth: &HelixAuth, cred: UserCredentials<'_>) -> surf::Result<User> {
    Ok(_get_user(auth, &[cred])
        .await?
        .next()
        .expect("helix api response was invalid: no User object found"))
}
