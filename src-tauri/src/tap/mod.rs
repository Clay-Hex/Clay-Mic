//! HID Tap: a native hook DLL injected into WUDFHost to catch the remote's
//! back / volume± keys.
//!
//! kbdhid drops this remote's non-standard keyboard-page usages (0xF1 / 0x80 /
//! 0x81), so Interception never sees them. The raw bytes are still readable
//! from the HID-over-GATT characteristic read inside WUDFHost
//! (IOCTL `0x80018483`, 9-byte report). We load `clay_tap.dll` (built from
//! the `tap-dll` workspace member) and it reports those three buttons over
//! loopback UDP (one-way, no connection state).
//!
//! Design constraints:
//! - The hook DLL ships with the app (no runtime download).
//! - The suppression toggle only injects when the component is present, and
//!   skips silently otherwise.
//! - Inject / remove are explicit actions under DriverSect.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use host::{clear_marker_pid, find_host_pid, read_marker_pid,
    resolve_injected_pid, write_marker_pid,
};
use process::{eject_dll_from, inject_dll_into};
use status::{dll_ready, emit_status};
use udp::{ensure_server, stop_server};

use crate::keyslots::to_wide;

mod host;
mod process;
mod status;
mod udp;

pub use process::run_elevated_action;
pub use status::{refresh_status_async, status, status_cached};
pub use udp::ensure_server_public;


// ---------------------------------------------------------------------------
// Component identity
// ---------------------------------------------------------------------------

/// Hook DLL produced by the `tap-dll` crate; searched next to the exe first.
const TAP_DLL_NAME: &str = "clay_tap.dll";
const STATUS_FILE_NAME: &str = "inject-status.json";
const INJECTED_PID_FILE: &str = "injected.pid";

/// Loopback UDP — one-way hook DLL→host datagrams; no connection to go stale.
const UDP_PORT: u16 = 49733;
const UDP_BIND: &str = "127.0.0.1:49733";
/// Consider the hook DLL alive if it sent anything (heartbeat) within this.
/// Heartbeat is 200ms but runs inside UMFD where threads can stall.
const CLIENT_TTL: std::time::Duration = std::time::Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Runtime state
// ---------------------------------------------------------------------------

static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// Set while the UDP recv thread should keep running.
static SERVER_RUNNING: AtomicU32 = AtomicU32::new(0);
/// Last datagram time from the hook DLL (events or heartbeat).
static LAST_PACKET: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// Version reported by the resident hook DLL's latest heartbeat.
static DLL_VERSION: Mutex<Option<String>> = Mutex::new(None);
/// Previous `client_connected()` as last pushed — used to emit on 1→0.
static WAS_CONNECTED: AtomicU32 = AtomicU32::new(0);
/// When 0 (after remove), packets are ignored so a resident hook DLL cannot
/// look connected. Starts at 1 so a leftover marker can heartbeat again.
static ACCEPTING: AtomicU32 = AtomicU32::new(1);
/// Serializes ensure_server / stop_server so two threads cannot race.
static SERVER_LOCK: Mutex<()> = Mutex::new(());
/// UDP recv-thread handle; joined in stop_server so the port is free.
static SERVER_THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
/// Serializes inject/eject so concurrent triggers cannot double-LoadLibrary.
static INJECT_LOCK: Mutex<()> = Mutex::new(());

fn client_connected() -> bool {
    if ACCEPTING.load(Ordering::SeqCst) != 1 {
        return false;
    }
    LAST_PACKET
        .lock()
        .unwrap()
        .is_some_and(|at| at.elapsed() <= CLIENT_TTL)
}

fn lock_inject() -> std::sync::MutexGuard<'static, ()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match INJECT_LOCK.try_lock() {
            Ok(guard) => return guard,
            Err(std::sync::TryLockError::Poisoned(p)) => return p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    log::warn!("inject lock busy >30s; waiting anyway (stuck UAC/helper?)");
                    return INJECT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// Snapshot for `get_tap_status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TapStatus {
    pub version: String,
    /// hook DLL present beside the exe, in resources/, or in target/.
    pub dll_ready: bool,
    pub host_pid: Option<u32>,
    pub host_alive: bool,
    /// Marker file matches a live WUDFHost (or this session injected it).
    pub injected: bool,
    /// UDP recv thread is running.
    pub listening: bool,
    /// Hook DLL heartbeat received within the last few seconds.
    pub client_connected: bool,
    pub last_error: Option<String>,
    /// Configured remote identity, e.g. `VID=2717 PID=32B8`.
    pub device_identity: Option<String>,
    /// Why HostPid lookup did / did not succeed (UI diagnostics).
    pub lookup: Option<String>,
    /// Version reported by the resident hook's heartbeat (None while not
    /// connected, or when the hook predates the versioned heartbeat).
    pub dll_version: Option<String>,
    /// Resident hook differs from this build; an unversioned hook counts.
    pub dll_needs_update: bool,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// `%LOCALAPPDATA%\clay-mic\tap` — hook DLL copy, status files.
fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("tap")
}

/// Deployed copy injected into WUDFHost (fixed name — never rename per build;
/// a hook DLL must fully unload before a replacement is loaded).
pub(super) fn deployed_tap_dll() -> PathBuf {
    data_dir().join(TAP_DLL_NAME)
}

/// Built `clay_tap.dll` next to the exe, in the Tauri resources subdirectory,
/// or in this workspace's target dir (dev fallback).
pub(super) fn find_tap_dll_source() -> Option<PathBuf> {
    // 1. Beside the exe (portable / flat layout).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(TAP_DLL_NAME);
            if beside.is_file() {
                return Some(beside);
            }
            // 2. Tauri bundle.resources lands files under $INSTDIR\resources\.
            let resources = dir.join("resources").join(TAP_DLL_NAME);
            if resources.is_file() {
                return Some(resources);
            }
        }
    }
    // 3. Dev fallback: workspace target dir.
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(TAP_DLL_NAME);
    target.is_file().then_some(target)
}

/// True for `clay_tap.dll`.
fn is_tap_module_name(name: &str) -> bool {
    name.to_ascii_lowercase() == TAP_DLL_NAME
}

fn status_path() -> PathBuf {
    data_dir().join(STATUS_FILE_NAME)
}

fn injected_pid_path() -> PathBuf {
    data_dir().join(INJECTED_PID_FILE)
}

fn set_error(message: impl Into<String>) {
    let message = message.into();
    log::warn!("tap: {message}");
    *LAST_ERROR.lock().unwrap() = Some(message);
}

fn clear_error() {
    *LAST_ERROR.lock().unwrap() = None;
}


// ---------------------------------------------------------------------------
// Orchestration used by commands + apply_suppression
// ---------------------------------------------------------------------------

/// Launch the elevated helper and wait for its status file.
fn run_elevated_helper(args: [&str; 2]) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
    use windows::core::PCWSTR;

    let exe = std::env::current_exe().map_err(|error| format!("定位程序失败：{error}"))?;

    // Drop any status file from a previous run so a UAC cancel cannot be
    // mistaken for success via a stale ok=true.
    let _ = std::fs::remove_file(status_path());

    let params = format!(
        "{} \"{}\"",
        args.iter()
            .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.to_string() })
            .collect::<Vec<_>>()
            .join(" "),
        exe.display()
    );
    let verb = to_wide("runas");
    let file = to_wide(&exe.display().to_string());
    let params_wide = to_wide(&params);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        ..Default::default()
    };

    let process = unsafe {
        match ShellExecuteExW(&mut info) {
            Ok(()) => info.hProcess,
            Err(error) => {
                if error.code() == ERROR_CANCELLED.to_hresult() {
                    return Err("操作被取消".into());
                }
                return Err(format!("启动提权进程失败：{error}"));
            }
        }
    };

    if process.is_invalid() {
        return Err("未获取到进程句柄".into());
    }

    let exit_code = unsafe {
        WaitForSingleObject(process, u32::MAX);
        let mut code = 0u32;
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        code
    };

    if exit_code != 0 {
        log::warn!("elevated helper exit {exit_code}");
        if let Ok(raw) = std::fs::read_to_string(status_path()) {
            log::warn!("inject-status.json: {raw}");
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
                    if !err.is_empty() {
                        return Err(err.to_string());
                    }
                }
            }
        }
        return Err("操作未完成：可能被取消或未获得管理员权限".into());
    }
    if !read_status_file_ok() {
        log::warn!("elevated helper exit 0 but inject-status.json missing/stale");
        return Err("操作未完成：可能被取消或未获得管理员权限".into());
    }
    log::info!("elevated helper finished ok");
    Ok(())
}

fn read_status_file_ok() -> bool {
    // Helper writes this file only after its own work; require a fresh write
    // (deleted before launch) so an old ok cannot authorize a cancel.
    std::fs::read_to_string(status_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|value| {
            value.get("ok").and_then(|ok| ok.as_bool()).unwrap_or(false)
        })
}

/// Record a verified inject: marker + status push.
fn mark_injected(pid: u32) {
    ACCEPTING.store(1, Ordering::SeqCst);
    write_marker_pid(pid);
    clear_error();
    emit_status();
}

