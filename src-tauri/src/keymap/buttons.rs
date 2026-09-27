//! Logical remote buttons and their platform identities.
//!
//! Button identity is tracked two ways:
//! - HID keyboard-page (0x07) usages — for Raw Input and BLE report decoding.
//! - Interception scan codes — for the low-level keyboard hook path.

/// Logical button id -> HID keyboard-page (0x07) usage.
pub const BUTTON_USAGES: &[(&str, u16)] = &[
    ("mic", 0x003E),
    ("back", 0x00F1),
    ("ok", 0x0028),
    ("tv", 0x0035),
    ("home", 0x004A),
    ("right", 0x004F),
    ("left", 0x0050),
    ("down", 0x0051),
    ("up", 0x0052),
    ("menu", 0x0065),
    ("power", 0x0066),
    ("volume_mute", 0x007F),
    ("volume_up", 0x0080),
    ("volume_down", 0x0081),
];

/// Logical button id -> Interception scan code.
///
/// Scan codes are what Interception delivers in `KeyEvent.scan_code`; each value
/// was observed by pressing the corresponding button on the remote.
pub const BUTTON_SCAN_CODES: &[(&str, u16)] = &[
    ("mic", 0x3F),
    ("power", 0x5E),
    ("home", 0x47),
    ("tv", 0x29),
    ("menu", 0x5D),
    ("ok", 0x1C),
    ("up", 0x48),
    ("down", 0x50),
    ("left", 0x4B),
    ("right", 0x4D),
    ("back", 0x6A),
    ("volume_up", 0x30),
    ("volume_down", 0x2E),
];

pub fn button_ids() -> Vec<&'static str> {
    BUTTON_USAGES.iter().map(|(button, _)| *button).collect()
}

pub fn usage_to_button(usage: u16) -> Option<&'static str> {
    BUTTON_USAGES
        .iter()
        .find(|(_, value)| *value == usage)
        .map(|(button, _)| *button)
}

pub fn button_to_usage(button: &str) -> Option<u16> {
    BUTTON_USAGES
        .iter()
        .find(|(id, _)| *id == button)
        .map(|(_, usage)| *usage)
}

/// Map an Interception scan code to a logical button id.
pub fn scan_code_to_button(code: u16) -> Option<&'static str> {
    BUTTON_SCAN_CODES
        .iter()
        .find(|(_, sc)| *sc == code)
        .map(|(button, _)| *button)
}
