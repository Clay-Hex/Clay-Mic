//! Runtime bindings for the Interception keyboard filter driver.
//!
//! `interception.dll` is opened with `libloading` the first time it is needed,
//! so the executable has no link-time dependency on the driver. Everything that
//! crosses the DLL boundary is declared `#[repr(C)]`, which keeps the in-memory
//! layout byte-for-byte identical to what the driver expects.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};

// ===========================================================================
// Wire format exchanged with the driver
// ===========================================================================

/// Filter mask that captures every key of a device.
pub const KEY_FILTER_ALL: u16 = 0xffff;
/// Filter mask that captures nothing; releases a device filtered earlier.
pub const KEY_FILTER_NONE: u16 = 0x0000;
/// [`KeyEvent::flags`] bit marking the release edge of a key.
pub const STROKE_KEY_UP: u16 = 0x0001;
/// [`KeyEvent::flags`] bit marking a key delivered with the E0 prefix.
pub const STROKE_KEY_E0: u16 = 0x0002;
/// Highest keyboard device id the driver exposes; ids are 1-based.
pub const KEYBOARD_SLOT_COUNT: i32 = 10;

/// One keyboard event in the layout `interception_receive` produces.
///
/// `scan_code` holds the Set-1 scan code (the HID usage id for HID keyboards),
/// `flags` combines [`STROKE_KEY_UP`] and [`STROKE_KEY_E0`], and `reserved` is
/// driver-private bookkeeping that callers routinely ignore.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyEvent {
    pub scan_code: u16,
    pub flags: u16,
    pub reserved: u32,
}

// ===========================================================================
// Raw ABI
// ===========================================================================

/// Opaque handle returned by `interception_create_context`.
type Context = *mut c_void;

/// Predicate the driver invokes once per device while a filter is installed.
///
/// A non-zero answer means "filter this device as well". It has to be a real
/// `extern "C"` function pointer: the driver jumps straight to whatever address
/// it is handed, so passing an integer here would take the process down.
type DeviceSelector = unsafe extern "C" fn(i32) -> i32;

/// Device the pending [`Driver::set_key_filter`] call wants to target.
static FILTER_DEVICE: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" fn only_filtered_device(device: i32) -> i32 {
    i32::from(device == FILTER_DEVICE.load(Ordering::Relaxed))
}

/// Declare the DLL export table in one place: every entry pairs a struct field
/// with the C symbol it is resolved from.
macro_rules! dll_exports {
    ($( $field:ident : $signature:ty = $symbol:literal ),+ $(,)?) => {
        /// Function pointers resolved from the loaded library.
        struct Exports {
            $( $field: $signature, )+
        }

        impl Exports {
            /// Resolve every declared export, failing on the first miss.
            ///
            /// # Safety
            /// Each C symbol must really carry the signature declared for it.
            unsafe fn resolve(library: &libloading::Library) -> Result<Self, String> {
                Ok(Self {
                    $(
                        $field: {
                            let handle = library
                                .get::<$signature>(concat!($symbol, "\0").as_bytes())
                                .map_err(|error| format!("{}: {error}", $symbol))?;
                            *handle
                        },
                    )+
                })
            }
        }
    };
}

dll_exports! {
    make_context: unsafe extern "C" fn() -> Context = "interception_create_context",
    drop_context: unsafe extern "C" fn(Context) = "interception_destroy_context",
    set_key_filter: unsafe extern "C" fn(Context, DeviceSelector, u16) = "interception_set_filter",
    wait_for_device: unsafe extern "C" fn(Context, u32) -> i32 = "interception_wait_with_timeout",
    read_events: unsafe extern "C" fn(Context, i32, *mut KeyEvent, u32) -> i32 = "interception_receive",
    write_events: unsafe extern "C" fn(Context, i32, *const KeyEvent, u32) -> i32 = "interception_send",
    hardware_id: unsafe extern "C" fn(Context, i32, *mut c_void, u32) -> u32 = "interception_get_hardware_id",
}

// ===========================================================================
// Loaded driver
// ===========================================================================

/// A loaded `interception.dll` together with its resolved exports.
///
/// The library is kept alive inside the struct, so the function pointers stay
/// valid for exactly as long as the `Driver` does.
pub struct Driver {
    _library: libloading::Library,
    exports: Exports,
}

