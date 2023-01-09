mod eventsub;

use std::sync::{atomic::AtomicBool, Arc};

use eventsub::event::*;

mod helix;
use helix::{HelixAuth, User};

mod args;
use args::*;

mod irc;

mod filename;

use futures::{TryStreamExt, channel::oneshot};
use async_std::{channel::Receiver, path::Path, io::WriteExt};

const CHAT_BUFFER: usize = 16384;

use async_once_cell::OnceCell;
static FORMATTER: OnceCell<filename::Formatter> = OnceCell::new();
static TW_STREAM_AUTH: OnceCell<Box<str>> = OnceCell::new();

use chrono::Local;
async fn chat_log(
    (rx, bool): (Receiver<Box<str>>, Arc<AtomicBool>),
    path: &Path,
    start_time: chrono::DateTime<Local>,
    mut noti: futures::channel::oneshot::Receiver<()>
)-> Result<(), Box<dyn std::error::Error>> {
    use futures::{
        future::{select, Either},
        io::BufWriter
    };
    use async_std::fs;
    use std::sync::atomic::Ordering;

    if let Err(_) = bool.compare_exchange(
        false, true,
        Ordering::Relaxed,
        Ordering::Relaxed
    ) {
        return Err("irc channel was unexpectedly open!".into());
    }

    let mut file = BufWriter::with_capacity(CHAT_BUFFER,
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path).await?
    );
    
    loop {
        let msg = match select(rx.recv(), noti).await {
            Either::Left((msg, next_noti)) => {
                noti = next_noti;
                msg?
            }
            Either::Right(_) => {
                file.flush().await?;
                bool.store(false, Ordering::Relaxed);
                return Ok(())
            },
        };

        log::trace!("received irc message: {}", msg);

        let x = (Local::now() - start_time).num_milliseconds();
        if x < 0 { continue; }

        file.write_all(x.to_string().as_bytes()).await?;
        file.write_all(b": ").await?;
        file.write_all(msg.as_bytes()).await?;
    }
}

async fn cmd(program: &str, args: &[&str]) -> Result<(), ()> {
    use std::time::{UNIX_EPOCH, Duration};
    use async_process::Stdio;

    log::trace!("running command :{} {:?}", program, args);
    let out = async_process::Command::new(&program)
        .args(args)
        .stdout(Stdio::null())
        .output().await
        .map_err(|e| log::error!("error while executing {}: {}", program, e))?;

    if out.status.success() { return Ok(()) };

    let t = UNIX_EPOCH.elapsed().unwrap_or(Duration::ZERO).as_secs();
    let log = format!("[{}] twitch-archiver @ {}.log", t, program);

    log::error!(
        "{}: command exited with status {}: logged at {}",
        program, out.status.code().unwrap_or(-1), &log
    );

    let _ = async_std::fs::write(&log, &out.stderr).await;
    return Err(());
}

async fn download(stream: &helix::Stream, chat: &(Receiver<Box<str>>, Arc<AtomicBool>), chn: &ChannelSettings) -> Result<(), ()> {
    use async_std::{fs, task::spawn};

    let (tx, rx) = oneshot::channel();

    let filename = FORMATTER.get().unwrap().format(stream);
    let path = Path::new(&filename);
    if let Some(x) = path.parent() {
        fs::create_dir_all(x).await
            .map_err(|e| {
                eprintln!("could not create parent directory for download file: {}", e);
            })?;
    };

    let chat_handle = spawn( {
        let mut path = path.to_path_buf();
        let started_at = stream.started_at();
        let user = stream.user().clone();
        let chat = chat.clone();
        async move {
            path.set_extension("log");
            if let Err(e) = chat_log(chat, &path, started_at, rx).await {
                log::error!("error while logging chat for channel {}: {}", user, e);
            };
        }
    });

    log::info!("downloading stream for channel {}: {}", stream.user(), filename);

    if let Some(x) = TW_STREAM_AUTH.get() {
        cmd("streamlink", &[
            "--loglevel", "warning",
            "--twitch-disable-hosting", "--twitch-disable-ads",
            "--twitch-api-header", &**x,
            "-f", "-o", &filename,
            &format!("https://twitch.tv/{}", stream.user().login()),
            &chn.format]
        ).await?;
    } else {
        cmd("streamlink", &[
            "--loglevel", "warning",
            "--twitch-disable-hosting", "--twitch-disable-ads",
            "-f", "-o", &filename,
            &format!("https://twitch.tv/{}", stream.user().login()),
            &chn.format]
        ).await?;
    }
    tx.send(())?;
    chat_handle.await;

    return Ok(());
}

async fn archive(
    auth: HelixAuth,
    port: u16,
    channels: impl IntoIterator<Item = (
        User,
        (Receiver<Box<str>>, Arc<AtomicBool>),
        ChannelSettings
    )>
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

    join_all(channels.into_iter().map(|(user, rx, settings)| {
        let auth = &auth;
        let events = &events;
        async move { loop {
            let id = user.id();

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
                log::debug!("received event for stream #{}", msg.id());

                let stream = match helix::get_streams(auth.clone(),
                    std::iter::once(
                        helix::StreamFilter::User( &msg.user() )
                    )).try_next().await {
                    Ok(Some(x)) => x,
                    Ok(None) => continue,
                    Err(e) => {
                        log::error!("could not fetch stream object from endpoint: {}", e);  
                        continue;
                    }
                };
                log::debug!("fetched stream object for stream #{}", stream.id());

                if download(&stream, &rx, &settings).await.is_err() {
                    log::error!("download failed!");
                }
            }
        }
    } } )).await;
}

async fn run() {
    use futures::future::join_all;

    let (cid, csec, port, fname, twauth, ch) = parse_args();

    let auth = match HelixAuth::new(cid, csec).await {
        Ok(x) => x,
        Err(e) => {
            log::error!("error while obtaining helix authorization:\n\t{:?}", e);
            return;
        }
    };

    FORMATTER.get_or_init(async { fname }).await;

    let mut irc = irc::IrcClientBuilder::new();
    let mut v: Vec<(User, (Receiver<Box<str>>, Arc<AtomicBool>), ChannelSettings)> = Vec::new();
    for (user, settings) in join_all(
        ch.into_iter().map(|(cred, settings)| async {
            let user = match cred {
                UserCredentials::Full {id, login, name} => User::new(id, login, name),
                UserCredentials::Id {id} => match User::from_id(&id, &auth).await {
                    Ok(x) => x,
                    Err(ref e) => {
                        log::error!("could not retrieve user with id {:?}: {}", id, e);
                        return None;
                    }
                },
                UserCredentials::Login {login} => match User::from_login(&login, &auth).await {
                    Ok(x) => x,
                    Err(e) => {
                        log::error!("could not retrieve user with login {:?}: {}", login, e);
                        return None;
                    }
                }
            };
            Some((user, settings))
        })
    ).await.into_iter().flatten() {
        let rx = irc.join(user.login());
        v.push((user, rx, settings));
    }
    irc.build();

    if let Some(x) = twauth { TW_STREAM_AUTH.get_or_init(async {x}).await; }

    eventsub::wipe(&auth).await.expect("error while wiping leftover subscriptions");

    archive(auth, port, v).await;
}

fn main() {
    use futures::executor::block_on;

    env_logger::init();
    block_on(run());
}
