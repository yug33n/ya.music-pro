//! Настройки (config.json), папка данных приложения и лог.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Токен OAuth в памяти. На диске лежит только зашифрованным (`token_sealed`).
    #[serde(skip)]
    pub token: Option<String>,
    #[serde(skip)]
    pub refresh_token: Option<String>,
    /// Токены под DPAPI или в связке ключей Linux (src/secret.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    token_sealed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_sealed: Option<String>,
    /// Токен открытым текстом: старый формат или Linux без связки ключей. Как только спрятать есть
    /// куда, при запуске переносится туда.
    #[serde(rename = "token", skip_serializing_if = "Option::is_none")]
    token_plain: Option<String>,
    /// Unix-время, когда токен истечёт.
    pub token_expires: Option<u64>,
    pub volume: f32,
    pub discord_enabled: bool,
    pub discord_app_id: String,
    /// system / light / dark
    pub theme: String,
    /// 0 превосходное, 1 оптимальное, 2 экономичное
    pub quality: u8,
    pub eq_on: bool,
    pub eq_preset: usize,
    pub eq_preamp: f32,
    pub eq_gains: [f32; 10],
    /// Язык интерфейса Яндекса: ru, en, uz, kk — уходит в Accept-Language.
    pub lang: String,
    pub navbar_collapsed: bool,
    /// smooth — все анимации, lite — без анимаций и с урезанными кэшами.
    pub mode: String,
    /// Куда скачивать треки. Пусто — Музыка\Яндекс Музыка.
    pub download_dir: String,
    /// Скачивать FLAC, если он есть у трека.
    pub download_flac: bool,
    /// Громкость в децибелах, а не квадратом положения ползунка.
    pub log_volume: bool,
    /// Блоки главной и «Книг и подкастов», которые скрыл пользователь.
    pub hidden_blocks: Vec<HiddenBlock>,
    /// Раз в полчаса спрашивать GitHub о новой версии.
    pub check_updates: bool,
    /// «Треки 18+» в меню профиля.
    pub allow_explicit: bool,
    /// Добавлять треки в конец плейлиста, а не в начало.
    pub add_to_end: bool,
    /// Обычное окно при выходе, в пикселях экрана. На Wayland места окна не узнать.
    pub window_pos: Option<[i32; 2]>,
    pub window_size: Option<[u32; 2]>,
    pub window_maximized: bool,
    /// Место мини-плеера при выходе.
    pub mini_pos: Option<[i32; 2]>,
    #[serde(default = "default_hk_play_pause")]
    pub hk_play_pause: String,
    #[serde(default = "default_hk_next")]
    pub hk_next: String,
    #[serde(default = "default_hk_prev")]
    pub hk_prev: String,
    #[serde(default = "default_hk_forward")]
    pub hk_forward: String,
    #[serde(default = "default_hk_backward")]
    pub hk_backward: String,
    #[serde(default = "default_hk_like")]
    pub hk_like: String,
}

pub fn default_hk_play_pause() -> String { "".into() }
pub fn default_hk_next() -> String { "Ctrl + Right".into() }
pub fn default_hk_prev() -> String { "Ctrl + Left".into() }
pub fn default_hk_forward() -> String { "Ctrl + Shift + Right".into() }
pub fn default_hk_backward() -> String { "Ctrl + Shift + Left".into() }
pub fn default_hk_like() -> String { "Ctrl + L".into() }

/// Блок запоминается по id из скелета: он постоянный («recommended_playlists»), а заголовок
/// меняется («Летняя» → «Осенняя»). Заголовок — только чтобы показать список в настройках.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct HiddenBlock {
    pub id: String,
    pub title: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            token: None,
            refresh_token: None,
            token_sealed: None,
            refresh_sealed: None,
            token_plain: None,
            token_expires: None,
            volume: 0.6,
            discord_enabled: true,
            discord_app_id: String::new(),
            theme: "system".into(),
            quality: 0,
            eq_on: false,
            eq_preset: 1,
            eq_preamp: 1.0,
            eq_gains: [0.0; 10],
            lang: "ru".into(),
            navbar_collapsed: false,
            mode: "smooth".into(),
            download_dir: String::new(),
            download_flac: false,
            log_volume: false,
            hidden_blocks: Vec::new(),
            check_updates: true,
            allow_explicit: true,
            add_to_end: false,
            window_pos: None,
            window_size: None,
            window_maximized: false,
            mini_pos: None,
            hk_play_pause: default_hk_play_pause(),
            hk_next: default_hk_next(),
            hk_prev: default_hk_prev(),
            hk_forward: default_hk_forward(),
            hk_backward: default_hk_backward(),
            hk_like: default_hk_like(),
        }
    }
}