// SAFETY: the Interception context API is documented as safe to drive from any
// thread, and the symbols outlive the `Driver` that owns them.
unsafe impl Send for Driver {}
unsafe impl Sync for Driver {}

impl Driver {
    /// Open the DLL from the first usable location and resolve its exports.
    pub fn open() -> Result<Self, String> {
        let library = open_library()?;
        // SAFETY: `Exports::resolve` reads function pointers out of the genuine
        // Interception build, and the declared signatures mirror its header.
        let exports = unsafe { Exports::resolve(&library)? };
        Ok(Self {
            _library: library,
            exports,
        })
    }

    /// Create a fresh interception context (`interception_create_context`).
    pub fn make_context(&self) -> Context {
        unsafe { (self.exports.make_context)() }
    }

    /// Release a context obtained from [`Driver::make_context`].
    pub fn drop_context(&self, context: Context) {
        unsafe { (self.exports.drop_context)(context) }
    }

    /// Install a key filter on a single device id (1-based).
    ///
    /// [`KEY_FILTER_ALL`] captures every key from that device, while
    /// [`KEY_FILTER_NONE`] releases it again. The driver probes devices through
    /// `only_filtered_device`, which answers for `device` alone; the predicate
    /// must therefore be a genuine function pointer.
    pub fn set_key_filter(&self, context: Context, device: i32, mask: u16) {
        FILTER_DEVICE.store(device, Ordering::Relaxed);
        unsafe { (self.exports.set_key_filter)(context, only_filtered_device, mask) }
    }

    /// Block until a filtered device has input, or `timeout_ms` elapses.
    ///
    /// Returns the device id (> 0) on success and 0 on timeout or error.
    pub fn wait_for_device(&self, context: Context, timeout_ms: u32) -> i32 {
        unsafe { (self.exports.wait_for_device)(context, timeout_ms) }
    }

    /// Pull up to `events.len()` strokes out of `device`.
    ///
    /// Returns the number of events written, or 0 on error.
    pub fn read_events(&self, context: Context, device: i32, events: &mut [KeyEvent]) -> i32 {
        unsafe {
            (self.exports.read_events)(context, device, events.as_mut_ptr(), events.len() as u32)
        }
    }

    /// Push `events` back onto `device`.
    ///
    /// Returns the number of events the driver accepted, or 0 on error.
    pub fn write_events(&self, context: Context, device: i32, events: &[KeyEvent]) -> i32 {
        unsafe {
            (self.exports.write_events)(context, device, events.as_ptr(), events.len() as u32)
        }
    }

    /// Copy the hardware id of `device` into `buffer`.
    ///
    /// Returns the number of characters written, null terminator included.
    pub fn hardware_id(&self, context: Context, device: i32, buffer: &mut [u8]) -> u32 {
        unsafe {
            (self.exports.hardware_id)(
                context,
                device,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
            )
        }
    }
}

/// Try each known DLL location until one loads.
fn open_library() -> Result<libloading::Library, String> {
    let mut searched = Vec::new();
    for candidate in dll_candidates() {
        // SAFETY: loading a DLL runs its initializer, which is the intent here;
        // the candidate is the official Interception build.
        match unsafe { libloading::Library::new(&candidate) } {
            Ok(library) => {
                log::info!("opened Interception library: {}", candidate.display());
                return Ok(library);
            }
            Err(error) => {
                log::debug!("not usable: {} ({error})", candidate.display());
                searched.push(candidate);
            }
        }
    }
    Err(format!(
        "interception.dll was not found; searched {searched:?}"
    ))
}

// ===========================================================================
// DLL search paths
// ===========================================================================

/// Locations searched for `interception.dll`, most specific first.
fn dll_candidates() -> Vec<PathBuf> {
    const DLL_NAME: &str = "interception.dll";
    let mut candidates = Vec::new();

    // 1. The directory this app stages its own download into.
    candidates.push(dll_path());

    // 2. Next to the running executable.
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join(DLL_NAME));
    }

    // 3. The bare name, letting the Windows loader use its default search.
    candidates.push(PathBuf::from(DLL_NAME));

    // 4. The system directory, where the official installer puts it.
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        candidates.push(PathBuf::from(system_root).join("System32").join(DLL_NAME));
    }

    // 5. Every directory on PATH.
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(DLL_NAME)));
    }

    candidates
}

