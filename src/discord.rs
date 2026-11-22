//! Discord Rich Presence через локальный IPC Discord (named pipe на Windows, unix-сокет на Linux) — без сторонних библиотек.
//! Показывает «Слушает …»: трек, исполнитель, обложка, полоска прогресса, кнопка на трек.

use crate::api::Track;
use crate::config::log;
use crate::player::{Player, Status};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Приложение Discord, под которым все пользователи показывают статус «Слушает».
/// Его имя видно в профиле («Слушает …»), картинка `logo` — значок в углу обложки.
pub const APP_ID: &str = "1570815647133474856";
/// Ссылка второй кнопки статуса.
const PROJECT_URL: &str = env!("CARGO_PKG_REPOSITORY");

#[derive(Clone, Default)]
pub struct DiscordCfg {
    pub enabled: bool,
    /// Своё приложение вместо общего (для сборок-форков); пусто — `APP_ID`.
    pub app_id: String,
}

pub struct Discord {
    pub cfg: Mutex<DiscordCfg>,
    pub status: Mutex<String>,
}

#[cfg(windows)]
type Pipe = std::fs::File;
#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;

struct Conn {
    pipe: Pipe,
    nonce: u64,
}

#[cfg(windows)]
fn open_pipe(i: u32) -> Option<Pipe> {
    std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")).ok()
}

/// Сокет лежит в XDG_RUNTIME_DIR, у Flatpak и Snap — в своих подпапках.
#[cfg(unix)]
fn open_pipe(i: u32) -> Option<Pipe> {
    let dirs = ["XDG_RUNTIME_DIR", "TMPDIR"].into_iter().filter_map(std::env::var_os).map(std::path::PathBuf::from);
    dirs.chain([std::path::PathBuf::from("/tmp")])
        .flat_map(|d| ["", "app/com.discordapp.Discord", "snap.discord"].map(|sub| d.join(sub).join(format!("discord-ipc-{i}"))))
        .find_map(|p| Pipe::connect(p).ok())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Conn {
    fn connect(app_id: &str) -> Result<Conn, String> {
        let mut last = "Discord не запущен".to_string();
        for i in 0..10 {
            let Some(pipe) = open_pipe(i) else {
                continue;
            };
            let mut c = Conn { pipe, nonce: 0 };
            c.send(0, &json!({ "v": 1, "client_id": app_id }))?;
            let (op, v) = c.recv()?;
            if op == 1 && v.get("evt").and_then(|e| e.as_str()) == Some("READY") {
                return Ok(c);
            }
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
            let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            if code == 4000 || msg.to_lowercase().contains("client") {
                return Err("Discord не принял Application ID".into());
            }
            last = format!("Discord ответил: {msg}");
        }
        Err(last)
    }

    fn send(&mut self, op: u32, v: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(v).map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(8 + body.len());
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        self.pipe.write_all(&buf).map_err(|e| format!("запись в Discord: {e}"))
    }

    fn recv(&mut self) -> Result<(u32, Value), String> {
        let mut h = [0u8; 8];
        self.pipe.read_exact(&mut h).map_err(|e| format!("чтение из Discord: {e}"))?;
        let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        if len > 1 << 20 {
            return Err("слишком большой ответ Discord".into());
        }
        let mut b = vec![0u8; len];
        self.pipe.read_exact(&mut b).map_err(|e| format!("чтение из Discord: {e}"))?;
        Ok((op, serde_json::from_slice(&b).unwrap_or(Value::Null)))
    }

    /// Ok(None) — принято, Ok(Some(msg)) — Discord вернул ошибку на эту активность.
    fn set_activity(&mut self, activity: Value) -> Result<Option<String>, String> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        self.send(
            1,
            &json!({
                "cmd": "SET_ACTIVITY",
                "args": { "pid": std::process::id(), "activity": activity },
                "nonce": nonce,
            }),
        )?;
        loop {
            let (op, v) = self.recv()?;
            match op {
                2 => return Err("Discord закрыл соединение".into()),
                3 => self.send(4, &v)?, // ping → pong
                _ => {
                    if v.get("nonce").and_then(|n| n.as_str()) != Some(nonce.as_str()) {
                        continue;
                    }
                    if v.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
                        let msg = v.pointer("/data/message").and_then(|m| m.as_str()).unwrap_or("?");
                        return Ok(Some(msg.to_string()));
                    }
                    return Ok(None);
                }
            }
        }
    }
}

fn fit(s: &str, fallback: &str) -> String {
    let s = s.trim();
    let s = if s.chars().count() < 2 { fallback } else { s };
    s.chars().take(128).collect()
}

