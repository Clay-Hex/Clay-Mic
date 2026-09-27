//! Win32 inject/eject into WUDFHost and the elevated helper.

use super::*;
use super::host::module_loaded;


// ---------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------

/// Elevated entry: `Clay-Mic.exe --inject-tap <pid>` / `--eject-tap <pid>`.
/// Handled in `run()` before the Tauri builder (single-instance must not
/// swallow this second launch).
pub fn run_elevated_action(args: &[String]) -> i32 {
    let mode = args.iter().position(|a| a == "--inject-tap").map(|i| (i, true)).or_else(|| {
        args.iter()
            .position(|a| a == "--eject-tap")
            .map(|i| (i, false))
    });
    let Some((idx, inject)) = mode else {
        return -1; // not our invocation
    };
    let pid: u32 = match args.get(idx + 1).and_then(|value| value.parse().ok()) {
        Some(pid) => pid,
        None => {
            eprintln!("missing pid");
            return 1;
        }
    };
    log::info!("elevated helper: inject={inject} pid={pid}");
    let result = if inject {
        inject_dll_into(pid)
    } else {
        eject_dll_from(pid)
    };
    match &result {
        Ok(()) => log::info!("elevated helper: {inject} ok pid={pid}"),
        Err(error) => log::warn!("elevated helper: {inject} failed: {error}"),
    }
    write_status_file(pid, inject, result.as_ref().err().map(String::as_str));
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn write_status_file(pid: u32, inject: bool, error: Option<&str>) {
    let payload = serde_json::json!({
        "pid": pid,
        "inject": inject,
        "ok": error.is_none(),
        "error": error,
        "unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(
        status_path(),
        serde_json::to_vec_pretty(&payload).unwrap_or_default(),
    );
}

#[cfg(target_os = "windows")]
fn enable_se_debug_privilege() -> Result<(), String> {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
        SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| format!("OpenProcessToken: {error}"))?;

        let mut luid = windows::Win32::Foundation::LUID::default();
        LookupPrivilegeValueW(None, w!("SeDebugPrivilege"), &mut luid)
            .map_err(|error| format!("LookupPrivilegeValue: {error}"))?;

        let mut tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let ok = AdjustTokenPrivileges(token, false, Some(&mut tp), 0, None, None);
        let _ = CloseHandle(token);
        if ok.is_err() {
            return Err("AdjustTokenPrivileges failed".into());
        }
        Ok(())
    }
}

/// WUDFHost (UMFD) must be able to read the hook DLL under %LOCALAPPDATA%.
/// %LOCALAPPDATA% is user-only by default, so grant RX on the tap dir.
#[cfg(target_os = "windows")]
fn ensure_data_dir_readable() {
    let dir = data_dir();
    if !dir.is_dir() {
        return;
    }
    let mut command = std::process::Command::new("icacls");
    command.arg(dir.as_os_str()).args([
        "/grant",
        "*S-1-1-0:(OI)(CI)RX",
        "/T",
        "/Q",
        "/C",
    ]);
    crate::process::hide_console(&mut command);
    match command.status() {
        Ok(status) if status.success() => {
            log::info!("tap: granted RX on {}", dir.display());
        }
        Ok(status) => {
            log::warn!("tap: icacls exit {}", status.code().unwrap_or(-1));
        }
        Err(error) => log::warn!("tap: icacls spawn failed: {error}"),
    }
}

#[cfg(target_os = "windows")]
fn find_tap_module(
    process: windows::Win32::Foundation::HANDLE,
) -> Result<Option<windows::Win32::Foundation::HMODULE>, String> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::ProcessStatus::{EnumProcessModules, GetModuleBaseNameW};

    unsafe {
        let mut modules = [HMODULE::default(); 256];
        let mut needed = 0u32;
        EnumProcessModules(
            process,
            modules.as_mut_ptr(),
            (modules.len() * std::mem::size_of::<usize>()) as u32,
            &mut needed,
        )
        .map_err(|error| format!("枚举模块失败：{error}"))?;
        let count = (needed as usize / std::mem::size_of::<usize>()).min(modules.len());
        let mut name_buf = [0u16; 260];
        for module in &modules[..count] {
            name_buf.fill(0);
            let len = GetModuleBaseNameW(process, Some(*module), &mut name_buf);
            if len == 0 {
                continue;
            }
            let name = String::from_utf16_lossy(&name_buf[..len as usize]);
            if is_tap_module_name(&name) {
                return Ok(Some(*module));
            }
        }
        Ok(None)
    }
}

