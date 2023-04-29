use async_std::channel;
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use twitchchat::AsyncRunner;

use crate::prelude::*;

const CHANNEL_BOUND: usize = 16;

macro_rules! try_send {
    ($map:expr, $msg:expr) => {
        if let Some(tx) = $map.get($msg.channel()) {
            if let Err(e) = tx.try_send($msg.raw().into()) {
                log::warn!(
                    "failed to send IRC to matching handler {}: {e:?}",
                    $msg.channel()
                );
            }
        } else {
            log::warn!(
                "received IRC message for unknown channel {}",
                $msg.channel()
            );
        }
    };
    ($map:expr, $chname:expr, $msg:expr) => {
        if let Some(tx) = $map.get($chname) {
            if let Err(e) = tx.try_send($msg.into()) {
                log::warn!("failed to send IRC to matching handler {}: {e:?}", $chname);
            }
        } else {
            log::warn!("received IRC message for unknown channel {}", $chname);
        }
    };
}

pub struct IrcClientBuilder {
    map: HashMap<Box<str>, IrcSend>,
}

impl IrcClientBuilder {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    //TODO: accepting channels at creation means that joining afterwords is impossible;
    // Change to a custom TLS stream impl to handle this!
    pub fn join(&mut self, channel: &str) -> IrcRecv {
        let (tx, rx) = channel::bounded(CHANNEL_BOUND);
        let is_open = Arc::new(AtomicBool::new(false));
        self.map.insert(
            format!("#{}", channel).into(),
            IrcSend {
                tx,
                is_open: is_open.clone(),
            },
        );
        IrcRecv { rx, is_open }
    }

    pub fn build(self) {
        use async_std::task;

        log::debug!("spawning IRC handler");
        task::Builder::new()
            .name("irc".to_owned())
            .spawn(async move {
                use core::time::Duration;
                use twitchchat::{messages::Commands, Status};

                async fn _connect() -> Result<AsyncRunner, twitchchat::runner::Error> {
                    use twitchchat::{
                        connector::async_std::ConnectorTls, twitch::Capability, UserConfig,
                    };

                    let conn = ConnectorTls::twitch()?;
                    let config = UserConfig::builder()
                        .anonymous()
                        .capabilities(&[Capability::Tags])
                        .build()
                        .unwrap();

                    let runner = AsyncRunner::connect(conn, &config).await?;
                    log::info!("connected to the IRC server");
                    log::trace!("IRC identity: {:?}", runner.identity);

                    Ok(runner)
                }

                async fn _handle(
                    mut runner: AsyncRunner,
                    map: &HashMap<Box<str>, IrcSend>,
                ) -> Result<(), twitchchat::runner::Error> {
                    loop {
                        let msg = runner.next_message().await?;

                        // I could probably make a macro for this... but I'm laaaaazy :P
                        match msg {
                            x @ (Status::Quit | Status::Eof) => {
                                log::info!("Received signal {x:?}");
                                return Ok(());
                            }
                            Status::Message(Commands::Raw(raw)) => {
                                log::trace!("Recieved raw IRC message: {}", raw.get_raw());
                                continue;
                            }
                            Status::Message(Commands::ClearChat(x)) => try_send!(map, x),
                            Status::Message(Commands::ClearMsg(x)) => try_send!(map, x),
                            Status::Message(Commands::HostTarget(x)) => try_send!(map, x.source(), x.raw()),
                            Status::Message(Commands::Join(x)) => try_send!(map, x),
                            Status::Message(Commands::Notice(x)) => try_send!(map, x),
                            Status::Message(Commands::Part(x)) => try_send!(map, x),
                            Status::Message(Commands::Privmsg(x)) => try_send!(map, x),
                            Status::Message(Commands::RoomState(x)) => try_send!(map, x),
                            Status::Message(Commands::UserNotice(x)) => try_send!(map, x),
                            Status::Message(Commands::UserState(x)) => try_send!(map, x),
                            Status::Message(x) => log::trace!("received IRC command: {x:?}"),
                        };
                    }
                }

                let map = self.map;
                let mut try_count: u8 = 0;
                while try_count <= 10 {
                    match _connect().await {
                        Ok(mut runner) => {
                            try_count = 0;

                            for channel in map.keys() {
                                if let Err(e) = runner.join(&(**channel)[1..]).await {
                                    log::warn!("error while joining channel {channel}: {e:?}");
                                }
                            }

                            log::trace!("irc map: {map:?}");

                            if let Err(e) = _handle(runner, &map).await {
                                log::error!("error while listening to irc: {e:?}");
                            }
                        }
                        Err(e) => {
                            if try_count < 10 {
                                log::warn!("cannot connect to irc; retrying ({try_count}): {e}");
                            } else {
                                log::error!("cannot connect to irc; aborting: {e}");
                                std::process::exit(1);
                            }
                        }
                    }

                    async_std::task::sleep(Duration::from_secs(10)).await;
                    try_count += 1;
                    continue;
                }
            })
            .expect("cannot spawn task");
    }
}

#[derive(Clone, Debug)]
pub struct IrcRecv {
    rx: channel::Receiver<Box<str>>,
    is_open: Arc<AtomicBool>,
}

impl IrcRecv {
    pub fn open(&self) -> bool {
        use std::sync::atomic::Ordering;

        self.is_open
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    pub fn close(&self) -> bool {
        use std::sync::atomic::Ordering;

        self.is_open
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    pub fn recv(&self) -> async_std::channel::Recv<'_, Box<str>> {
        self.rx.recv()
    }
}

#[derive(Clone, Debug)]
pub struct IrcSend {
    tx: channel::Sender<Box<str>>,
    is_open: Arc<AtomicBool>,
}

impl IrcSend {
    pub fn is_open(&self) -> bool {
        use std::sync::atomic::Ordering;

        self.is_open.load(Ordering::Relaxed)
    }

    pub fn try_send(&self, msg: Box<str>) -> Result<bool, channel::TrySendError<Box<str>>> {
        if !self.is_open() {
            return Ok(false);
        }

        self.tx.try_send(msg)?;
        Ok(true)
    }
}
