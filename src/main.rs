mod eventsub;
pub use eventsub::*;

mod subscription;
pub use subscription::Subscription;

mod helix;
pub use helix::HelixAuth;

mod args;
pub use args::*;

use std::{fmt, time::{UNIX_EPOCH, Duration}, net::SocketAddr};
use futures::{try_join, executor::block_on, future::join_all, StreamExt};

use async_process::Command;

use serde::Deserialize;
#[derive(fmt::Debug, Deserialize)]
struct Online {
    id: String,
    #[serde(rename = "broadcaster_user_id")]
    streamer_id: String,
    #[serde(rename = "broadcaster_user_login")]
    streamer_login: String,
    started_at: String
}

async fn cmd(program: &str, args: &[&str]) -> Result<String, ()> {
    log::trace!("running command :{} {:?}", program, args);
    let cmd = Command::new(&program)
        .args(args)
        .output().await.unwrap();

    if cmd.status.success() { return Ok(String::from_utf8(cmd.stdout).unwrap()); }

    let t = UNIX_EPOCH.elapsed().unwrap_or(Duration::ZERO).as_secs();
    let outlog = format!("ta-{}-0.log", t);
    let errlog = format!("ta-{}-1.log", t);

    let l0 = async_std::fs::write(&outlog, &cmd.stdout);
    let l1 = async_std::fs::write(&errlog, &cmd.stderr);
    try_join!(l0, l1).unwrap();

    log::error!(
        "{}: command exited with status {:?}\n\
        \tlog files: {}, {}",
        program, cmd.status.code(), outlog, errlog
    );
    return Err(());
}

async fn download(event: &Online, chn: &Channel) -> Result<(), ()> {
    fn pretty_date(date: &str) -> String {
        date[2..].split_once(':')
            .expect(&format!("unknown date string {}", date)).0
            .replace("-", "")
    }

    log::debug!("downloading stream for channel {}", event.streamer_id);

    let url = cmd("youtube-dl",
        &["-g", "-f", &chn.format, &format!("https://twitch.tv/{}", &event.streamer_login)]).await?;

    let filename = format!(
        "file:{}-{}-{}.ts",
        &event.streamer_login,
        &pretty_date(&event.started_at),
        &event.id
    );

    cmd("ffmpeg", &["-hide_banner", "-n", "-i", &url, "-c", "copy", &filename]).await?;
    return Ok(());
}

use std::collections::HashMap;
async fn archive(
    auth: HelixAuth,
    addr: SocketAddr,
    channels: HashMap<Id, Channel>
) {
    let tunnel = ngrok::builder()
          .https()
          .port(8080)
          .run().unwrap();

    let public_url = tunnel.public_url().unwrap();
    log::info!("ngrok tunnel started at: {}", &public_url);

    let (events, rx) = EventSub::new(addr, public_url, auth).await;
    join_all(channels.keys().map(|x| {
        let events = &events;
        async move {
            if let Err(e) = events.online(x).await {
                log::error!(
                    "error while subscribing to event 'stream.online' for channel {}:\n\t{:?}",
                    x, e
                );
            } else { log::debug!("subscribed to event 'stream.online' for channel {}", x) }
        }
    })).await;

    log::debug!("listening for events...");
    rx.for_each_concurrent(None, |(event, _)| async {
        match event {
            Message::Notification(noti) => {
                match serde_json::from_str::<Online> (noti.get()) {
                    Ok(o) => {
                        if let Some(x) = channels.get(&o.streamer_id) {
                            if download(&o, x).await.is_err() {
                                log::error!("download failed\n\t{:?}", o);
                            }
                        } else {
                            log::warn!("received unknown 'stream.online' event for streamer {:?}", o.streamer_id);
                        }
                    },
                    Err(e) => {
                        log::error!("error while parsing event: {:?}", e);
                        return;
                    }
                }
            }
            Message::Revocation => ()
        };
    }).await;
}

async fn run() {
    let (cid, csec, addr, ch) = parse_args();

    let auth = match HelixAuth::new(cid, &csec).await {
        Ok(x) => x,
        Err(e) => {
            log::error!("error while obtaining helix authorization:\n\t{:?}", e);
            return;
        }
    };

    eventsub::wipe(&auth).await.expect("error while wiping leftover subscriptions");

    archive(auth, addr, ch).await;
}

fn main() {
    env_logger::init();
    block_on(run());
}
