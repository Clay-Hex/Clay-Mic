use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub name: String,
    pub protocol: String,
    pub sample_rate: u32,
    pub header_size: usize,
    #[serde(default)]
    pub device_id: Option<String>,
    /// PnP ids from the device's Device Information Service (0x2A50). Windows
    /// derives the HID hardware id (`VID_2717&PID_32B8`) from them, so the
    /// capture layer matches on these rather than a hardcoded vendor.
    #[serde(default)]
    pub vendor_id: Option<u16>,
    #[serde(default)]
    pub product_id: Option<u16>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            protocol: "atvv-1.0".to_string(),
            sample_rate: 16000,
            header_size: 2,
            device_id: None,
            vendor_id: None,
            product_id: None,
        }
    }
}

pub trait DeviceProfileExt {
    fn atvv_header_size(&self) -> usize;
}

impl DeviceProfileExt for DeviceProfile {
    fn atvv_header_size(&self) -> usize {
        if self.protocol.contains("v0.4") {
            4
        } else {
            2
        }
    }
}
