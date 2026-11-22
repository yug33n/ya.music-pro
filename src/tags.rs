//! Теги скачанных треков: ID3v2.3 в mp3, Vorbis comment и картинка во FLAC.

pub struct Tags<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub year: Option<u32>,
    pub track: Option<u32>,
    /// JPEG или PNG.
    pub cover: Option<&'a [u8]>,
}

fn text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    // Кодировка 1: UTF-16 с BOM — её понимают все проигрыватели, в отличие от UTF-8 в v2.3.
    let mut body = vec![1u8, 0xFF, 0xFE];
    for u in text.encode_utf16() {
        body.extend_from_slice(&u.to_le_bytes());
    }
    frame(id, body)
}

fn frame(id: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
    let mut f = Vec::with_capacity(10 + body.len());
    f.extend_from_slice(id);
    f.extend_from_slice(&(body.len() as u32).to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(&body);
    f
}

fn synchsafe(n: u32) -> [u8; 4] {
    [((n >> 21) & 0x7F) as u8, ((n >> 14) & 0x7F) as u8, ((n >> 7) & 0x7F) as u8, (n & 0x7F) as u8]
}

/// Готовый тег ID3v2.3 целиком (заголовок и кадры).
pub fn id3(t: &Tags) -> Vec<u8> {
    let mut frames = Vec::new();
    if !t.title.is_empty() {
        frames.extend(text_frame(b"TIT2", t.title));
    }
    if !t.artist.is_empty() {
        frames.extend(text_frame(b"TPE1", t.artist));
    }
    if !t.album.is_empty() {
        frames.extend(text_frame(b"TALB", t.album));
    }
    if let Some(y) = t.year {
        frames.extend(text_frame(b"TYER", &y.to_string()));
    }
    if let Some(n) = t.track {
        frames.extend(text_frame(b"TRCK", &n.to_string()));
    }
    if let Some(img) = t.cover {
        let mime = mime(img);
        let mut body = vec![0u8];
        body.extend_from_slice(mime);
        body.push(0);
        body.push(3); // обложка спереди
        body.push(0); // пустое описание
        body.extend_from_slice(img);
        frames.extend(frame(b"APIC", body));
    }
    let mut out = Vec::with_capacity(10 + frames.len());
    out.extend_from_slice(b"ID3\x03\x00\x00");
    out.extend_from_slice(&synchsafe(frames.len() as u32));
    out.extend(frames);
    out
}

fn mime(img: &[u8]) -> &'static [u8] {
    if img.starts_with(&[0x89, b'P', b'N', b'G']) {
        b"image/png"
    } else {
        b"image/jpeg"
    }
}

/// Блоки метаданных FLAC: теги (тип 4) и обложка (тип 6).
pub fn flac_blocks(t: &Tags) -> Vec<crate::flac::Block> {
    fn put(out: &mut Vec<u8>, s: &[u8]) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s);
    }
    let year = t.year.map(|y| y.to_string()).unwrap_or_default();
    let track = t.track.map(|n| n.to_string()).unwrap_or_default();
    let fields: Vec<String> = [("TITLE", t.title), ("ARTIST", t.artist), ("ALBUM", t.album), ("DATE", &year), ("TRACKNUMBER", &track)]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let mut vc = Vec::new();
    put(&mut vc, concat!("yamusic-rs ", env!("CARGO_PKG_VERSION")).as_bytes());
    vc.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for f in &fields {
        put(&mut vc, f.as_bytes());
    }
    let mut blocks = vec![(4, vc)];
    if let Some(img) = t.cover {
        let mime = mime(img);
        let mut p = Vec::with_capacity(img.len() + 48);
        p.extend_from_slice(&3u32.to_be_bytes()); // обложка спереди
        p.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        p.extend_from_slice(mime);
        p.extend_from_slice(&0u32.to_be_bytes()); // пустое описание
        p.extend_from_slice(&[0; 16]); // размеры и глубина цвета необязательны
        p.extend_from_slice(&(img.len() as u32).to_be_bytes());
        p.extend_from_slice(img);
        blocks.push((6, p));
    }
    blocks
}

/// Сколько байт в начале файла занимает уже имеющийся тег ID3v2 (0 — тега нет).
pub fn existing_id3_len(head: &[u8]) -> usize {
    if head.len() < 10 || &head[..3] != b"ID3" {
        return 0;
    }
    let size = head[6..10].iter().fold(0usize, |a, b| (a << 7) | (*b as usize & 0x7F));
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_len() {
        let tag = id3(&Tags { title: "Песня", artist: "Исполнитель", album: "", year: Some(2010), track: Some(3), cover: Some(&[0xFF, 0xD8, 1, 2]) });
        assert_eq!(&tag[..3], b"ID3");
        assert_eq!(existing_id3_len(&tag), tag.len());
        assert_eq!(existing_id3_len(b"fLaC0000000000"), 0);
    }

    #[test]
    fn vorbis_comment() {
        let blocks = flac_blocks(&Tags { title: "Песня", artist: "Исполнитель", album: "", year: Some(2010), track: None, cover: None });
        assert_eq!(blocks.len(), 1);
        let (kind, b) = &blocks[0];
        assert_eq!(*kind, 4);
        let vendor = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
        let rest = &b[4 + vendor..];
        assert_eq!(u32::from_le_bytes(rest[..4].try_into().unwrap()), 3);
        let first = u32::from_le_bytes(rest[4..8].try_into().unwrap()) as usize;
        assert_eq!(&rest[8..8 + first], "TITLE=Песня".as_bytes());
    }
}
