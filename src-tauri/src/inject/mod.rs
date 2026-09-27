pub mod keyboard;

/// Insert `text` into the currently focused window using the configured
/// method. Focus is never changed — the target must already hold it.
pub fn inject(text: &str, method: &str) -> Result<(), String> {
    if method == "keyboard" {
        keyboard::inject_via_keyboard(text)
    } else {
        keyboard::inject_via_clipboard(text)
    }
    .map_err(|error| error.to_string())
}

/// Whether the foreground window belongs to this process.
#[cfg(target_os = "windows")]
pub fn foreground_is_self() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        pid != 0 && pid == GetCurrentProcessId()
    }
}

#[cfg(not(target_os = "windows"))]
pub fn foreground_is_self() -> bool {
    false
}
