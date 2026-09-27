//! Ships, registers and inspects the keyboard/mouse device-numbering helper.
//!
//! Windows increments `\Device\KeyboardClassN` / `PointerClassN` on every
//! device re-enumeration (sleep/resume, replug). The Interception driver parses
//! that suffix as a single character, so `KeyboardClass10` reads as slot `0` and
//! wedges the device stack. `clay-mic-keyslots.exe` occupies the high-numbered
//! names with links that fold back onto `0`..`9`, and runs as a service at boot
//! because the links are gone after every restart.
//!
//! This module never creates a link itself: it copies the helper out of the
//! install directory, registers it, and reads back the status file the helper
//! writes.

use std::path::PathBuf;

pub const SERVICE_NAME: &str = "clay-mic-keyslots";
const HELPER_EXE: &str = "clay-mic-keyslots.exe";
const DEFAULT_COUNT: usize = 1000;

/// Whether the helper is registered and what its last run did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KeySlotStatus {
    /// The service key exists, so the fix is meant to run at boot.
    pub service_installed: bool,
    pub helper_found: bool,
    pub helper_path: Option<String>,
    /// UNIX seconds of the last run, absent when the helper never ran.
    pub applied_unix: Option<u64>,
    pub keyboard: Option<u32>,
    pub pointer: Option<u32>,
    pub ok: Option<bool>,
    pub error: Option<String>,
}

/// Shape of `keyslots.json`, written by the helper.
#[derive(Debug, Clone, serde::Deserialize)]
struct StatusFile {
    applied_unix: u64,
    keyboard: u32,
    pointer: u32,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// `%LOCALAPPDATA%\clay-mic\keyslots` — helper binary + status file.
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("keyslots")
}

fn installed_helper() -> PathBuf {
    data_dir().join(HELPER_EXE)
}

/// The helper next to the app, the copy in the data directory, and — for dev
/// builds — the release artifact in this workspace's target dir.
fn helper_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            candidates.push(dir.join(HELPER_EXE));
        }
    }
    candidates.push(installed_helper());
    // Dev fallback: `npm run build:keyslots:release` never stages a copy, and
    // `tauri dev` runs from target/debug where no sibling helper exists.
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("release")
            .join(HELPER_EXE),
    );
    candidates
}

fn read_status_file() -> Option<StatusFile> {
    let raw = std::fs::read_to_string(data_dir().join("keyslots.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn status() -> KeySlotStatus {
    let helper = helper_candidates().into_iter().find(|path| path.is_file());
    let file = read_status_file();
    KeySlotStatus {
        service_installed: service_installed(),
        helper_found: helper.is_some(),
        helper_path: helper.map(|path| path.to_string_lossy().into_owned()),
        applied_unix: file.as_ref().map(|file| file.applied_unix),
        keyboard: file.as_ref().map(|file| file.keyboard),
        pointer: file.as_ref().map(|file| file.pointer),
        ok: file.as_ref().map(|file| file.ok),
        error: file.as_ref().and_then(|file| file.error.clone()),
    }
}

/// Copy the helper out of the install directory into the app data directory,
/// which is where the service is registered to run from. Needs no elevation.
pub fn stage_helper() -> Result<PathBuf, String> {
    let source = helper_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| format!("未找到 {HELPER_EXE}，请重新安装 clay-mic"))?;
    let target = installed_helper();
    if source == target {
        return Ok(target);
    }
    // size + mtime stand in for a byte compare; `set_modified` below makes a
    // fresh copy converge so the next launch skips the write entirely.
    if let (Ok(src), Ok(dst)) = (std::fs::metadata(&source), std::fs::metadata(&target)) {
        if src.len() == dst.len() && src.modified().ok() == dst.modified().ok() {
            return Ok(target);
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建数据目录失败：{error}"))?;
    }
    std::fs::copy(&source, &target).map_err(|error| format!("复制 helper 失败：{error}"))?;
    if let Ok(time) = std::fs::metadata(&source).and_then(|meta| meta.modified()) {
        if let Ok(file) = std::fs::File::options().write(true).open(&target) {
            let _ = file.set_modified(time);
        }
    }
    Ok(target)
}

/// Keep the copy the boot service runs from in step with this build. The
/// service re-reads the same data_dir path on every boot, so a silent copy
/// here is all an app update needs — no re-registration.
pub fn refresh_staged_helper() {
    if !service_installed() {
        return;
    }
    if let Err(error) = stage_helper() {
        log::warn!("keyslots: staged helper refresh failed: {error}");
    }
}

#[cfg(target_os = "windows")]
pub fn service_installed() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };

    let path: Vec<u16> = format!(r"SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut key = HKEY(std::ptr::null_mut());
        let opened = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        );
        if opened.0 != 0 {
            return false;
        }
        let _ = RegCloseKey(key);
        true
    }
}

