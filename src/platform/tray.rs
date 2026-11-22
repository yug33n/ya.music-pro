use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TrayError {
    #[error("Tray icon error: {0}")]
    Icon(#[from] tray_icon::Error),

    #[error("Failed to build icon bitmap")]
    BitmapError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEventAction {
    RestoreWindow,
    MinimizeToTray,
    PlayPause,
    NextTrack,
    PreviousTrack,
    Quit,
}

pub struct HeadlessTrayMode {
    tray_icon: Option<TrayIcon>,
    is_headless: bool,
    restore_id: tray_icon::menu::MenuId,
    play_pause_id: tray_icon::menu::MenuId,
    next_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
}

impl HeadlessTrayMode {
    pub fn new() -> Result<Self, TrayError> {
        let menu = Menu::new();

        let restore_item = MenuItem::new("Открыть ya.music-pro", true, None);
        let play_pause_item = MenuItem::new("Воспроизведение / Пауза", true, None);
        let next_item = MenuItem::new("Следующий трек", true, None);
        let quit_item = MenuItem::new("Выход", true, None);

        let restore_id = restore_item.id().clone();
        let play_pause_id = play_pause_item.id().clone();
        let next_id = next_item.id().clone();
        let quit_id = quit_item.id().clone();

        let _ = menu.append(&restore_item);
        let _ = menu.append(&play_pause_item);
        let _ = menu.append(&next_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit_item);

        let icon = Self::generate_app_icon()?;

        let tray_icon = match TrayIconBuilder::new()
            .with_tooltip("ya.music-pro — Ultra-light Native Client")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
        {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("Failed to initialize tray icon: {}", e);
                None
            }
        };

        Ok(Self {
            tray_icon,
            is_headless: false,
            restore_id,
            play_pause_id,
            next_id,
            quit_id,
        })
    }

    /// Generate crisp 32x32 RGBA Yandex Music yellow circular icon
    pub fn generate_app_icon() -> Result<Icon, TrayError> {
        const SIZE: u32 = 32;
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        let radius = (SIZE as f32 / 2.0) - 1.0;
        let center = SIZE as f32 / 2.0;

        for y in 0..SIZE {
            for x in 0..SIZE {
                let dx = x as f32 - center + 0.5;
                let dy = y as f32 - center + 0.5;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist <= radius {
                    // Yandex Yellow: #FFCC00 (255, 204, 0)
                    rgba.push(255);
                    rgba.push(204);
                    rgba.push(0);
                    rgba.push(255);
                } else if dist <= radius + 0.8 {
                    // Anti-aliased outer edge
                    let alpha = ((1.0 - (dist - radius) / 0.8) * 255.0) as u8;
                    rgba.push(255);
                    rgba.push(204);
                    rgba.push(0);
                    rgba.push(alpha);
                } else {
                    // Transparent
                    rgba.push(0);
                    rgba.push(0);
                    rgba.push(0);
                    rgba.push(0);
                }
            }
        }

        Icon::from_rgba(rgba, SIZE, SIZE).map_err(|_| TrayError::BitmapError)
    }

    pub fn set_headless(&mut self, headless: bool) {
        self.is_headless = headless;
        if let Some(ref tray) = self.tray_icon {
            let tooltip = if headless {
                "ya.music-pro (Режим «Ультра-лайт» ~15 МБ ОЗУ) — нажмите для открытия"
            } else {
                "ya.music-pro — Аудиофильский плеер"
            };
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }

    pub fn is_headless(&self) -> bool {
        self.is_headless
    }

    pub fn update_track_tooltip(&self, title: &str, artists: &str) {
        if let Some(ref tray) = self.tray_icon {
            let tip = format!("ya.music-pro: {} — {}", title, artists);
            let _ = tray.set_tooltip(Some(tip));
        }
    }

    /// Check for incoming tray mouse clicks or context menu selections
    pub fn poll_tray_events(&self) -> Option<TrayEventAction> {
        // 1. Check direct tray icon left click (restore/toggle)
        if let Ok(TrayIconEvent::Click { button, button_state, .. }) = TrayIconEvent::receiver().try_recv() {
            if button == MouseButton::Left && button_state == MouseButtonState::Up {
                return Some(TrayEventAction::RestoreWindow);
            }
        }

        // 2. Check context menu actions
        if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
            if menu_event.id == self.restore_id {
                return Some(TrayEventAction::RestoreWindow);
            } else if menu_event.id == self.play_pause_id {
                return Some(TrayEventAction::PlayPause);
            } else if menu_event.id == self.next_id {
                return Some(TrayEventAction::NextTrack);
            } else if menu_event.id == self.quit_id {
                return Some(TrayEventAction::Quit);
            }
        }

        None
    }
}

impl Default for HeadlessTrayMode {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            tray_icon: None,
            is_headless: false,
            restore_id: tray_icon::menu::MenuId::new("restore"),
            play_pause_id: tray_icon::menu::MenuId::new("play_pause"),
            next_id: tray_icon::menu::MenuId::new("next"),
            quit_id: tray_icon::menu::MenuId::new("quit"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_icon_generation() {
        let icon_res = HeadlessTrayMode::generate_app_icon();
        assert!(icon_res.is_ok());
    }

    #[test]
    fn test_headless_state_toggle() {
        let mut tray = HeadlessTrayMode::default();
        assert!(!tray.is_headless());
        tray.set_headless(true);
        assert!(tray.is_headless());
        tray.set_headless(false);
        assert!(!tray.is_headless());
    }

    #[test]
    fn test_tray_event_action_variants() {
        let act1 = TrayEventAction::RestoreWindow;
        let act2 = TrayEventAction::PlayPause;
        let act3 = TrayEventAction::Quit;
        assert_ne!(act1, act2);
        assert_ne!(act2, act3);
    }
}
