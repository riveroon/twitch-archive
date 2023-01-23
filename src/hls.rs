use std::{mem, time};
use async_std::{io::{self, BufWriter}, fs, path, task};
use futures::AsyncWriteExt;
use m3u8_rs::AlternativeMediaType;

pub async fn download(uri: impl AsRef<str>, dest: &path::Path, format: impl Iterator<Item = &str>) -> Result<(), ()> {
    let client = surf::client();

    let res = client.get(uri)
        .recv_bytes()
        .await
        .map_err(|e| log::error!("error while retrieving master playlist: {}", e))?; 
    
    let (_, master) = m3u8_rs::parse_master_playlist(&res)
        .map_err(|e| log::error!("malformed m3u8 hls master playlist: {}", e))?;

    let mut alt = None;
    for f in format {
        alt = master.alternatives.iter()
            .find(|x| x.name == f);
        if alt.is_some() { break; }
    }
    let Some(alt) = alt else { return Ok(()) };
    let url = if let Some(url) = &alt.uri {url} else {
        let Some(url) = master.variants.iter()
            .find(|x| {
                match &alt.media_type {
                    AlternativeMediaType::Video => x.video.as_ref() == Some(&alt.group_id),
                    AlternativeMediaType::Audio => x.audio.as_ref() == Some(&alt.group_id),
                    AlternativeMediaType::Subtitles => x.subtitles.as_ref() == Some(&alt.group_id),
                    AlternativeMediaType::ClosedCaptions => false,
                    AlternativeMediaType::Other(_) => false
                }
            }) else {
            log::error!("could not find matching STREAM-INF for MEDIA tag :{}", dest.to_string_lossy());
            return Err(());
        };
        &url.uri
    };

    let dest = dest.join(&alt.name);

    fs::create_dir_all(&dest).await
        .map_err(|e| log::error!("failed to create directory {}: {}", dest.to_string_lossy(), e))?;

    fs::write(dest.join("index.m3u8"), res).await
        .map_err(|e| log::error!("failed to write to index file: {}", e))?;

    let mut i = 0;
    
    let mut buf = Vec::new();

    let mut pos = 0;
    'outer: loop {
        let dest_seg = dest.join(i.to_string());
        fs::create_dir(&dest_seg).await
            .map_err(|e| log::error!("failed to create directory {}: {}", dest_seg.to_string_lossy(), e))?;
        let segpath = dest_seg.join("segments.m3u8");
        let mut segfile = fs::File::create(&segpath).await
            .map_err(|e| log::error!("failed to create segments file {:?}: {}", segpath.to_string_lossy(), e))?;
        let mut init = false;

        loop {
            let ts = time::Instant::now();
    
            log::trace!("retrieving hls media playlist: {}", url);
            let res = client.get(url)
                .recv_bytes().await
                .map_err(|e| log::error!("failed to GET media playlist: {}", e))?;
    
            let (_, mut media) = m3u8_rs::parse_media_playlist(&res)
                .map_err(|e| log::error!("malformed m3u8 hls media playlist: {}", e))?;
    
            let list = mem::take(&mut media.segments);

            if !init {
                media.write_to(&mut buf).unwrap();
                segfile.write_all(&buf).await
                    .map_err(|e| log::error!("failed to write to segments file {:?}: {}", segpath.to_string_lossy(), e))?;
                buf.clear();
                pos = media.media_sequence;
                init = true;
            } else if media.media_sequence > pos {
                log::trace!("creating next segment because {} > {}", media.media_sequence, pos);
                i += 1;
                break;
            }
    
            let len = list.len();
            log::trace!("retrieved hls media playlist with {} segments", len);
    
            for (n, mut e) in list.into_iter().enumerate() {
                let idx = media.media_sequence + n as u64;
                if idx < pos {
                    log::trace!("skipping segment #{} because we need {}", idx, pos);
                    continue;
                }
    
                log::trace!("downloading media segment #{}: {}", idx, e.uri);
    
                let res = client.get(&e.uri)
                    .send().await
                    .map_err(|e| log::error!("failed to GET media segment: {}", e))?;
    
                e.uri = format!("{:04}.ts", idx);
    
                let path = dest_seg.join(&e.uri);
                let file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .await
                    .map_err(|e| log::error!("failed to create segment file {:?}: {}", path, e))?;
                    
                io::copy(res, file).await
                    .map_err(|e| log::error!("failed to write media segment to file {:?}: {}", path, e))?;
    
                e.write_to(&mut buf).unwrap();
                segfile.write_all(&buf).await
                    .map_err(|e| log::error!("failed to write segment data to segments file {:?}: {}", segpath.to_string_lossy(), e))?;
                buf.clear();
            }
    
            pos = media.media_sequence + len as u64;
    
            if media.end_list { break 'outer; }
            task::sleep(
                (ts + time::Duration::from_secs_f32(media.target_duration * 2.0)) - time::Instant::now()
            ).await;
        }
    }

    Ok(())
}