#[cfg(not(target_os = "windows"))]
pub fn service_installed() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn quote(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

/// Register the helper as an auto-start service and apply the links once, so
/// the user does not have to reboot before seeing a result.
#[cfg(target_os = "windows")]
pub(crate) fn install_script(helper: &std::path::Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$name = '{name}'
$helper = '{helper}'
if (Get-Service -Name $name -ErrorAction SilentlyContinue) {{
  Stop-Service -Name $name -Force -ErrorAction SilentlyContinue
  sc.exe delete $name | Out-Null
  for ($i = 0; $i -lt 20; $i++) {{
    sc.exe query $name 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {{ break }}
    Start-Sleep -Milliseconds 250
  }}
}}
        $binary = '"' + $helper + '" --service --count {count} --data-dir "{data_dir}"'
New-Service -Name $name -BinaryPathName $binary -DisplayName 'clay-mic 驱动补丁' -StartupType Automatic | Out-Null
Start-Service -Name $name
"#,
        name = SERVICE_NAME,
        helper = quote(helper),
        count = DEFAULT_COUNT,
        data_dir = quote(&data_dir()),
    )
}

/// Deleting the links needs SYSTEM for the same reason creating them does, so
/// the removal goes through the service too: `sc start` passes its extra
/// arguments into the service process, which then sees `--remove`.
#[cfg(target_os = "windows")]
pub(crate) fn uninstall_script() -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$name = '{name}'
if (Get-Service -Name $name -ErrorAction SilentlyContinue) {{
  sc.exe start $name --remove | Out-Null
  if ($LASTEXITCODE -ne 0) {{ throw '无法停止服务' }}
  for ($i = 0; $i -lt 30; $i++) {{
    $service = Get-Service -Name $name -ErrorAction SilentlyContinue
    if ($null -eq $service -or $service.Status -eq 'Stopped') {{ break }}
    Start-Sleep -Milliseconds 500
  }}
  Stop-Service -Name $name -Force -ErrorAction SilentlyContinue
  sc.exe delete $name | Out-Null
  if ($LASTEXITCODE -ne 0) {{ throw '无法删除服务' }}
}}
"#,
        name = SERVICE_NAME,
    )
}

/// Run a PowerShell script from a temp file with administrator rights via
/// `ShellExecuteExW("runas")`. A cancelled UAC prompt returns `ERROR_CANCELLED`.
#[cfg(target_os = "windows")]
pub(crate) fn run_elevated(body: &str, tag: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
    use windows::core::PCWSTR;

    let script = std::env::temp_dir().join(format!("clay-mic-{tag}.ps1"));
    let log = std::env::temp_dir().join(format!("clay-mic-{tag}.log"));
    let _ = std::fs::remove_file(&log);

    // Wrap the body in try/catch so the elevated script reports its actual
    // error to a temp file — the Win32 API cannot capture stdout/stderr.
    let wrapped = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $log = '{log}'\n\
         try {{\n\
         {body}\n\
         [System.IO.File]::WriteAllText($log, 'OK')\n\
         }} catch {{\n\
         [System.IO.File]::WriteAllText($log, $_.Exception.Message)\n\
         exit 1\n\
         }}",
        log = log.display().to_string().replace('\'', "''"),
        body = body,
    );

    // Windows PowerShell 5.1 decodes a BOM-less script with the system ANSI
    // code page, which mangles non-ASCII text. A UTF-8 BOM forces UTF-8.
    std::fs::write(&script, format!("\u{FEFF}{wrapped}"))
        .map_err(|error| format!("写入脚本失败：{error}"))?;

    let params = format!(
        "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"",
        script.display()
    );
    let verb = to_wide("runas");
    let file = to_wide("powershell.exe");
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
                let _ = std::fs::remove_file(&script);
                let _ = std::fs::remove_file(&log);
                // HRESULT_FROM_WIN32(ERROR_CANCELLED) = 0x800704C7
                if error.code() == ERROR_CANCELLED.to_hresult() {
                    return Err("操作被取消".into());
                }
                return Err(format!("启动提权进程失败：{error}"));
            }
        }
    };

    if process.is_invalid() {
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&log);
        return Err("未获取到进程句柄".into());
    }

    let exit_code = unsafe {
        WaitForSingleObject(process, u32::MAX);
        let mut code = 0u32;
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        code
    };
    let _ = std::fs::remove_file(&script);

    if exit_code == 0 {
        // Only trust success when the elevated script wrote "OK" to the log.
        let confirmed = std::fs::read_to_string(&log)
            .ok()
            .is_some_and(|content| content.trim() == "OK");
        let _ = std::fs::remove_file(&log);
        if confirmed {
            Ok(())
        } else {
            Err("操作未完成：可能被取消或未获得管理员权限".into())
        }
    } else {
        let detail = std::fs::read_to_string(&log)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "OK")
            .unwrap_or_else(|| "可能被取消或未获得管理员权限".into());
        let _ = std::fs::remove_file(&log);
        Err(format!("操作未完成（exit {exit_code}）：{detail}"))
    }
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
