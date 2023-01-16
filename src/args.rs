use std::{env, fs};

use async_once_cell::OnceCell;
use serde::Deserialize;

use crate::filename::Formatter;

static NAME: OnceCell<Box<str>> = OnceCell::new();
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Argv {
    pub client_id: String,
    pub client_secret: String,
    pub server_port: u16,
    pub server_addr: Option<String>,
    pub fmt: Formatter,
    pub save_to_dir: bool,
    pub twitch_auth_header: Option<String>,
    pub channels: Vec<(UserCredentials, ChannelSettings)>,
}

#[derive(Deserialize)]
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
            \n  -P, --server-port    <u16>  The address for the webhook to listen to.\
            \n                              (Default: 8080)\
            \n  -A, --server-addr    <str>  The host address the server will receive requests to.\
            \n                              If not set, a ngrok tunnel will be set up automatically.\
            \n                              (Default: None)
            \n  -d, --sub-data       <path> The location where the subscription list is saved.\
            \n                              The contents should follow a specific json format;\
            \n                              See below for more information.\
            \n                              (Default: \"subscriptions.json\")\
            \n  -f, --file-name      <str>  Formats the output file name.\
            \n                              See below for more information.\
            \n                              (Default: \"%Sl\\[%si] %st\")\
            \n  --save-to-dir               Whether to save the outputs to a directory.\
            \n                              If not set, downloads will be archived to a .tar file.\
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
            \n    {{\"login\": \"twitch\"}},
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
    futures::executor::block_on(NAME.get_or_init(async { name.into() }));

    let mut client_id = None;
    let mut client_secret = None;
    let mut server_port = 8080;
    let mut server_addr = None;
    let mut sub_data = Some("subscriptions.json".to_owned());
    let mut file_name = "%Sl\\[%si] %st".to_owned();
    let mut save_to_dir = false;
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
                    Some(x)
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
    if file_name.is_empty() {
        eprint_err("File names cannot be an empty string!");
        std::process::exit(1);
    };
    let Some(sub_data) = sub_data else {
        eprint_err("Subscription data file missing!");
        std::process::exit(1);
    };
    let sub = match fs::read(sub_data) {
        Ok(x) => x,
        Err(e) => {
            eprint_err(&format!("sub-data file is missing or corrupt: {}", e));
            std::process::exit(2);
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
        server_port,
        server_addr,
        fmt: Formatter::new(&file_name),
        save_to_dir,
        twitch_auth_header,
        channels: channels
            .into_iter()
            .map(|c| (c.user, c.channel.unwrap_or_default()))
            .collect(),
    }
}
