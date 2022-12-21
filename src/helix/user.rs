use super::HelixAuth;
use async_once_cell::OnceCell;
use serde::Deserialize;
use surf::{ RequestBuilder, http};
use url::Url;

const USER_API: &'static str = "https://api.twitch.tv/helix/users";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    Admin,
    GlobalMod,
    Staff,
    #[serde(rename = "")]
    None
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcasterType {
    Affiliate,
    Partner,
    #[serde(rename = "")]
    None
}

#[derive(Debug, Deserialize)]
struct Details {
    user_type: UserType,
    broadcaster_type: BroadcasterType,
    desciption: Box<str>,
    profile_image_url: Box<Url>,
    offline_image_url: Box<Url>,
    created_at: Box<str>
}

#[derive(Debug, Deserialize)]
struct Credentials {
    id: Box<str>,
    login: Box<str>,
    #[serde(rename = "display_name")]
    name: Box<str>,
}

#[derive(Debug)]
pub struct User {
    credentials: Credentials,
    details: OnceCell<Details>,
}

impl User {
    pub fn new(id: impl ToString, login: impl ToString, name: impl ToString) -> Self {
        Self {
            credentials: Credentials {
                id: id.to_string().into_boxed_str(),
                login: login.to_string().into_boxed_str(),
                name: name.to_string().into_boxed_str()
            },
            details: OnceCell::new(),
        }
    }

    pub async fn from_id(id: &str, auth: &HelixAuth) -> surf::Result<Self> {
        get_user(&auth, UserCredentials::Id(id)).await
    }

    pub async fn from_login(login: &str, auth: &HelixAuth) -> surf::Result<Self> {
        get_user(&auth, UserCredentials::Login(login)).await
    }

    pub fn id(&self) -> &str { &self.credentials.id }

    pub fn login(&self) -> &str { &self.credentials.login }

    pub async fn details(&self, auth: &HelixAuth) -> surf::Result<&Details> {
        self.details.get_or_try_init(async {
            Ok(get_user(auth, UserCredentials::Id(&self.credentials.id)).await?
                .details.into_inner().unwrap())
        }).await
    }
}

pub enum UserCredentials<'a> {
    Id(&'a str),
    Login(&'a str),
}

// users should have a maximum length of 100.
async fn _get_user(auth: &HelixAuth, users: &[UserCredentials<'_>]) -> surf::Result<impl Iterator<Item = User>>
{
    #[derive(Deserialize)]
    struct GetUserRes {
        data: Vec<UserDes>
    }

    #[derive(Deserialize)]
    struct UserDes {
        #[serde(flatten)]
        credentials: Credentials,
        #[serde(flatten)]
        details: Details
    }

    let mut url: Url = USER_API.parse().unwrap();
    url.query_pairs_mut()
        .extend_pairs(
            users.iter().map(|user| {
                match user {
                    UserCredentials::Id(id) => ("user_id", *id),
                    UserCredentials::Login(login) => ("user_login", *login),
                }
            })
        );

    let res: GetUserRes = RequestBuilder::new(http::Method::Get, url)
        .header("Authorization", auth.auth())
        .header("Client-Id", auth.client_id())
        .recv_json().await?;

    return Ok(res.data.into_iter().map(|UserDes { credentials, details }|
        User { credentials, details: OnceCell::new_with(Some(details)) }
    ));
}

/*
pub fn get_users(auth: HelixAuth, users: &[UserCredentials<'_>]) -> impl TryStream<Ok = User, Error = surf::Error>
{
    return futures::stream::iter( users.chunks(100) )
        .map(|c| _get_user(&auth, c).await? );
}
*/

pub async fn get_user(auth: &HelixAuth, cred: UserCredentials<'_>) -> surf::Result<User> {
    return Ok(_get_user(&auth, &[cred]).await?.next()
        .expect("helix api response was invalid: no User object found"));
}