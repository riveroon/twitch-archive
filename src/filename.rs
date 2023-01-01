use chrono::{Datelike, Timelike};
use sanitize_filename::Options;

use crate::helix::*;

enum Elements {
    UserId,
    UserLogin,
    UserName,
    Year4,
    Year2,
    Month,
    Day,
    Hour,
    Minute,
    Timezone,
    StreamId,
    StreamTitle,
    Escape,
    String(Box<str>),
}

pub struct Formatter {
    inner: Box<[Elements]>
}

impl Formatter {
    pub fn new(fmt: &str) -> Self {
        for p in fmt.split('\\') {
            if !sanitize_filename::is_sanitized(p) {
                eprintln!("ERROR: filename contains unsanitized contents: {:?}", p);
                std::process::exit(-1);
            }
        }

        let mut vec = Vec::new();
    
        let mut skip = true;
        for s in fmt.split('%') {
            if skip {
                vec.push( Elements::String(s.into()) );
                skip = false;
                continue;
            }
    
            if s.is_empty() {
                vec.push( Elements::Escape );
                skip = true;
                continue;
            }
    
            let next = match &s[..2] {
                "Si" => Elements::UserId,
                "Sl" => Elements::UserLogin,
                "Sn" => Elements::UserName,
                "TY" => Elements::Year4,
                "Ty" => Elements::Year2,
                "TM" => Elements::Month,
                "TD" => Elements::Day,
                "TH" => Elements::Hour,
                "Tm" => Elements::Minute,
                "TZ" => Elements::Timezone,
                "si" => Elements::StreamId,
                "st" => Elements::StreamTitle,
                x => {
                    eprintln!("ERROR: filename contains unknown symbol {:?}", x);
                    std::process::exit(-1);
                },
            };
            vec.push(next);
            skip = false;

            vec.push(Elements::String( (&s[2..]).into() ));
        }
        return Self { inner: vec.into_boxed_slice() };
    }

    pub fn format<'a> (&self, stream: &Stream) -> String {
        fn san(value: &str) -> String {
            sanitize_filename::sanitize_with_options(
                value,
                Options {
                    truncate: false,
                    replacement: "_",
                    ..Options::default()
                }
            )
        }

        let mut name = String::new();

        for e in self.inner.iter() {
            let tmp;
            let next = match e {
                Elements::UserId => {
                    tmp = san( stream.user().id() );
                    tmp.as_str()
                },
                Elements::UserLogin => {
                    tmp = san( stream.user().login() );
                    tmp.as_str()
                },
                Elements::UserName => {
                    tmp = san( stream.user().name() );
                    tmp.as_str()
                },
                Elements::Year4 => {
                    tmp = stream.started_at().date_naive().year().to_string();
                    tmp.as_str()
                },
                Elements::Year2 => {
                    tmp = (stream.started_at().date_naive().year() % 100).to_string();
                    tmp.as_str()
                },
                Elements::Month => {
                    tmp = stream.started_at().date_naive().month().to_string();
                    tmp.as_str()
                },
                Elements::Day => {
                    tmp = stream.started_at().date_naive().day().to_string();
                    tmp.as_str()
                },
                Elements::Hour => {
                    tmp = stream.started_at().time().hour().to_string();
                    tmp.as_str()
                },
                Elements::Minute => {
                    tmp = stream.started_at().time().minute().to_string();
                    tmp.as_str()
                },
                Elements::Timezone => {
                    tmp = stream.started_at().offset().to_string();
                    tmp.as_str()
                },
                Elements::StreamId => {
                    tmp = san( stream.id() );
                    tmp.as_str()
                },
                Elements::StreamTitle => {
                    tmp = san( stream.title() );
                    tmp.as_str()
                },
                Elements::Escape => "%",
                Elements::String(x) => &*x,
            };
            name.push_str(next);
        }
        
        return name;
    }
}