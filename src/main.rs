mod eventsub;
use eventsub::event::*;

mod helix;
use helix::{HelixAuth, Stream, User};

mod args;
use args::*;

mod irc;
use irc::IrcRecv;

mod filename;
mod fs_utils;
mod rand;

use async_std::{
    fs,
    io::{self, WriteExt},
    path,
};
use futures::{channel::oneshot, StreamExt, TryStreamExt};

const CHAT_BUFFER: usize = 16384;
const RAND_DIR_LEN: usize = 12;
const ASYNC_BUF_FACTOR: usize = 8;

use async_once_cell::OnceCell;

static FORMATTER: OnceCell<(filename::Formatter, bool)> = OnceCell::new();
static TW_STREAM_AUTH: OnceCell<Box<str>> = OnceCell::new();

async fn datafile(path: &path::Path, stream: &Stream) -> io::Result<()> {
    use chrono::{DateTime, Local, SecondsFormat};
    use serde::Serialize;

    #[derive(Serialize)]
    struct Data<'a> {
        data: StreamSer<'a>,
        segments: Vec<Segments>,
    }

    #[derive(Serialize)]
    struct StreamSer<'a> {
        id: &'a str,
        user: &'a User,
        game: GameDes<'a>,
        title: &'a str,
        started_at: String,
    }

    #[derive(Serialize)]
    struct GameDes<'a> {
        id: &'a str,
        name: &'a str,
    }

    #[derive(Serialize)]
    struct Segments {
        timestamp: String,
    }

    let datapath = path.join("stream.json");
    let mut file = fs::File::create(&datapath).await?;
    let segments: io::Result<Vec<Segments>> = fs::read_dir(&path)
        .await?
        .try_filter_map(|entry| async move {
            if entry
                .file_name()
                .to_str()
                .map(|x| x.starts_with("stream") && x.ends_with(".ts"))
                .unwrap_or(false)
            {
                let time: DateTime<Local> = fs::metadata(&entry.path()).await?.created()?.into();
                let timestamp = time.to_rfc3339_opts(SecondsFormat::AutoSi, true);
                Ok(Some(Segments { timestamp }))
            } else {
                Ok(None)
            }
        })
        .try_collect()
        .await;

    let data = Data {
        data: StreamSer {
            id: stream.id(),
            user: stream.user(),
            game: GameDes {
                id: stream.game_id(),
                name: stream.game_name(),
            },
            title: stream.title(),
            started_at: stream
                .started_at()
                .to_rfc3339_opts(SecondsFormat::AutoSi, true),
        },
        segments: segments?,
    };

    file.write_all(&serde_json::to_vec(&data)?).await?;
    file.sync_all().await
}

async fn chat_log(
    rx: IrcRecv,
    path: &path::Path,
    start_time: chrono::DateTime<chrono::Local>,
    mut noti: futures::channel::oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::{Duration, Local};
    use futures::{
        future::{select, Either},
        io::BufWriter,
    };

    if !rx.open() {
        return Err("irc channel was unexpectedly open!".into());
    }

    let mut file = BufWriter::with_capacity(
        CHAT_BUFFER,
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?,
    );

    loop {
        let msg = match select(rx.recv(), noti).await {
            Either::Left((msg, next_noti)) => {
                noti = next_noti;
                msg?
            }
            Either::Right(_) => {
                file.flush().await?;
                rx.close();
                return Ok(());
            }
        };

        log::trace!("received irc message: {}", msg);

        let ts = Local::now() - start_time;

        if ts < Duration::zero() {
            continue;
        }

        file.write_all(ts.to_string().as_bytes()).await?;
        file.write_all(b": ").await?;
        file.write_all(msg.as_bytes()).await?;
    }
}

