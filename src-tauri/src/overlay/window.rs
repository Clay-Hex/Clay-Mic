use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::overlay::TextItem;

pub const OVERLAY_LABEL: &str = "overlay";
pub const INDICATOR_LABEL: &str = "indicator";

#[derive(Clone, Serialize)]
struct OverlayPayload {
    style: String,
    items: Vec<TextItem>,
}

/// Register the global hotkey that pops the overlay near the cursor.
pub fn register_hotkey(app: &AppHandle, hotkey: &str) {
    let handle = app.clone();
    let result = app
        .global_shortcut()
        .on_shortcut(hotkey, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                show_overlay(&handle);
            }
        });
    match result {
        Ok(()) => log::info!("overlay hotkey registered: {hotkey}"),
        Err(error) => log::warn!("failed to register overlay hotkey {hotkey:?}: {error}"),
    }
}

/// Re-register the overlay hotkey after the config changed.
pub fn reregister_hotkey(app: &AppHandle, hotkey: &str) {
    let _ = app.global_shortcut().unregister_all();
    register_hotkey(app, hotkey);
}

pub fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = window.hide();
    }
}

pub fn set_indicator_visible(app: &AppHandle, visible: bool) {
    if let Some(window) = app.get_webview_window(INDICATOR_LABEL) {
        let _ = if visible { window.show() } else { window.hide() };
    }
}

/// Apply the remembered overlay size on startup.
pub fn apply_saved_size(app: &AppHandle) {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return;
    };
    let Some(state) = crate::runtime::state() else {
        return;
    };
    let (width, height) = {
        let config = state.config.lock().unwrap();
        (config.overlay.width, config.overlay.height)
    };
    if width > 0 && height > 0 {
        let _ = window.set_size(PhysicalSize::new(width, height));
    }
}

/// Persist the overlay size after the user resizes it.
pub fn remember_size(width: u32, height: u32) {
    let Some(state) = crate::runtime::state() else {
        return;
    };
    let mut config = state.config.lock().unwrap();
    if config.overlay.width == width && config.overlay.height == height {
        return;
    }
    config.overlay.width = width;
    config.overlay.height = height;
    if let Err(error) = config.save() {
        log::warn!("overlay: saving size failed: {error}");
    }
}

pub fn show_overlay(app: &AppHandle) {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        log::warn!("overlay window not found");
        return;
    };
    position_overlay(&window);
    let _ = window.set_always_on_top(true);
    let _ = window.show();
    let _ = window.set_focus();

    let (style, items) = match crate::runtime::state() {
        Some(state) => {
            let style = state.config.lock().unwrap().overlay.style.clone();
            let (mut items, _, _) = state.items_page(1, 10);
            // The page is newest-first; flip it so the newest item sits at the
            // bottom, closest to the input box the overlay is anchored above.
            items.reverse();
            (style, items)
        }
        None => ("aurora".to_string(), Vec::new()),
    };
    // Ship the list with the event so the window renders without an extra
    // round-trip to the backend.
    let _ = window.emit("overlay://refresh", OverlayPayload { style, items });
}

fn position_overlay<R: Runtime>(window: &WebviewWindow<R>) {
    let size = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(360, 420));
    let caret = caret_position();
    if caret.is_none() {
        log::info!("overlay: no text caret from foreground window; using cursor position");
    }
    let anchor = caret.unwrap_or_else(|| {
        window
            .cursor_position()
            .unwrap_or_else(|_| PhysicalPosition::new(0.0, 0.0))
    });
    let x = (anchor.x - size.width as f64 / 2.0).max(0.0);
    let y = (anchor.y - size.height as f64 - 12.0).max(0.0);
    let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
}

/// Screen position of the text caret in the foreground window, when available.
#[cfg(target_os = "windows")]
fn caret_position() -> Option<PhysicalPosition<f64>> {
    native_caret_position().or_else(browser_caret_position)
}

#[cfg(not(target_os = "windows"))]
fn caret_position() -> Option<PhysicalPosition<f64>> {
    None
}

/// Native Win32 caret (classic edit controls).
#[cfg(target_os = "windows")]
fn native_caret_position() -> Option<PhysicalPosition<f64>> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(foreground, None);
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(thread, &mut info).is_err() || info.hwndCaret.0.is_null() {
            return None;
        }
        if info.rcCaret.bottom <= info.rcCaret.top {
            return None;
        }
        let mut point = POINT {
            x: info.rcCaret.left,
            y: info.rcCaret.bottom,
        };
        if !ClientToScreen(info.hwndCaret, &mut point).as_bool() {
            return None;
        }
        Some(PhysicalPosition::new(point.x as f64, point.y as f64))
    }
}

/// UI Automation caret — also covers apps that draw their own caret, such as
/// browsers and Electron.
#[cfg(target_os = "windows")]
fn browser_caret_position() -> Option<PhysicalPosition<f64>> {
    use std::ffi::c_void;

    use windows::core::{IUnknown, BOOL};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern2, UIA_TextPattern2Id,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None::<&IUnknown>, CLSCTX_INPROC_SERVER).ok()?;
        let element = automation.GetFocusedElement().ok()?;

        // Preferred: the caret itself, for apps that expose TextPattern2.
        let caret: Option<PhysicalPosition<f64>> = (|| {
            let pattern: IUIAutomationTextPattern2 =
                element.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
            let mut active = BOOL(0);
            let range = pattern.GetCaretRange(&mut active).ok()?;
            let array = range.GetBoundingRectangles().ok()?;
            if array.is_null() {
                return None;
            }
            let mut data: *mut c_void = std::ptr::null_mut();
            if SafeArrayAccessData(array, &mut data).is_err() {
                return None;
            }
            let lower = SafeArrayGetLBound(array, 1).unwrap_or(0);
            let upper = SafeArrayGetUBound(array, 1).unwrap_or(-1);
            let count = (upper - lower + 1).max(0) as usize;
            let mut position = None;
            if count >= 4 && !data.is_null() {
                let values = std::slice::from_raw_parts(data as *const f64, count);
                position = Some(PhysicalPosition::new(values[0], values[1] + values[3]));
            }
            let _ = SafeArrayUnaccessData(array);
            position
        })();
        if caret.is_some() {
            return caret;
        }

        // Fallback: the focused element's own rectangle, for apps that draw
        // their own caret without exposing TextPattern2 (browsers, Electron).
        // Anchor above the input box so the overlay lands on the edit field
        // instead of the mouse.
        let rect = element.CurrentBoundingRectangle().ok()?;
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return None;
        }
        Some(PhysicalPosition::new(
            (rect.left as f64 + rect.right as f64) / 2.0,
            rect.top as f64,
        ))
    }
}
