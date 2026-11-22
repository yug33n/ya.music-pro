//! Яндекс Музыка RS — неофициальный клиент Яндекс Музыки. Точка входа: один экземпляр
//! на пользователя, выбор рендерера, запуск окна.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ahead;
mod api;
mod assets;
mod auth;
mod client;
mod config;
mod discord;
mod eq;
mod flac;
mod gui;
mod history;
mod i18n;
mod images;
mod landing;
#[cfg(target_os = "linux")]
mod mpris;
mod net;
mod palette;
mod player;
mod save;
mod secret;
mod stream;
#[cfg_attr(windows, path = "sys/windows.rs")]
#[cfg_attr(target_os = "linux", path = "sys/linux.rs")]
mod sys;
mod tags;
#[cfg(windows)]
mod tray;
mod update;
mod yandex_id;

// ya.music-pro specialized modules
pub mod audio;
pub mod core;
pub mod db;
pub mod dsp;
pub mod lyrics;
pub mod platform;
pub mod pro_api;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(target_os = "linux")]
    sys::audio_identity();
    // После обновления старая версия ещё закрывается: ждём её до 15 секунд.
    let after_update = std::env::args().any(|a| a == update::AFTER_UPDATE);
    let mut claimed = sys::claim_instance();
    for _ in 0..60 {
        if claimed || !after_update {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        claimed = sys::claim_instance();
    }
    if !claimed {
        // Второй запуск просто показывает уже открытое окно.
        sys::wake_existing();
        return Ok(());
    }
    // Skia без видеокарты: память не зависит от драйвера (с OpenGL или D3D окно занимает
    // в 2–3 раза больше). Режим работы («Плавный» или «Лёгкий») меняет только анимации
    // и кэши и переключается на лету (gui::apply_mode).
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name("skia-software".into()).select()?;
    // По app_id (Wayland) и WM_CLASS (X11) окружение находит .desktop и значок.
    #[cfg(target_os = "linux")]
    slint::set_xdg_app_id("yamusic-rs")?;
    update::cleanup();
    sys::refresh_autostart();
    // Без трея на Linux спрятанное окно было бы не вернуть.
    let start_hidden = cfg!(windows) && std::env::args().any(|a| a == "--tray");
    gui::run(start_hidden)
}