/// One FreeLibrary against an already-open process handle.
#[cfg(target_os = "windows")]
fn free_library_round(
    process: windows::Win32::Foundation::HANDLE,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Threading::{
        CreateRemoteThread, WaitForSingleObject, PROCESS_CREATE_THREAD, PROCESS_VM_OPERATION,
        PROCESS_VM_WRITE,
    };

    unsafe {
        let kernel32 = GetModuleHandleW(windows::core::w!("kernel32.dll"))
            .map_err(|error| format!("获取 kernel32 失败：{error}"))?;
        let free_library = GetProcAddress(kernel32, windows::core::s!("FreeLibrary"))
            .ok_or_else(|| "获取 FreeLibrary 失败".to_string())?;
        let start = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut c_void) -> u32,
        >(free_library);
        // CreateRemoteThread needs its own OpenProcess access mask in practice;
        // reuse the same handle if it already has create-thread rights.
        let _ = PROCESS_CREATE_THREAD | PROCESS_VM_OPERATION | PROCESS_VM_WRITE;
        let thread = CreateRemoteThread(
            process,
            None,
            0,
            Some(start),
            Some(module.0 as *mut c_void),
            0,
            None,
        )
        .map_err(|error| format!("创建远程线程失败：{error}"))?;
        let wait = WaitForSingleObject(thread, 10_000);
        let _ = CloseHandle(thread);
        if wait != WAIT_OBJECT_0 {
            return Err("等待 FreeLibrary 超时".into());
        }
    }
    Ok(())
}

/// Copy source → dest, retrying while dest is file-locked (os error 32).
/// Windows allows renaming a loaded DLL but not overwriting it, so on the
/// first lock failure move dest aside and write a fresh copy.
///
/// Returns `Ok(true)` when the rename-aside path was needed — that means the
/// old file was still mapped and the caller must NOT LoadLibrary yet (the
/// elevated path can enumerate and FreeLibrary properly).
fn copy_tap_dll(source: &std::path::Path, dest: &std::path::Path) -> Result<bool, String> {
    use std::io::ErrorKind;
    let mut renamed = false;
    for attempt in 0..8u32 {
        match std::fs::copy(source, dest) {
            Ok(_) => return Ok(renamed),
            Err(e)
                if e.raw_os_error() == Some(32) || e.kind() == ErrorKind::PermissionDenied =>
            {
                if attempt == 0 {
                    let stale = dest.with_extension("dll.stale");
                    let _ = std::fs::remove_file(&stale);
                    if std::fs::rename(dest, &stale).is_ok() {
                        renamed = true;
                    } else {
                        log::warn!("tap dll rename-aside failed: {e}");
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1) as u64));
            }
            Err(e) => return Err(format!("复制 hook DLL 失败：{e}")),
        }
    }
    Err("复制 hook DLL 失败：文件被占用，请先移除再注入".into())
}

/// Address of an exported function inside the target's already-loaded hook DLL.
/// The injector loads the same binary locally (DllMain is inert) just to read
/// the export RVA, then adds it to the remote module base.
#[cfg(target_os = "windows")]
fn remote_export_address(
    process: windows::Win32::Foundation::HANDLE,
    export: &str,
) -> Result<*const std::ffi::c_void, String> {
    use std::ffi::c_void;
    use windows::core::{PCSTR, PCWSTR};
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    let local_path = find_tap_dll_source()
        .or_else(|| deployed_tap_dll().is_file().then(deployed_tap_dll))
        .ok_or_else(|| String::from("组件未就绪（缺少 clay_tap.dll）"))?;
    let wide: Vec<u16> = local_path
        .display()
        .to_string()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut name: Vec<u8> = export.bytes().collect();
    name.push(0);

    unsafe {
        let local: HMODULE = LoadLibraryW(PCWSTR(wide.as_ptr()))
            .map_err(|error| format!("本地加载 hook DLL 失败：{error}"))?;
        let Some(proc) = GetProcAddress(local, PCSTR(name.as_ptr())) else {
            let _ = FreeLibrary(local);
            return Err(format!("hook DLL 缺少导出 {export}"));
        };
        let rva = proc as usize - local.0 as usize;
        let _ = FreeLibrary(local);
        let remote_base = find_tap_module(process)?
            .ok_or_else(|| "注入后未找到 hook DLL 模块".to_string())?;
        Ok((remote_base.0 as usize + rva) as *const c_void)
    }
}

/// Run the hook DLL's `clay_tap_init` in the target on its own thread, so the
/// socket + MinHook setup never runs under the loader lock.
#[cfg(target_os = "windows")]
fn call_remote_init(process: windows::Win32::Foundation::HANDLE) -> Result<(), String> {
    remote_call_export(process, "clay_tap_init", "创建初始化线程失败", "等待 hook 初始化超时", "hook 初始化失败（socket 或 hook 安装）")
}

