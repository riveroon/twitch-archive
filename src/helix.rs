use serde::Deserialize;
use surf::http::mime;

const AUTH_API: &'static str = "https://id.twitch.tv/oauth2/token";

pub struct HelixAuth {
    pub(crate) auth: String,
    pub(crate) client_id: String,
    _expires: u32,
}

impl HelixAuth {
    pub async fn new(cid: String, csec: &str) -> surf::Result<Self> {
        #[derive(Deserialize)]
        struct AuthRes {
            access_token: String,
            expires_in: u32,
        }

        let res: AuthRes = surf::post(AUTH_API)
            .body_string(format!(
                "client_id={}\
                &client_secret={}\
                &grant_type={}",
                cid, csec, "client_credentials"
            )).content_type(mime::FORM)
            .recv_json().await?;
        
        log::debug!("retrieved auth: expires in {}", res.expires_in);

        Ok( Self {
            auth: format!("Bearer {}", res.access_token),
            client_id: cid,
            _expires: res.expires_in,
        } )
    }
}