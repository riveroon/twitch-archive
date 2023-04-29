use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use async_recursion::async_recursion;
use async_std::{
    fs,
    io::{self, WriteExt},
    path,
    sync::Arc,
    task,
};
use core::time;
use futures::{channel::oneshot, StreamExt, TryStreamExt};

use args::*;
use eventsub::event::*;
use fs_utils::san;
use helix::{HelixAuth, Stream, User};
use irc::IrcRecv;
use prelude::*;

mod args;
mod eventsub;
mod filename;
mod fs_utils;
mod helix;
mod hls;
mod irc;
mod live;
mod logger;
mod prelude;
mod rand;
mod retry;
//mod tar;

const CHAT_BUFFER: usize = 16384;
const RAND_DIR_LEN: usize = 12;
const ASYNC_BUF_FACTOR: usize = 64;

static FORMATTER: OnceCell<(filename::Formatter, bool)> = OnceCell::new();
static TW_STREAM_AUTH: OnceCell<Box<str>> = OnceCell::new();
static EXTRACTOR: OnceCell<Extractor> = OnceCell::new();

async fn datafile(
    path: &path::Path,
    stream: &Stream,
    stream_data: Option<&hls::StreamData>,
) -> Result<()> {
    use chrono::SecondsFormat;

    #[derive(Serialize)]
    struct Data<'a> {
        version: String,
        data: StreamSer<'a>,
        segments: Vec<Segments<'a>>,
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
    struct Segments<'a> {
        path: String,
        group_id: &'a str,
        name: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_bitrate: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bitrate: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolution: Option<Resolution>,
        #[serde(skip_serializing_if = "Option::is_none")]
        frame_rate: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        codecs: Option<&'a str>,
    }

    #[derive(Serialize)]
    struct Resolution {
        width: u64,
        height: u64,
    }

    let datapath = path.join("info.json");
    let mut file = fs::File::create(&datapath).await?;
    let (segpath, alt, var);
    let segments = if let Some(x) = stream_data {
        (segpath, alt, var) = (&x.0, &x.1, &x.2);
        vec![Segments {
            path: segpath.to_string_lossy().into_owned(),
            group_id: alt.group_id.as_str(),
            name: alt.name.as_str(),
            language: alt.language.as_deref(),
            max_bitrate: var.as_ref().map(|x| x.bandwidth),
            bitrate: var.as_ref().and_then(|x| x.average_bandwidth),
            resolution: var.as_ref().and_then(|x| x.resolution).map(|x| Resolution {
                width: x.width,
                height: x.height,
            }),
            frame_rate: var.as_ref().and_then(|x| x.frame_rate),
            codecs: var.as_ref().and_then(|x| x.codecs.as_deref()),
        }]
    } else {
        vec![]
    };

    let data = Data {
        version: format!("0.1/{}", args::VERSION),
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
        segments,
    };

    file.write_all(&serde_json::to_vec(&data)?).await?;
    file.sync_all().await.map_err(From::from)
}

async fn chat_log(
    rx: IrcRecv,
    path: impl AsRef<path::Path>,
    mut noti: futures::channel::oneshot::Receiver<()>,
) -> Result<()> {
    use futures::{
        future::{select, Either},
        io::BufWriter,
    };

    if !rx.open() {
        return Err(anyhow!("irc channel was unexpectedly open!"));
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

        file.write_all(msg.as_bytes()).await?;
    }
}

