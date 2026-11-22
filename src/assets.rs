//! Шрифты и иконки берутся из установленного официального приложения (app.asar).
//! В репозитории их нет. Если приложение не установлено, те же файлы один раз скачиваются
//! с веб-версии music.yandex.ru в кэш (`config::cache_dir`) и дальше читаются оттуда.

use crate::config::log;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

pub struct Asar {
    file: File,
    base: u64,
    index: serde_json::Value,
}

/// Официального клиента под Linux нет, там ассеты только с сайта.
fn asar_path() -> Option<PathBuf> {
    if cfg!(not(windows)) {
        return None;
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    Some(PathBuf::from(local).join("Programs").join("YandexMusic").join("resources").join("app.asar"))
}

impl Asar {
    pub fn open() -> Option<Asar> {
        let mut file = File::open(asar_path()?).ok()?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head).ok()?;
        let u = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
        let (header_size, json_len) = (u(4) as u64, u(12) as usize);
        let mut json = vec![0u8; json_len];
        file.read_exact(&mut json).ok()?;
        let index = serde_json::from_slice(&json).ok()?;
        Some(Asar { file, base: 8 + header_size, index })
    }

    /// Имена файлов в папке архива.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let mut node = &self.index;
        for part in dir.split('/') {
            match node.get("files").and_then(|f| f.get(part)) {
                Some(n) => node = n,
                None => return Vec::new(),
            }
        }
        node.get("files").and_then(|f| f.as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    /// Первые `n` байт файла — чтобы не читать целиком всё подряд.
    pub fn read_prefix(&mut self, path: &str, n: usize) -> Option<Vec<u8>> {
        let mut node = &self.index;
        for part in path.split('/') {
            node = node.get("files")?.get(part)?;
        }
        let offset: u64 = node.get("offset")?.as_str()?.parse().ok()?;
        let size = (node.get("size")?.as_u64()? as usize).min(n);
        let mut buf = vec![0u8; size];
        self.file.seek(SeekFrom::Start(self.base + offset)).ok()?;
        self.file.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    pub fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut node = &self.index;
        for part in path.split('/') {
            node = node.get("files")?.get(part)?;
        }
        let offset: u64 = node.get("offset")?.as_str()?.parse().ok()?;
        let size = node.get("size")?.as_u64()? as usize;
        let mut buf = vec![0u8; size];
        self.file.seek(SeekFrom::Start(self.base + offset)).ok()?;
        self.file.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// Откуда читать шрифты и спрайт: архив оригинала или кэш файлов веб-версии.
pub enum Source {
    Asar(Asar),
    Web(WebCache),
}

impl Source {
    pub fn open() -> Source {
        match Asar::open() {
            Some(a) => Source::Asar(a),
            None => Source::Web(WebCache::new()),
        }
    }

    /// `path` — как в архиве: `app/fonts/…`, `app/icons/…`.
    pub fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        match self {
            Source::Asar(a) => a.read(path),
            Source::Web(w) => w.read(path),
        }
    }
}

/// Файлы веб-версии: music.yandex.ru отдаёт те же `/fonts/*.woff` и `/icons/sprite.svg`.
pub struct WebCache {
    dir: PathBuf,
    http: Option<reqwest::blocking::Client>,
}

const WEB_ORIGIN: &str = "https://music.yandex.ru";

impl WebCache {
    fn new() -> WebCache {
        WebCache { dir: crate::config::cache_dir().join("assets"), http: None }
    }

    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let rel = path.strip_prefix("app/")?;
        if !(rel.starts_with("fonts/") || rel.starts_with("icons/")) || rel.contains("..") {
            return None;
        }
        let file = self.dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Ok(b) = std::fs::read(&file) {
            if !b.is_empty() {
                return Some(b);
            }
        }
        let http = self.http.get_or_insert_with(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("http client")
        });
        let resp = match http.get(format!("{WEB_ORIGIN}/{rel}")).send() {
            Ok(r) => r,
            Err(e) => {
                log(format!("ассет {rel}: {e}"));
                return None;
            }
        };
        if !resp.status().is_success() {
            log(format!("ассет {rel}: HTTP {}", resp.status()));
            return None;
        }
        let bytes = resp.bytes().ok()?.to_vec();
        let ok = if rel.ends_with(".woff") { bytes.starts_with(b"wOFF") } else { bytes.windows(4).take(512).any(|w| w == b"<svg") };
        if !ok {
            log(format!("ассет {rel}: неожиданный ответ"));
            return None;
        }
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = file.with_extension("part");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
        Some(bytes)
    }
}

