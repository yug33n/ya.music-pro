//! FLAC из MP4, в котором Яндекс отдаёт lossless: кадры те же, меняется только обёртка.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

type Res<T> = Result<T, String>;

/// Блок метаданных FLAC: тип и содержимое.
pub type Block = (u8, Vec<u8>);

fn be32(b: &[u8], at: usize) -> Res<u32> {
    b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]])).ok_or_else(|| "MP4 обрывается".into())
}

fn be64(b: &[u8], at: usize) -> Res<u64> {
    Ok(((be32(b, at)? as u64) << 32) | be32(b, at + 4)? as u64)
}

/// Вложенные боксы: тип и содержимое без заголовка.
fn children(mut b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while b.len() >= 8 {
        let size = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let kind = [b[4], b[5], b[6], b[7]];
        let (head, size) = match size {
            0 => (8, b.len()),
            1 => match be64(b, 8) {
                Ok(s) => (16, s as usize),
                Err(_) => break,
            },
            s => (8, s),
        };
        if size < head || size > b.len() {
            break;
        }
        out.push((kind, &b[head..size]));
        b = &b[size..];
    }
    out
}

fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> Res<&'a [u8]> {
    children(b).into_iter().find(|(k, _)| k == kind).map(|(_, c)| c).ok_or_else(|| format!("в MP4 нет {}", String::from_utf8_lossy(kind)))
}

/// moov целиком: он маленький, кадры в mdat читаются потом по смещениям.
fn read_moov(f: &mut File) -> Res<Vec<u8>> {
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        f.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8]).map_err(|e| e.to_string())?;
        let (head, size) = match be32(&h, 0)? {
            0 => (8, len - at),
            1 => {
                f.read_exact(&mut h[8..]).map_err(|e| e.to_string())?;
                (16, be64(&h, 8)?)
            }
            s => (8, s as u64),
        };
        if size < head {
            break;
        }
        if &h[4..8] == b"moov" {
            let mut moov = vec![0u8; (size - head) as usize];
            f.read_exact(&mut moov).map_err(|e| e.to_string())?;
            return Ok(moov);
        }
        at += size;
    }
    Err("в MP4 нет moov".into())
}

struct Layout {
    /// Блоки метаданных из dfLa.
    blocks: Vec<Block>,
    /// Куски файла с кадрами по порядку: смещение и длина.
    parts: Vec<(u64, u64)>,
}

fn layout(moov: &[u8]) -> Res<Layout> {
    let stbl = children(moov)
        .into_iter()
        .filter(|(k, _)| k == b"trak")
        .find_map(|(_, t)| child(t, b"mdia").and_then(|m| child(m, b"minf")).and_then(|m| child(m, b"stbl")).ok().filter(|s| has_flac(s)))
        .ok_or("в MP4 нет дорожки FLAC")?;

    let stsd = child(stbl, b"stsd")?;
    let entry = child(stsd.get(8..).unwrap_or_default(), b"fLaC")?;
    let dfla = child(entry.get(28..).unwrap_or_default(), b"dfLa")?;
    let mut blocks = Vec::new();
    let mut b = dfla.get(4..).unwrap_or_default();
    while b.len() >= 4 {
        let len = ((b[1] as usize) << 16) | ((b[2] as usize) << 8) | b[3] as usize;
        let body = b.get(4..4 + len).ok_or("dfLa обрывается")?;
        blocks.push((b[0] & 0x7F, body.to_vec()));
        if b[0] & 0x80 != 0 {
            break;
        }
        b = &b[4 + len..];
    }
    if blocks.first().map(|(t, _)| *t) != Some(0) {
        return Err("в dfLa нет STREAMINFO".into());
    }

    let stsz = child(stbl, b"stsz")?;
    let (fixed, count) = (be32(stsz, 4)?, be32(stsz, 8)? as usize);
    let size = |i: usize| if fixed != 0 { Ok(fixed as u64) } else { be32(stsz, 12 + 4 * i).map(|s| s as u64) };

    let offsets: Vec<u64> = if let Ok(co) = child(stbl, b"stco") {
        (0..be32(co, 4)? as usize).map(|i| be32(co, 8 + 4 * i).map(|o| o as u64)).collect::<Res<_>>()?
    } else {
        let co = child(stbl, b"co64")?;
        (0..be32(co, 4)? as usize).map(|i| be64(co, 8 + 8 * i)).collect::<Res<_>>()?
    };

    let stsc = child(stbl, b"stsc")?;
    let runs: Vec<(usize, usize)> =
        (0..be32(stsc, 4)? as usize).map(|i| Ok((be32(stsc, 8 + 12 * i)? as usize, be32(stsc, 12 + 12 * i)? as usize))).collect::<Res<_>>()?;

    let mut parts: Vec<(u64, u64)> = Vec::new();
    let mut sample = 0;
    for (c, &off) in offsets.iter().enumerate() {
        let per = runs.iter().take_while(|(first, _)| *first <= c + 1).last().map(|r| r.1).unwrap_or(0);
        let mut len = 0;
        for _ in 0..per.min(count - sample) {
            len += size(sample)?;
            sample += 1;
        }
        match parts.last_mut() {
            Some((o, l)) if *o + *l == off => *l += len,
            _ => parts.push((off, len)),
        }
    }
    if sample != count {
        return Err(format!("в MP4 {count} кадров, а по таблицам нашлось {sample}"));
    }
    Ok(Layout { blocks, parts })
}

