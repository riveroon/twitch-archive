use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};

use async_std::channel;
use twitchchat::AsyncRunner;

const CHANNEL_BOUND: usize = 16;

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
        use async_std::task::spawn;

        log::debug!("spawning IRC handler");
        spawn(async move {
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
                            log::info!("Received signal {:?}; shuttind down irc!", x);
                            return Ok(());
                        }
                        Status::Message(Commands::Raw(raw)) => {
                            log::trace!("Recieved raw IRC message: {}", raw.get_raw());
                            continue;
                        }
                        Status::Message(Commands::ClearChat(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::ClearMsg(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::HostTarget(x)) => {
                            if let Some(tx) = map.get(x.source()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.source(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.source()
                                )
                            }
                        }
                        Status::Message(Commands::Join(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::Notice(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::Part(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::Privmsg(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::RoomState(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::UserNotice(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(Commands::UserState(x)) => {
                            if let Some(tx) = map.get(x.channel()) {
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!(
                                        "failed to send IRC to matching handler {}: {}",
                                        x.channel(),
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "received IRC message for unknown channel {}",
                                    x.channel()
                                )
                            }
                        }
                        Status::Message(x) => {
                            log::trace!("received IRC command: {:?}", x);
                        }
                    };
                }
            }

            let map = self.map;
            let mut try_count: u8 = 0;
            while try_count <= 10 {
                if let Ok(mut runner) = _connect().await {
                    try_count = 0;

                    for channel in map.keys() {
                        if let Err(e) = runner.join(&(**channel)[1..]).await {
                            log::warn!("error while joining channel {}: {}", channel, e);
                        }
                    }

                    log::trace!("irc map: {:?}", map);

                    if let Err(e) = _handle(runner, &map).await {
                        log::error!("error while listening to irc: {}", e);
                    } else {
                        return;
                    }
                }

                async_std::task::sleep(Duration::from_secs(60)).await;
                try_count += 1;
                continue;
            }
        });
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
