//! Host discovery: registry HostPid, process checks, inject/script markers.

use super::*;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

fn configured_identity() -> Option<(u16, u16)> {
    let state = crate::runtime::state()?;
    let config = state.config.lock().ok()?;
    Some((config.device.vendor_id?, config.device.product_id?))
}

pub(super) fn configured_identity_display() -> Option<String> {
    let (vid, pid) = configured_identity()?;
    Some(format!("VID={vid:04X} PID={pid:04X}"))
}

/// MAC from `device_id` (`BTHLE\DEV_14BEFCF3D837\...`) without separators.
pub(super) fn configured_mac() -> Option<String> {
    let state = crate::runtime::state()?;
    let config = state.config.lock().ok()?;
    let id = config.device.device_id.as_ref()?;
    let upper = id.to_ascii_uppercase();
    let start = upper.find("DEV_")? + 4;
    let rest = &upper[start..];
    let mac: String = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if mac.len() == 12 {
        Some(mac)
    } else {
        None
    }
}

/// Walk `BTHLEDevice` for a WUDFHost pid belonging to the configured remote.
pub(super) fn find_host_pid() -> Result<Option<u32>, String> {
    find_host_pid_ex().map(|(pid, _)| pid)
}

/// Same as [`find_host_pid`] plus a human-readable lookup note for the UI.
pub(super) fn find_host_pid_ex() -> Result<(Option<u32>, Option<String>), String> {
    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY,
            HKEY_LOCAL_MACHINE, KEY_READ,
        };

        let identity = configured_identity();
        let mac = configured_mac();
        if identity.is_none() && mac.is_none() {
            return Ok((
                None,
                Some("配置中无设备身份（请先连接遥控器）".into()),
            ));
        }

        fn open_sub(parent: HKEY, name: &[u16]) -> Option<HKEY> {
            let mut key = HKEY(std::ptr::null_mut());
            let opened =
                unsafe { RegOpenKeyExW(parent, PCWSTR(name.as_ptr()), None, KEY_READ, &mut key) };
            if opened.0 == 0 {
                Some(key)
            } else {
                None
            }
        }

        fn to_wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        /// BLE instance keys zero-pad ids (`VID&012717`); only the low 4 hex
        /// digits carry the real VID/PID — same rule as `hid::win::parse_vid_pid`.
        fn id_after(haystack: &str, tag: &str) -> Option<u16> {
            let rest = &haystack[haystack.find(tag)? + tag.len()..];
            let digits: String = rest
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if digits.len() < 4 {
                return None;
            }
            u16::from_str_radix(&digits[digits.len() - 4..], 16).ok()
        }

        fn key_matches(
            upper: &str,
            identity: Option<(u16, u16)>,
            mac: Option<&str>,
        ) -> bool {
            if let Some((vid, pid)) = identity {
                let key_vid = id_after(upper, "VID&").or_else(|| id_after(upper, "VID_"));
                let key_pid = id_after(upper, "PID&").or_else(|| id_after(upper, "PID_"));
                if key_vid == Some(vid) && key_pid == Some(pid) {
                    return true;
                }
            }
            // Registry key suffix is the Bluetooth MAC, e.g. `..._14BEFCF3D837`.
            if let Some(mac) = mac {
                if upper.ends_with(mac) || upper.contains(&format!("_{mac}")) {
                    return true;
                }
            }
            false
        }

        fn enum_key_names(key: HKEY) -> Vec<String> {
            let mut names = Vec::new();
            let mut index = 0u32;
            loop {
                let mut name_buf = [0u16; 256];
                let mut name_len = name_buf.len() as u32;
                let status = unsafe {
                    RegEnumKeyExW(
                        key,
                        index,
                        Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                        &mut name_len,
                        None,
                        None,
                        None,
                        None,
                    )
                };
                if status.0 != 0 {
                    break;
                }
                names.push(String::from_utf16_lossy(&name_buf[..name_len as usize]));
                index += 1;
                if index > 512 {
                    break;
                }
            }
            names
        }

        fn read_dword(key: HKEY, name: &str) -> Option<u32> {
            let wide = to_wide(name);
            let mut data = [0u8; 8];
            let mut data_len = 8u32;
            let status = unsafe {
                RegQueryValueExW(
                    key,
                    PCWSTR(wide.as_ptr()),
                    None,
                    None,
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if status.0 != 0 {
                return None;
            }
            // HostPid may be REG_DWORD (4) or REG_QWORD (8); both little-endian.
            match data_len {
                4 => Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
                8 => Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
                _ => None,
            }
        }

        /// DFS for `WUDFDiagnosticInfo\HostPid`. Returns (pid, saw_diag_key).
        fn find_host_pid_under(key: HKEY, depth: u32) -> (Option<u32>, bool) {
            if depth > 8 {
                return (None, false);
            }
            let mut saw_diag = false;
            for name in enum_key_names(key) {
                let wide = to_wide(&name);
                let Some(sub) = open_sub(key, &wide) else {
                    continue;
                };
                let (hit, child_saw) = if name.eq_ignore_ascii_case("WUDFDiagnosticInfo") {
                    saw_diag = true;
                    (read_dword(sub, "HostPid").filter(|pid| *pid > 0), true)
                } else {
                    find_host_pid_under(sub, depth + 1)
                };
                saw_diag = saw_diag || child_saw;
                unsafe {
                    let _ = RegCloseKey(sub);
                }
                if hit.is_some() {
                    return (hit, true);
                }
            }
            (None, saw_diag)
        }

        let root_wide = to_wide(r"SYSTEM\CurrentControlSet\Enum\BTHLEDevice");
        let Some(root) = open_sub(HKEY_LOCAL_MACHINE, &root_wide) else {
            return Ok((None, Some("无法打开 BTHLEDevice 注册表项".into())));
        };

        // Prefer the HID-over-GATT service (0x1812): only that node is hosted
        // by WUDFHost. Custom services (ATVV ab5e0001, etc.) have no HostPid.
        let mut candidates: Vec<String> = enum_key_names(root)
            .into_iter()
            .filter(|name| {
                let upper = name.to_ascii_uppercase();
                key_matches(&upper, identity, mac.as_deref())
            })
            .collect();
        candidates.sort_by_key(|name| {
            let upper = name.to_ascii_uppercase();
            // Lower sort key = earlier: HID first, then the rest.
            if upper.contains("00001812") {
                0
            } else {
                1
            }
        });

        let mut found = None;
        let mut stale = None;
        let mut matched_key = None;
        let mut tried: Vec<String> = Vec::new();
        let mut saw_diag = false;
        for service_name in &candidates {
            tried.push(service_name.clone());
            matched_key = Some(service_name.clone());
            let service_wide = to_wide(service_name);
            let Some(service) = open_sub(root, &service_wide) else {
                continue;
            };
            let (hit, child_saw) = find_host_pid_under(service, 0);
            saw_diag = saw_diag || child_saw;
            unsafe {
                let _ = RegCloseKey(service);
            }
            if let Some(pid) = hit {
                // Registry HostPid can outlive the process (WUDFHost restarted
                // under a new instance key). Prefer a PID that is still WUDFHost.
                if pid_is_wudfhost(pid) {
                    found = hit;
                    break;
                }
                stale.get_or_insert(pid);
            }
        }
        unsafe {
            let _ = RegCloseKey(root);
        }

        let note = if let Some(pid) = found {
            Some(format!("HostPid={pid}"))
        } else if let Some(pid) = stale {
            Some(format!("HostPid={pid}（进程已退出）"))
        } else if saw_diag {
            Some("找到 WUDFDiagnosticInfo 但读 HostPid 失败（值缺失或权限不足）".into())
        } else if matched_key.is_some() {
            let preview: Vec<_> = tried.iter().take(3).map(|k| short_key(k)).collect();
            Some(format!(
                "已匹配 {} 个设备键但均无 WUDFDiagnosticInfo：{}",
                tried.len(),
                preview.join(" | ")
            ))
        } else {
            let ident = identity
                .map(|(v, p)| format!("VID={v:04X} PID={p:04X}"))
                .unwrap_or_else(|| "无 VID/PID".into());
            let mac_note = mac.map(|m| format!(" MAC={m}")).unwrap_or_default();
            Some(format!("BTHLEDevice 中无匹配键（{ident}{mac_note}）"))
        };
        if found.is_none() {
            log::warn!("tap lookup: {}", note.as_deref().unwrap_or("?"));
        }
        Ok((found, note))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok((None, Some("仅支持 Windows".into())))
    }
}

