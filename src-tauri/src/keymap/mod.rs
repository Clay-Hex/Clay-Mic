//! Config-driven key mapping for the remote's buttons.
//!
//! `keymap.json` decides, per button, what happens when it is pressed:
//! `voice` / `ignore` (swallow) / `pass` (native) / `send` (swallow + inject a
//! recorded key or combo) / `exec` (swallow + run a command) / `clear` (swallow
//! + select-all and delete in the focused field) / `backspace` / `inject_latest`
//! (swallow + push the newest transcript into the focused window). Suppression
//! is derived from this config.

pub mod buttons;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    /// One of: `voice`, `ignore`, `pass`, `send`, `exec`, `clear`, `backspace`,
    /// `inject_latest`.
    pub action: String,
    /// For `send`: the target key/combo, e.g. `Enter`, `Ctrl+C`.
    #[serde(default)]
    pub key: Option<String>,
    /// For `exec`: the command line or script path to run.
    #[serde(default)]
    pub command: Option<String>,
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            action: "ignore".to_string(),
            key: None,
            command: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapConfig {
    /// Master switch: swallow the remote's native keys.
    pub suppress: bool,
    /// What to do with the console opened for an `exec` binding:
    /// `keep` / `close` / `auto`. See [`TerminalExit`].
    #[serde(default = "default_terminal_exit")]
    pub terminal_exit: String,
    pub bindings: BTreeMap<String, Binding>,
}

fn default_terminal_exit() -> String {
    "keep".to_string()
}

impl Default for KeymapConfig {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        for button in buttons::button_ids() {
            let action = if button == "mic" { "voice" } else { "ignore" };
            bindings.insert(
                button.to_string(),
                Binding {
                    action: action.to_string(),
                    key: None,
                    command: None,
                },
            );
        }
        Self {
            suppress: true,
            terminal_exit: default_terminal_exit(),
            bindings,
        }
    }
}

/// Lifecycle of the console window opened for an `exec` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalExit {
    /// Leave the window open after the command finishes.
    Keep,
    /// Close the window as soon as the command finishes.
    Close,
    /// Close on success, keep the window on a non-zero exit code.
    Auto,
}

impl TerminalExit {
    fn from_config(value: &str) -> Self {
        match value {
            "close" => Self::Close,
            "auto" => Self::Auto,
            _ => Self::Keep,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("keymap.json")
}

impl KeymapConfig {
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("keymap parse failed: {}; using defaults", error);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }

    pub fn action_of(&self, button: &str) -> &str {
        self.bindings
            .get(button)
            .map(|binding| binding.action.as_str())
            .unwrap_or("ignore")
    }

    pub fn key_of(&self, button: &str) -> Option<&str> {
        self.bindings
            .get(button)
            .and_then(|binding| binding.key.as_deref())
    }

    pub fn command_of(&self, button: &str) -> Option<&str> {
        self.bindings
            .get(button)
            .and_then(|binding| binding.command.as_deref())
    }

    /// Whether a button's native key should be swallowed.
    fn swallows(&self, button: &str) -> bool {
        matches!(
            self.action_of(button),
            "ignore" | "send" | "voice" | "exec" | "clear" | "backspace" | "inject_latest"
        )
    }

    pub fn suppresses_active(&self, button: &str) -> bool {
        self.suppress && self.swallows(button)
    }
}

static CURRENT: OnceLock<Mutex<KeymapConfig>> = OnceLock::new();

fn store() -> &'static Mutex<KeymapConfig> {
    CURRENT.get_or_init(|| Mutex::new(KeymapConfig::load()))
}

pub fn snapshot() -> KeymapConfig {
    store().lock().unwrap().clone()
}

pub fn set(config: KeymapConfig) {
    *store().lock().unwrap() = config;
}

pub fn suppresses(button: &str) -> bool {
    store().lock().unwrap().suppresses_active(button)
}

