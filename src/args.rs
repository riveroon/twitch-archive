use std::{env, fs};

use serde::Deserialize;

use crate::filename::Formatter;

#[derive(Deserialize)]
pub struct ChannelSettings {
    pub format: String,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self { format: "best".to_owned() }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum UserCredentials {
    Full { id: String, login: String, name: String },
    Id { id: String },
    Login { login: String },
}

pub fn parse_args() -> (String, String, u16, Formatter, Option<Box<str>>, Vec<(UserCredentials, ChannelSettings)>) {
    let mut argv = env::args();
    let name = argv.next().unwrap();
    let print_help = || println!("\
                USAGE: {} [ARGS]\n\
                \n\
                ARGS:\n\
                \r  -C, --client-id      <str>  The client authorization id .\n\
                \r  -S, --client-secret  <str>  The client authorization secret .\n\
                \r  -P, --server-port    <u16>  The address for the webhook to listen to.\n\
                \r                              (Default: 8080)\n\
                \r  -d, --sub-data       <path> The location where the subscription list is saved.\n\
                \r                              The contents should follow a specific json format;\n\
                \r                              See below for more information.\n\
                \r                              (Default: \"subscriptions.json\")\n\
                \r  -f, --file-name      <str>  Formats the output file name.\n\
                \r                              See below for more information.\n\
                \r                              (Default: \"%Sl\\[%si] %st.ts\")
                \r  --twitch-auth-header <str>  Authentication header to pass to streamlink for\n\
                \r                              acquiring stream access tokens.\n\
                \r                              (Default: \"\")\n\
                \r  --version                   Prints the program version.\n\
                \r  -h, --help                  Prints this help message.\n\
                \n\
                SUBSCRIPTION FORMAT:\n\
                The subscription information file contains the list of streamer ids to subscribe to,\n\
                and optionally the specific download settings for each channels.\n\
                \n\
                channel <object>\n\
                \r  'id':         <str>     The streamer id to subscribe to.\n\
                \r  'login':      <str>     The streamer login to subscribe to.\n\
                \r  'format':     <str>     The download quality the stream should be downloaded at.\n\
                \r                          This value should be either 'video' for videos,\n\
                \r                          or 'audio' for audios. (Default: 'video')\n\
                \n\
                The subscription list file is a json list of the above channel object.\n\
                \n\
                The final file contents should look like this:\n\
                \r  [\n\
                \r    {{\"id\": \"0000000\"}},\n\
                \r    {{\"id\": \"0000001\", \"format\": \"audio\"}},\n\
                \r    {{\"login\": \"twitch\"}},
                \r  ]\n\
                \n\
                FILE NAME FORMATTING:\n\
                The file name format is a normal string, with the following placeholders for\n\
                the variables which are replaced with their respective values.\n\
                \n\
                \r  %Si: Streamer ID\n\
                \r  %Sl: Streamer Login\n\
                \r  %Sn: Streamer Name\n\
                \n\
                \r  %TY: Stream start year, 4 digits\n\
                \r  %Ty: Stream start year, 2 digits\n\
                \r  %TM: Stream start month, 2 digits\n\
                \r  %TD: Stream start day, 2 digits\n\
                \r  %TH: Stream start hour, 2 digits, 24-hours, Local time\n\
                \r  %Tm: Stream start minute, 2 digits, Local time\n\
                \r  %TZ: Local date timezone\n\
                \n\
                \r  %si: Stream ID\n\
                \r  %st: Stream Name\n\
                \n\
                \r  %%: Escape (\"%\")\n\
                ", name);

    fn err(t: &str, x: &str) -> String {
        format!("ERROR: <{}> expected after {:?}", t, x)
    }

    let panic_help = |s: &str| {
        eprintln!("ERROR: {}\n", s);
        print_help();
        std::process::exit(-1);
    };

    let mut cid = None;
    let mut csec = None;
    let mut port = 8080;
    let mut fname = "%Sl\\[%si] %st.ts".to_owned();
    let mut twautheadr = "".to_owned();
    let mut subs = fs::read("subscriptions.json").ok();

    while let Some(x) = argv.next() {
        match x.as_str() {
            "-C" | "--client-id" => cid = Some(argv.next().expect(&err("str", &x))),
            "-S" | "--client-secret" => csec = Some(argv.next().expect(&err("str", &x))),
            "-P" | "--server-port" => port = argv.next()
                .and_then(|x| x.parse().ok())
                .expect(&err("u16", &x)),
            "-d" | "--sub-data" => {
                let path = argv.next().expect(&err("path", &x));
                subs = Some(fs::read(&path).expect(&format!("failed to read file {:?}", &path)));
            },
            "-f" | "--file-name" => fname = argv.next().expect(&err("str", &x)),
            "--twitch-auth-header" => twautheadr = argv.next().expect(&err("str", &x)),
            "--version" => {
                println!("\
                twitch-archive v0.3.0-beta\n\
                Author: riveroon (github.com/riveroon)");
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            },
            _ => panic_help(&format!("Unknown argument {:?}", x))
        }
    }

    if cid.is_none() { panic_help("Client-Id missing!") }
    if csec.is_none() { panic_help("Client Secret missing!") }
    if fname.is_empty() { panic_help("File names cannot be an empty string!") }
    if subs.is_none() { panic_help("Subscription data file missing!") }

    #[derive(Deserialize)]
    struct ChannelDes {
        #[serde(flatten)]
        user: UserCredentials,
        #[serde(flatten)]
        channel: Option<ChannelSettings>,
    }

    let channels: Vec<ChannelDes> = serde_json::from_slice(&subs.unwrap())
        .expect("Subscription list data is invalid!");
    log::info!("Retrieved {} subscription target(s)", channels.len());

    return (
        cid.unwrap(),
        csec.unwrap(),
        port,
        Formatter::new(&fname),
        if twautheadr.is_empty() { None } else { Some(twautheadr.into()) },
        channels.into_iter()
            .map(|c| (c.user, c.channel.unwrap_or_default()))
            .collect()
    );
}