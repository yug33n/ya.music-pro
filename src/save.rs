//! Сохранение трека в файл с тегами: общее для десктопа и Android.

use crate::api::{Api, Track};
use crate::config::log;
use std::time::Duration;

/// Имя файла или папки, которое Windows точно примет: без запрещённых и управляющих
/// символов, без точки и пробела в конце, не зарезервированное (CON, NUL, COM1…), не длиннее `max`.
pub fn file_name_max(s: &str, max: usize) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    if out.chars().count() > max {
        out = out.chars().take(max).collect();
    }
    let out = out.trim_end_matches(['.', ' ']).trim_start().to_string();
    if out.is_empty() {
        return "_".into();
    }
    let stem = out.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{out}")
    } else {
        out
    }
}

/// Расширение по первым байтам: mp3, AAC (ADTS), MP4 или FLAC.
pub fn audio_ext(b: &[u8]) -> &'static str {
    match b {
        [b'I', b'D', b'3', ..] => "mp3",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "m4a",
        [b'f', b'L', b'a', b'C', ..] => "flac",
        [0xFF, x, ..] if x & 0xF6 == 0xF0 => "aac",
        _ => "mp3",
    }
}

pub fn track_file_stem(t: &Track) -> String {
    file_name_max(&format!("{} — {}", t.artists, t.title), 120)
}

/// Трек уже скачан: готовый файл (не `.part`) с ненулевым размером.
pub fn saved_already(dir: &std::path::Path, t: &Track) -> bool {
    let stem = track_file_stem(t);
    ["mp3", "m4a", "aac", "flac"]
        .iter()
        .any(|e| std::fs::metadata(dir.join(format!("{stem}.{e}"))).is_ok_and(|m| m.len() > 0))
}

/// Скачать трек: сначала в `.part`, теги — в mp3 и FLAC, и только после этого переименовать в
/// готовое имя. Обрыв оставит лишь `.part`, который следующая попытка перезапишет.
pub fn save_track(api: &Api, dir: &std::path::Path, t: &Track, flac: bool) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = track_file_stem(t);
    let part = dir.join(format!("{stem}.part"));
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt));
        }
        let url = || {
            if flac {
                match api.flac_url(&t.id) {
                    Ok(Some(u)) => return Ok(u),
                    Ok(None) => {}
                    Err(e) => log(format!("flac {}: {e}", t.id)),
                }
            }
            api.stream_url(&t.id)
        };
        match url().and_then(|u| api.download_to(&u, &part)) {
            Ok(n) if n > 0 => {
                last.clear();
                break;
            }
            Ok(_) => last = "пустой файл".into(),
            Err(e) => last = e,
        }
    }
    if !last.is_empty() {
        let _ = std::fs::remove_file(&part);
        return Err(last);
    }
    let finish = || -> Result<std::path::PathBuf, String> {
        let mut head = [0u8; 16];
        {
            use std::io::Read as _;
            let mut f = std::fs::File::open(&part).map_err(|e| e.to_string())?;
            let n = f.read(&mut head).map_err(|e| e.to_string())?;
            if n < head.len() {
                return Err("файл слишком короткий".into());
            }
        }
        let ext = audio_ext(&head);
        let path = dir.join(format!("{stem}.{ext}"));
        let remux = flac && ext == "m4a";
        let title = if t.version.is_empty() { t.title.clone() } else { format!("{} ({})", t.title, t.version) };
        let cover = if ext == "mp3" || remux { t.cover_url(400).and_then(|u| api.fetch_bytes(&u).ok()) } else { None };
        let tags = crate::tags::Tags { title: &title, artist: &t.artists, album: &t.album_title, year: t.year, track: t.position, cover: cover.as_deref() };
        if remux {
            let path = dir.join(format!("{stem}.flac"));
            let tmp = dir.join(format!("{stem}.flac.part"));
            match crate::flac::from_mp4(&part, &tmp, crate::tags::flac_blocks(&tags)) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&part);
                    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
                    return Ok(path);
                }
                // Не разобрали MP4 — сохраним как .m4a, FLAC в нём тоже играется.
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    log(format!("flac {}: {e}", t.id));
                }
            }
        }
        if ext == "mp3" {
            let tag = crate::tags::id3(&tags);
            let tagged = dir.join(format!("{stem}.tagged.part"));
            {
                use std::io::{Read as _, Seek as _, Write as _};
                let mut src = std::fs::File::open(&part).map_err(|e| e.to_string())?;
                let mut probe = vec![0u8; 10];
                let _ = src.read(&mut probe);
                let skip = crate::tags::existing_id3_len(&probe) as u64;
                src.seek(std::io::SeekFrom::Start(skip)).map_err(|e| e.to_string())?;
                let mut out = std::fs::File::create(&tagged).map_err(|e| e.to_string())?;
                out.write_all(&tag).map_err(|e| e.to_string())?;
                std::io::copy(&mut src, &mut out).map_err(|e| e.to_string())?;
                out.sync_all().map_err(|e| e.to_string())?;
            }
            let _ = std::fs::remove_file(&part);
            std::fs::rename(&tagged, &path).map_err(|e| e.to_string())?;
        } else {
            std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        }
        Ok(path)
    };
    finish().inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })
}
