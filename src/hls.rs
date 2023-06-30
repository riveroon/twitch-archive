use anyhow::{anyhow, Context};
use async_std::{
    fs, io::{self, BufWriter, WriteExt}, path, task, 
};
use futures::{SinkExt, AsyncWrite, Stream, io::AllowStdIo, StreamExt, future};
use m3u8_rs::{AlternativeMedia, AlternativeMediaType, VariantStream, MediaPlaylist, MediaPlaylistType, MediaSegment};
use once_cell::sync::Lazy;
use std::{time, sync::Arc};
use surf::{Client, Response, Url, http::Method, RequestBuilder};

use crate::{prelude::*, poll_dbg::PollDbg};
use crate::retry::retry;

pub type StreamData = (path::PathBuf, AlternativeMedia, Option<VariantStream>);

static CLIENT: Lazy<Client> = Lazy::new(|| surf::Config::new()
        .set_timeout(Some(time::Duration::from_secs(10)))
        .try_into()
        .unwrap()
    );

pub async fn get(uri: impl Into<Url>, context: &str) -> Result<Response> {
    let uri = uri.into();
    log::trace!("sending {context}: {uri}");

    retry(|| async {
        let req = RequestBuilder::new(Method::Get, uri.clone()).build();

        let res = CLIENT.send(req).await
            .map_err(|e| e.into_inner())
            .with_context(|| format!("{context} failed"))?;
        
        if res.status().is_client_error() {
            return Err(anyhow!(
                "{context} returned status {}",
                res.status()
            ))
        }

        Ok(res)
    }, time::Duration::ZERO, 10, context)
        .await
}

pub async fn get2(uri: impl Into<Url>, context: &str) -> Result<Response> {
    let uri = uri.into();
    log::trace!("sending {context}: {uri}");
    let pctx = format!("client for {context}");

    retry(|| async {
        let req = RequestBuilder::new(Method::Get, uri.clone()).build();

        let fut = PollDbg::new(CLIENT.send(req), &pctx).await;
        let res = fut.await
            .map_err(|e| e.into_inner())
            .with_context(|| format!("{context} failed"))?;
        
        if res.status().is_client_error() {
            return Err(anyhow!(
                "{context} returned status {}",
                res.status()
            ))
        }

        Ok(res)
    }, time::Duration::ZERO, 10, context)
        .await
}

pub struct MediaPlaylistWriter<W> {
    buf: Vec<u8>,
    writer: Option<BufWriter<W>>
}

impl<W> MediaPlaylistWriter<W> {
    fn new(media: &MediaPlaylist) -> std::io::Result<Self> {
        let mut buf = Vec::new();
        media.write_to(&mut buf)?;

        Ok( Self {
            buf,
            writer: None
        } )
    }
}

impl<W: AsyncWrite + Unpin> MediaPlaylistWriter<W> {
    async fn write_buf(&mut self) -> io::Result<()> {
        if let Some(w) = &mut self.writer {
            io::copy(AllowStdIo::new(self.buf.as_slice()), w).await?;
        }
        self.buf.clear();
        Ok(())
    }

    pub async fn init(&mut self, writer: W) -> io::Result<()> {
        self.writer = Some(BufWriter::new(writer));
        self.write_buf().await
    }

    pub async fn write_segment(&mut self, segment: MediaSegment) -> io::Result<()> {
        segment.write_to(&mut self.buf)?;
        self.write_buf().await
    }

    pub async fn finish(&mut self) -> io::Result<()> {
        self.buf.extend(b"#EXT-X-ENDLIST");
        
        if let Some(w) = &mut self.writer {
            io::copy(AllowStdIo::new(self.buf.as_slice()), &mut *w).await?;
            w.flush().await?;
        }

        Ok(())
    }
}