/// WOFF 1.0 → обычный sfnt (TTF/OTF), который понимает fontique.
pub fn woff_to_sfnt(w: &[u8]) -> Option<Vec<u8>> {
    let be32 = |i: usize| -> Option<u32> { Some(u32::from_be_bytes(w.get(i..i + 4)?.try_into().ok()?)) };
    let be16 = |i: usize| -> Option<u16> { Some(u16::from_be_bytes(w.get(i..i + 2)?.try_into().ok()?)) };
    if w.get(0..4)? != b"wOFF" {
        return None;
    }
    let flavor = be32(4)?;
    let n = be16(12)? as usize;
    let mut tables = Vec::with_capacity(n);
    for i in 0..n {
        let e = 44 + i * 20;
        let (tag, off, comp, orig, sum) = (be32(e)?, be32(e + 4)? as usize, be32(e + 8)? as usize, be32(e + 12)? as usize, be32(e + 16)?);
        let raw = w.get(off..off + comp)?;
        let data = if comp < orig { miniz_oxide::inflate::decompress_to_vec_zlib(raw).ok()? } else { raw.to_vec() };
        tables.push((tag, sum, data));
    }
    let mut pow = 1;
    let mut log2 = 0u16;
    while pow * 2 <= n {
        pow *= 2;
        log2 += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&((pow * 16) as u16).to_be_bytes());
    out.extend_from_slice(&log2.to_be_bytes());
    out.extend_from_slice(&((n * 16 - pow * 16) as u16).to_be_bytes());
    let mut offset = 12 + n * 16;
    for (tag, sum, data) in &tables {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += (data.len() + 3) & !3;
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        out.resize((out.len() + 3) & !3, 0);
    }
    Some(out)
}

const FONTS: &[&str] = &[
    "YSText-Regular",
    "YSText-Medium",
    "YSText-Bold",
    "YSText-Heavy",
    "YSText-Black",
    "YSText-Light",
    "YSMusic-HeadlineBold",
];

pub fn register_fonts(asar: &mut Source) -> bool {
    use slint::fontique_011::fontique;
    let mut collection = slint::fontique_011::shared_collection();
    let mut ok = false;
    for name in FONTS {
        let Some(sfnt) = asar.read(&format!("app/fonts/{name}.woff")).and_then(|w| woff_to_sfnt(&w)) else {
            log(format!("шрифт {name} не найден"));
            continue;
        };
        collection.register_fonts(fontique::Blob::new(std::sync::Arc::new(sfnt)), None);
        ok = true;
    }
    ok
}

/// Символы спрайта как отдельные SVG. `currentColor` становится белым, цвет задаёт `colorize` в Slint.
pub struct Sprite {
    src: String,
}

impl Sprite {
    pub fn load(asar: &mut Source) -> Option<Sprite> {
        let bytes = asar.read("app/icons/sprite.svg")?;
        Some(Sprite { src: String::from_utf8(bytes).ok()? })
    }

    fn element_by_id(&self, id: &str) -> Option<&str> {
        let needle = format!("id=\"{id}\"");
        let at = self.src.find(&needle)?;
        let start = self.src[..at].rfind('<')?;
        let tag: String = self.src[start + 1..].chars().take_while(|c| c.is_alphanumeric()).collect();
        let close = format!("</{tag}>");
        let head_end = start + self.src[start..].find('>')?;
        if self.src[..=head_end].ends_with("/>") {
            return Some(&self.src[start..=head_end]);
        }
        let end = head_end + self.src[head_end..].find(&close)? + close.len();
        Some(&self.src[start..end])
    }

    pub fn svg(&self, id: &str, vars: &HashMap<&str, &str>) -> Option<String> {
        let sym = self.element_by_id(id)?;
        let head_end = sym.find('>')?;
        let head = &sym[..head_end];
        let view_box = head.split("viewBox=\"").nth(1)?.split('"').next()?;
        let body = &sym[head_end + 1..sym.len() - "</symbol>".len()];
        let mut defs = String::new();
        for part in body.split("url(#").skip(1) {
            let rid = part.split(')').next().unwrap_or("");
            if !body.contains(&format!("id=\"{rid}\"")) {
                if let Some(el) = self.element_by_id(rid) {
                    defs.push_str(el);
                }
            }
        }
        let mut body = body.replace("currentColor", vars.get("currentColor").copied().unwrap_or("#fff"));
        while let Some(at) = body.find("var(--") {
            let end = at + body[at..].find(')')?;
            let inner = &body[at + 4..end];
            let (name, fallback) = inner.split_once(',').unwrap_or((inner, "#000"));
            let value = vars.get(name.trim()).copied().unwrap_or(fallback.trim()).to_string();
            body.replace_range(at..=end, &value);
        }
        // Заливка корня — как у символа: без атрибута путь заливается чёрным, как в браузере.
        let fill = if head.contains("fill=\"none\"") { " fill=\"none\"" } else { "" };
        Some(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{view_box}\"{fill}><defs>{defs}</defs>{body}</svg>"
        ))
    }

    pub fn image(&self, id: &str, vars: &HashMap<&str, &str>) -> slint::Image {
        self.svg(id, vars)
            .and_then(|s| slint::Image::load_from_svg_data(s.as_bytes()).ok())
            .unwrap_or_else(|| {
                log(format!("иконка {id} не найдена"));
                slint::Image::default()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asar_fonts_and_sprite() {
        let Some(a) = Asar::open() else { return };
        let mut a = Source::Asar(a);
        let sfnt = woff_to_sfnt(&a.read("app/fonts/YSText-Regular.woff").unwrap()).unwrap();
        assert_eq!(&sfnt[0..4], &[0, 1, 0, 0]);
        let s = Sprite::load(&mut a).unwrap();
        let svg = s.svg("addToPlaylist_xxs", &HashMap::new()).unwrap();
        assert!(svg.contains("<clipPath"));
    }
}
