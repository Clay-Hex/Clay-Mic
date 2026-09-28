//! Windows HID capture via the Interception driver.
//!
//! Replaces the previous WH_KEYBOARD_LL + Raw Input implementation. The
//! Interception driver sits at the kernel level and lets us intercept keystrokes
//! from a single configured device before they reach any application, with no
//! global hook needed.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::interception::{
    Driver, KeyEvent, KEYBOARD_SLOT_COUNT, KEY_FILTER_ALL, KEY_FILTER_NONE, STROKE_KEY_UP,
};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static CAPTURE_THREAD: Mutex<Option<u32>> = Mutex::new(None);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Interception device id currently filtered (0 = not bound yet).
static TARGET_DEVICE: AtomicI32 = AtomicI32::new(0);

/// The loaded Interception DLL bindings. Once loaded, kept for the process
/// lifetime (the driver must stay resident anyway).
static API: Mutex<Option<Driver>> = Mutex::new(None);

/// Per-device filter state held alongside the context.
struct CaptureState {
    ctx: usize,
}

static STATE: Mutex<Option<CaptureState>> = Mutex::new(None);

struct SuppressState {
    enabled: bool,
    target_path: Option<String>,
}

static SUPPRESS: Mutex<SuppressState> = Mutex::new(SuppressState {
    enabled: false,
    target_path: None,
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start the Interception capture thread and bind to the configured remote.
///
/// If `probe_devices` cannot find the remote the thread still runs and keeps
/// re-probing once a second until the device appears.
pub fn start_capture() -> Result<(), String> {
    if CAPTURE_THREAD.lock().unwrap().is_some() {
        return Ok(()); // already running
    }

    // Ensure the Interception DLL is loaded.
    let api = ensure_api()?;

    let ctx = api.make_context();
    if ctx.is_null() {
        return Err("interception_create_context returned NULL".into());
    }

    // Probe devices and set the filter on the target (if found). If the remote
    // is not connected yet, the capture loop keeps re-probing.
    let target = probe_devices(&api, ctx);
    let target_dev = match target {
        Some(dev) => {
            api.set_key_filter(ctx, dev, KEY_FILTER_ALL);
            log::info!("set KEY_FILTER_ALL on device {dev}");
            dev
        }
        None => {
            log::warn!("no configured device found; will keep probing");
            0
        }
    };
    TARGET_DEVICE.store(target_dev, Ordering::SeqCst);

    *STATE.lock().unwrap() = Some(CaptureState {
        ctx: ctx as usize,
    });

    STOP_REQUESTED.store(false, Ordering::SeqCst);

    // Extract raw pointer before move into closure (*mut c_void is not Send).
    let raw_ctx = ctx as usize;
    std::thread::spawn(move || {
        let ptr = raw_ctx as *mut std::ffi::c_void;
        run_capture_loop(&api, ptr);
        // Cleanup context on exit.
        api.drop_context(ptr);
        *STATE.lock().unwrap() = None;
        *CAPTURE_THREAD.lock().unwrap() = None;
        TARGET_DEVICE.store(0, Ordering::SeqCst);
        log::info!("interception capture thread exited");
    });

    // Mark capture as running (the thread takes over from here).
    *CAPTURE_THREAD.lock().unwrap() = Some(0);
    log::info!("Interception capture started");
    Ok(())
}

/// Signal the capture thread to stop.
pub fn stop_capture() -> Result<(), String> {
    STOP_REQUESTED.store(true, Ordering::SeqCst);

    // Clear the filter so the driver stops intercepting.
    let target = TARGET_DEVICE.load(Ordering::SeqCst);
    if target > 0 {
        if let (Some(state), Ok(api)) = (STATE.lock().unwrap().as_ref(), ensure_api()) {
            api.set_key_filter(
                state.ctx as *mut std::ffi::c_void,
                target,
                KEY_FILTER_NONE,
            );
        }
    }
    TARGET_DEVICE.store(0, Ordering::SeqCst);

    log::info!("HID capture stop requested");
    Ok(())
}

/// Start capture if it is not already running.
pub fn start_capture_if_needed() -> Result<(), String> {
    if CAPTURE_THREAD.lock().unwrap().is_some() {
        return Ok(());
    }
    start_capture()
}

/// Enable/disable suppression of the target remote's keys.
///
/// When enabled, keystrokes from the target device are consumed by the driver
/// and never reach other applications. When disabled they pass through.
pub fn set_suppression(enabled: bool, device_path: Option<String>) -> Result<(), String> {
    let mut state = SUPPRESS.lock().unwrap();
    state.enabled = enabled;
    state.target_path = device_path.filter(|p| !p.is_empty());
    log::info!(
        "HID suppression enabled={} target={:?}",
        state.enabled,
        state.target_path
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Interception event loop
// ---------------------------------------------------------------------------

/// Main capture loop, runs on its own thread.
///
/// Blocks on `interception_wait_with_timeout` with a 50 ms timeout so the
/// thread can check `STOP_REQUESTED` periodically, and re-probes for the
/// remote about once a second until it is bound.
fn run_capture_loop(api: &Driver, ctx: *mut std::ffi::c_void) {
    const TIMEOUT_MS: u32 = 50;
    let mut strokes = [KeyEvent::default(); 8];
    let mut first = true;
    let mut next_probe = Instant::now();

    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        // Late-bind: if the remote was not connected at startup, keep looking.
        let now = Instant::now();
        if now >= next_probe {
            next_probe = now + Duration::from_secs(1);
            if TARGET_DEVICE.load(Ordering::SeqCst) == 0 {
                if let Some(dev) = probe_devices(api, ctx) {
                    api.set_key_filter(ctx, dev, KEY_FILTER_ALL);
                    TARGET_DEVICE.store(dev, Ordering::SeqCst);
                    log::info!("late-bound configured device {dev}");
                }
            }
        }

        let device = api.wait_for_device(ctx, TIMEOUT_MS);
        if device <= 0 {
            // Timeout or error — loop back to check STOP_REQUESTED.
            continue;
        }

        if first {
            log::info!("interception_wait returned first device {device}");
            first = false;
        }

        let count = api.read_events(ctx, device, &mut strokes);
        if count <= 0 {
            continue;
        }

        let target_device = TARGET_DEVICE.load(Ordering::SeqCst);
        let is_target = device == target_device && target_device > 0;
        let suppression_enabled = SUPPRESS.lock().unwrap().enabled;

        // Process each received stroke.
        let mut to_send: Vec<KeyEvent> = Vec::new();
        for stroke in &strokes[..count as usize] {
            let key_up = stroke.flags & STROKE_KEY_UP != 0;

            // Swallow only keys whose button is configured to be swallowed
            // (`pass` and unknown keys pass through natively).
            let swallow = is_target && suppression_enabled
                && crate::keymap::buttons::scan_code_to_button(stroke.scan_code)
                    .map(crate::keymap::suppresses)
                    .unwrap_or(false);

            if swallow {
                // Both edges are needed so hold-to-talk (voice) can stop.
                process_target_stroke(stroke, key_up);
            } else {
                to_send.push(*stroke);
            }
        }

        // Passthrough: send events we didn't swallow.
        if !to_send.is_empty() {
            api.write_events(ctx, device, &to_send);
        }
    }
}

/// Run the bound action for a swallowed key from the target device.
///
/// `voice` is driven by the remote's ATVV stream events (see `crate::voice`),
/// not by the HID key edge — here the key is only swallowed so it does not
/// leak F5 into the focused application. Everything else is resolved by
/// [`crate::keymap::on_edge`], which fires a long press once the hold reaches
/// the threshold while the button is still down, and lets a hold that ends
/// sooner answer on the release edge.
fn process_target_stroke(stroke: &KeyEvent, key_up: bool) {
    let Some(button_id) = crate::keymap::buttons::scan_code_to_button(stroke.scan_code) else {
        return;
    };
    let Some((action, key, command, terminal_exit)) = crate::keymap::on_edge(button_id, !key_up)
    else {
        return;
    };

    log::info!("button={button_id} action={action} key={key:?} command={command:?}");
    if let Err(error) = crate::keymap::execute_action(
        &action,
        key.as_deref(),
        command.as_deref(),
        &terminal_exit,
    ) {
        log::warn!("button={button_id} action={action} failed: {error}");
    }
}

// ---------------------------------------------------------------------------
// Device probing
// ---------------------------------------------------------------------------

/// Scan all keyboard slots and return the device id whose hardware id matches
/// the device the user configured, or `None` if it is not present.
fn probe_devices(api: &Driver, ctx: *mut std::ffi::c_void) -> Option<i32> {
    let Some(wanted) = configured_identity() else {
        log::debug!("probe: no device configured yet");
        return None;
    };

    for dev in 1..=KEYBOARD_SLOT_COUNT {
        let mut buf = vec![0u8; 1024];
        let len = api.hardware_id(ctx, dev, &mut buf) as usize;
        if len < 2 {
            continue;
        }
        let hwid = decode_wide(&buf, len);
        if hwid.is_empty() {
            continue;
        }
        log::debug!("interception device {dev}: {hwid}");
        if parse_vid_pid(&hwid) == Some(wanted) {
            log::info!("probe: device {dev} matches configured remote ({hwid})");
            return Some(dev);
        }
    }

    log::warn!("probe: configured device not found in slots 1..{KEYBOARD_SLOT_COUNT}");
    None
}

/// Interception returns hardware ids as UTF-16LE wide strings (NUL-terminated).
/// Decode into a Rust `String`, stopping at the first NUL.
fn decode_wide(buf: &[u8], len: usize) -> String {
    let end = len.min(buf.len());
    let mut words: Vec<u16> = buf[..end]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if let Some(pos) = words.iter().position(|word| *word == 0) {
        words.truncate(pos);
    }
    String::from_utf16_lossy(&words)
}

fn configured_identity() -> Option<(u16, u16)> {
    let state = crate::runtime::state()?;
    let config = state.config.lock().ok()?;
    Some((config.device.vendor_id?, config.device.product_id?))
}

/// Two encodings appear in practice, both carrying the same numeric value:
/// - Classic: `HID\VID_2717&PID_32B8`
/// - BLE:     `HID\{...}_DEV_VID&012717_PID&32B8_REV&00A4`
/// The BLE form zero-pads the id, so only the low four hex digits matter.
fn parse_vid_pid(hwid: &str) -> Option<(u16, u16)> {
    let upper = hwid.to_ascii_uppercase();
    Some((id_after(&upper, "VID")?, id_after(&upper, "PID")?))
}

fn id_after(haystack: &str, tag: &str) -> Option<u16> {
    let rest = &haystack[haystack.find(tag)? + tag.len()..];
    let digits: String = rest
        .trim_start_matches(['_', '&'])
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if digits.len() < 4 {
        return None;
    }
    u16::from_str_radix(&digits[digits.len() - 4..], 16).ok()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Lazily load the Interception DLL and return a reference to it.
fn ensure_api() -> Result<&'static Driver, String> {
    {
        let guard = API.lock().unwrap();
        if guard.is_some() {
            // SAFETY: we never remove from the Mutex until process exit, so
            // the reference stays valid for the entire lifetime.
            let ptr = guard.as_ref().unwrap() as *const Driver;
            return Ok(unsafe { &*ptr });
        }
    }
    let api = Driver::open()?;
    *API.lock().unwrap() = Some(api);
    let guard = API.lock().unwrap();
    let ptr = guard.as_ref().unwrap() as *const Driver;
    Ok(unsafe { &*ptr })
}

#[cfg(test)]
mod tests {
    use super::parse_vid_pid;

    #[test]
    fn parses_the_classic_hid_encoding() {
        assert_eq!(parse_vid_pid("HID\\VID_2717&PID_32B8"), Some((0x2717, 0x32B8)));
    }

    #[test]
    fn parses_the_ble_encoding_with_zero_padded_vid() {
        assert_eq!(
            parse_vid_pid("HID\\{00001812-0000-1000-8000-00805F9B34FB}_DEV_VID&012717_PID&32B8_REV&00A4"),
            Some((0x2717, 0x32B8))
        );
    }

    #[test]
    fn lower_case_input_is_accepted() {
        assert_eq!(
            parse_vid_pid("hid\\{...}_dev_vid&012717_pid&32b8_rev&00a4"),
            Some((0x2717, 0x32B8))
        );
    }

    #[test]
    fn non_target_vendors_are_rejected() {
        assert_ne!(parse_vid_pid("USB\\VID_2717&PID_D002"), Some((0x2717, 0x32B8)));
        assert_eq!(parse_vid_pid("HID\\VID_1234&PID_5678"), Some((0x1234, 0x5678)));
    }

    #[test]
    fn missing_ids_yield_none() {
        assert_eq!(parse_vid_pid("SOMETHING\\ELSE"), None);
        assert_eq!(parse_vid_pid("HID\\VID_2717"), None);
    }
}