async fn cmd(program: &str, args: &[&str], output: bool) -> Result<Option<String>, ()> {
    use async_process::Stdio;
    use std::time::{Duration, UNIX_EPOCH};

    log::trace!("running command :{} {:?}", program, args);
    let out = async_process::Command::new(program)
        .args(args)
        .stdout(if output {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .output()
        .await
        .map_err(|e| log::error!("error while executing {}: {}", program, e))?;

    if out.status.success() {
        if output {
            return String::from_utf8(out.stdout).map(Some).map_err(|e| {
                log::error!("{}: output was not valid utf-8: {}", program, e);
            });
        } else {
            return Ok(None);
        }
    };

    let t = UNIX_EPOCH.elapsed().unwrap_or(Duration::ZERO).as_secs();
    let log = format!("[{}] twitch-archiver @ {}.log", t, program);

    log::error!(
        "{}: command exited with status {}: logged at {}",
        program,
        out.status.code().unwrap_or(-1),
        &log
    );

    let _ = async_std::fs::write(&log, &out.stderr).await;
    Err(())
}

async fn download(stream: &Stream, chat: &IrcRecv, chn: &ChannelSettings) -> Result<(), ()> {
    async fn _stream(path: path::PathBuf, stream: &Stream, format: &str) -> Result<(), ()> {
        let filepath = path.join("stream.ts").to_str().unwrap().to_owned();
        let logpath = path.join("output.log").to_str().unwrap().to_owned();

        log::debug!(
            "download location for stream {}: {}",
            stream.id(),
            path.to_string_lossy()
        );

        let link = format!("https://twitch.tv/{}", stream.user().login());
        let mut args = vec![
            "--twitch-disable-hosting",
            "--twitch-disable-ads",
            "--logfile",
            &logpath,
            "-f",
            "-o",
            &filepath,
            &link,
            format,
        ];

        if let Some(x) = TW_STREAM_AUTH.get() {
            args.insert(0, "--twitch-api-header");
            args.insert(1, x);
        }

        if cmd("streamlink", &args, false).await.is_ok() {
            return Ok(());
        }

        log::warn!("streamlink download failed; falling back to youtube-dl!");

        let filepath = path.join("stream-1.ts").to_str().unwrap().to_owned();

        for f in format.split(',') {
            let mut args = vec!["-o", &filepath, &link];

            if !f.is_empty() {
                args.insert(0, "-f");
                args.insert(1, f);
            }

            if cmd("youtube-dl", &args, false).await.is_ok() {
                return Ok(());
            };
        }

        log::error!(
            "could not download stream {} for channel {}",
            stream.id(),
            stream.user()
        );
        Err(())
    }

    async fn _chat(
        path: path::PathBuf,
        stream: Stream,
        chat: IrcRecv,
        noti: oneshot::Receiver<()>,
    ) -> Result<(), ()> {
        return chat_log(chat, &path.join("chat"), stream.started_at(), noti)
            .await
            .map_err(|e| {
                log::error!(
                    "error while logging chat {} for stream {}: {}",
                    stream.user(),
                    stream.id(),
                    e
                )
            });
    }

    async fn _dl(
        path: path::PathBuf,
        stream: &Stream,
        chat: &IrcRecv,
        chn: &ChannelSettings,
    ) -> Result<(), ()> {
        use async_std::task::spawn;

        let (tx, rx) = oneshot::channel();

        let chat_handle = spawn(_chat(path.clone(), stream.clone(), chat.clone(), rx));
        let res = _stream(path, stream, &chn.format).await;

        tx.send(())?;
        return chat_handle.await.and(res);
    }

    async fn move_dir(orig: &path::Path, dest: &path::Path) -> io::Result<Box<path::Path>> {
        let dir = fs_utils::create_dedup_dir(dest).await?;
        // see async-std issue#1053
        fs::read_dir(&orig)
            .await?
            .map(|entry| async move {
                let entry = entry?;
                fs::rename(entry.path(), dest.join(entry.path().file_name().unwrap())).await
            })
            .buffer_unordered(ASYNC_BUF_FACTOR)
            .try_collect()
            .await?;

        fs::remove_dir_all(orig).await?;

        Ok(dir)
    }

    async fn tar(tarpath: &path::Path, path: &path::Path) -> io::Result<Box<path::Path>> {
        let (tarpath, tarfile) = fs_utils::create_dedup_file(tarpath).await?;
        let mut tar = async_tar::Builder::new(tarfile);

        let mut dir_entry = fs::read_dir(&path)
            .await?
            .map(|entry| async move {
                let entry = entry?;
                return io::Result::Ok((entry.path().is_dir().await, entry));
            })
            .buffer_unordered(ASYNC_BUF_FACTOR);

        while let Some(x) = dir_entry.next().await {
            let (is_dir, entry) = x?;
            if is_dir {
                tar.append_dir_all(&entry.path(), entry.file_name()).await?;
            } else {
                tar.append_path_with_name(&entry.path(), entry.file_name())
                    .await?;
            };
        }
        tar.finish().await?;
        fs::remove_dir_all(path).await?;

        Ok(tarpath)
    }

    let (fmt, to_dir) = FORMATTER.get().unwrap();
    let filename = fmt.format(stream);
    let path = if *to_dir {
        path::Path::new(&filename).to_path_buf()
    } else {
        path::Path::new(&filename).with_extension("tar")
    };

    log::info!(
        "downloading stream #{} for channel {}: {}",
        stream.id(),
        stream.user(),
        filename
    );

    //Create a folder as a temporary download directory
    let dl_path = loop {
        let new_path = path::Path::new(".download").join(rand::rand_hex(RAND_DIR_LEN));
        if fs_utils::create_new_dir(&new_path).await.map_err(|e| {
            log::error!(
                "error while creating temporary directory {:?}: {}",
                new_path.to_string_lossy(),
                e
            )
        })? {
            break new_path;
        }
    };

    let res = _dl(dl_path.clone(), stream, chat, chn).await;

    datafile(&dl_path, stream).await.map_err(|e| {
        log::error!(
            "could not write datafile to {:?}: {}",
            dl_path.to_string_lossy(),
            e
        )
    })?;

    return if *to_dir {
        move_dir(&dl_path, &path)
            .await
            .map(|x| {
                log::info!(
                    "finished downloading stream #{} from {}: {}",
                    stream.id(),
                    stream.user(),
                    x.to_string_lossy()
                )
            })
            .map_err(|e| {
                log::error!(
                    "could not move directory {:?} to {:?}: {}",
                    dl_path.to_string_lossy(),
                    path.to_string_lossy(),
                    e
                )
            })
    } else {
        tar(&path, &dl_path)
            .await
            .map(|x| {
                log::info!(
                    "finished downloading stream #{} from {}: {}",
                    stream.id(),
                    stream.user(),
                    x.to_string_lossy()
                )
            })
            .map_err(|e| {
                log::error!(
                    "could not make tar {:?} from contents of directory {:?}: {}",
                    path.to_string_lossy(),
                    dl_path.to_string_lossy(),
                    e
                )
            })
            .and(res)
    };
}

async fn archive(
    auth: HelixAuth,
    port: u16,
    public_url: &url::Url,
    channels: impl IntoIterator<Item = (User, IrcRecv, ChannelSettings)>,
) {
    use async_std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use futures::future::join_all;

    let events = eventsub::EventSub::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        public_url,
        auth.clone(),
    )
    .await;

    join_all(channels.into_iter().map(|(user, rx, settings)| {
        let auth = &auth;
        let events = &events;
        async move {
            loop {
                let id = user.id();

                let sub = match events
                    .subscribe::<stream::Online>(stream::OnlineCond::from_id(id))
                    .await
                {
                    Ok(x) => x,
                    Err(e) => {
                        log::error!(
                            "could not subscribe to event 'stream.online' for user {}: {}",
                            &id,
                            e
                        );
                        return;
                    }
                };

                log::debug!("subscribed to event 'stream.online' for user {}", id);

                loop {
                    let msg = match sub.recv().await {
                        Ok(Some(x)) => x,
                        Ok(None) => {
                            log::warn!("subscription revoked: {:?}", sub.status());
                            break;
                        }
                        Err(e) => {
                            log::error!(
                                "unexpected error while trying to recieve message from webhook: {}",
                                e
                            );
                            break;
                        }
                    };
                    log::debug!("received event for stream #{}", msg.id());

                    let stream = match helix::get_streams(
                        auth.clone(),
                        std::iter::once(helix::StreamFilter::User(msg.user())),
                    )
                    .try_next()
                    .await
                    {
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
        }
    }))
    .await;
}

async fn run(argv: Argv) {
    use futures::future::join_all;

    let auth = match HelixAuth::new(argv.client_id, argv.client_secret).await {
        Ok(x) => x,
        Err(e) => {
            log::error!("error while obtaining helix authorization:\n\t{:?}", e);
            return;
        }
    };

    FORMATTER
        .get_or_init(async { (argv.fmt, argv.save_to_dir) })
        .await;

    let mut irc = irc::IrcClientBuilder::new();
    let mut v: Vec<(User, IrcRecv, ChannelSettings)> = Vec::new();

    let channels: Vec<Result<(User, ChannelSettings), ()>> =
        join_all(argv.channels.into_iter().map(|(cred, settings)| async {
            let user = match cred {
                UserCredentials::Full { id, login, name } => User::new(id, login, name),
                UserCredentials::Id { id } => User::from_id(&id, &auth)
                    .await
                    .map_err(|e| log::error!("could not retrieve user with id {:?}: {}", id, e))?,
                UserCredentials::Login { login } => {
                    User::from_login(&login, &auth).await.map_err(|e| {
                        log::error!("could not retrieve user with login {:?}: {}", login, e)
                    })?
                }
            };
            Ok((user, settings))
        }))
        .await;

    for (user, settings) in channels.into_iter().flatten() {
        let rx = irc.join(user.login());
        v.push((user, rx, settings));
    }
    irc.build();

    if let Some(x) = argv.twitch_auth_header {
        TW_STREAM_AUTH.get_or_init(async { x.into() }).await;
    }

    eventsub::wipe(&auth)
        .await
        .expect("error while wiping leftover subscriptions");

    if let Some(x) = argv.server_addr {
        let public_url = &x.parse().expect("provided server address is not valid!");
        archive(auth, argv.server_port, public_url, v).await;
    } else {
        let tunnel = ngrok::builder()
            .https()
            .port(argv.server_port)
            .run()
            .unwrap();

        let public_url = tunnel.public_url().unwrap();
        log::info!("ngrok tunnel started at: {}", &public_url);

        archive(auth, argv.server_port, public_url, v).await;
    }
}

fn main() {
    env_logger::init();
    let argv = parse_args();
    futures::executor::block_on(run(argv));
}