async fn cmd(program: &str, args: &[&str], output: bool) -> Result<Option<String>> {
    use async_std::process::Stdio;

    log::trace!("running command :{program} {args:?}");
    let out = async_std::process::Command::new(program)
        .args(args)
        .stdout(if output {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .output()
        .await?;

    if out.status.success() {
        if output {
            return String::from_utf8(out.stdout).map(Some).map_err(From::from);
        } else {
            return Ok(None);
        }
    };

    let (logpath, file) = fs_utils::create_dedup_file(path::Path::new(&format!(
        "{}.{}.log",
        task::current().name().unwrap_or("<unknown>"),
        san(program)
    )))
    .await?;

    log::error!(
        "{program} exited with status {}: logging at {}",
        out.status.code().unwrap_or(-1),
        logpath.display()
    );

    let mut writer = futures::io::BufWriter::new(file);

    writer.write_all("=== STDOUT ===\n".as_bytes()).await?;
    writer.write_all(&out.stdout).await?;
    writer.write_all("\n\n=== STDERR ===\n".as_bytes()).await?;
    writer.write_all(&out.stderr).await?;
    writer.flush().await?;
    Err(anyhow!("program exited abnormally!"))
}

async fn streamlink(login: impl AsRef<str>) -> Result<Option<String>> {
    let link = format!("https://twitch.tv/{}", login.as_ref());
    let mut args = vec!["--stream-url", &link];

    if let Some(x) = TW_STREAM_AUTH.get() {
        args.insert(0, "--twitch-api-header");
        args.insert(1, x);
    }

    cmd("streamlink", &args, true).await
}

async fn download(stream: Stream, chat: IrcRecv, chn: ChannelSettings) -> Result<()> {
    async fn _stream(
        path: path::PathBuf,
        stream: &Stream,
        format: &str,
    ) -> Result<Option<hls::StreamData>> {
        log::debug!("download location: {}", path.display());

        let mut n = 0;
        let url = loop {
            n += 1;
            let url = match EXTRACTOR.get().unwrap() {
                Extractor::Internal => live::get_hls(stream.user().login(), TW_STREAM_AUTH.get().map(AsRef::as_ref)).await,
                Extractor::Streamlink => streamlink(stream.user().login()).await
            }.context("failed to fetch hls playlist url")?;

            if let Some(x) = url {
                break x;
            }

            async_std::task::sleep(time::Duration::from_secs(5)).await;
            if n >= 4 {
                log::error!("could not find m3u8 url!");
                return Err(anyhow!("could not find m3u8 url!"));
            }
        };

        hls::download(url, &path, format.split(',').map(str::trim))
            .await
            .context("failed to download hls playlist")
    }

    async fn _dl(
        path: path::PathBuf,
        stream: &Stream,
        chat: &IrcRecv,
        chn: &ChannelSettings,
    ) -> Result<Option<hls::StreamData>> {
        let (tx, rx) = oneshot::channel();

        let chat_handle = task::Builder::new()
            .name(task::current().name().unwrap_or_default().to_owned())
            .local(chat_log(chat.clone(), path.join("chat.log"), rx))
            .context("failed to download chat")?;
        let res = _stream(path, stream, &chn.format).await;

        tx.send(())
            .or(Err(anyhow!("notification channel dropped before send")))?;
        chat_handle.await.and(res)
    }

    async fn move_dir(orig: &path::Path, dest: &path::Path) -> Result<Box<path::Path>> {
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

    async fn tar(tarpath: &path::Path, path: &path::Path) -> Result<Box<path::Path>> {
        use async_tar::Builder;
        #[async_recursion]
        async fn put_recursive(
            builder: &mut Builder<fs::File>,
            path: &path::Path,
            base: &path::Path,
            uc: &path::Path
        ) -> Result<()> {
            use fs::{DirEntry, FileType};

            async fn entry_check(entry: io::Result<DirEntry>, uc: &path::Path) -> Result<(FileType, DirEntry)> {
                let entry = entry?;
                let canon = entry.path()
                    .canonicalize()
                    .await?;

                if !canon.starts_with(uc) {
                    panic!(
                        "reached unpexpected location while creating tar: {}, bound: {}",
                        canon.display(),
                        uc.display()
                    );
                }

                let file_type = entry.file_type().await?;
                Ok((file_type, entry))
            }

            let mut dir_entry = fs::read_dir(&path)
                .await?
                .map(|entry| entry_check(entry, uc))
                .buffer_unordered(ASYNC_BUF_FACTOR);

            while let Some(x) = dir_entry.next().await {
                let (file_type, entry) = x?;

                if file_type.is_dir() {
                    log::trace!(
                        "appending directory {}: {}",
                        entry.path().display(),
                        entry.file_name().to_string_lossy()
                    );
                    builder.append_dir(base.join(entry.file_name()), &entry.path()).await?;

                    put_recursive(builder, &entry.path(), &base.join(entry.file_name()), uc).await?;
                    let _ = fs::remove_dir(entry.path()).await;
                } else if file_type.is_file() {
                    log::trace!(
                        "appending file {}: {}",
                        entry.path().display(),
                        entry.file_name().to_string_lossy()
                    );
                    builder.append_path_with_name(&entry.path(), base.join(entry.file_name()))
                        .await?;
                    let _ = fs::remove_file(entry.path()).await;
                } else {
                    log::warn!("file {} was not a file or a directory", entry.path().display())
                }
            }

            Ok(())
        }

        let (tarpath, tarfile) = fs_utils::create_dedup_file(tarpath).await?;
        let mut tar = async_tar::Builder::new(tarfile);
        let canon = path.canonicalize().await?;
        
        put_recursive(&mut tar, path, path::Path::new(""), &canon).await?;

        tar.finish().await?;
        fs::remove_dir_all(path).await?;

        Ok(tarpath)
    }

    let (fmt, to_dir) = FORMATTER.get().unwrap();
    let filename = fmt.format(&stream);
    let path = if *to_dir {
        path::Path::new(&filename).to_path_buf()
    } else {
        path::Path::new(&filename).with_extension("tar")
    };

    log::info!(
        "downloading stream #{} for channel {}",
        stream.id(),
        stream.user()
    );

    //Create a folder as a temporary download directory
    let dl_path = loop {
        let new_path = path::Path::new(".download").join(rand::rand_hex(RAND_DIR_LEN));
        if fs_utils::create_new_dir(&new_path)
            .await
            .context("cannot create temporary directory")?
        {
            break new_path;
        }
    };

    let res = match _dl(dl_path.clone(), &stream, &chat, &chn).await {
        Ok(Some(x)) => Ok(x),
        Ok(None) => {
            return fs::remove_dir_all(&dl_path)
                .await
                .context("failed to clean up download directory")
        }
        Err(e) => Err(e),
    };

    datafile(&dl_path, &stream, res.as_ref().ok())
        .await
        .context("could not write datafile")?;

    return if *to_dir {
        move_dir(&dl_path, &path)
            .await
            .map(|x| log::info!("finished downloading: {}", x.display()))
            .context("could not move directory")
    } else {
        res.and(
            tar(&path, &dl_path)
                .await
                .map(|x| log::info!("finished downloading: {}", x.display()))
                .context("could not make tar archive"),
        )
    };
}

async fn listen(
    auth: HelixAuth,
    events: Arc<eventsub::EventSub>,
    user: User,
    rx: IrcRecv,
    settings: ChannelSettings,
) {
    loop {
        let sub = match events
            .subscribe::<stream::Online>(stream::OnlineCond::from_id(user.id()))
            .await
        {
            Ok(x) => x,
            Err(e) => {
                log::error!("could not subscribe to event 'stream.online': {e:?}");
                return;
            }
        };

        log::debug!("subscribed to event `stream.online`");

        'listen: loop {
            let msg = match sub.recv().await {
                Ok(Some(x)) => x,
                Ok(None) => {
                    log::warn!("subscription revoked: {:?}", sub.status());
                    break;
                }
                Err(e) => {
                    log::error!(
                        "unexpected error while trying to recieve message from webhook: {e:?}"
                    );
                    break;
                }
            };
            log::debug!("received event for stream #{}", msg.id());

            let stream = {
                let mut count = 0;
                'get_streams: loop {
                    match helix::get_streams(
                        auth.clone(),
                        std::iter::once(helix::StreamFilter::User(msg.user())),
                    )
                    .try_next()
                    .await
                    {
                        Ok(Some(x)) => break x,
                        Ok(None) => {
                            // Due to caching, the Get Streams api may not return a value even though if the stream is online.
                            // Therefore, the current (temporary) solution is to poll the api until it returns something.
                            // This will change in the future since retrieving and downloading the stream works regardless of
                            // this api returning nothing.
                            // An alternative (better!) fix is to use the Get Channel Information
                            // (https://dev.twitch.tv/docs/api/reference/#get-channel-information) to fill out the missing values
                            // and use that instead.
                            // The current wait limit is set to 2 minutes, which seems to be about the timepoint when the api
                            // reliabely returns something.
                            log::warn!("streams matching criteria not found; expected stream #{} ({count})", msg.id());
                            count += 1;
                            if count >= 12 {
                                continue 'listen;
                            }
                            task::sleep(time::Duration::from_secs(10)).await;
                            continue 'get_streams;
                        }
                        Err(e) => {
                            log::error!("could not fetch stream object from endpoint: {e:?}");
                            continue 'listen;
                        }
                    };
                }
            };
            log::debug!("fetched stream object for stream #{}", stream.id());

            let task = match task::Builder::new()
                .name(format!("#{}", stream.id()))
                .spawn(download(stream, rx.clone(), settings.clone()))
            {
                Ok(x) => x,
                Err(e) => {
                    log::error!("failed to spawn task: {e:?}");
                    continue;
                }
            };

            if let Err(e) = task.await {
                log::error!("download failed: {e:?}");
            }
        }
    }
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
    );
    let shared = Arc::new(events);

    async_std::task::yield_now().await;
    join_all(channels.into_iter().map(|(user, rx, settings)| {
        task::Builder::new()
            .name(format!("user-{}", user.id()))
            .local(listen(
                auth.clone(),
                Arc::clone(&shared),
                user,
                rx,
                settings,
            ))
            .unwrap()
    }))
    .await;
}

