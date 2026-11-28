use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HotkeyError {
    #[error("Global hotkey manager error: {0}")]
    Manager(#[from] global_hotkey::Error),

    #[error("Action not mapped")]
    NotMapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalHotkeyAction {
    PlayPause,
    NextTrack,
    PreviousTrack,
    SeekForward10s,
    SeekBackward10s,
    ToggleLike,
    Dislike,
    NextEqPreset,
}

pub fn cyrillic_to_latin(c: char) -> char {
    match c {
        'й' | 'Й' => 'q',
        'ц' | 'Ц' => 'w',
        'у' | 'У' => 'e',
        'к' | 'К' => 'r',
        'е' | 'Е' => 't',
        'н' | 'Н' => 'y',
        'г' | 'Г' => 'u',
        'ш' | 'Ш' => 'i',
        'щ' | 'Щ' => 'o',
        'з' | 'З' => 'p',
        'х' | 'Х' => '[',
        'ъ' | 'Ъ' => ']',
        'ф' | 'Ф' => 'a',
        'ы' | 'Ы' => 's',
        'в' | 'В' => 'd',
        'а' | 'А' => 'f',
        'п' | 'П' => 'g',
        'р' | 'Р' => 'h',
        'о' | 'О' => 'j',
        'л' | 'Л' => 'k',
        'д' | 'Д' => 'l',
        'ж' | 'Ж' => ';',
        'э' | 'Э' => '\'',
        'я' | 'Я' => 'z',
        'ч' | 'Ч' => 'x',
        'с' | 'С' => 'c',
        'м' | 'М' => 'v',
        'и' | 'И' => 'b',
        'т' | 'Т' => 'n',
        'ь' | 'Ь' => 'm',
        'б' | 'Б' => ',',
        'ю' | 'Ю' => '.',
        'ё' | 'Ё' => '`',
        other => other,
    }
}

pub fn parse_hotkey_string(s: &str) -> Option<HotKey> {
    if s.trim().is_empty() {
        return None;
    }
    let normalized: String = s.chars().map(cyrillic_to_latin).collect();
    normalized.parse::<HotKey>().ok()
}

pub struct GlobalHotkeysEngine {
    manager: Option<GlobalHotKeyManager>,
    hotkey_to_action: HashMap<u32, GlobalHotkeyAction>,
    action_to_hotkey: HashMap<GlobalHotkeyAction, HotKey>,
    is_enabled: bool,
}

impl GlobalHotkeysEngine {
    pub fn new() -> Result<Self, HotkeyError> {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("Failed to initialize global hotkey manager: {}", e);
                None
            }
        };

        let mut engine = Self {
            manager,
            hotkey_to_action: HashMap::new(),
            action_to_hotkey: HashMap::new(),
            is_enabled: true,
        };

        engine.register_default_bindings();
        Ok(engine)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn unregister_all(&mut self) {
        if let Some(ref manager) = self.manager {
            for hotkey in self.action_to_hotkey.values() {
                let _ = manager.unregister(*hotkey);
            }
        }
        self.hotkey_to_action.clear();
        self.action_to_hotkey.clear();
    }

    pub fn register_default_bindings(&mut self) {
        self.update_bindings(
            "Ctrl + Space",
            "Ctrl + Right",
            "Ctrl + Left",
            "Ctrl + .",
            "Ctrl + ,",
            "Ctrl + L",
        );
    }

    pub fn update_bindings(
        &mut self,
        hk_play_pause: &str,
        hk_next: &str,
        hk_prev: &str,
        hk_forward: &str,
        hk_backward: &str,
        hk_like: &str,
    ) {
        self.unregister_all();

        let bindings = [
            (GlobalHotkeyAction::PlayPause, hk_play_pause),
            (GlobalHotkeyAction::NextTrack, hk_next),
            (GlobalHotkeyAction::PreviousTrack, hk_prev),
            (GlobalHotkeyAction::SeekForward10s, hk_forward),
            (GlobalHotkeyAction::SeekBackward10s, hk_backward),
            (GlobalHotkeyAction::ToggleLike, hk_like),
        ];

        for (action, s) in bindings {
            if let Some(hotkey) = parse_hotkey_string(s) {
                let _ = self.register_hotkey(action, hotkey);
            }
        }
    }

    pub fn register_hotkey(
        &mut self,
        action: GlobalHotkeyAction,
        hotkey: HotKey,
    ) -> Result<(), HotkeyError> {
        if let Some(ref manager) = self.manager {
            // If action was already registered, unregister previous key
            if let Some(prev) = self.action_to_hotkey.get(&action) {
                let _ = manager.unregister(*prev);
                self.hotkey_to_action.remove(&prev.id());
            }

            if let Err(e) = manager.register(hotkey) {
                log::warn!("Failed to register hotkey with OS manager: {}", e);
            }
        }

        self.hotkey_to_action.insert(hotkey.id(), action);
        self.action_to_hotkey.insert(action, hotkey);
        Ok(())
    }

    pub fn get_action_for_id(&self, id: u32) -> Option<GlobalHotkeyAction> {
        self.hotkey_to_action.get(&id).copied()
    }

    pub fn get_hotkey_for_action(&self, action: GlobalHotkeyAction) -> Option<HotKey> {
        self.action_to_hotkey.get(&action).copied()
    }

    /// Non-blocking check for triggered global hotkey events
    pub fn poll_event(&self) -> Option<GlobalHotkeyAction> {
        if !self.is_enabled {
            return None;
        }

        let mut triggered = None;
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed {
                if let Some(act) = self.get_action_for_id(event.id) {
                    triggered = Some(act);
                }
            }
        }

        triggered
    }
}

