use global_hotkey::hotkey::{Code, HotKey, Modifiers};
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

    pub fn register_default_bindings(&mut self) {
        let ctrl_alt = Some(Modifiers::CONTROL | Modifiers::ALT);

        let defaults = [
            (GlobalHotkeyAction::PlayPause, HotKey::new(ctrl_alt, Code::Space)),
            (GlobalHotkeyAction::NextTrack, HotKey::new(ctrl_alt, Code::ArrowRight)),
            (GlobalHotkeyAction::PreviousTrack, HotKey::new(ctrl_alt, Code::ArrowLeft)),
            (GlobalHotkeyAction::SeekForward10s, HotKey::new(ctrl_alt, Code::Period)),
            (GlobalHotkeyAction::SeekBackward10s, HotKey::new(ctrl_alt, Code::Comma)),
            (GlobalHotkeyAction::ToggleLike, HotKey::new(ctrl_alt, Code::KeyL)),
            (GlobalHotkeyAction::Dislike, HotKey::new(ctrl_alt, Code::KeyD)),
            (GlobalHotkeyAction::NextEqPreset, HotKey::new(ctrl_alt, Code::KeyE)),
        ];

        for (action, hotkey) in defaults {
            let _ = self.register_hotkey(action, hotkey);
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

            manager.register(hotkey)?;
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

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Released {
                return self.get_action_for_id(event.id);
            }
        }

        None
    }
}

impl Default for GlobalHotkeysEngine {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            manager: None,
            hotkey_to_action: HashMap::new(),
            action_to_hotkey: HashMap::new(),
            is_enabled: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotkey_mapping_and_lookup() {
        let engine = GlobalHotkeysEngine::default();
        let ctrl_alt = Some(Modifiers::CONTROL | Modifiers::ALT);
        let space_key = HotKey::new(ctrl_alt, Code::Space);

        let action = engine.get_action_for_id(space_key.id());
        assert_eq!(action, Some(GlobalHotkeyAction::PlayPause));

        let retrieved_key = engine.get_hotkey_for_action(GlobalHotkeyAction::PlayPause);
        assert_eq!(retrieved_key, Some(space_key));
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
