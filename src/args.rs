use std::collections::HashMap;
use std::fs;
use std::env;
use std::net::{SocketAddr, IpAddr, Ipv4Addr};

use serde::Deserialize;

pub type Id = String;

#[derive(Deserialize)]
pub struct Channel {
    pub format: String,
}

impl Default for Channel {
    fn default() -> Self {
        Self { format: "best".to_owned() }
    }
}

pub fn parse_args() -> (String, String, SocketAddr, HashMap<Id, Channel>) {
    let mut argv = env::args();
    let name = argv.next().unwrap();
    let print_help = || println!("\
                USAGE: {} [ARGS]\n\
                \n\
                ARGS:\n\
                \r  -C, --client-id      <str>  Sets the client id authorization.\n\
                \r  -S, --client-secret  <str>  Sets the client secret authorization.\n\
                \r  -P, --server-port    <u16>  Sets the port for the webhook to listen to.\n\
                \r                              (Default: 8080)
                \r  -D, --sub-data       <path> The location where the subscription list is saved.\n\
                \r                              The contents should follow a specific json format;\n\
                \r                              See below for more information.\n\
                \r                              (Default: subscriptions.json)\n\
                \r  -h, --help                  Prints this help message.\n\
                \n\
                SUBSCRIPTION FORMAT:\n\
                The subscription information file contains the list of streamer ids to subscribe to,\n\
                and optionally the specific download settings for each channels.\n\
                \n\
                channel <object>\n\
                \r  'id':         <str>     The streamer id to subscribe to.\n\
                \r  'format':     <str>     The download quality the stream should be downloaded at.\n\
                \r                          This value should be either 'video' for videos,\n\
                \r                          or 'audio' for audios. (Default: 'video')\n\
                \r  'transcode':  <object>  The quality that the video should be transcoded to;\n\
                \r                          This value will be ignored if the 'format' value\n\
                \r                          is set to 'audio'. (Default: {{}})\n\
                \r                          NOTE: This feature is currently unimplemented.\n\
                \r                          All values will be ignored.\n\
                \n\
                The subscription list file is a json list of the above channel object.\n\
                \n\
                The final file contents should look like this:\n\
                \r  [\n\
                \r    {{\"id\": \"0000000\"}},\n\
                \r    {{\"id\": \"0000001\", \"format\": \"audio\"}},\n\
                \r  ]\n\
                ", name);

    fn err(t: &str, x: &str) -> String {
        format!("ERROR: <{}> expected after {:?}", t, x)
    }

    let panic_help = |s: &str| {
        println!("{}\n", s);
        print_help();
        std::process::exit(-1);
    };

    let mut cid = None;
    let mut csec = None;
    let mut addr = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST), 8080
    );
    let mut subs = fs::read("subscriptions.json").ok();

    while let Some(x) = argv.next() {
        match x.as_str() {
            "-C" | "--client-id" => cid = Some(argv.next().expect(&err("str", &x))),
            "-S" | "--client-secret" => csec = Some(argv.next().expect(&err("str", &x))),
            "-P" | "--server-port" => addr = SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST), 
                argv.next().and_then(|x| x.parse().ok())
                    .expect(&err("u16", &x))
                ),
            "-D" | "--sub-data" => {
                let path = argv.next().expect(&err("path", &x));
                subs = Some(fs::read(&path).expect(&format!("failed to read file {:?}", &path)));
            },
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            },
            _ => panic_help(&format!("Unknown argument {:?}", x))
        }
    }

    if cid.is_none() { panic_help("Client-Id missing!") }
    if csec.is_none() { panic_help("Client Secret missing!") }
    if subs.is_none() { panic_help("Subscription data file missing!") }

    #[derive(Deserialize)]
    struct ChannelDes {
        id: String,
        #[serde(flatten)]
        channel: Option<Channel>,
    }

    let channels: Vec<ChannelDes> = serde_json::from_slice(&subs.unwrap()).expect("Subscription list data is invalid!");
    log::debug!("Retrieved {} subscription targets", channels.len());

    let mut map = HashMap::new();

    for c in channels {
        map.insert(c.id, c.channel.unwrap_or_default());
    }

    return (cid.unwrap(), csec.unwrap(), addr, map);
}