use super::{HelixAuth, User};

use serde::{Serialize, Deserialize};

const STREAM_API: &'static str = "https://api.twitch.tv/helix/streams";

#[derive(Deserialize)]
pub struct Stream {
    id: Box<str>,
    user_id: Box<str>,
    user_login: Box<str>,
    user_name: Box<str>,
    game_id: Box<str>,
    game_name: Box<str>,
    title: Box<str>,
    started_at: Box<str>,
    is_mature: bool
}

pub enum StreamFilter {
    User(User),
    GameId(String),
    //Type(String),
    Language(String),
}

use url::Url;
use futures::{TryStream, StreamExt};
use std::collections::VecDeque;
use surf::{http, RequestBuilder};
pub async fn get_streams<'a, T, U> (auth: HelixAuth, filter: T) -> impl TryStream<Ok = Stream, Error = surf::Error>
where
    T: IntoIterator<Item = &'a StreamFilter>
{
    #[derive(Deserialize)]
    struct Pagination { cursor: Box<str> }

    #[derive(Deserialize)]
    struct GetStreamsRes {
        data: VecDeque<Stream>,
        pagination: Pagination
    }

    let mut url: Url = STREAM_API.parse().unwrap();
    url.query_pairs_mut()
        .extend_pairs(
            filter.into_iter().map(|x|
                    match x {
                        StreamFilter::User(user) => ("user_id", user.id()),
                        StreamFilter::GameId(x) => ("game_id", x.as_str()),
                        StreamFilter::Language(x) => ("language", x.as_str()),
                    }
                )
        );


    let res = RequestBuilder::new(http::Method::Get, url)
        .header("Authorization", auth.auth())
        .header("Client-Id", auth.client_id())
        .recv_json::<GetStreamsRes> ().await;
    

    return match res {
        Ok(x) => futures::stream::try_unfold((x.data.into_iter(), x.pagination, auth),
            |(mut data, page, auth)| async {
                if let Some(x) = data.next() { return Ok(Some( (x, (data, page, auth)) )) }

                #[derive(Serialize)]
                struct Query<'a> {
                    first: u8,
                    after: &'a str,
                }

                let res: GetStreamsRes = surf::get(STREAM_API)
                    .header("Authorization", auth.auth())
                    .header("Client-Id", auth.client_id())
                    .query(&Query { first: 100, after: &page.cursor })?
                    .recv_json().await?;
                
                let (mut data, page) = (res.data.into_iter(), res.pagination);
                
                return Ok(data.next()
                    .map(|x| (x, (data, page, auth)))
                );
            }
        ).fuse().boxed(),
        Err(e) => futures::stream::once(async{ Err(e) }).boxed()
    }
}