async fn run(argv: Argv) {
    use futures::future::join_all;

    logger::init(argv.log_output, argv.log_level, argv.log_stderr);

    log::info!("twitch-archive version {} © 2023. riveroon", args::VERSION);

    let auth = match HelixAuth::new(argv.client_id, argv.client_secret).await {
        Ok(x) => x,
        Err(e) => {
            log::error!("error while obtaining helix authorization:\n\t{e:?}");
            return;
        }
    };

    FORMATTER.set((argv.fmt, argv.save_to_dir));

    EXTRACTOR.set(argv.use_extractor);

    let mut irc = irc::IrcClientBuilder::new();
    let mut v: Vec<(User, IrcRecv, ChannelSettings)> = Vec::new();

    let channels: Vec<Result<(User, ChannelSettings), ()>> =
        join_all(argv.channels.into_iter().map(|(cred, settings)| async {
            let user = match cred {
                UserCredentials::Full { id, login, name } => User::new(id, login, name),
                UserCredentials::Id { id } => User::from_id(&id, &auth)
                    .await
                    .map_err(|e| log::error!("could not retrieve user with id {id:?}: {e:?}"))?,
                UserCredentials::Login { login } => {
                    User::from_login(&login, &auth).await.map_err(|e| {
                        log::error!("could not retrieve user with login {login:?}: {e:?}")
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
        TW_STREAM_AUTH.set(x.into());
    }

    eventsub::wipe(&auth)
        .await
        .expect("error while wiping leftover subscriptions");

    match argv.tunnel {
        Tunnel::Provided(addr) => {
            let public_url = addr.parse().expect("provided server address is not valid!");
            archive(auth, argv.server_port, &public_url, v).await;
        }
        Tunnel::Wrapper => {
            let tunnel = ngrok::builder()
                .https()
                .port(argv.server_port)
                .run()
                .await
                .unwrap();

            let public_url = tunnel.public_url().await.unwrap();
            log::info!("ngrok tunnel started at: {public_url}");

            archive(auth, argv.server_port, public_url, v).await;
        }
        // Using ngrok-rs failed b/c a tunnel established with ngrok-rs
        // doesn't return the response for the first unknown requests
        /*
        Tunnel::Run(auth) => {
            use ngrok::prelude::*;

            let forward_to = format!("localhost:{}", argv.server_port);
            let mut tunnel = ngrok::Session::builder()
                .authtoken(auth)
                .connect()
                .await
                .unwrap()
                .http_endpoint()
                .forwards_to(&forward_to)
                .listen()
                .await
                .unwrap();

            let public_url = tunnel.url().parse().unwrap();

            async_std::task::spawn( async move {
                log::info!("servicing tunnel");
                tunnel.forward_tcp(forward_to).await
            });
            async_std::task::yield_now().await;

            log::info!("ngrok tunnel started at: {public_url}");
            public_url
            
        }
        */
    };
    log::info!("shutting down...");
}

fn main() {
    let argv = parse_args();
    async_std::task::block_on(run(argv));
}
