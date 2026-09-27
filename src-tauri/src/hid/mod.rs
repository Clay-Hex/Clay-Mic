#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "windows")]
pub use win::{set_suppression, start_capture, start_capture_if_needed, stop_capture};

#[cfg(not(target_os = "windows"))]
pub fn start_capture() -> Result<(), String> {
    Err("Interception capture is Windows-only".into())
}

#[cfg(not(target_os = "windows"))]
pub fn start_capture_if_needed() -> Result<(), String> {
    Err("Interception capture is Windows-only".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop_capture() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn set_suppression(_enabled: bool, _device_path: Option<String>) -> Result<(), String> {
    Ok(())
}