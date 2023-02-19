use ::log::LevelFilter;
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Config, Logger, Root},
    encode::pattern::PatternEncoder,
    filter::threshold::ThresholdFilter,
    Handle,
};

pub fn init(file: String, level: LevelFilter, stderr: bool) -> Handle {
    let mut config = Config::builder();
    let mut root = Root::builder();
    let mut main = Logger::builder().additive(false);

    {
        let stdout = ConsoleAppender::builder()
            .target(if stderr {
                Target::Stderr
            } else {
                Target::Stdout
            })
            .encoder(Box::new(PatternEncoder::new(
                "{d(%Y-%m-%d %H:%M:%S %Z)(local)} [ {h({l}):5.5} ] {T}: {M} > {m}{n}",
            )))
            .build();
        let filter = ThresholdFilter::new(level);

        config = config.appender(
            Appender::builder()
                .filter(Box::new(filter))
                .build("console-main", Box::new(stdout)),
        );
        main = main.appender("console-main");
    }

    {
        let stdout = ConsoleAppender::builder()
            .target(if stderr {
                Target::Stderr
            } else {
                Target::Stdout
            })
            .encoder(Box::new(PatternEncoder::new(
                "{d(%Y-%m-%d %H:%M:%S %Z)(local)} [ {h({l}):5.5} ] {T}: {M} > {m}{n}",
            )))
            .build();
        let filter = ThresholdFilter::new(LevelFilter::Warn);

        config = config.appender(
            Appender::builder()
                .filter(Box::new(filter))
                .build("console", Box::new(stdout)),
        );
        root = root.appender("console");
    }

    if !file.is_empty() {
        let file = FileAppender::builder()
            .append(true)
            .encoder(Box::new(PatternEncoder::new(
                "{d(%Y-%m-%d %H:%M:%S %Z)(local)} [ {({l}):5.5} ] {T}: {M}:{L}> {m}{n}",
            )))
            .build(file)
            .unwrap();

        config = config.appender(Appender::builder().build("file", Box::new(file)));
        root = root.appender("file");
        main = main.appender("file");
    }

    let config = config
        .logger(main.build("twitch_archive", LevelFilter::Trace))
        .build(root.build(LevelFilter::Debug))
        .unwrap();

    log4rs::init_config(config).unwrap()
}