/// Action to run for a button press, if the keymap has one.
pub fn binding_for(button: &str) -> (String, Option<String>, Option<String>, String) {
    let config = store().lock().unwrap();
    (
        config.action_of(button).to_string(),
        config.key_of(button).map(|key| key.to_string()),
        config.command_of(button).map(|command| command.to_string()),
        config.terminal_exit.clone(),
    )
}

/// Execute the bound action for a button press.
#[cfg(feature = "inject")]
pub fn execute_action(
    action: &str,
    key: Option<&str>,
    command: Option<&str>,
    terminal_exit: &str,
) -> Result<(), String> {
    match action {
        "send" => {
            let spec = key.ok_or("未录制按键")?;
            send_key_spec(spec)
        }
        "exec" => {
            let line = command.ok_or("未配置命令")?;
            open_terminal(line, TerminalExit::from_config(terminal_exit))
        }
        "clear" => clear_input(),
        "backspace" => backspace(),
        "inject_latest" => inject_latest(),
        // `swallows()` routes every action it swallows into this match; these
        // three are no-ops by design, anything else is a missed arm.
        "voice" | "ignore" | "pass" => Ok(()),
        action => Err(format!("未实现的动作：{action}")),
    }
}

/// Run `line` in a fresh console window whose lifetime follows `exit`.
///
/// `start` detaches the console from clay-mic, so closing the app neither kills
/// the command nor waits on it. `keep` uses `cmd /K` to hold the window open;
/// `close` runs `cmd /C` and exits; `auto` wraps the command so a non-zero exit
/// code keeps the window (via `pause`) while success closes it.
#[cfg(feature = "inject")]
fn open_terminal(line: &str, exit: TerminalExit) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        let shell_switch = if matches!(exit, TerminalExit::Keep) {
            "/K"
        } else {
            "/C"
        };
        let wrapped = match exit {
            TerminalExit::Auto => format!(
                "{line} & if errorlevel 1 (echo. & echo 执行失败，退出码 %errorlevel% & pause)"
            ),
            _ => line.to_string(),
        };
        let mut process = std::process::Command::new("cmd");
        process.args(["/C", "start", "", "cmd", shell_switch, &wrapped]);
        crate::process::hide_console(&mut process);
        process.spawn().map(|_| ()).map_err(|e| e.to_string())
    } else {
        let tail = if matches!(exit, TerminalExit::Keep) {
            "; exec sh"
        } else {
            ""
        };
        let mut process = std::process::Command::new("x-terminal-emulator");
        process.args(["-e", "sh", "-c", &format!("{line}{tail}")]);
        process.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(not(feature = "inject"))]
pub fn execute_action(
    _action: &str,
    _key: Option<&str>,
    _command: Option<&str>,
    _terminal_exit: &str,
) -> Result<(), String> {
    Ok(())
}

#[cfg(feature = "inject")]
fn send_key_spec(spec: &str) -> Result<(), String> {
    use enigo::{Direction, Enigo, Keyboard, Settings};

    let parts: Vec<&str> = spec
        .split('+')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    let (main, modifiers) = parts
        .split_last()
        .ok_or_else(|| "空按键".to_string())?;

    // Modifier chords need real virtual-key events: enigo's Unicode path
    // cannot map a character to a VK code and fails outright ("Could not
    // translate the character..."), which is why Ctrl+A style bindings broke.
    let chars: Vec<char> = main.chars().collect();
    if !modifiers.is_empty() && chars.len() == 1 {
        #[cfg(target_os = "windows")]
        return send_modified_char(chars[0], modifiers);
        #[cfg(not(target_os = "windows"))]
        let _ = chars;
    }

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let mut pressed = Vec::new();
    for modifier in modifiers {
        let key = modifier_key(modifier).ok_or_else(|| format!("未知修饰键：{}", modifier))?;
        enigo
            .key(key, Direction::Press)
            .map_err(|e| e.to_string())?;
        pressed.push(key);
    }

    let main_key = named_key(main)
        .or_else(|| single_char_key(main))
        .ok_or_else(|| format!("未知按键：{}", main))?;
    let result = enigo
        .key(main_key, Direction::Click)
        .map_err(|e| e.to_string());

    for key in pressed.into_iter().rev() {
        let _ = enigo.key(key, Direction::Release);
    }
    result
}