impl Default for GlobalHotkeysEngine {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            let mut engine = Self {
                manager: None,
                hotkey_to_action: HashMap::new(),
                action_to_hotkey: HashMap::new(),
                is_enabled: true,
            };
            engine.register_default_bindings();
            engine
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn test_hotkey_mapping_and_lookup() {
        let engine = GlobalHotkeysEngine::default();
        let ctrl = Some(Modifiers::CONTROL);
        let space_key = HotKey::new(ctrl, Code::Space);

        let action = engine.get_action_for_id(space_key.id());
        assert_eq!(action, Some(GlobalHotkeyAction::PlayPause));

        let retrieved_key = engine.get_hotkey_for_action(GlobalHotkeyAction::PlayPause);
        assert_eq!(retrieved_key, Some(space_key));
    }

    #[test]
    fn test_cyrillic_and_string_parsing() {
        let hk = parse_hotkey_string("Ctrl + Д").unwrap();
        assert_eq!(hk.key, Code::KeyL);
        assert_eq!(hk.mods, Modifiers::CONTROL);

        let hk2 = parse_hotkey_string("Ctrl + Right").unwrap();
        assert_eq!(hk2.key, Code::ArrowRight);

        let hk3 = parse_hotkey_string("Ctrl + .").unwrap();
        assert_eq!(hk3.key, Code::Period);
    }

    #[test]
    fn test_rebind_action() {
        let mut engine = GlobalHotkeysEngine::default();
        let ctrl_alt = Some(Modifiers::CONTROL | Modifiers::ALT);
        let new_key = HotKey::new(ctrl_alt, Code::KeyP);

        engine.register_hotkey(GlobalHotkeyAction::PlayPause, new_key).unwrap();
        assert_eq!(engine.get_action_for_id(new_key.id()), Some(GlobalHotkeyAction::PlayPause));
    }

    #[test]
    fn test_enabled_toggle() {
        let mut engine = GlobalHotkeysEngine::default();
        assert!(engine.is_enabled());
        engine.set_enabled(false);
        assert!(!engine.is_enabled());
    }
}
