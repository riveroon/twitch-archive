use anyhow::{anyhow, Context};
use async_std::{
    fs,
    io::{self, BufWriter},
    path, task,
};
use futures::AsyncWriteExt;
use m3u8_rs::{AlternativeMedia, AlternativeMediaType, VariantStream};
use std::{mem, time};

use crate::prelude::*;

pub type StreamData = (path::PathBuf, AlternativeMedia, Option<VariantStream>);

pub async fn download(
    uri: impl AsRef<str>,
    dest: &path::Path,
    format: impl Iterator<Item = &str>,
) -> Result<Option<StreamData>> {
    let client: surf::Client = surf::Config::new()
        .set_timeout(Some(time::Duration::from_secs(15)))
        .try_into()
        .unwrap();

    let master = {
        let mut res = client.get(uri).send().await.map_err(|e| e.into_inner())?;

        if !res.status().is_success() {
            return Err(anyhow!(
                "master playlist request returned status {}",
                res.status()
            ));
        }

        let body = res
            .body_bytes()
            .await
            .map_err(|e| e.into_inner().context("failed to retrieve master playlist"))?;

        let (_, master) = m3u8_rs::parse_master_playlist(&body).map_err(|e| {
            log::error!("malformed m3u8 hls master playlist: {e:?}");
            anyhow!("malformed m3u8 hls master playlist")
        })?;

        master
    };

    let (format, alt) = {
        let format: Vec<&str> = format.collect();
        let Some((format, alt)) = format.iter()
            .find_map(|&f| {
                if f == "best" {
                    master.alternatives.get(0)
                } else {
                    master.alternatives.iter()
                        .find(|x| x.name.starts_with(f))
                }.map(|x| (f, x))
            }) else {
            log::info!("no matching quality found: expected {format:?}, found {:?}", master.alternatives);
            return Ok(None);
        };

        (format, alt)
    };

    let var = master.variants.iter().find(|x| match &alt.media_type {
        AlternativeMediaType::Video => x.video.as_ref() == Some(&alt.group_id),
        AlternativeMediaType::Audio => x.audio.as_ref() == Some(&alt.group_id),
        AlternativeMediaType::Subtitles => x.subtitles.as_ref() == Some(&alt.group_id),
        AlternativeMediaType::ClosedCaptions => false,
        AlternativeMediaType::Other(_) => false,
    });

    let url = if let Some(url) = &alt.uri {
        url
    } else {
        let Some(var) = var else {
            log::error!("could not find matching STREAM-INF for MEDIA tag :{}", dest.display());
            return Err(anyhow!("url missing for format {}", format));
        };

        &var.uri
    };

    let mut buf = Vec::new();
    let mut pos = 0;

    let mediapath = dest.join(format!("{}.m3u8", &alt.name));
    let mut mediafile = BufWriter::new(
        fs::File::create(&mediapath)
            .await
            .context("failed to create segments file")?,
    );
    let mut init = false;
    let mut ad_filt = false;

    let segdest = dest.join(&alt.name);
    fs::create_dir_all(&segdest)
        .await
        .context("failed to create segment directory")?;

    loop {
        let ts = time::Instant::now();

        let mut media = {
            log::trace!("retrieving hls media playlist: {url}");
            let mut res = client.get(url).send().await.map_err(|e| e.into_inner())?;

            if !res.status().is_success() {
                return Err(anyhow!(
                    "media playlist request returned status {}",
                    res.status()
                ));
            }

            let body = res
                .body_bytes()
                .await
                .map_err(|e| e.into_inner().context("failed to retrieve media playlist"))?;

            let (_, media) = m3u8_rs::parse_media_playlist(&body).map_err(|e| {
                log::error!("failed to parse m3u8 hls media playlist: {e:?}");
                anyhow!("failed to parse m3u8 hls media playlist")
            })?;

            media
        };

        let mut list = mem::take(&mut media.segments);
        let len = list.len();
        log::trace!("retrieved hls media playlist with {len} segments");

        if !init {
            let end_list = media.end_list;
            media.end_list = false;
            media.playlist_type = Some(m3u8_rs::MediaPlaylistType::Vod);
            media.write_to(&mut buf).unwrap();
            mediafile
                .write_all(&buf)
                .await
                .context("failed to write media palylist")?;
            buf.clear();
            pos = media.media_sequence;
            init = true;
            media.end_list = end_list;
        } else if media.media_sequence > pos {
            //log::trace!("creating next segment because {} > {}", media.media_sequence, pos);
            //break;
            log::warn!("media sequence bigger than expected pos ({} > {pos}); stream may not be continuous!", media.media_sequence);
            if let Some(x) = list.get_mut(0) {
                x.discontinuity = true;
            }
            pos = media.media_sequence;
        }

        if !ad_filt {
            //remove ad segment
            for (n, s) in list.iter().enumerate() {
                let idx = media.media_sequence + n as u64;
                if idx < pos {
                    log::trace!("skipping segment #{idx} because we need {pos}");
                    continue;
                }

                if let Some(x) = &s.title {
                    if x.starts_with("Amazon") {
                        log::trace!("skipping ad segment {pos}");
                        pos += 1;
                        continue;
                    }
                }
                ad_filt = true;
                break;
            }
        }

        for (n, mut e) in list.into_iter().enumerate() {
            let idx = media.media_sequence + n as u64;
            if idx < pos {
                log::trace!("skipping segment #{idx} because we need {pos}");
                continue;
            }

            log::trace!("downloading media segment #{idx}: {}", e.uri);

            let res = client
                .get(&e.uri)
                .send()
                .await
                .map_err(|e| e.into_inner().context("failed to GET media segment"))?;

            e.uri = format!("{}/{:04}.ts", &alt.name, idx);

            let path = dest.join(&e.uri);
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .await
                .context("failed to create segment")?;

            io::copy(res, &mut file)
                .await
                .context("failed to write segment")?;

            file.flush().await.context("failed to flush segment")?;

            e.write_to(&mut buf).unwrap();
            mediafile
                .write_all(&buf)
                .await
                .context("failed to write to media segment")?;
            buf.clear();
        }

        pos = media.media_sequence + len as u64;

        if media.end_list {
            log::trace!("received ENDLIST; finishing stream");
            break;
        }

        task::sleep(
            (ts + time::Duration::from_secs_f32(media.target_duration)) - time::Instant::now(),
        )
        .await;
    }

    mediafile
        .write_all(b"#EXT-X-ENDLIST")
        .await
        .context("failed to finish hls playlist")?;

    mediafile
        .flush()
        .await
        .context("failed to flush media segment")?;

    Ok(Some((mediapath, alt.to_owned(), var.cloned())))
}
