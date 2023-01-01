mod eventsub;
use eventsub::event::*;

mod helix;
use helix::{HelixAuth, User};

mod args;
use args::*;

mod filename;

use futures::TryStreamExt;

use async_once_cell::OnceCell;
static FORMATTER: OnceCell<filename::Formatter> = OnceCell::new();

async fn cmd(program: &str, args: &[&str]) -> Result<String, ()> {
    use std::time::{UNIX_EPOCH, Duration};

    log::trace!("running command :{} {:?}", program, args);
    let cmd = async_process::Command::new(&program)
        .args(args)
        .output().await.unwrap();

    if cmd.status.success() { return Ok(String::from_utf8(cmd.stdout).unwrap()); }

    let t = UNIX_EPOCH.elapsed().unwrap_or(Duration::ZERO).as_secs();
    let (outl, errl) = (format!("ta-{}-0.log", t), format!("ta-{}-1.log", t));

    let l0 = async_std::fs::write(&outl, &cmd.stdout);
    let l1 = async_std::fs::write(&errl, &cmd.stderr);
    futures::try_join!(l0, l1).unwrap();

    log::error!(
        "{}: command exited with status {:?}\n\
        \tlog files: {}, {}",
        program, cmd.status.code().unwrap_or(-1), outl, errl
    );
    return Err(());
}

async fn download(stream: &helix::Stream, chn: &ChannelSettings) -> Result<(), ()> {
    use async_std::{fs, path};

    let filename = FORMATTER.get().unwrap().format(stream);
    if let Some(x) = path::Path::new(&filename).parent() {
        fs::create_dir_all(x).await
            .map_err(|e| {
                eprintln!("could not create parent directory for download file: {}", e);
            })?;
    }
    log::debug!("downloading stream for channel {}: {}", stream.user(), filename);

    let url = cmd("youtube-dl",
        &["-g", "-f", &chn.format, &format!("https://twitch.tv/{}", stream.user().login())]).await?;

    cmd("ffmpeg", &["-hide_banner", "-n", "-i", &url, "-c", "copy", &filename]).await?;
    return Ok(());
}

async fn archive(
    auth: HelixAuth,
    port: u16,
    channels: impl IntoIterator<Item = (UserCredentials, ChannelSettings)>
) {
    use futures::future::join_all;
    use async_std::net::{SocketAddr, IpAddr, Ipv4Addr};

    let tunnel = ngrok::builder()
          .https()
          .port(port)
          .run().unwrap();

    let public_url = tunnel.public_url().unwrap();
    log::info!("ngrok tunnel started at: {}", &public_url);

    let events = eventsub::EventSub::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port), 
        public_url, auth.clone()
    ).await;

    join_all(channels.into_iter().map(|(cred, settings)| {
        let auth = &auth;
        let events = &events;
        async move { loop {
            let tmp;
            let id = match cred {
                UserCredentials::Full { ref id, ..} => id.as_str(),
                UserCredentials::Id { ref id } => id.as_str(),
                UserCredentials::Login { ref login } => {
                    tmp = User::from_login(login, auth).await;
                    match tmp {
                        Ok(ref x) => x.id(),
                        Err(ref e) => {
                            log::error!("could not retrieve user with login {:?}: {}", login, e);
                            return;
                        }
                    }
                }
            };

            let sub = match events.subscribe::<stream::Online> (stream::OnlineCond::from_id(id)).await {
                Ok(x) => {
                    log::debug!("subscribed to event 'stream.online' for user {}", id);
                    x
                },
                Err(e) => {
                    log::error!("could not subscribe to event 'stream.online' for user {}: {}", &id, e);
                    return;
                }
            };

            loop {
                let msg = match sub.recv().await {
                    Ok(Some(x)) => x,
                    Ok(None) => {
                        log::warn!("subscription revoked: {:?}", sub.status());
                        break;
                    },
                    Err(e) => {
                        log::error!("unexpected error while trying to recieve message from webhook: {}", e);
                        break;
                    }
                };

                let stream = match helix::get_streams(auth.clone(), std::iter::once(
                    helix::StreamFilter::User( &msg.user() )
                )).try_next().await {
                    Ok(Some(x)) => x,
                    Ok(None) => continue,
                    Err(e) => {
                        log::error!("could not fetch stream object from endpoint: {}", e);
                        continue;
                    }
                };

                if download(&stream, &settings).await.is_err() {
                    log::error!("download failed!");
                }
            }
        }
    } } )).await;
}

async fn run() {
    let (cid, csec, port, fname, ch) = parse_args();

    let auth = match HelixAuth::new(cid, csec).await {
        Ok(x) => x,
        Err(e) => {
            log::error!("error while obtaining helix authorization:\n\t{:?}", e);
            return;
        }
    };

    FORMATTER.get_or_init(async { fname }).await;

    eventsub::wipe(&auth).await.expect("error while wiping leftover subscriptions");

    archive(auth, port, ch).await;
}

fn main() {
    use futures::executor::block_on;

    env_logger::init();
    block_on(run());
}
