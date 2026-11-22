use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, Host, SupportedStreamConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

pub struct DeviceManager {
    host: Host,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    pub fn list_output_devices(&self) -> Vec<AudioDeviceInfo> {
        let default_name = self
            .host
            .default_output_device()
            .and_then(|d| d.name().ok());

        let mut result = Vec::new();

        if let Ok(devices) = self.host.output_devices() {
            for device in devices {
                if let Ok(name) = device.name() {
                    let is_default = default_name.as_deref() == Some(&name);
                    result.push(AudioDeviceInfo { name, is_default });
                }
            }
        }

        result
    }

    pub fn get_default_device(&self) -> Option<Device> {
        self.host.default_output_device()
    }

    pub fn get_device_by_name(&self, name: &str) -> Option<Device> {
        let devices = self.host.output_devices().ok()?;
        for device in devices {
            if let Ok(dev_name) = device.name() {
                if dev_name == name {
                    return Some(device);
                }
            }
        }
        None
    }

    pub fn get_device_config(device: &Device) -> Result<SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        device.default_output_config()
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_manager_initialization() {
        let mgr = DeviceManager::new();
        let devices = mgr.list_output_devices();
        println!("Available audio output devices: {:?}", devices);
        // On any system with sound card or CI, this manager initializes cleanly
    }
}
