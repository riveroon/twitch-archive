use std::{collections::HashMap, sync::{atomic::AtomicBool, Arc}};

use twitchchat::AsyncRunner;
use async_std::channel::{Sender, Receiver};

const CHANNEL_BOUND: usize = 4;

pub struct IrcClientBuilder {
    map: HashMap<Box<str>, (Sender<Box<str>>, Arc<AtomicBool>)>
}

impl IrcClientBuilder {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    //TODO: accepting channels at creation means that joining afterwords is impossible;
    // Change to a custom TLS stream impl to handle this!
    pub fn join(&mut self, channel: &str) -> (Receiver<Box<str>>, Arc<AtomicBool>) {
        use async_std::channel::bounded;

        let (tx, rx) = bounded(CHANNEL_BOUND);
        let bool = Arc::new(AtomicBool::new(false));
        self.map.insert(format!("#{}", channel).into(), (tx, bool.clone()));
        return (rx, bool);
    }

    pub fn build(self) {
        use async_std::task::spawn;

        log::debug!("spawning irc handler");
        spawn( async move {
            use twitchchat::{Status, messages::Commands};
            use core::time::Duration;

            async fn _connect() -> Result<AsyncRunner, twitchchat::runner::Error> {
                use twitchchat::{
                    UserConfig,
                    twitch::Capability,
                    connector::async_std::ConnectorTls
                };
        
                let conn = ConnectorTls::twitch()?;
                let config = UserConfig::builder()
                    .anonymous()
                    .capabilities(&[Capability::Tags, ])
                    .build()
                    .unwrap();
        
                let runner = AsyncRunner::connect(conn, &config).await?;
                log::info!("Connected to the Twitch IRC server");
                log::trace!("IRC Identity: {:?}", runner.identity);
        
                return Ok(runner);
            }

            async fn _handle(mut runner: AsyncRunner, map: &HashMap<Box<str>, (Sender<Box<str>>, Arc<AtomicBool>)>) -> Result<(), twitchchat::runner::Error> {
                use std::sync::atomic::Ordering;

                loop {
                    let msg = runner.next_message().await?;

                    // I could probably make a macro for this... but I'm laaaaazy :P
                    match msg {
                        x @ ( Status::Quit | Status::Eof ) => {
                            log::info!("Received signal {:?}; shuttind down irc!", x);
                            return Ok(());
                        }
                        Status::Message(Commands::Raw(raw)) => {
                            log::trace!("Recieved raw IRC message: {}", raw.get_raw());
                            continue;
                        }
                        Status::Message(Commands::ClearChat(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::ClearMsg(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::HostTarget(x)) => {
                            if let Some((tx, bool)) = map.get(x.source()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.source(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.source()) }
                        }
                        Status::Message(Commands::Join(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::Notice(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::Part(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::Privmsg(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::RoomState(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::UserNotice(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
                        }
                        Status::Message(Commands::UserState(x)) => {
                            if let Some((tx, bool)) = map.get(x.channel()) {
                                if !bool.load(Ordering::Relaxed) { continue; }
                                if let Err(e) = tx.try_send(x.raw().into()) {
                                    log::warn!("failed to send IRC to matching handler {}: {}", x.channel(), e);
                                }
                            } else { log::warn!("received IRC message for unknown channel {}", x.channel()) }
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
                    } else { return; }
                }
                
                async_std::task::sleep(Duration::from_secs(60)).await;
                try_count += 1;
                continue;
            }
        });
    }
}