use once_cell::sync::OnceCell;
use std::{env, fs};

use crate::{filename::Formatter, prelude::*};

static NAME: OnceCell<Box<str>> = OnceCell::new();
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Extractor {
    Internal,
    Streamlink
}

pub enum Tunnel {
    Provided(String),
    Wrapper
    //Run(String)
}

pub struct Argv {
    pub client_id: String,
    pub client_secret: String,
    pub tunnel: Tunnel,
    pub fmt: Formatter,
    pub log_output: String,
    pub log_level: log::LevelFilter,
    pub log_stderr: bool,
    pub server_port: u16,
    pub save_to_dir: bool,
    pub use_extractor: Extractor,
    pub twitch_auth_header: Option<String>,
    pub channels: Vec<(UserCredentials, ChannelSettings)>,
}

#[derive(Clone, Deserialize)]
pub struct ChannelSettings {
    pub format: String,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            format: "best".to_owned(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum UserCredentials {
    Full {
        id: String,
        login: String,
        name: String,
    },
    Id {
        id: String,
    },
    Login {
        login: String,
    },
}

fn info() -> String {
    format!(
        "twitch-archive\n\
        version {}\n\
        Author: riveroon (github.com/riveroon)",
        VERSION
    )
}

fn help() -> String {
    format!(
            "{}
            \nUSAGE: {} [ARGS]\
            \n\
            \nARGS:\
            \n  -C, --client-id      <str>  The client authorization id .\
            \n  -S, --client-secret  <str>  The client authorization secret .\
            \n  -f, --file-name      <str>  Formats the output file name.\
            \n                              See below for more information.\
            \n                              (Default: \"%Sl/[%si] %st\")\
            \n  --log-output         <path> Write log output to file.\
            \n                              When empty, does not log to file.\
            \n                              (Default: `archive.log`)
            \n  --log-level          <str>  Sets the log level threshold for stdout.\
            \n                              Valid levels are:\
            \n                                `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`, `OFF`\
            \n  --log-stderr                Redirects log output to stderr.\
            \n  -P, --server-port    <u16>  The address for the webhook to listen to.\
            \n                              (Default: 8080)\
            \n  -A, --server-addr    <str>  The host address the server will receive requests to.\
            \n                              If not set, a ngrok tunnel will be set up automatically.\
            \n                              (Default: None)
            \n  -d, --sub-data       <path> The location where the subscription list is saved.\
            \n                              The contents should follow a specific json format;\
            \n                              See below for more information.\
            \n                              (Default: `subscriptions.json`)\
            \n  --save-to-dir               Save the output to a directory.\
            \n                              If not set, downloads will be archived to a .tar file.\
            \n  --use-extractor      <str>  Uses the given extractor for extracting m3u8 playlists.\
            \n                              Valid values are:\
            \n                                `internal`, `streamlink`\
            \n  --twitch-auth-header <str>  Authentication header to pass to streamlink for\
            \n                              acquiring stream access tokens.\
            \n                              (Default: \"\")\
            \n  --version                   Prints the program version.\
            \n  -h, --help                  Prints this help message.\
            \n\
            \nSUBSCRIPTION FORMAT:\
            \nThe subscription information file contains the list of streamer ids to subscribe to,\
            \nand optionally the specific download settings for each channels.\
            \n\
            \nchannel <object>\
            \n  'id':         <str>     The streamer id to subscribe to.\
            \n  'login':      <str>     The streamer login to subscribe to.\
            \n  'format':     <str>     The download quality the stream should be downloaded at.\
            \n                          This value should be either 'video' for videos,\
            \n                          or 'audio' for audios. (Default: 'video')\
            \n\
            \nThe subscription list file is a json list of the above channel object.\
            \n\
            \nThe final file contents should look like this:\
            \n  [\
            \n    {{\"id\": \"0000000\"}},\
            \n    {{\"id\": \"0000001\", \"format\": \"audio\"}},\
            \n    {{\"login\": \"twitch\"}},\
            \n  ]\
            \n\
            \nFILE NAME FORMATTING:\
            \nThe file name format is a normal string, with the following placeholders for\
            \nthe variables which are replaced with their respective values.\
            \n\
            \n  %Si: Streamer ID\
            \n  %Sl: Streamer Login\
            \n  %Sn: Streamer Name\
            \n\
            \n  %TY: Stream start year, 4 digits\
            \n  %Ty: Stream start year, 2 digits\
            \n  %TM: Stream start month, 2 digits\
            \n  %TD: Stream start day, 2 digits\
            \n  %TH: Stream start hour, 2 digits, 24-hours, Local time\
            \n  %Tm: Stream start minute, 2 digits, Local time\
            \n  %TZ: Local date timezone\
            \n\
            \n  %si: Stream ID\
            \n  %st: Stream Name\
            \n\
            \n  %%: Escape (\"%\")",
            info(), NAME.get().unwrap()
        )
}

fn print_help() {
    println!("{}", help());
}

fn eprint_err(err: &str) {
    eprintln!("{}\n\nERROR: {}", help(), err);
}

fn type_err(t: &str, x: &str) {
    eprint_err(&format!("<{}> expected after {:?}", t, x));
}

pub fn parse_args() -> Argv {
    let mut argv = env::args();

    let name = argv.next().unwrap();
    NAME.set(name.into()).unwrap();

    let mut client_id = None;
    let mut client_secret = None;
//    let mut ngrok_authtoken = None;
    let mut file_name = "%Sl/[%si] %st".to_owned();
    let mut log_output = "archive.log".to_owned();
    let mut log_level = log::LevelFilter::Info;
    let mut log_stderr = false;
    let mut server_port = 8080;
    let mut server_addr = None;
    let mut sub_data = "subscriptions.json".to_owned();
    let mut save_to_dir = false;
    let mut use_extractor = "internal".to_string();
    let mut twitch_auth_header = None;

    while let Some(x) = argv.next() {
        match x.as_str() {
            "-C" | "--client-id" => {
                client_id = if let Some(x) = argv.next() {
                    Some(x)
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "-S" | "--client-secret" => {
                client_secret = if let Some(x) = argv.next() {
                    Some(x)
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
//            "-N" | "--ngrok-authtoken" => {
//                ngrok_authtoken = if let Some(x) = argv.next() {
//                    Some(x)
//                } else {
//                    type_err("str", &x);
//                    std::process::exit(1);
//                }
//            }
            "--log-output" => {
                log_output = if let Some(x) = argv.next() {
                    x
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "--log-level" => {
                log_level = if let Some(x) = argv.next() {
                    x.parse().expect("unexpected value after --log-level")
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "--log-stderr" => log_stderr = true,
            "-P" | "--server-port" => {
                server_port = if let Some(x) = argv.next().and_then(|x| x.parse().ok()) {
                    x
                } else {
                    type_err("u16", &x);
                    std::process::exit(1);
                }
            }
            "-A" | "--server-addr" => {
                server_addr = if let Some(x) = argv.next() {
                    Some(x)
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "-d" | "--sub-data" => {
                sub_data = if let Some(x) = argv.next() {
                    x
                } else {
                    type_err("path", &x);
                    std::process::exit(1);
                }
            }
            "-f" | "--file-name" => {
                file_name = if let Some(x) = argv.next() {
                    x
                } else {
                    type_err("str", &x);
                    std::process::exit(1)
                }
            }
            "--save-to-dir" => save_to_dir = true,
            "--use-extractor" => {
                use_extractor = if let Some(x) = argv.next() {
                    x
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "--twitch-auth-header" => {
                twitch_auth_header = if let Some(x) = argv.next() {
                    Some(x)
                } else {
                    type_err("str", &x);
                    std::process::exit(1);
                }
            }
            "--version" => {
                println!("{}", VERSION);
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                eprint_err(&format!("Unknown argument {:?}", x));
                std::process::exit(1);
            }
        }
    }

    let Some(client_id) = client_id else {
        eprint_err("client-id missing!");
        std::process::exit(1);
    };
    let Some(client_secret) = client_secret else {
        eprint_err("client-secret missing!");
        std::process::exit(1);
    };
//    let tunnel = match (server_addr, ngrok_authtoken) {
//        (Some(addr), _) => Tunnel::Provided(addr),
//        (None, Some(auth)) => Tunnel::Run(auth),
//        (None, None) => {
//            eprint_err("ngrok-authtoken missing!");
//            std::process::exit(1);
//        }
//    };
    let tunnel = match server_addr {
        Some(addr) => Tunnel::Provided(addr),
        None => Tunnel::Wrapper
    };
    if file_name.is_empty() {
        eprint_err("File names cannot be an empty string!");
        std::process::exit(1);
    };
    let sub = match fs::read(sub_data) {
        Ok(x) => x,
        Err(e) => {
            eprint_err(&format!("sub-data file is missing or corrupt: {e}"));
            std::process::exit(2);
        }
    };
    let use_extractor = match use_extractor.to_lowercase().as_str() {
        "internal" => Extractor::Internal,
        "streamlink" => Extractor::Streamlink,
        x => {
            eprint_err(&format!("unexpected value for `--use_extractor`: {x}"));
            std::process::exit(1);
        }
    };

    #[derive(Deserialize)]
    struct ChannelDes {
        #[serde(flatten)]
        user: UserCredentials,
        #[serde(flatten)]
        channel: Option<ChannelSettings>,
    }

    let channels: Vec<ChannelDes> =
        serde_json::from_slice(&sub).expect("Subscription list data is invalid!");
    log::info!("Retrieved {} subscription target(s)", channels.len());

    Argv {
        client_id,
        client_secret,
        tunnel,
        log_output,
        log_level,
        log_stderr,
        server_port,
        fmt: Formatter::new(&file_name),
        save_to_dir,
        use_extractor,
        twitch_auth_header,
        channels: channels
            .into_iter()
            .map(|c| (c.user, c.channel.unwrap_or_default()))
            .collect(),
    }
}
