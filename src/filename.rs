use std::borrow::Cow;

use crate::helix::*;

use chrono::{Datelike, Timelike};
use sanitize_filename::Options;

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
    Seperator,
    String(Box<str>),
}

pub struct Formatter {
    inner: Box<[Elements]>,
}

impl Formatter {
    pub fn new(fmt: &str) -> Self {
        let mut vec = Vec::new();

        for p in fmt.split(['\\', '/']) {
            if p.contains(['\\', '/']) {
                eprintln!("ERROR: filename contains unsanitized contents: {:?}", p);
                std::process::exit(-1);
            }

            let mut skip = true;
            for s in p.split('%') {
                if skip {
                    if !s.is_empty() {
                        vec.push(Elements::String(s.into()));
                    }
                    skip = false;
                    continue;
                }

                if s.is_empty() {
                    vec.push(Elements::Escape);
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
                    }
                };
                vec.push(next);
                skip = false;

                if !&s[2..].is_empty() {
                    vec.push(Elements::String((&s[2..]).into()));
                }
            }

            vec.push(Elements::Seperator);
        }

        if !vec.is_empty() {
            vec.pop();
        }

        Self {
            inner: vec.into_boxed_slice(),
        }
    }

    pub fn format(&self, stream: &Stream) -> String {
        fn san(value: &str) -> String {
            sanitize_filename::sanitize_with_options(
                value,
                Options {
                    truncate: false,
                    replacement: "_",
                    ..Options::default()
                },
            )
        }

        let mut name = String::new();

        for e in self.inner.iter() {
            let next: Cow<str> = match e {
                Elements::UserId => san(stream.user().id()).into(),
                Elements::UserLogin => san(stream.user().login()).into(),
                Elements::UserName => san(stream.user().name()).into(),
                Elements::Year4 => stream.started_at().date_naive().year().to_string().into(),
                Elements::Year2 => (stream.started_at().date_naive().year() % 100).to_string().into(),
                Elements::Month => stream.started_at().date_naive().month().to_string().into(),
                Elements::Day => stream.started_at().date_naive().day().to_string().into(),
                Elements::Hour => stream.started_at().time().hour().to_string().into(),
                Elements::Minute => stream.started_at().time().minute().to_string().into(),
                Elements::Timezone => stream.started_at().offset().to_string().into(),
                Elements::StreamId => san(stream.id()).into(),
                Elements::StreamTitle => san(stream.title()).into(),
                Elements::Escape => "%".into(),
                Elements::Seperator => std::path::MAIN_SEPARATOR.to_string().into(),
                Elements::String(x) => (&**x).into(),
            };
            name.push_str(&next);
        }

        name
    }
}
