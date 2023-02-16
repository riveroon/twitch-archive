use async_std::{fs, io, path};
use sanitize_filename::{sanitize_with_options, Options};

const MAX_FILENAME_DUP: usize = 65536;

pub fn san(value: &str) -> String {
    sanitize_with_options(
        value,
        Options {
            truncate: false,
            replacement: "_",
            ..Options::default()
        },
    )
}

pub async fn create_new_file(path: &path::Path) -> io::Result<Option<fs::File>> {
    log::trace!("download::create_new_file: {}", path.to_string_lossy());

    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await;

    file.map(Some).or_else(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

pub async fn create_dedup_file(path: &path::Path) -> io::Result<(Box<path::Path>, fs::File)> {
    log::trace!("download::create_dedup_file: {}", path.to_string_lossy());

    // fs::create_dir_all() does not error on duplicate paths; see async_std issue #1051
    if let Some(x) = path.parent() {
        fs::create_dir_all(x).await?
    }

    if let Some(x) = create_new_file(path).await? {
        return Ok((path.into(), x));
    };

    for i in 1..MAX_FILENAME_DUP {
        let mut new_name = path.file_stem().unwrap_or_default().to_os_string();
        new_name.push(format!("-{}", i));
        let mut new_path = path.with_file_name(&new_name);
        new_path.set_extension(path.extension().unwrap_or_default());

        if let Some(x) = create_new_file(&new_path).await? {
            return Ok((new_path.into(), x));
        };
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

pub async fn create_new_dir(path: &path::Path) -> io::Result<bool> {
    log::trace!("download::create_new_dir: {}", path.to_string_lossy());

    if let Some(x) = path.parent() {
        if !x.is_dir().await {
            fs::create_dir_all(path).await?;
            return Ok(true);
        }
    }

    fs::create_dir(path).await.map(|_| true).or_else(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(e)
        }
    })
}

pub async fn create_dedup_dir(path: &path::Path) -> io::Result<Box<path::Path>> {
    log::trace!("download::create_dedup_dir: {}", path.to_string_lossy());

    if create_new_dir(path).await? {
        return Ok(path.into());
    }

    for i in 1..MAX_FILENAME_DUP {
        let mut new_name = path.file_name().unwrap_or_default().to_os_string();
        new_name.push(format!("-{}", i));
        let new_path = path.with_file_name(&new_name);

        if create_new_dir(&new_path).await? {
            return Ok(new_path.into_boxed_path());
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}