/// Select everything in the focused control and delete it.
#[cfg(all(feature = "inject", target_os = "windows"))]
fn clear_input() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_A, VK_CONTROL, VK_DELETE,
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

    // One SendInput call per chord — Win32 serializes a batch, so the shortcut
    // cannot be split apart. Virtual-key events are mandatory here: Ctrl+letter
    // sent through enigo's Unicode path never registers as a shortcut.
    let select = [
        key(VK_CONTROL, false),
        key(VK_A, false),
        key(VK_A, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&select, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != select.len() {
        return Err(format!("清空失败：Ctrl+A 注入（{sent}/{}）", select.len()));
    }
    // Some controls apply the selection asynchronously; without this pause
    // Delete lands before Ctrl+A is processed and nothing gets cleared.
    std::thread::sleep(std::time::Duration::from_millis(30));
    let remove = [key(VK_DELETE, false), key(VK_DELETE, true)];
    let sent = unsafe { SendInput(&remove, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != remove.len() {
        return Err(format!("清空失败：Delete 注入（{sent}/{}）", remove.len()));
    }
    Ok(())
}

#[cfg(all(feature = "inject", not(target_os = "windows")))]
fn clear_input() -> Result<(), String> {
    send_key_spec("Ctrl+A")?;
    std::thread::sleep(std::time::Duration::from_millis(30));
    send_key_spec("Delete")
}

#[cfg(all(feature = "inject", target_os = "windows"))]
fn backspace() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_BACK,
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

    let input = [key(VK_BACK, false), key(VK_BACK, true)];
    let sent = unsafe { SendInput(&input, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != input.len() {
        return Err(format!("退格注入失败（{sent}/{}）", input.len()));
    }
    Ok(())
}

#[cfg(all(feature = "inject", not(target_os = "windows")))]
fn backspace() -> Result<(), String> {
    send_key_spec("Backspace")
}

/// Push the newest transcript into the focused window.
///
/// Text is picked like the history list's inject button (formatted, otherwise
/// raw), guarded like `voice`'s automatic injection (own window in front →
/// skip). Only the newest entry counts: an entry without text yet (failed or
/// still transcribing) reports an error rather than falling back to an older
/// transcript.
#[cfg(feature = "inject")]
fn inject_latest() -> Result<(), String> {
    let Some(state) = crate::runtime::state() else {
        return Err("应用尚未初始化".into());
    };
    let Some(item) = state.items_page(1, 1).0.into_iter().next() else {
        return Err("没有可注入的文本".into());
    };
    let Some(text) = item.injectable_text() else {
        return Err("最新一条还没有文字".into());
    };

    if crate::inject::foreground_is_self() {
        return Err("本程序窗口在前台，已跳过注入".into());
    }

    let method = state.config.lock().unwrap().inject.method.clone();
    crate::inject::inject(text, &method)
}

/// Send `<modifiers>+<single char>` as one `SendInput` batch of virtual-key
/// events — the only form a Windows shortcut can be delivered in.
#[cfg(all(feature = "inject", target_os = "windows"))]
fn send_modified_char(ch: char, modifiers: &[&str]) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
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

    fn modifier_vk(name: &str) -> Option<VIRTUAL_KEY> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(VK_CONTROL),
            "shift" => Some(VK_SHIFT),
            "alt" => Some(VK_MENU),
            "win" | "meta" | "super" => Some(VK_LWIN),
            _ => None,
        }
    }

    // Lowercase first: VkKeyScanW('A') reports an implied Shift that must not
    // leak into the chord (Ctrl+A is recorded without Shift).
    let normalized = if ch.is_ascii_alphabetic() {
        ch.to_ascii_lowercase()
    } else {
        ch
    };
    let code = u16::try_from(normalized as u32)
        .map_err(|_| format!("无法发送按键：{normalized}（超出 BMP）"))?;
    let scanned = unsafe { VkKeyScanW(code) };
    if scanned == -1 {
        return Err(format!("无法发送按键：{normalized}（无法映射为虚拟键）"));
    }
    let scan = scanned as u16;
    let main_vk = VIRTUAL_KEY(scan & 0xFF);
    let needs_shift = scan & 0x100 != 0;

    let mut inputs: Vec<INPUT> = Vec::new();
    let mut pressed: Vec<VIRTUAL_KEY> = Vec::new();
    for modifier in modifiers {
        let Some(modifier) = modifier_vk(modifier) else {
            return Err(format!("未知修饰键：{}", modifier));
        };
        inputs.push(key(modifier, false));
        pressed.push(modifier);
    }
    if needs_shift && !pressed.contains(&VK_SHIFT) {
        inputs.push(key(VK_SHIFT, false));
        pressed.push(VK_SHIFT);
    }
    inputs.push(key(main_vk, false));
    inputs.push(key(main_vk, true));
    for virtual_key in pressed.into_iter().rev() {
        inputs.push(key(virtual_key, true));
    }

    let count = inputs.len();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != count {
        return Err(format!("快捷键注入失败（{sent}/{count}）"));
    }
    Ok(())
}

#[cfg(feature = "inject")]
fn modifier_key(name: &str) -> Option<enigo::Key> {
    use enigo::Key;
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Key::Control),
        "shift" => Some(Key::Shift),
        "alt" => Some(Key::Alt),
        "win" | "meta" | "super" => Some(Key::Meta),
        _ => None,
    }
}

