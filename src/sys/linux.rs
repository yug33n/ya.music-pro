//! Linux: окно через winit, пути XDG, один экземпляр через unix-сокет, диалоги zenity/kdialog.
//! Трея пока нет, поэтому закрытие окна закрывает приложение.

use slint::winit_030::{winit, WinitWindowAccessor};
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Как app_id в main.rs и имя .desktop в пакете.
const APP_ID: &str = "yamusic-rs";

/// Имя и значок в микшере громкости. Звук идёт через ALSA, и без этого PulseAudio и PipeWire
/// показывают поток как «ALSA plug-in [yamusic-rs]». PULSE_PROP_* читает модуль pulse из
/// alsa-plugins, PIPEWIRE_PROPS — pipewire-alsa. Заданное пользователем не трогаем.
/// Вызывать первым делом в main: менять окружение, когда есть другие потоки, нельзя.
pub fn audio_identity() {
    let name = env!("APP_NAME");
    let set = |key: &str, value: &str| {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    };
    set("PULSE_PROP_application.name", name);
    set("PULSE_PROP_application.icon_name", APP_ID);
    set("PULSE_PROP_media.role", "music");
    set("PIPEWIRE_PROPS", &format!(r#"{{ application.name = "{name}" application.icon-name = {APP_ID} media.role = Music }}"#));
}

pub fn created(w: &slint::Window) -> bool {
    w.with_winit_window(|_| ()).is_some()
}

/// Значок окна для X11 (`_NET_WM_ICON`). На Wayland его берут из .desktop по app_id.
pub fn prepare(w: &slint::Window) {
    let Ok(img) = image::load_from_memory(include_bytes!("../../packaging/icons/256.png")) else { return };
    let img = img.to_rgba8();
    let (iw, ih) = img.dimensions();
    if let Ok(icon) = winit::window::Icon::from_rgba(img.into_raw(), iw, ih) {
        w.with_winit_window(|w| w.set_window_icon(Some(icon)));
    }
}

/// Wayland не даёт приложению забрать фокус без токена активации, там окно просто покажется.
pub fn raise(w: &slint::Window) {
    w.set_minimized(false);
    w.with_winit_window(|w| w.focus_window());
}

pub fn scale(w: &slint::Window) -> f32 {
    w.with_winit_window(|w| w.scale_factor() as f32).unwrap_or(1.0)
}

/// Монитор с окном целиком: панели окружения winit не знает. На Wayland монитор
/// бывает неизвестен — тогда отсчёт от размера окна.
pub fn work_area(w: &slint::Window) -> (i32, i32, i32, i32) {
    w.with_winit_window(|w| {
        let m = w.current_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some((p.x, p.y, p.x + s.width as i32, p.y + s.height as i32))
    })
    .flatten()
    .unwrap_or_else(|| {
        let s = w.size();
        (0, 0, s.width as i32, s.height as i32)
    })
}

/// Запустить диалог и вернуть выбранный путь. Диалог модальный, как и на Windows.
fn dialog(cmd: &str, args: &[&str]) -> Option<Option<String>> {
    let out = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string();
    Some((out.status.success() && !path.is_empty()).then_some(path))
}

pub fn pick_image(_: &slint::Window, title: &str) -> Option<String> {
    dialog("zenity", &["--file-selection", "--title", title, "--file-filter", "*.jpg *.jpeg *.png"])
        .or_else(|| dialog("kdialog", &["--title", title, "--getopenfilename", ".", "*.jpg *.jpeg *.png"]))
        .flatten()
}

pub fn pick_folder(_: &slint::Window, title: &str, start: &str) -> Option<String> {
    let start = format!("{}/", start.trim_end_matches('/'));
    dialog("zenity", &["--file-selection", "--directory", "--title", title, "--filename", &start])
        .or_else(|| dialog("kdialog", &["--title", title, "--getexistingdirectory", &start]))
        .flatten()
}

pub fn open_url(url: &str) {
    if let Ok(mut child) = Command::new("xdg-open").arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

pub fn trim_memory() {
    #[cfg(target_env = "gnu")]
    unsafe {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        malloc_trim(0);
    }
}

pub fn release_memory() {
    trim_memory();
}

/// Тема из настроек окружения через xdg-desktop-portal: 1 — тёмная, 2 — светлая,
/// 0 — без предпочтения. Портала нет — считаем светлой, как Windows без ключа в реестре.
pub fn system_light_theme() -> bool {
    let out = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .stderr(Stdio::null())
        .output();
    !matches!(out, Ok(o) if String::from_utf8_lossy(&o.stdout).contains("uint32 1"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

pub fn config_base() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn cache_base() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// Папка «Музыка» из user-dirs.dirs: в русской локали это ~/Музыка.
pub fn music_dir() -> PathBuf {
    let dirs = std::fs::read_to_string(config_base().join("user-dirs.dirs")).unwrap_or_default();
    dirs.lines()
        .find_map(|l| l.strip_prefix("XDG_MUSIC_DIR="))
        .map(|v| v.trim().trim_matches('"').replace("$HOME", &home().to_string_lossy()))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join("Music"))
}

pub fn hostname() -> Option<String> {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    Some(name.trim().to_string()).filter(|n| !n.is_empty())
}

thread_local! {
    /// На X11 текст живёт в буфере, пока жив его владелец, поэтому буфер держим всё время.
    static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

pub fn copy_text(text: &str) {
    CLIPBOARD.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = arboard::Clipboard::new().ok();
        }
        if let Some(c) = c.as_mut() {
            let _ = c.set_text(text);
        }
    });
}

fn autostart_file() -> PathBuf {
    config_base().join("autostart").join(format!("{APP_ID}.desktop"))
}

pub fn autostart_enabled() -> bool {
    autostart_file().exists()
}

/// Запись в ~/.config/autostart. Без трея запускаемся с окном, а не скрытыми.
pub fn set_autostart(on: bool) -> bool {
    let file = autostart_file();
    if !on {
        return std::fs::remove_file(&file).is_ok() || !file.exists();
    }
    // Из AppImage current_exe указывает внутрь временного монтирования, а сам файл
    // рантайм AppImage кладёт в $APPIMAGE.
    let Some(exe) = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()) else { return false };
    let entry = format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nIcon={APP_ID}\n", env!("APP_NAME"), exe.display());
    file.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&file, entry).is_ok()
}

pub fn refresh_autostart() {
    if autostart_enabled() {
        set_autostart(true);
    }
}

fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join(format!("{APP_ID}.sock")),
        None => std::env::temp_dir().join(format!("{APP_ID}-{}.sock", std::env::var("USER").unwrap_or_default())),
    }
}

static LISTENER: Mutex<Option<UnixListener>> = Mutex::new(None);

/// Занять сокет единственного экземпляра. Сокет остаётся на диске и после падения,
/// поэтому занятый, но молчащий файл считается брошенным.
pub fn claim_instance() -> bool {
    let path = socket_path();
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return false;
            }
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(_) => return true,
            }
        }
        // Нет доступа к папке — работаем без проверки второго запуска.
        Err(_) => return true,
    };
    *LISTENER.lock().unwrap() = Some(listener);
    true
}

/// Второй запуск: попросить работающий экземпляр показать окно.
pub fn wake_existing() {
    use std::io::Write;
    if let Ok(mut s) = UnixStream::connect(socket_path()) {
        let _ = s.write_all(b"show");
    }
}

/// Звать `f` из фонового потока, когда второй запуск просит показать окно.
/// Пустое соединение — это проверка из `claim_instance`, на неё не реагируем.
pub fn on_wake(f: impl Fn() + Send + 'static) {
    let Some(listener) = LISTENER.lock().unwrap().take() else { return };
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4];
            if s.read(&mut buf).is_ok_and(|n| n > 0) {
                f();
            }
        }
    });
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
