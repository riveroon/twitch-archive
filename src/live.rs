use rand::Rng;

use crate::prelude::*;

async fn send_req(login: &str, auth: Option<&str>) -> surf::Result<surf::Response> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Req<'a> {
        operation_name: &'static str,
        extensions: Extensions,
        variables: ReqVar<'a>
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ReqVar<'a> {
        is_live: bool,
        login: &'a str,
        is_vod: bool,
        #[serde(rename = "vodID")]
        vod_id: &'static str,
        player_type: &'static str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Extensions {
        persisted_query: PersistedQuery
    }

    #[derive(Serialize)]
    struct PersistedQuery {
        version: u16,
        #[serde(rename = "sha256Hash")]
        hash: &'static str
    }

    let body = Req {
        operation_name: "PlaybackAccessToken",
        extensions: Extensions {
            persisted_query: PersistedQuery {
                version: 1,
                hash: "0828119ded1c13477966434e15800ff57ddacf13ba1911c129dc2200705b0712"
            }
        },
        variables: ReqVar {
            is_live: true,
            login,
            is_vod: false,
            vod_id: "",
            player_type: "embed"
        }
    };

    let mut req = surf::post("https://gql.twitch.tv/gql")
        .header("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.111 Safari/537.36");

    if let Some(auth) = auth {
        req = req.header("Authorization", format!("OAuth {}", auth));
    }

    req.body_json(&body)?
        .send()
        .await
}

async fn parse_res(login: &str, mut res: surf::Response) -> surf::Result<Option<String>> {
    #[derive(Deserialize)]
    struct Res {
        data: ResData
    }

    #[derive(Deserialize)]
    struct ResData {
        #[serde(rename = "streamPlaybackAccessToken")]
        token: Option<Token>
    }

    #[derive(Deserialize)]
    struct Token {
        value: String,
        signature: String,
    }

    let res: Res = res.body_json().await?;

    if let Some(token) = res.data.token {
        Ok(Some(format!(
            "http://usher.ttvnw.net/api/channel/hls/{}.m3u8?player=twitchweb&&token={}&sig={}&allow_audio_only=true&allow_source=true&type=any&p={}",
            login, token.value, token.signature, rand::thread_rng().gen_range(0..=999999)
        )))
    } else { Ok(None) }
    
}

pub async fn get_hls(login: impl AsRef<str>, auth: Option<&str>) -> anyhow::Result<Option<String>> {
    let login = login.as_ref();
    let res = send_req(login, auth).await
        .map_err(surf::Error::into_inner)?;

    if !res.status().is_success() {
        return Ok(None);
    }

    parse_res(login, res).await
        .map_err(surf::Error::into_inner)
}