/// %APPDATA%\yamusic-rs или ~/.config/yamusic-rs. Папка прежнего названия (yamusic-lite, ещё раньше yawave) переносится
/// молча, вместе с токеном и настройками. Если её держит запущенная старая версия, этот запуск
/// работает со старой папкой, а перенос повторится в следующий раз.
pub fn dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = crate::sys::config_base();
        let dir = base.join("yamusic-rs");
        if !dir.exists() {
            if let Some(old) = ["yamusic-lite", "yawave"].iter().map(|n| base.join(n)).find(|p| p.exists()) {
                if std::fs::rename(&old, &dir).is_err() {
                    return old;
                }
            }
        }
        dir
    })
    .clone()
}

/// Скачанные шрифты и значки. На Windows лежат рядом с настройками, на Linux — в ~/.cache.
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    return dir();
    #[cfg(not(windows))]
    return crate::sys::cache_base().join("yamusic-rs");
}

impl Config {
    pub fn load() -> Self {
        let mut c: Config = std::fs::read_to_string(dir().join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        c.token = c.token_sealed.as_deref().and_then(crate::secret::open);
        c.refresh_token = c.refresh_sealed.as_deref().and_then(crate::secret::open);
        if c.token.is_none() && c.token_sealed.is_some() {
            log("сохранённый токен не прочитался (другая учётная запись, связка ключей заперта или в ней нет записи?) — нужно войти заново");
        }
        if let Some(plain) = c.token_plain.clone().filter(|t| !t.trim().is_empty()) {
            if c.token.is_none() {
                c.token = Some(plain.trim().to_string());
            }
            // Открытый токен с диска убираем сразу.
            c.save();
        }
        c
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        let mut disk = self.clone();
        // Токен не прочитался (связка ключей заперта) — запись о нём не трогаем, иначе после
        // разблокировки входить придётся заново. Стирают её только forget_token и forget_refresh.
        if let Some(t) = &self.token {
            disk.token_sealed = crate::secret::seal("token", t);
        }
        if let Some(t) = &self.refresh_token {
            disk.refresh_sealed = crate::secret::seal("refresh", t);
        }
        // Спрятать некуда (Linux без связки ключей) — открытым текстом, в файле только для владельца.
        disk.token_plain = if disk.token_sealed.is_none() { self.token.clone() } else { None };
        if let Ok(s) = serde_json::to_string_pretty(&disk) {
            let path = dir().join("config.json");
            let tmp = dir().join("config.json.tmp");
            if write_private(&tmp, s.as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// Выход или токен отозван: убрать и из памяти, и с диска.
    pub fn forget_token(&mut self) {
        self.token = None;
        self.token_sealed = None;
        crate::secret::forget("token");
    }

    pub fn forget_refresh(&mut self) {
        self.refresh_token = None;
        self.refresh_sealed = None;
        crate::secret::forget("refresh");
    }
}

/// На Linux без связки ключей токен лежит открытым текстом, поэтому файл сразу создаётся
/// только для владельца, а не получает права после записи.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
        // Права задаются только новому файлу, у оставшегося старого .tmp они бы не поменялись.
        let _ = std::fs::remove_file(path);
    }
    o.open(path)?.write_all(data)
}

/// Ошибка вместе с причинами. reqwest сам пишет только «error sending request», а что
/// случилось на самом деле (DNS, TLS, прокси, таймаут), лежит в source().
pub fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(x) = src {
        s.push_str(": ");
        s.push_str(&x.to_string());
        src = x.source();
    }
    s
}

/// Лог в log.txt папки настроек.
pub fn log(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    #[cfg(debug_assertions)]
    eprintln!("{msg}");
    let _ = std::fs::create_dir_all(dir());
    let path = dir().join("log.txt");
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{t}] {msg}");
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

