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

use host::{clear_marker_pid, find_host_pid, pid_is_wudfhost, read_marker_pid,
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
/// PID reported by the resident hook DLL's latest heartbeat.
static HOOK_PID: Mutex<Option<u32>> = Mutex::new(None);
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

/// PID the resident hook last reported, if any.
fn hook_pid() -> Option<u32> {
    *HOOK_PID.lock().unwrap()
}

/// Whether a hook in `host_pid` is proven live: a fresh heartbeat naming
/// exactly that process. A pooled WUDFHost can inherit a stale marker, so the
/// heartbeat — not the marker — is what makes a hook "healthy".
fn hook_healthy_for(host_pid: u32) -> bool {
    client_connected() && hook_pid() == Some(host_pid)
}

/// Wait (bounded) for a heartbeat from `host_pid`.
fn wait_hook_heartbeat(host_pid: u32, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if hook_healthy_for(host_pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Whether a live hook is proven for the current host: a fresh heartbeat that
/// names it. The marker is deliberately NOT consulted here — a pooled WUDFHost
/// can carry a stale marker and would then show a false "injected".
pub(super) fn injected_in_host(host_pid: Option<u32>) -> bool {
    host_pid.is_some_and(hook_healthy_for)
}

/// Whether a hook was injected before (a marker exists). Startup uses this —
/// not `injected` — to decide whether to bring the UDP server up: `injected`
/// is derived from a heartbeat that needs that very server, so gating on it
/// would deadlock the resident-hook case.
pub fn marker_present() -> bool {
    read_marker_pid().is_some()
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

/// Why launching the elevated helper ended, so callers do not have to
/// string-match the UAC wording.
enum ElevationError {
    /// The user dismissed the UAC prompt.
    Cancelled,
    Failed(String),
}

/// Why an inject attempt ended.
enum InjectFailure {
    /// The hook needs administrator rights and was left un-injected; an
    /// unattended call must not pop UAC, so the user triggers it from the UI.
    NeedsElevation(String),
    /// The user dismissed the UAC prompt (UI-initiated calls only).
    Cancelled,
    /// The remote's WUDFHost is not present.
    HostMissing,
    /// A real failure worth surfacing as an error.
    Failed(String),
}

/// Launch the elevated helper and wait for its status file.
fn run_elevated_helper(args: [&str; 2]) -> Result<(), ElevationError> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
    use windows::core::PCWSTR;

    let exe = std::env::current_exe()
        .map_err(|error| ElevationError::Failed(format!("定位程序失败：{error}")))?;

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
                    return Err(ElevationError::Cancelled);
                }
                return Err(ElevationError::Failed(format!("启动提权进程失败：{error}")));
            }
        }
    };

    if process.is_invalid() {
        return Err(ElevationError::Failed("未获取到进程句柄".into()));
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
                        return Err(ElevationError::Failed(err.to_string()));
                    }
                }
            }
        }
        return Err(ElevationError::Failed(
            "操作未完成：可能被取消或未获得管理员权限".into(),
        ));
    }
    if !read_status_file_ok() {
        log::warn!("elevated helper exit 0 but inject-status.json missing/stale");
        return Err(ElevationError::Failed(
            "操作未完成：可能被取消或未获得管理员权限".into(),
        ));
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

/// Shared inject path for the attended (suppression/connect/UI) and the
/// unattended (heartbeat self-heal) triggers.
///
/// `force = false` skips when the hook DLL is already loaded.
/// `allow_elevation = false` never launches the elevated helper, so an
/// unattended call cannot pop UAC; it reports `NeedsElevation` instead.
fn inject_common(force: bool, allow_elevation: bool) -> Result<(), InjectFailure> {
    if !dll_ready() {
        if force {
            return Err(InjectFailure::Failed("组件未就绪（缺少 clay_tap.dll）".into()));
        }
        log::info!("tap dll not present; skipped");
        return Ok(());
    }
    let host_pid = match find_host_pid() {
        Ok(Some(pid)) => pid,
        Ok(None) => return Err(InjectFailure::HostMissing),
        Err(error) => return Err(InjectFailure::Failed(error)),
    };
    if let Err(error) = ensure_server() {
        return Err(InjectFailure::Failed(error));
    }

    if !force && resolve_injected_pid(Some(host_pid)).is_some() {
        // A marker says a hook may be resident, but only a heartbeat from THIS
        // host proves it: a pooled WUDFHost can carry the marker with no DLL.
        // Give the beat a moment before deciding to reload.
        if hook_healthy_for(host_pid)
            || wait_hook_heartbeat(host_pid, std::time::Duration::from_millis(1200))
        {
            log::info!("tap already injected (heartbeat from host {host_pid}); skipping reload");
            mark_injected(host_pid);
            return Ok(());
        }
        log::info!("tap marker present but no heartbeat from host {host_pid}; re-injecting");
    }

    match inject_dll_into(host_pid) {
        Ok(()) => {
            mark_injected(host_pid);
            log::info!("tap injected (direct) into pid {host_pid}");
            wait_udp_heartbeat();
            Ok(())
        }
        Err(first) => {
            if !allow_elevation {
                // Unattended self-heal must never pop UAC: report so the UI can
                // ask the user to re-inject, and stop here.
                log::info!("direct inject failed ({first}); not elevating (unattended path)");
                return Err(InjectFailure::NeedsElevation(first));
            }
            log::info!("direct inject failed ({first}); elevating");
            match run_elevated_helper(["--inject-tap", &host_pid.to_string()]) {
                Ok(()) if read_status_file_ok() => {
                    mark_injected(host_pid);
                    log::info!("tap injected (elevated) into pid {host_pid}");
                    wait_udp_heartbeat();
                    Ok(())
                }
                Ok(()) => {
                    // Do not write marker: elevated path did not verify LoadLibrary.
                    let message = "注入未确认生效".to_string();
                    set_error(&message);
                    emit_status();
                    Err(InjectFailure::Failed(message))
                }
                Err(ElevationError::Cancelled) => Err(InjectFailure::Cancelled),
                Err(ElevationError::Failed(error)) => {
                    set_error(&error);
                    emit_status();
                    Err(InjectFailure::Failed(error))
                }
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
fn ensure_injected(allow_elevation: bool) -> Result<(), InjectFailure> {
    let _guard = lock_inject();
    inject_common(false, allow_elevation)
}

/// Explicit UI action: re-inject even if a stale marker exists. May elevate.
pub fn inject_command() -> Result<(), String> {
    let _guard = lock_inject();
    inject_common(true, true).map_err(inject_failure_message)
}

/// Human-readable message for a failed inject, shared by the UI entry points.
fn inject_failure_message(failure: InjectFailure) -> String {
    match failure {
        InjectFailure::NeedsElevation(reason) => reason,
        InjectFailure::Cancelled => "操作已取消".to_string(),
        InjectFailure::HostMissing => "未找到遥控器 WUDFHost（请先连接设备）".to_string(),
        InjectFailure::Failed(error) => error,
    }
}

/// Explicit UI action: stop accepting packets, unload the DLL (verified),
/// then tear the UDP server down completely. On success the process returns
/// to a clean Idle: no DLL, no listener, no marker.
pub fn remove_command() -> Result<(), String> {
    let _guard = lock_inject();
    let marker = read_marker_pid();
    let host = find_host_pid().ok().flatten();
    let target = host.or(marker);

    let mut cancelled = false;
    let eject_result = match target {
        Some(pid) => match eject_dll_from(pid) {
            Ok(()) => Ok(()),
            Err(first) => {
                log::info!("direct eject failed ({first}); elevating");
                match run_elevated_helper(["--eject-tap", &pid.to_string()]) {
                    // Helper exits 0 only when its own module scan is clean.
                    Ok(()) if read_status_file_ok() => Ok(()),
                    Ok(()) => Err("卸载未通过模块校验".to_string()),
                    Err(ElevationError::Cancelled) => {
                        cancelled = true;
                        Err("操作已取消".to_string())
                    }
                    Err(ElevationError::Failed(error)) => Err(error),
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
            // A dismissed UAC is the user's choice, not a failure to surface.
            if !cancelled {
                set_error(&format!("卸载 DLL 未完成：{error}"));
                emit_status();
            }
            Err(error)
        }
    }
}

/// End the WUDFHost that hosts the configured remote (needs elevation).
#[cfg(target_os = "windows")]
pub fn kill_wudfhost_command() -> Result<String, String> {
    let pid = find_host_pid()?.ok_or("未找到遥控器对应的 WUDFHost（请先连接设备）")?;
    log::info!("kill_wudfhost: ending pid={pid}");

    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue\n\
         if ($null -eq $p) {{ throw '进程已不存在' }}\n\
         if ($p.ProcessName -ne 'WUDFHost') {{ throw ('PID {pid} 不是 WUDFHost，而是 ' + $p.ProcessName) }}\n\
         Stop-Process -Id {pid} -Force\n\
         Start-Sleep -Milliseconds 300"
    );
    crate::keyslots::run_elevated(&script, "kill-wudfhost")?;

    let gone = !pid_is_wudfhost(pid);
    log::info!("kill_wudfhost: pid={pid} still_wudfhost={gone}");
    clear_marker_pid();
    ACCEPTING.store(0, Ordering::SeqCst);
    emit_status();

    Ok(if gone {
        format!("已结束 WUDFHost 进程（PID {pid}）；Windows 会自动重启它，注入已随之失效")
    } else {
        format!("已请求结束 PID {pid}，但它仍在运行；请重试或以管理员身份手动结束")
    })
}

#[cfg(not(target_os = "windows"))]
pub fn kill_wudfhost_command() -> Result<String, String> {
    Err("仅支持 Windows".into())
}

/// Attended path from `apply_suppression(true)` / connect / launch restore:
/// may elevate once. Never downloads; never fails the suppression toggle.
pub fn try_inject_if_ready() {
    run_inject(true);
}

/// Unattended path from the heartbeat-loss self-heal: a silent direct inject
/// only, so no UAC pops while the user is away.
pub fn try_reinject_if_ready() {
    run_inject(false);
}

fn run_inject(allow_elevation: bool) {
    if !allow_elevation {
        // Unattended self-heal runs only while the user actually wants
        // suppression and has not explicitly removed the hook. This covers
        // every silent caller (heartbeat loss, BLE reconnect).
        if ACCEPTING.load(Ordering::SeqCst) != 1 || !crate::keymap::snapshot().suppress {
            return;
        }
    }
    match ensure_injected(allow_elevation) {
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
        Err(InjectFailure::HostMissing) => {
            log::info!("tap inject skipped: no WUDFHost yet");
        }
        Err(InjectFailure::NeedsElevation(reason)) => {
            log::info!("tap needs elevation to re-inject ({reason}); waiting for the user");
            set_error("需要管理员权限重新注入：请在「HID Tap」点「注入」（可能弹一次 UAC）");
            emit_status();
        }
        Err(InjectFailure::Cancelled) => {
            // Only UI-initiated calls elevate, so this should not occur here.
        }
        Err(InjectFailure::Failed(error)) => {
            set_error(error);
            emit_status();
        }
    }
}
