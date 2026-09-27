use enigo::{Enigo, Keyboard, Settings};
use std::error::Error;

/// Set the clipboard and paste it — fast for any text, including CJK. The
/// previous clipboard text is restored afterwards.
#[cfg(target_os = "windows")]
pub fn inject_via_clipboard(text: &str) -> Result<(), Box<dyn Error>> {
    let previous = get_clipboard_text();
    set_clipboard(text)?;
    std::thread::sleep(std::time::Duration::from_millis(30));
    paste()?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    if let Some(previous) = previous {
        let _ = set_clipboard(&previous);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn inject_via_clipboard(text: &str) -> Result<(), Box<dyn Error>> {
    inject_via_keyboard(text)
}

/// Type the text character by character.
pub fn inject_via_keyboard(text: &str) -> Result<(), Box<dyn Error>> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.text(text)?;
    Ok(())
}

/// Ctrl+V as a single `SendInput` batch: Win32 serializes one call, so V can
/// never be processed without Ctrl held or interleave with other input.
#[cfg(target_os = "windows")]
fn paste() -> Result<(), Box<dyn Error>> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };

    fn key(virtual_key: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    let inputs = [
        key(VK_CONTROL, false),
        key(VK_V, false),
        key(VK_V, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != inputs.len() {
        return Err(format!("Ctrl+V 注入失败（{sent}/{}）", inputs.len()).into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_clipboard(text: &str) -> Result<(), Box<dyn Error>> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        OpenClipboard(None)?;
        let result = (|| -> Result<(), Box<dyn Error>> {
            EmptyClipboard()?;
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let pointer = GlobalLock(handle);
            if pointer.is_null() {
                return Err("GlobalLock failed".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer as *mut u16, wide.len());
            let _ = GlobalUnlock(handle);
            // Ownership of the memory transfers to the clipboard on success.
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// The clipboard's current text, or `None` when it holds no text.
#[cfg(target_os = "windows")]
fn get_clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        if OpenClipboard(None).is_err() {
            return None;
        }
        let result = (|| {
            let handle: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            if handle.is_invalid() {
                return None;
            }
            let pointer = GlobalLock(HGLOBAL(handle.0));
            if pointer.is_null() {
                return None;
            }
            let wide = pointer as *const u16;
            let mut length = 0usize;
            while *wide.add(length) != 0 {
                length += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(wide, length));
            let _ = GlobalUnlock(HGLOBAL(handle.0));
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}