/// Shared inject path for both the background (suppression) and explicit UI
/// triggers. `force = false` skips when the hook DLL is already loaded.
fn inject_common(force: bool) -> Result<(), String> {
    if !dll_ready() {
        if force {
            return Err("组件未就绪（缺少 clay_tap.dll）".into());
        }
        log::info!("tap dll not present; skipped");
        return Ok(());
    }
    let host_pid = find_host_pid()?
        .ok_or_else(|| "未找到遥控器 WUDFHost（请先连接设备）".to_string())?;
    ensure_server()?;

    if !force && resolve_injected_pid(Some(host_pid)).is_some() {
        log::info!("tap already injected; skipping reload");
        mark_injected(host_pid);
        return Ok(());
    }

    match inject_dll_into(host_pid) {
        Ok(()) => {
            mark_injected(host_pid);
            log::info!("tap injected (direct) into pid {host_pid}");
            wait_udp_heartbeat();
            Ok(())
        }
        Err(first) => {
            log::info!("direct inject failed ({first}); elevating");
            if let Err(error) = run_elevated_helper(["--inject-tap", &host_pid.to_string()]) {
                // UAC cancel is expected: keep status, only the button message
                // carries the reason. Real helper failures still surface.
                if !error.contains("可能被取消") {
                    set_error(&error);
                    emit_status();
                }
                return Err(error);
            }
            if read_status_file_ok() {
                mark_injected(host_pid);
                log::info!("tap injected (elevated) into pid {host_pid}");
                wait_udp_heartbeat();
                Ok(())
            } else {
                // Do not write marker: elevated path did not verify LoadLibrary.
                let message = "注入未确认生效".to_string();
                set_error(&message);
                emit_status();
                Err(message)
            }
        }
    }
}

/// After LoadLibrary + `clay_tap_init` the hook must report in before
/// FreeLibrary is safe. The first UDP heartbeat proves init completed;
/// without this wait, an immediate remove races DETACH against a live hook.
fn wait_udp_heartbeat() {
    ensure_server().ok();
    for _ in 0..25 {
        if client_connected() {
            log::info!("tap heartbeat received; init complete");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    log::warn!("tap injected but no UDP heartbeat within 5s");
}

/// Background path from `apply_suppression(true)`: never downloads, never fails
/// the toggle; silent no-op when the component is absent.
fn ensure_injected() -> Result<(), String> {
    let _guard = lock_inject();
    inject_common(false)
}

/// Explicit UI action: re-inject even if a stale marker exists.
pub fn inject_command() -> Result<(), String> {
    let _guard = lock_inject();
    inject_common(true)
}

/// Explicit UI action: stop accepting packets, unload the DLL (verified),
/// then tear the UDP server down completely. On success the process returns
/// to a clean Idle: no DLL, no listener, no marker.
pub fn remove_command() -> Result<(), String> {
    let _guard = lock_inject();
    let marker = read_marker_pid();
    let host = find_host_pid().ok().flatten();
    let target = host.or(marker);

    let eject_result = match target {
        Some(pid) => match eject_dll_from(pid) {
            Ok(()) => Ok(()),
            Err(first) => {
                log::info!("direct eject failed ({first}); elevating");
                match run_elevated_helper(["--eject-tap", &pid.to_string()]) {
                    // Helper exits 0 only when its own module scan is clean.
                    Ok(()) if read_status_file_ok() => Ok(()),
                    Ok(()) => Err("卸载未通过模块校验".to_string()),
                    Err(error) => Err(error),
                }
            }
        },
        None => Ok(()),
    };

    match eject_result {
        Ok(()) => {
            ACCEPTING.store(0, Ordering::SeqCst);
            stop_server();
            clear_marker_pid();
            clear_error();
            log::info!("tap removed; UDP server stopped, marker cleared");
            emit_status();
            Ok(())
        }
        Err(error) => {
            // UAC cancel is an expected user choice: leave status untouched
            // and let the UI button message carry the reason.
            if !error.contains("可能被取消") {
                set_error(&format!("卸载 DLL 未完成：{error}"));
                emit_status();
            }
            Err(error)
        }
    }
}

/// Called from `apply_suppression(true)` on a background thread.
/// Never downloads; never fails the suppression toggle.
pub fn try_inject_if_ready() {
    match ensure_injected() {
        Ok(()) => {
            // Hook DLL may take a moment to init and send a heartbeat.
            std::thread::spawn(|| {
                for _ in 0..30 {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if client_connected() {
                        log::info!("tap heartbeat received (UDP)");
                        return;
                    }
                }
                log::warn!(
                    "tap injected but no UDP heartbeat within 6s \
                     (check clay_tap.dll / port {UDP_PORT})"
                );
            });
        }
        Err(error) => {
            log::info!("try_inject skipped/failed: {error}");
            if !error.contains("未找到遥控器") && !error.contains("组件未就绪") {
                set_error(error);
            }
        }
    }
}