fn activity(t: &Track, start: i64, end: Option<i64>, with_display_type: bool) -> Value {
    let url = match &t.album_id {
        Some(a) => format!("https://music.yandex.ru/album/{a}/track/{}", t.id),
        None => format!("https://music.yandex.ru/track/{}", t.id),
    };
    let mut a = json!({
        "type": 2, // «Слушает»
        "details": fit(&t.title, "Без названия"),
        "state": fit(&t.artists, "Моя волна"),
        "timestamps": { "start": start },
        "buttons": [
            { "label": "Слушать в Яндекс Музыке", "url": url },
            { "label": env!("APP_NAME"), "url": PROJECT_URL },
        ],
    });
    if let Some(end) = end {
        a["timestamps"]["end"] = json!(end);
    }
    if with_display_type {
        // В списке участников: «Слушает <название трека>», а не имя приложения.
        a["status_display_type"] = json!(2);
    }
    let album = if t.album_title.is_empty() { env!("APP_NAME").to_string() } else { fit(&t.album_title, env!("APP_NAME")) };
    a["assets"] = json!({
        "large_image": t.cover_url(400).unwrap_or_else(|| "logo".into()),
        "large_text": album,
        "small_image": "logo",
        "small_text": env!("APP_NAME"),
    });
    a
}

impl Discord {
    pub fn spawn(player: Player, cfg: DiscordCfg) -> Arc<Discord> {
        let d = Arc::new(Discord { cfg: Mutex::new(cfg), status: Mutex::new(String::new()) });
        let me = d.clone();
        thread::Builder::new()
            .name("discord".into())
            .spawn(move || me.run(player))
            .expect("discord thread");
        d
    }

    fn set_status(&self, s: impl Into<String>) {
        *self.status.lock().unwrap() = s.into();
    }

    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    fn run(&self, player: Player) {
        let mut conn: Option<Conn> = None;
        let mut conn_app = String::new();
        let mut next_try = Instant::now();
        // Что сейчас показано: (id трека, время старта). None — ничего.
        let mut shown: Option<(String, i64)> = None;
        let mut display_type_ok = true;
        loop {
            thread::sleep(Duration::from_secs(1));
            let cfg = self.cfg.lock().unwrap().clone();
            let app = match cfg.app_id.trim() {
                "" => APP_ID.to_string(),
                own => own.to_string(),
            };
            if !cfg.enabled || app != conn_app {
                conn = None; // закрытие канала само убирает статус в Discord
                shown = None;
                conn_app = app.clone();
                next_try = Instant::now();
            }
            if !cfg.enabled {
                self.set_status("Выключено");
                continue;
            }
            if app.is_empty() {
                self.set_status("В этой сборке не задан Application ID");
                continue;
            }
            if conn.is_none() {
                if Instant::now() < next_try {
                    continue;
                }
                match Conn::connect(&app) {
                    Ok(c) => {
                        conn = Some(c);
                        shown = None;
                        self.set_status("Подключено, статус виден в профиле");
                    }
                    Err(e) => {
                        self.set_status(e);
                        next_try = Instant::now() + Duration::from_secs(15);
                        continue;
                    }
                }
            }
            let Some(c) = conn.as_mut() else { continue };

            let s = player.state();
            let want = if s.status == Status::Ready && s.playing && !s.loading { s.track.clone() } else { None };
            let res = match want {
                Some(t) => {
                    let start = now_ms() - (s.position * 1000.0) as i64;
                    let end = (s.duration > 1.0).then(|| start + (s.duration * 1000.0) as i64);
                    let changed = match &shown {
                        Some((id, st)) => *id != t.id || (st - start).abs() > 3000, // новый трек или перемотка
                        None => true,
                    };
                    if !changed {
                        Ok(())
                    } else {
                        let mut r = c.set_activity(activity(&t, start, end, display_type_ok));
                        if display_type_ok && matches!(&r, Ok(Some(_))) {
                            // Старый клиент Discord не знает status_display_type — повторим без него.
                            display_type_ok = false;
                            r = c.set_activity(activity(&t, start, end, false));
                        }
                        match r {
                            Ok(None) => {
                                shown = Some((t.id.clone(), start));
                                Ok(())
                            }
                            Ok(Some(msg)) => {
                                log(format!("discord SET_ACTIVITY: {msg}"));
                                shown = Some((t.id.clone(), start)); // не долбим Discord повторами
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                None if shown.is_some() => {
                    shown = None;
                    c.set_activity(Value::Null).map(|_| ())
                }
                None => Ok(()),
            };
            if let Err(e) = res {
                log(format!("discord: {e}"));
                conn = None;
                shown = None;
                self.set_status("Связь с Discord потеряна, переподключение…");
                next_try = Instant::now() + Duration::from_secs(5);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Нужен запущенный Discord: с неверным ID он должен вежливо отказать.
    #[test]
    #[ignore]
    fn handshake_rejects_bad_id() {
        let r = Conn::connect("1");
        println!("{:?}", r.as_ref().err());
        assert!(matches!(r, Err(ref e) if e.contains("Application ID")));
    }
}