// ===========================================================================
// Driver package lifecycle (status / download / install)
// ===========================================================================

#[cfg(target_os = "windows")]
const PACKAGE_URL: &str =
    "https://github.com/oblitum/Interception/releases/download/v1.0.1/Interception.zip";

/// Directory this app populates with a downloaded Interception package.
pub fn install_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("interception")
}

fn dll_path() -> PathBuf {
    install_dir().join("interception.dll")
}

/// Driver state surfaced to the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DriverStatus {
    pub dll_found: bool,
    pub dll_path: Option<String>,
    pub driver_ready: bool,
    pub installer_found: bool,
    pub install_dir: String,
}

pub fn status() -> DriverStatus {
    let dll = dll_candidates().into_iter().find(|path| path.is_file());
    let driver_ready = dll.is_some() && driver_responds();
    DriverStatus {
        dll_found: dll.is_some(),
        dll_path: dll.map(|path| path.to_string_lossy().into_owned()),
        driver_ready,
        installer_found: locate_installer().is_some(),
        install_dir: install_dir().to_string_lossy().into_owned(),
    }
}

/// Open a context and drop it right away: this only succeeds while the kernel
/// driver is actually resident.
fn driver_responds() -> bool {
    let Ok(driver) = Driver::open() else {
        return false;
    };
    let context = driver.make_context();
    if context.is_null() {
        return false;
    }
    driver.drop_context(context);
    true
}

/// List `dir` as (files, subdirectories); both empty when it cannot be read.
fn scan(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (files, subdirs);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else {
            files.push(path);
        }
    }
    (files, subdirs)
}

fn name_matches(path: &Path, name: &str) -> bool {
    path.file_name()
        .is_some_and(|file| file.eq_ignore_ascii_case(name))
}

/// Depth-first search for a file whose name matches `name` (case-insensitive).
fn locate_file(root: &Path, name: &str) -> Option<PathBuf> {
    let (files, subdirs) = scan(root);
    if let Some(hit) = files.into_iter().find(|path| name_matches(path, name)) {
        return Some(hit);
    }
    subdirs.into_iter().find_map(|dir| locate_file(&dir, name))
}

/// Depth-first search for `<arch>/<name>`, requiring the parent folder to match.
fn locate_arch_file(root: &Path, name: &str, arch: &str) -> Option<PathBuf> {
    let (files, subdirs) = scan(root);
    if let Some(hit) = files.into_iter().find(|path| {
        name_matches(path, name)
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|parent| parent.eq_ignore_ascii_case(arch))
    }) {
        return Some(hit);
    }
    subdirs
        .into_iter()
        .find_map(|dir| locate_arch_file(&dir, name, arch))
}

pub(crate) fn locate_installer() -> Option<PathBuf> {
    let staged = install_dir().join("install-interception.exe");
    if staged.is_file() {
        return Some(staged);
    }
    locate_file(&install_dir(), "install-interception.exe")
}

/// Download the official package and stage `interception.dll` + the installer.
pub async fn download() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let dir = install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{e}"))?;

        let zip = std::env::temp_dir().join("clay-mic-interception.zip");
        crate::download::stream_download(PACKAGE_URL, &zip, "interception").await?;

        crate::download::emit_progress("interception", "extracting", 0);
        let script = format!(
            "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
            zip.display(),
            dir.display()
        );
        let mut command = std::process::Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        crate::process::hide_console(&mut command);
        let status = command
            .status()
            .map_err(|e| format!("调用 PowerShell 解压失败：{e}"))?;
        let _ = std::fs::remove_file(&zip);
        if !status.success() {
            return Err("解压 Interception 压缩包失败".into());
        }

        // The archive ships both x86 and x64 under `library/`; stage the one
        // matching this build, plus the installer.
        let arch = if cfg!(target_arch = "x86_64") {
            "x64"
        } else {
            "x86"
        };
        let library = locate_arch_file(&dir, "interception.dll", arch)
            .ok_or_else(|| format!("压缩包中未找到 {arch} 版 interception.dll"))?;
        let target = dll_path();
        if let Err(error) = std::fs::copy(&library, &target) {
            // A DLL this process already loaded is locked; if the existing copy
            // is working there is nothing to update.
            if target.is_file() && driver_responds() {
                log::info!("interception.dll in use; keeping the loaded copy ({error})");
            } else {
                return Err(format!("复制 DLL 失败：{error}"));
            }
        }
        if let Some(installer) = locate_file(&dir, "install-interception.exe") {
            std::fs::copy(&installer, dir.join("install-interception.exe"))
                .map_err(|e| format!("复制安装程序失败：{e}"))?;
        }
        crate::download::emit_progress("interception", "done", 100);
        Ok(dir)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Interception 驱动仅支持 Windows".into())
    }
}

