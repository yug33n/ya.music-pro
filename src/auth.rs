//! Вход в аккаунт Яндекса по коду устройства: приложение показывает код,
//! пользователь вводит его на ya.ru/device в любом браузере, приложение получает токен.

use crate::client::{OAUTH_BASE, OAUTH_CLIENT_ID, OAUTH_CLIENT_SECRET};
use serde_json::Value;
use std::time::Duration;

/// Права токена, которые просим при входе.
const SCOPE: &str = "music:content music:read music:write";

pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub struct Token {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_in: Option<u64>,
}

pub enum Poll {
    /// Пользователь ещё не ввёл код.
    Pending,
    Done(Token),
    /// Код истёк или вход отклонён — нужен новый код.
    Failed(String),
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

fn post(form: &[(&str, &str)], path: &str) -> Result<Value, String> {
    let resp = client().post(format!("{OAUTH_BASE}{path}")).form(form).send().map_err(|e| format!("сеть: {}", crate::config::chain(&e)))?;
    let text = resp.text().map_err(|e| format!("сеть: {}", crate::config::chain(&e)))?;
    serde_json::from_str(&text).map_err(|_| format!("ответ OAuth: {}", text.chars().take(200).collect::<String>()))
}

/// Постоянный id устройства: Яндекс показывает его в списке устройств аккаунта.
pub fn device_id() -> String {
    let path = crate::config::dir().join("device");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if id.len() == 32 {
            return id;
        }
    }
    let seed = format!("{:?}{}{}", std::time::SystemTime::now(), std::process::id(), crate::sys::hostname().unwrap_or_default());
    let id: String = {
        use md5::{Digest, Md5};
        Md5::digest(seed.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
    };
    let _ = std::fs::create_dir_all(crate::config::dir());
    let _ = std::fs::write(&path, &id);
    id
}

pub fn device_code() -> Result<DeviceCode, String> {
    let id = device_id();
    let host = crate::sys::hostname().unwrap_or_else(|| "PC".into());
    // Это имя Яндекс показывает при вводе кода и в списке устройств аккаунта.
    let name = format!("{} ({host})", env!("APP_NAME"));
    let v = post(
        &[("client_id", OAUTH_CLIENT_ID), ("device_id", &id), ("device_name", &name), ("scope", SCOPE)],
        "/device/code",
    )?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let n = |k: &str| v.get(k).and_then(|x| x.as_u64());
    match (s("device_code"), s("user_code")) {
        (Some(device_code), Some(user_code)) => Ok(DeviceCode {
            device_code,
            user_code,
            interval: n("interval").unwrap_or(5).max(1),
            expires_in: n("expires_in").unwrap_or(300),
        }),
        _ => Err(s("error_description").unwrap_or_else(|| "Яндекс не выдал код входа".into())),
    }
}

fn token_from(v: &Value) -> Option<Token> {
    Some(Token {
        access: v.get("access_token")?.as_str()?.to_string(),
        refresh: v.get("refresh_token").and_then(|x| x.as_str()).map(String::from),
        expires_in: v.get("expires_in").and_then(|x| x.as_u64()),
    })
}

pub fn poll(device_code: &str) -> Poll {
    let v = match post(
        &[
            ("grant_type", "device_code"),
            ("code", device_code),
            ("client_id", OAUTH_CLIENT_ID),
            ("client_secret", OAUTH_CLIENT_SECRET),
        ],
        "/token",
    ) {
        Ok(v) => v,
        // Сеть мигнула — спросим ещё раз на следующем шаге.
        Err(_) => return Poll::Pending,
    };
    if let Some(t) = token_from(&v) {
        return Poll::Done(t);
    }
    match v.get("error").and_then(|e| e.as_str()) {
        Some("authorization_pending") | Some("slow_down") => Poll::Pending,
        _ => Poll::Failed(
            v.get("error_description").and_then(|e| e.as_str()).unwrap_or("вход не удался").to_string(),
        ),
    }
}

/// Отозвать токен на сервере. Яндекс отзывает только токены, выданные с device_id.
pub fn revoke(access: &str) -> Result<(), String> {
    let v = post(
        &[("access_token", access), ("client_id", OAUTH_CLIENT_ID), ("client_secret", OAUTH_CLIENT_SECRET)],
        "/revoke_token",
    )?;
    if v.get("status").and_then(|s| s.as_str()) == Some("ok") {
        Ok(())
    } else {
        Err(v.get("error_description").and_then(|e| e.as_str()).unwrap_or("Яндекс не отозвал токен").to_string())
    }
}

/// Продлить токен по refresh_token, пока старый не истёк.
pub fn refresh(refresh_token: &str) -> Result<Token, String> {
    let v = post(
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OAUTH_CLIENT_ID),
            ("client_secret", OAUTH_CLIENT_SECRET),
        ],
        "/token",
    )?;
    token_from(&v).ok_or_else(|| v.get("error_description").and_then(|e| e.as_str()).unwrap_or("не удалось продлить вход").to_string())
}