fn has_flac(stbl: &[u8]) -> bool {
    child(stbl, b"stsd").ok().and_then(|s| s.get(8..)).is_some_and(|e| child(e, b"fLaC").is_ok())
}

/// Переложить FLAC из MP4 `src` в файл `dst`; `extra` — свои блоки метаданных (теги, обложка).
pub fn from_mp4(src: &Path, dst: &Path, extra: Vec<Block>) -> Res<()> {
    let mut f = File::open(src).map_err(|e| e.to_string())?;
    let Layout { mut blocks, parts } = layout(&read_moov(&mut f)?)?;
    // Свои теги и обложка вместо тех, что были; отступ не нужен.
    blocks.retain(|(t, _)| !matches!(t, 1 | 4 | 6));
    blocks.extend(extra);
    let mut out = std::io::BufWriter::new(File::create(dst).map_err(|e| e.to_string())?);
    let mut head = b"fLaC".to_vec();
    for (i, (t, body)) in blocks.iter().enumerate() {
        let last = if i + 1 == blocks.len() { 0x80 } else { 0 };
        let len = body.len() as u32;
        if len >= 1 << 24 {
            return Err("блок метаданных FLAC больше 16 МБ".into());
        }
        head.push(last | t);
        head.extend_from_slice(&len.to_be_bytes()[1..]);
        head.extend_from_slice(body);
    }
    out.write_all(&head).map_err(|e| e.to_string())?;
    for (off, len) in parts {
        f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        let n = std::io::copy(&mut (&mut f).take(len), &mut out).map_err(|e| e.to_string())?;
        if n != len {
            return Err("MP4 короче, чем в его таблицах".into());
        }
    }
    out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn table(vals: &[u32]) -> Vec<u8> {
        vals.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// MP4 с тремя кадрами в двух кусках и разрывом между ними.
    fn sample_mp4() -> Vec<u8> {
        let streaminfo = [7u8; 34];
        let mut dfla = vec![0, 0, 0, 0, 0x80, 0, 0, 34];
        dfla.extend_from_slice(&streaminfo);
        let mut entry = vec![0u8; 28];
        entry.extend(boxed(b"dfLa", &dfla));
        let mut stsd = table(&[0, 1]);
        stsd.extend(boxed(b"fLaC", &entry));
        let head_len = 8 + 16; // ftyp и заголовок mdat
        let frames: [&[u8]; 3] = [b"AAA", b"BB", b"C"];
        let mut stbl = boxed(b"stsd", &stsd);
        stbl.extend(boxed(b"stsz", &table(&[0, 0, 3, 3, 2, 1])));
        stbl.extend(boxed(b"stsc", &table(&[0, 2, 1, 2, 1, 2, 1, 1])));
        stbl.extend(boxed(b"stco", &table(&[0, 2, head_len, head_len + 5 + 3])));
        let moov = boxed(b"moov", &boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &boxed(b"stbl", &stbl)))));
        let mut mdat = Vec::new();
        mdat.extend_from_slice(frames[0]);
        mdat.extend_from_slice(frames[1]);
        mdat.extend_from_slice(b"xxx");
        mdat.extend_from_slice(frames[2]);
        let mut file = boxed(b"ftyp", b"");
        file.extend(boxed(b"mdat", &[vec![0u8; 8], mdat].concat()));
        file.extend(moov);
        file
    }

    #[test]
    fn remux() {
        let dir = std::env::temp_dir().join(format!("yamusic-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("a.mp4"), dir.join("a.flac"));
        std::fs::write(&src, sample_mp4()).unwrap();
        from_mp4(&src, &dst, vec![(4, b"tags".to_vec())]).unwrap();
        let out = std::fs::read(&dst).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let mut want = b"fLaC\x00\x00\x00\x22".to_vec();
        want.extend_from_slice(&[7u8; 34]);
        want.extend_from_slice(b"\x84\x00\x00\x04tags");
        want.extend_from_slice(b"AAABBC");
        assert_eq!(out, want);
    }
}