// ===========================================================================
// Filter chain health
// ===========================================================================
//
// The driver keeps a fixed set of keyboard device slots (its own limit is the
// single-character device suffix, so KbdClass0..KbdClass9). Every device
// re-enumeration — including the one Windows performs on resume from sleep —
// consumes another slot. Once they run out, the filter fails to attach and the
// affected device stops delivering input at all, which cannot be reset without
// a reboot.
//
// The keyslot patch (see `keyslots`) keeps the suffix numbers inside 0..9 so
// the slots never run out; this module only reports whether the driver is
// still chained.

/// Keyboard device class, whose `UpperFilters` lists the filter drivers.
#[cfg(target_os = "windows")]
const KEYBOARD_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e96b-e325-11ce-bfc1-08002be10318}";
/// Mouse device class.
#[cfg(target_os = "windows")]
const MOUSE_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e96f-e325-11ce-bfc1-08002be10318}";

/// Name the driver registers for the keyboard filter.
#[cfg(target_os = "windows")]
const KEYBOARD_FILTER: &str = "keyboard";
/// Name the driver registers for the mouse filter.
#[cfg(target_os = "windows")]
const MOUSE_FILTER: &str = "mouse";

/// What the class filter chains currently contain.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilterChainStatus {
    pub keyboard: Vec<String>,
    pub mouse: Vec<String>,
    /// Whether the driver is still in either chain.
    pub interception_installed: bool,
}

#[cfg(target_os = "windows")]
pub fn filter_chain_status() -> FilterChainStatus {
    let keyboard = read_upper_filters(KEYBOARD_CLASS);
    let mouse = read_upper_filters(MOUSE_CLASS);
    let interception_installed = keyboard
        .iter()
        .any(|name| name.eq_ignore_ascii_case(KEYBOARD_FILTER))
        || mouse
            .iter()
            .any(|name| name.eq_ignore_ascii_case(MOUSE_FILTER));
    FilterChainStatus {
        keyboard,
        mouse,
        interception_installed,
    }
}

#[cfg(not(target_os = "windows"))]
pub fn filter_chain_status() -> FilterChainStatus {
    FilterChainStatus {
        keyboard: Vec::new(),
        mouse: Vec::new(),
        interception_installed: false,
    }
}

/// Read a `REG_MULTI_SZ` value, returning an empty list when it is absent.
#[cfg(target_os = "windows")]
fn read_upper_filters(path: &str) -> Vec<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_VALUE_TYPE,
    };

    let wide = |text: &str| -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    };

    unsafe {
        let path = wide(path);
        let mut key = HKEY(std::ptr::null_mut());
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
        .0 != 0
        {
            return Vec::new();
        }

        let name = wide("UpperFilters");
        let mut kind = REG_VALUE_TYPE(0);
        let mut size = 0u32;
        let mut status = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        );
        if status.0 != 0 || size == 0 {
            let _ = RegCloseKey(key);
            return Vec::new();
        }

        let mut buffer = vec![0u8; size as usize];
        status = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        if status.0 != 0 {
            return Vec::new();
        }
        parse_multi_string(&buffer[..size as usize])
    }
}

/// Split a `REG_MULTI_SZ` payload: UTF-16 units, entries separated by a single
/// NUL and the list terminated by an empty one.
#[cfg(target_os = "windows")]
fn parse_multi_string(bytes: &[u8]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    for pair in bytes.chunks_exact(2) {
        let unit = u16::from_le_bytes([pair[0], pair[1]]);
        if unit == 0 {
            if current.is_empty() {
                break;
            }
            entries.push(std::mem::take(&mut current));
        } else if let Some(character) = char::from_u32(u32::from(unit)) {
            current.push(character);
        }
    }
    entries
}
