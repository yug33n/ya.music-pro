use std::collections::HashSet;
use thiserror::Error;

#[cfg(target_os = "windows")]
use windows::core::{Interface, PWSTR};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::CloseHandle;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::{
    IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2,
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Error, Debug)]
pub enum SessionMonitorError {
    #[error("COM initialization error: {0}")]
    Com(String),

    #[error("Audio endpoint error: {0}")]
    Endpoint(String),

    #[error("Session enumeration error: {0}")]
    Enumeration(String),

    #[error("Unsupported platform")]
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeterInfo {
    pub pid: u32,
    pub process_name: String,
    pub peak_level: f32,
    pub is_voice_active: bool,
}

pub struct WindowsSessionMonitor {
    monitored_processes: HashSet<String>,
    peak_threshold: f32,
    is_com_initialized: bool,
}

impl WindowsSessionMonitor {
    pub const DEFAULT_THRESHOLD: f32 = 0.02;

    pub fn new() -> Self {
        let mut monitor = Self {
            monitored_processes: HashSet::new(),
            peak_threshold: Self::DEFAULT_THRESHOLD,
            is_com_initialized: false,
        };

        // Add standard VoIP and team chat executables
        monitor.add_monitored_process("discord.exe");
        monitor.add_monitored_process("telegram.exe");
        monitor.add_monitored_process("slack.exe");
        monitor.add_monitored_process("teams.exe");
        monitor.add_monitored_process("skype.exe");

        monitor
    }

    pub fn add_monitored_process(&mut self, process_name: &str) {
        self.monitored_processes.insert(process_name.to_lowercase());
    }

    pub fn remove_monitored_process(&mut self, process_name: &str) {
        self.monitored_processes.remove(&process_name.to_lowercase());
    }

    pub fn is_monitored(&self, process_name: &str) -> bool {
        let lower = process_name.to_lowercase();
        self.monitored_processes.iter().any(|p| lower.ends_with(p))
    }

    pub fn set_peak_threshold(&mut self, threshold: f32) {
        self.peak_threshold = threshold.clamp(0.001, 1.0);
    }

    pub fn peak_threshold(&self) -> f32 {
        self.peak_threshold
    }

    pub fn monitored_processes(&self) -> Vec<String> {
        self.monitored_processes.iter().cloned().collect()
    }

    /// Poll all current Core Audio sessions and return active monitored voice sessions
    pub fn poll_active_voice_sessions(&mut self) -> Result<Vec<SessionMeterInfo>, SessionMonitorError> {
        #[cfg(target_os = "windows")]
        {
            if !self.is_com_initialized {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                self.is_com_initialized = true;
            }

            unsafe {
                let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| SessionMonitorError::Endpoint(format!("Failed to create MMDeviceEnumerator: {}", e)))?;

                let device: IMMDevice = enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
                    .map_err(|e| SessionMonitorError::Endpoint(format!("Failed to get default audio endpoint: {}", e)))?;

                let session_manager: IAudioSessionManager2 = device
                    .Activate(CLSCTX_ALL, None)
                    .map_err(|e| SessionMonitorError::Endpoint(format!("Failed to activate IAudioSessionManager2: {}", e)))?;

                let session_enum: IAudioSessionEnumerator = session_manager
                    .GetSessionEnumerator()
                    .map_err(|e| SessionMonitorError::Enumeration(format!("Failed to get session enumerator: {}", e)))?;

                let session_count = session_enum
                    .GetCount()
                    .map_err(|e| SessionMonitorError::Enumeration(format!("Failed to get session count: {}", e)))?;

                let mut active_sessions = Vec::new();

                for i in 0..session_count {
                    let session_control = match session_enum.GetSession(i) {
                        Ok(sc) => sc,
                        Err(_) => continue,
                    };

                    let control2: IAudioSessionControl2 = match session_control.cast() {
                        Ok(c2) => c2,
                        Err(_) => continue,
                    };

                    let pid = match control2.GetProcessId() {
                        Ok(id) => id,
                        Err(_) => continue,
                    };

                    if pid == 0 {
                        continue;
                    }

                    if let Some(exe_name) = get_process_image_name(pid) {
                        if self.is_monitored(&exe_name) {
                            let meter: Result<IAudioMeterInformation, _> = session_control.cast();
                            if let Ok(m) = meter {
                                if let Ok(peak) = m.GetPeakValue() {
                                    active_sessions.push(SessionMeterInfo {
                                        pid,
                                        process_name: exe_name,
                                        peak_level: peak,
                                        is_voice_active: peak >= self.peak_threshold,
                                    });
                                }
                            }
                        }
                    }
                }

                Ok(active_sessions)
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(Vec::new())
        }
    }

    /// Check if any voice activity is currently detected on monitored VoIP processes
    pub fn is_voice_active(&mut self) -> bool {
        match self.poll_active_voice_sessions() {
            Ok(sessions) => sessions.iter().any(|s| s.is_voice_active),
            Err(e) => {
                log::debug!("Session monitor polling warning: {}", e);
                false
            }
        }
    }
}

impl Default for WindowsSessionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
fn get_process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 512];
        let mut size = buffer.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);

        if res.is_ok() && size > 0 {
            let full_path = String::from_utf16_lossy(&buffer[..size as usize]);
            let file_name = std::path::Path::new(&full_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
                .unwrap_or(full_path);
            Some(file_name)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_initialization_and_defaults() {
        let monitor = WindowsSessionMonitor::new();
        assert!(monitor.is_monitored("Discord.exe"));
        assert!(monitor.is_monitored("C:\\Users\\test\\AppData\\Local\\Discord\\app-1.0\\Discord.exe"));
        assert!(monitor.is_monitored("Telegram.exe"));
        assert!(!monitor.is_monitored("notepad.exe"));
        assert!((monitor.peak_threshold() - WindowsSessionMonitor::DEFAULT_THRESHOLD).abs() < 1e-4);
    }

    #[test]
    fn test_add_remove_monitored_process() {
        let mut monitor = WindowsSessionMonitor::new();
        monitor.add_monitored_process("custom_voip.exe");
        assert!(monitor.is_monitored("custom_voip.exe"));
        assert!(monitor.is_monitored("CUSTOM_VOIP.EXE"));

        monitor.remove_monitored_process("custom_voip.exe");
        assert!(!monitor.is_monitored("custom_voip.exe"));
    }

    #[test]
    fn test_threshold_clamping() {
        let mut monitor = WindowsSessionMonitor::new();
        monitor.set_peak_threshold(-0.5);
        assert_eq!(monitor.peak_threshold(), 0.001);

        monitor.set_peak_threshold(2.5);
        assert_eq!(monitor.peak_threshold(), 1.0);

        monitor.set_peak_threshold(0.05);
        assert_eq!(monitor.peak_threshold(), 0.05);
    }

    #[test]
    fn test_session_meter_info_struct() {
        let info = SessionMeterInfo {
            pid: 1234,
            process_name: "Discord.exe".to_string(),
            peak_level: 0.15,
            is_voice_active: true,
        };
        assert_eq!(info.pid, 1234);
        assert!(info.is_voice_active);
    }
}
