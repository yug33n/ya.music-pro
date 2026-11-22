//! Как приложение представляется серверам Яндекса.
//!
//! Всё, что зависит от чужих клиентов и может поменяться на стороне Яндекса, собрано здесь:
//! OAuth-приложение, ключи подписи, заголовки клиента. Если что-то сломается после обновления
//! сервиса, править нужно только этот файл.

use std::sync::OnceLock;

/// Адрес API Музыки.
pub const API_BASE: &str = "https://api.music.yandex.net";

/// OAuth-приложение Яндекс Музыки для Android: им пользуются сторонние клиенты.
/// Вход — по коду устройства на ya.ru/device.
pub const OAUTH_CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
pub const OAUTH_CLIENT_SECRET: &str = "53bc75238f0c4d08a118e51fe9203300";
pub const OAUTH_BASE: &str = "https://oauth.yandex.ru";

/// Соль ссылки на mp3 из download-info.
pub const MP3_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";
/// Ключ подписи запросов get-file-info и текстов песен.
pub const SIGN_KEY: &str = "p93jhgh689SBReK6ghtw62";

/// Заголовок клиента для старых методов (стрим, лайки, очередь).
pub const ANDROID_CLIENT: &str = "YandexMusicAndroid/24023621";
/// Версия десктопного клиента, если официальное приложение не установлено.
const DESKTOP_VERSION_FALLBACK: &str = "5.126.1";

/// Заголовок десктопного клиента для лендингов и блоков. Версия берётся из установленного
/// официального приложения, чтобы не устаревать вместе с ним.
pub fn desktop_client() -> &'static str {
    static CLIENT: OnceLock<String> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let version = crate::assets::Asar::open()
            .and_then(|mut a| a.read("package.json"))
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("version")?.as_str().map(String::from))
            .filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
            .unwrap_or_else(|| DESKTOP_VERSION_FALLBACK.to_string());
        format!("YandexMusicDesktopAppWindows/{version}")
    })
}

/// Заголовок `X-Yandex-Music-Device` в формате десктопного клиента:
/// `manufacturer=; model=; uuid=…; os=win32; os_version=; device_id=…; clid=0`.
/// uuid и device_id выводятся из постоянного id устройства (файл `device`), поэтому
/// между запусками не меняются.
/// Версия Android-клиента 2026.11.2: под неё сервер отдаёт нынешние страницы.
#[cfg(target_os = "android")]
pub const ANDROID_APP_CLIENT: &str = "YandexMusicAndroid/24026572";

#[cfg(target_os = "android")]
pub fn mobile_device_header() -> &'static str {
    static HEADER: OnceLock<String> = OnceLock::new();
    HEADER.get_or_init(|| device_header().replace("os=win32; os_version=", "os=Android; os_version=15"))
}

pub fn device_header() -> &'static str {
    static HEADER: OnceLock<String> = OnceLock::new();
    HEADER.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let id = crate::auth::device_id();
        let uuid = format!("{}-{}-{}-{}-{}", &id[..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32]);
        let device: String = Sha256::digest(id.as_bytes()).iter().map(|x| format!("{x:02x}")).collect();
        format!("manufacturer=; model=; uuid={uuid}; os=win32; os_version=; device_id={device}; clid=0")
    })
}

/// Токен уходит только по https и только на хосты Яндекса.
pub fn token_allowed(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    ["yandex.net", "yandex.ru"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Адрес, который можно отдать системе (ShellExecute, xdg-open), если он пришёл с сервера:
/// только https с хостом и без логина в адресе (`https://music.yandex.ru@evil.com` ведёт на
/// evil.com). `file:`, UNC-пути и обработчики протоколов (`ms-msdt:`, `search-ms:`) через
/// ShellExecute запускают программы, поэтому всё остальное отбрасывается.
pub fn browser_url(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    let ok = u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()) && u.username().is_empty() && u.password().is_none();
    ok.then(|| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_urls() {
        assert_eq!(browser_url("https://music.yandex.ru/promo").as_deref(), Some("https://music.yandex.ru/promo"));
        assert!(browser_url(" https://github.com/a/b/releases/tag/v1.2.0 ").is_some());
        assert!(browser_url("http://music.yandex.ru/").is_none());
        assert!(browser_url("file:///C:/Windows/System32/calc.exe").is_none());
        assert!(browser_url(r"\\evil.com\share\x.exe").is_none());
        assert!(browser_url("ms-msdt:/id PCWDiagnostic").is_none());
        assert!(browser_url("search-ms:query=x").is_none());
        assert!(browser_url("javascript:alert(1)").is_none());
        assert!(browser_url("httpx://music.yandex.ru/").is_none());
        assert!(browser_url("https://music.yandex.ru@evil.com/").is_none());
        assert!(browser_url("").is_none());
    }

    #[test]
    fn hosts() {
        let ok = |s: &str| token_allowed(&reqwest::Url::parse(s).unwrap());
        assert!(ok("https://api.music.yandex.net/account/status"));
        assert!(ok("https://storage.mds.yandex.net/download-info/1"));
        assert!(!ok("http://api.music.yandex.net/"));
        assert!(!ok("https://evil.com/yandex.net"));
        assert!(!ok("https://yandex.net.evil.com/"));
        assert!(!ok("https://notyandex.ru/"));
    }
}