/// Trim a long registry key for the status UI (`{guid}_Dev_…` → leading GUID).
fn short_key(key: &str) -> String {
    if key.len() <= 48 {
        return key.to_string();
    }
    format!("{}…", &key[..48])
}

pub(super) fn pid_is_wudfhost(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        // Snapshot enumeration works without opening the target process —
        // WUDFHost runs as UMFD and often denies OpenProcess to medium IL.
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return false;
            };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut ok = Process32FirstW(snap, &mut entry).is_ok();
            while ok {
                if entry.th32ProcessID == pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    let _ = CloseHandle(snap);
                    return name.eq_ignore_ascii_case("WUDFHost.exe");
                }
                ok = Process32NextW(snap, &mut entry).is_ok();
            }
            let _ = CloseHandle(snap);
            false
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = pid;
        false
    }
}

pub(super) fn read_marker_pid() -> Option<u32> {
    std::fs::read_to_string(injected_pid_path())
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

pub(super) fn write_marker_pid(pid: u32) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(injected_pid_path(), pid.to_string());
}

pub(super) fn clear_marker_pid() {
    let _ = std::fs::remove_file(injected_pid_path());
}

pub(super) fn module_loaded(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::ProcessStatus::{
            EnumProcessModules, GetModuleBaseNameW,
        };
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        };

        unsafe {
            let Ok(handle) =
                OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
            else {
                return false;
            };
            let mut modules = [windows::Win32::Foundation::HMODULE::default(); 256];
            let mut needed = 0u32;
            let ok = EnumProcessModules(
                handle,
                modules.as_mut_ptr(),
                (modules.len() * std::mem::size_of::<usize>()) as u32,
                &mut needed,
            );
            if ok.is_err() {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                return false;
            }
            let count = (needed as usize / std::mem::size_of::<usize>()).min(modules.len());
            let mut name_buf = [0u16; 260];
            for module in &modules[..count] {
                name_buf.fill(0);
                let len = GetModuleBaseNameW(handle, Some(*module), &mut name_buf);
                if len == 0 {
                    continue;
                }
                let name = String::from_utf16_lossy(&name_buf[..len as usize]);
                if is_tap_module_name(&name) {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                    return true;
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            false
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = pid;
        false
    }
}

pub(super) fn resolve_injected_pid(host_pid: Option<u32>) -> Option<u32> {
    // Marker is the authority: remove clears it, and a stale module listing
    // (or a DLL that FreeLibrary could not unload) must not resurrect the
    // "injected" status.
    let marker = read_marker_pid()?;
    // Medium IL often cannot enumerate UMFD modules; trust a marker whose
    // process is still WUDFHost even if HostPid drifted to another instance.
    if pid_is_wudfhost(marker) {
        return Some(marker);
    }
    if let Some(pid) = host_pid {
        if pid == marker && module_loaded(pid) {
            return Some(pid);
        }
    }
    if module_loaded(marker) {
        return Some(marker);
    }
    clear_marker_pid();
    None
}