/// Run `clay_tap_cleanup` in the target so MinHook unhooking happens on a
/// normal thread (not under the loader lock, which FreeLibrary holds).
#[cfg(target_os = "windows")]
fn call_remote_cleanup(process: windows::Win32::Foundation::HANDLE) -> Result<(), String> {
    remote_call_export(process, "clay_tap_cleanup", "创建清理线程失败", "等待 hook 清理超时", "hook 清理失败")
}

/// Generic helper: export an address from the hook DLL, call it via
/// `CreateRemoteThread`, wait for completion, check exit code.
#[cfg(target_os = "windows")]
fn remote_call_export(
    process: windows::Win32::Foundation::HANDLE,
    export: &str,
    thread_err: &str,
    timeout_err: &str,
    code_err: &str,
) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateRemoteThread, GetExitCodeThread, WaitForSingleObject,
    };

    let remote = remote_export_address(process, export)?;
    let start = unsafe {
        std::mem::transmute::<*const c_void, unsafe extern "system" fn(*mut c_void) -> u32>(remote)
    };
    unsafe {
        let thread = CreateRemoteThread(process, None, 0, Some(start), None, 0, None)
            .map_err(|error| format!("{thread_err}：{error}"))?;
        let wait = WaitForSingleObject(thread, 10_000);
        let mut code = 0u32;
        let got = GetExitCodeThread(thread, &mut code).is_ok();
        let _ = CloseHandle(thread);
        if wait != WAIT_OBJECT_0 {
            return Err(timeout_err.into());
        }
        if got && code != 0 {
            return Err(code_err.into());
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn inject_dll_into(pid: u32) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
    };
    use windows::Win32::System::Threading::{
        CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject,
        PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
        PROCESS_VM_WRITE,
    };

    let dest = deployed_tap_dll();
    let source = find_tap_dll_source()
        .or_else(|| dest.is_file().then_some(dest.clone()))
        .ok_or_else(|| String::from("组件未就绪（缺少 clay_tap.dll）"))?;

    std::fs::create_dir_all(data_dir())
        .map_err(|error| format!("创建目录失败：{error}"))?;
    let _ = enable_se_debug_privilege();
    ensure_data_dir_readable();

    // Canonicalize the destination directory (exists after create_dir_all) so
    // the remote LoadLibrary path points at deployed_tap_dll, not the source.
    let dir_abs = data_dir()
        .canonicalize()
        .map_err(|error| format!("解析路径失败：{error}"))?;
    let dest_abs = dir_abs.join(TAP_DLL_NAME);
    let mut path: Vec<u16> = dest_abs.display().to_string().encode_utf16().collect();
    path.push(0);
    let bytes = path.len() * 2;

    unsafe {
        // PROCESS_VM_READ is required for EnumProcessModules — without it the
        // pre-reload unload below can never see the old module.
        let access = PROCESS_CREATE_THREAD
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ
            | PROCESS_QUERY_INFORMATION;
        let Ok(process) = OpenProcess(access, false, pid) else {
            return Err("打开进程失败：需要管理员权限".into());
        };

        // Allocate and write the DLL path FIRST, while the target is in a
        // steady state. Unloading the old hook DLL is what makes VirtualAllocEx
        // fail transiently (loader lock / worker teardown) — so never alloc
        // after FreeLibrary.
        let remote = VirtualAllocEx(
            process,
            None,
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            let code = GetLastError().0;
            let _ = CloseHandle(process);
            return Err(format!("分配远程内存失败 (Win32 {code})"));
        }

        let mut written = 0usize;
        let write_result = windows::Win32::System::Diagnostics::Debug::WriteProcessMemory(
            process,
            remote,
            path.as_ptr().cast(),
            bytes,
            Some(&mut written),
        );
        if write_result.is_err() || written != bytes {
            let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            let _ = CloseHandle(process);
            return Err(format!("写入远程内存失败 (written={written})"));
        }

        // Unload first so dest is not file-locked.
        for round in 1..=8 {
            let Some(module) = find_tap_module(process)? else {
                break;
            };
            log::info!(
                "tap dll present in pid {pid}; FreeLibrary round {round} before reload"
            );
            free_library_round(process, module)?;
            if find_tap_module(process)?.is_none() {
                log::info!("tap dll unloaded after {round} round(s)");
                break;
            }
        }

        if source != dest {
            match copy_tap_dll(&source, &dest) {
                Ok(false) => {}
                Ok(true) => {
                    // dest was locked — old module still mapped; a direct
                    // LoadLibrary would reuse the stale module. Fail so
                    // inject_common elevates (admin can unload it properly).
                    let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
                    let _ = CloseHandle(process);
                    return Err("旧 hook DLL 仍占用文件，需要管理员权限卸载".into());
                }
                Err(error) => {
                    let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
                    let _ = CloseHandle(process);
                    return Err(error);
                }
            }
        }

        // Never LoadLibrary while the previous module is still mapped —
        // two copies would double-hook NtDeviceIoControlFile.
        if find_tap_module(process)?.is_some() {
            let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            let _ = CloseHandle(process);
            return Err("旧 hook DLL 仍在进程中，请先移除再注入".into());
        }

        let kernel32 = GetModuleHandleW(windows::core::w!("kernel32.dll"))
            .map_err(|error| format!("获取 kernel32 失败：{error}"))?;
        let load_library = GetProcAddress(kernel32, windows::core::s!("LoadLibraryW"))
            .ok_or_else(|| "获取 LoadLibraryW 失败".to_string())?;
        let load_start = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut c_void) -> u32,
        >(load_library);

        let thread = CreateRemoteThread(process, None, 0, Some(load_start), Some(remote), 0, None)
            .map_err(|error| format!("创建远程线程失败：{error}"))?;
        let wait = WaitForSingleObject(thread, 15_000);
        let mut exit_code = 0u32;
        let got_code = GetExitCodeThread(thread, &mut exit_code).is_ok();
        let _ = CloseHandle(thread);
        if wait != WAIT_OBJECT_0 {
            // The remote thread may still be reading `remote`; leaking the
            // buffer is safer than freeing memory the target is about to use.
            let _ = CloseHandle(process);
            return Err("等待 LoadLibrary 超时".into());
        }
        let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        // On x64 the HMODULE is truncated to u32; 0 still means LoadLibrary
        // returned NULL (path unreadable, dependency missing, or blocked).
        if got_code && exit_code == 0 {
            let _ = CloseHandle(process);
            return Err(
                "LoadLibraryW 返回失败：目标进程读不到 DLL（路径/权限）或依赖缺失".into(),
            );
        }
        if find_tap_module(process)?.is_none() {
            let _ = CloseHandle(process);
            return Err("注入未生效：DLL 未出现在目标模块列表".into());
        }

        // DllMain is inert; start the hook from its own thread so socket +
        // MinHook setup never runs under the loader lock.
        if let Err(error) = call_remote_init(process) {
            let _ = CloseHandle(process);
            return Err(error);
        }
        let _ = CloseHandle(process);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn inject_dll_into(_pid: u32) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

#[cfg(target_os = "windows")]
pub(super) fn eject_dll_from(pid: u32) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION,
        PROCESS_VM_READ, PROCESS_VM_WRITE,
    };

    // WUDFHost (UMFD) denies OpenProcess to a plain elevated token; SeDebug
    // is required the same way inject_dll_into enables it.
    let _ = enable_se_debug_privilege();

    let access = PROCESS_CREATE_THREAD
        | PROCESS_VM_OPERATION
        | PROCESS_VM_WRITE
        | PROCESS_QUERY_INFORMATION
        | PROCESS_VM_READ;
    let Ok(process) = (unsafe { OpenProcess(access, false, pid) }) else {
        return Err("打开进程失败：需要管理员权限".into());
    };

    // Clean up MinHook state on a normal remote thread (not under loader lock)
    // before attempting FreeLibrary — otherwise DETACH deadlocks.
    let _ = call_remote_cleanup(process);

    // The hook DLL may hold internal references; one FreeLibrary is not
    // always enough. Loop until the module list no longer contains it, then
    // re-verify so we never report success with a resident DLL.
    let result = (|| -> Result<(), String> {
        for round in 1..=8 {
            match find_tap_module(process)? {
                None => {
                    if module_loaded(pid) {
                        return Err("无法枚举 hook DLL 模块（权限不足？）".into());
                    }
                    log::info!("eject: hook DLL already gone before round {round}");
                    return Ok(());
                }
                Some(module) => {
                    log::info!("eject: FreeLibrary round {round} on pid {pid}");
                    free_library_round(process, module)?;
                    if find_tap_module(process)?.is_none() {
                        if !module_loaded(pid) {
                            log::info!("eject: verified unloaded after {round} round(s)");
                            return Ok(());
                        }
                        return Err("卸载后仍检测到 hook DLL 模块".into());
                    }
                }
            }
        }
        Err("旧 hook DLL 无法卸载（引用计数未归零）".into())
    })();
    unsafe {
        let _ = CloseHandle(process);
    }
    result
}

#[cfg(not(target_os = "windows"))]
pub(super) fn eject_dll_from(_pid: u32) -> Result<(), String> {
    Err("仅支持 Windows".into())
}
