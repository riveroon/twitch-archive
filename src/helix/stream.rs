use super::{HelixAuth, User};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

const STREAM_API: &str = "https://api.twitch.tv/helix/streams";

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "StreamDes")]
pub struct Stream {
    id: Box<str>,
    user: User,
    game_id: Box<str>,
    game_name: Box<str>,
    title: Box<str>,
    started_at: DateTime<Local>,
    is_mature: bool,
}

impl Stream {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn user(&self) -> &User {
        &self.user
    }
    pub fn game_id(&self) -> &str {
        &self.game_id
    }
    pub fn game_name(&self) -> &str {
        &self.game_name
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn started_at(&self) -> DateTime<Local> {
        self.started_at
    }
}

#[derive(Deserialize)]
pub struct StreamDes {
    id: Box<str>,
    user_id: Box<str>,
    user_login: Box<str>,
    user_name: Box<str>,
    game_id: Box<str>,
    game_name: Box<str>,
    title: Box<str>,
    started_at: Box<str>,
    is_mature: bool,
}

impl TryFrom<StreamDes> for Stream {
    type Error = chrono::ParseError;

    fn try_from(value: StreamDes) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            user: User::new(value.user_id, value.user_login, value.user_name),
            game_id: value.game_id,
            game_name: value.game_name,
            title: value.title,
            started_at: DateTime::parse_from_rfc3339(&value.started_at)?.with_timezone(&Local),
            is_mature: value.is_mature,
        })
    }
}

pub enum StreamFilter<'a> {
    User(&'a User),
    GameId(&'a str),
    //Type(String),
    Language(&'a str),
}

use futures::TryStream;
pub fn get_streams<'a, T>(
    auth: HelixAuth,
    filter: T,
) -> impl TryStream<Ok = Stream, Error = surf::Error> + Unpin
where
    T: IntoIterator<Item = StreamFilter<'a>>,
{
    use futures::StreamExt;
    use std::collections::VecDeque;
    use surf::{http, RequestBuilder};
    use url::Url;

    #[derive(Debug, Deserialize)]
    struct Pagination {
        cursor: Option<Box<str>>,
    }

    #[derive(Deserialize)]
    struct GetStreamsRes {
        data: VecDeque<Stream>,
        pagination: Pagination,
    }

    let mut url: Url = STREAM_API.parse().unwrap();
    url.query_pairs_mut()
        .extend_pairs(filter.into_iter().map(|x| match x {
            StreamFilter::User(user) => ("user_id", user.id()),
            StreamFilter::GameId(x) => ("game_id", x),
            StreamFilter::Language(x) => ("language", x),
        }));

    enum State<T: Iterator<Item = Stream>> {
        Init(Box<Url>),
        Next(T, Pagination),
    }

    return futures::stream::try_unfold(
        (State::Init(Box::new(url)), auth),
        |(state, auth)| async {
            let (mut data, page) = match state {
                State::Init(url) => {
                    log::trace!("fetching streams: {:?}", url.as_str());
                    let res: GetStreamsRes = auth
                        .send(RequestBuilder::new(http::Method::Get, *url).build())
                        .await?
                        .body_json()
                        .await?;
                    log::trace!("fetch successful: {:?}, {:?}", res.data, res.pagination);
                    (res.data.into_iter(), res.pagination)
                }
                State::Next(data, page) => (data, page),
            };

            if let Some(x) = data.next() {
                return Ok(Some((x, (State::Next(data, page), auth))));
            }
            log::trace!("no data: fetching next page");

            let Some(cursor) = page.cursor else { return Ok(None) };

            #[derive(Serialize)]
            struct Query<'a> {
                first: u8,
                after: &'a str,
            }

            let res: GetStreamsRes = auth
                .send(
                    surf::get(STREAM_API)
                        .query(&Query {
                            first: 100,
                            after: &cursor,
                        })?
                        .build(),
                )
                .await?
                .body_json()
                .await?;

            let (mut data, page) = (res.data.into_iter(), res.pagination);

            Ok(data.next().map(|x| (x, (State::Next(data, page), auth))))
        },
    )
    .fuse()
    .boxed();
}