#[cfg(feature = "inject")]
fn named_key(name: &str) -> Option<enigo::Key> {
    use enigo::Key;
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            let n: u8 = f[1..].parse().ok()?;
            f_key(n)?
        }
        _ => return None,
    })
}

#[cfg(feature = "inject")]
fn f_key(n: u8) -> Option<enigo::Key> {
    use enigo::Key;
    Some(match n {
        1 => Key::F1,
        2 => Key::F2,
        3 => Key::F3,
        4 => Key::F4,
        5 => Key::F5,
        6 => Key::F6,
        7 => Key::F7,
        8 => Key::F8,
        9 => Key::F9,
        10 => Key::F10,
        11 => Key::F11,
        12 => Key::F12,
        _ => return None,
    })
}

/// Open the native file picker and return the chosen script path.
#[cfg(target_os = "windows")]
pub fn pick_script() -> Result<Option<String>, String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
        OPENFILENAMEW,
    };

    let filter: Vec<u16> = "可执行文件与脚本 (*.exe;*.bat;*.cmd;*.ps1;*.vbs;*.py)\0\
        *.exe;*.bat;*.cmd;*.ps1;*.vbs;*.py\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut buffer = [0u16; 1024];
    let mut spec = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: w!("选择要执行的脚本"),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    let picked = unsafe { GetOpenFileNameW(&mut spec) };
    if !picked.as_bool() {
        // Returning Ok(None) keeps a user cancel from surfacing as an error.
        return Ok(None);
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    if end == 0 {
        return Ok(None);
    }
    Ok(Some(String::from_utf16_lossy(&buffer[..end])))
}

#[cfg(not(target_os = "windows"))]
pub fn pick_script() -> Result<Option<String>, String> {
    Err("文件选择仅支持 Windows".into())
}

#[cfg(feature = "inject")]
fn single_char_key(name: &str) -> Option<enigo::Key> {
    let mut chars = name.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(enigo::Key::Unicode(ch))
}