pub async fn spawn_downloader<W> (uri: Url) -> Result<(MediaPlaylistWriter<W>, impl Stream<Item = MediaSegment>)> {
    async fn fetch_media(uri: Url) -> Result<MediaPlaylist> {
        let mut res = get2(uri, "request for media playlist").await?;

        let body = res
            .body_bytes()
            .await
            .map_err(|e| e.into_inner().context("failed to retrieve media playlist"))?;

        let (_, media) = m3u8_rs::parse_media_playlist(&body).map_err(|e| {
            log::error!("failed to parse m3u8 hls media playlist: {e:?}");
            anyhow!("failed to parse m3u8 hls media playlist")
        })?;

        Ok(media)
    }

    let (mut tx, rx) = futures::channel::mpsc::unbounded();

    let media = fetch_media(uri.clone()).await?;
    let next_poll = time::Instant::now() + time::Duration::from_secs_f32(media.target_duration);
    let len = media.segments.len() as u64;

    log::trace!("received {len} segments ({} - {})", media.media_sequence, media.media_sequence + len);
    for e in media.segments {
        tx.send(e).await?;
    }

    let mw = MediaPlaylistWriter::new(&MediaPlaylist {
        segments: Vec::new(),
        end_list: false,
        playlist_type: Some(MediaPlaylistType::Vod),
        ..media
    })?;

    if media.end_list {
        log::trace!("received ENDLIST; finishing stream");
        tx.close_channel();
        return Ok((mw, rx));
    }

    let fut = async move {
        let mut pos = media.media_sequence + len;
        let mut tx = tx;

        task::sleep(next_poll - time::Instant::now()).await;

        loop {
            let ts = time::Instant::now();
            let media = PollDbg::new(fetch_media(uri.clone()), "media").await.await?;
            let next_poll = ts + time::Duration::from_secs_f32(media.target_duration);

            let mut list = media.segments;

            if list.is_empty() {
                let sleep = next_poll - time::Instant::now();
                log::debug!("received empty playlist; trying again in {:?}", sleep);
                task::sleep(sleep).await;
                continue;
            }

            let len = list.len() as u64;
            log::trace!("received {len} segments ({pos} - {})", pos + len);

            let skip = match pos.checked_sub(media.media_sequence) {
                Some(x) => {
                    log::trace!("skipping {x} duplicate segments ({} - {pos})", media.media_sequence);
                    x as usize
                }
                None => {
                    log::warn!("media sequence bigger than expected pos ({} > {pos}); stream may not be continuous!", media.media_sequence);
                    list[0].discontinuity = true;
                    0
                }
            };

            for e in list.into_iter().skip(skip) {
                tx.send(e).await?;
            }

            pos = media.media_sequence + len as u64;

            if media.end_list {
                log::trace!("received ENDLIST; finishing stream");
                tx.close_channel();
                break;
            }

            let sleep = next_poll - time::Instant::now();
            log::trace!("sleeping for {:?}", sleep);
            task::sleep(sleep).await;
        }

        Result::<()>::Ok(())
    };

    let _ = task::Builder::new()
        .name(format!("{}-hls", task::current().name().unwrap_or(&task::current().id().to_string())))
        .spawn(PollDbg::new(fut, "main").await);

    Ok((mw, rx))
}

pub async fn download_media(
    uri: impl AsRef<str>,
    dest: &path::Path,
    stream_name: &str,
) -> Result<path::PathBuf> {
    let uri: Arc<Url> = Arc::new(uri.as_ref().parse()?);

    let mediapath = dest.join(format!("{stream_name}.m3u8"));
    let mediafile = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&mediapath)
        .await
        .context("failed to create media playlist file")?;
    
    let segdest = dest.join(stream_name);
    fs::create_dir_all(&segdest)
        .await
        .context("failed to create segment directory")?;

    let (mut mw, rx) = spawn_downloader((*uri).clone()).await?;
    mw.init(mediafile).await?;

    let mut segments = {
        let s = rx
            .skip_while(|s| 
                future::ready( if let Some(x) = &s.title { x.starts_with("Amazon") } else { false } )
            )
            .enumerate()
            .map(|(i, mut s)| {
                let uri = Arc::clone(&uri);
                async move {
                    let uri = (*uri).join(&s.uri)?;
                    let res = get(uri, &format!("request for media segment #{i}")).await?;

                    s.uri = format!("{stream_name}/{i:05}.ts");
                    let path = dest.join(&s.uri);
                    let mut file = fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&path)
                        .await
                        .context("failed to create segment file")?;

                    io::copy(res, &mut file)
                        .await
                        .context("failed to write segment to file")?;

                    file.sync_all().await.context("failed to flush segment")?;

                    Result::<MediaSegment>::Ok(s)
                }
            })
            .buffered(6);
            
        async_std::stream::StreamExt::timeout(s, time::Duration::from_secs(300))
            .take_while(|r| {
                let p = match r {
                    Ok(_) => true,
                    Err(e) => {
                        log::error!("timeout while receiving media segments from worker: {:?}", e);
                        false
                    }
                };

                future::ready(p)
            })
            .map(Result::unwrap)
    };

    while let Some(s) = segments.next().await {
        mw.write_segment(s?).await?;
    }

    mw.finish().await?;

    Ok(mediapath)
}

pub async fn download(
    uri: impl AsRef<str>,
    dest: &path::Path,
    format: impl Iterator<Item = &str>
) -> Result<Option<StreamData>> {
    let master = {
        let uri: Url = uri.as_ref().parse()?;
        let mut res = get(uri, "request for master playlist").await?;

        let body = res
            .body_bytes()
            .await
            .map_err(|e| e.into_inner().context("failed to recieve master playlist"))?;

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

    let media_uri = if let Some(uri) = &alt.uri { uri } else {
        let Some(var) = var else {
            log::error!("could not find matching STREAM-INF for MEDIA tag :{}", dest.display());
            return Err(anyhow!("url missing for format {}", format));
        };

        &var.uri
    };

    let mediapath = download_media(media_uri, dest, &alt.name).await?;

    Ok(Some((mediapath, alt.to_owned(), var.cloned())))
}
