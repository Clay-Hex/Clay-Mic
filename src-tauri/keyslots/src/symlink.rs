//! Device-class symlinks in the kernel object namespace.
//!
//! Windows names keyboard/mouse class devices `\Device\KeyboardClass0`,
//! `KeyboardClass1`, … and increments the suffix on every device
//! re-enumeration (sleep/resume, replug). Interception parses that suffix as a
//! **single character**, so `KeyboardClass10` reads as slot `0` and collides
//! with the real slot 0 — the filter fails to attach and the device stops
//! delivering input entirely.
//!
//! Occupying the high-numbered names with symlinks that fold back onto `0`..`9`
//! keeps every device inside the range Interception can parse.

use std::ffi::c_void;

type NtStatus = i32;

const STATUS_SUCCESS: NtStatus = 0;
const STATUS_OBJECT_NAME_COLLISION: NtStatus = 0xC000_0035u32 as i32;
const STATUS_OBJECT_TYPE_MISMATCH: NtStatus = 0xC000_0024u32 as i32;
const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = 0xC000_0034u32 as i32;
const STATUS_PRIVILEGE_NOT_HELD: NtStatus = 0xC000_0061u32 as i32;
/// The `\Device` namespace grants create rights to SYSTEM only, so an elevated
/// administrator still gets this.
const STATUS_ACCESS_DENIED: NtStatus = 0xC000_0022u32 as i32;

/// Keep the link alive after its handle closes. Without it the object is
/// destroyed by `NtClose` and the create still reports success, so the pass
/// looks like it worked while leaving nothing behind. Requires
/// `SeCreatePermanentPrivilege`, which is why the token privileges below are
/// enabled before the first create.
const OBJ_PERMANENT: u32 = 0x0000_0010;
/// `STANDARD_RIGHTS_REQUIRED | SYMBOLIC_LINK_QUERY`.
const SYMBOLIC_LINK_ALL_ACCESS: u32 = 0x000F_0001;
/// `SYMBOLIC_LINK_QUERY`.
const SYMBOLIC_LINK_QUERY: u32 = 0x0000_0001;
/// `DELETE`.
const DELETE: u32 = 0x0001_0000;

/// `\Device\KeyboardClass` / `\Device\PointerClass` share this shape.
const KEYBOARD_CLASS: &str = "KeyboardClass";
const POINTER_CLASS: &str = "PointerClass";

/// Highest device number to fold back, exclusive. 1000 covers `KeyboardClass10`
/// through `KeyboardClass999`.
pub const DEFAULT_COUNT: usize = 1000;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: *mut c_void,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtCreateSymbolicLinkObject(
        link_handle: *mut *mut c_void,
        desired_access: u32,
        object_attributes: *mut ObjectAttributes,
        target_name: *mut UnicodeString,
    ) -> NtStatus;
    fn NtOpenSymbolicLinkObject(
        link_handle: *mut *mut c_void,
        desired_access: u32,
        object_attributes: *mut ObjectAttributes,
    ) -> NtStatus;
    fn NtMakeTemporaryObject(handle: *mut c_void) -> NtStatus;
    fn NtClose(handle: *mut c_void) -> NtStatus;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(
        process_handle: *mut c_void,
        desired_access: u32,
        token_handle: *mut *mut c_void,
    ) -> i32;
    fn LookupPrivilegeValueW(system_name: *const u16, name: *const u16, luid: *mut Luid) -> i32;
    fn AdjustTokenPrivileges(
        token_handle: *mut c_void,
        disable_all_privileges: i32,
        new_state: *mut TokenPrivileges,
        buffer_length: u32,
        previous_state: *mut c_void,
        return_length: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LuidAndAttributes {
    luid: Luid,
    attributes: u32,
}

#[repr(C)]
struct TokenPrivileges {
    privilege_count: u32,
    privileges: [LuidAndAttributes; 1],
}

const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_QUERY: u32 = 0x0008;
const SE_PRIVILEGE_ENABLED: u32 = 0x0000_0002;
const ERROR_SUCCESS: u32 = 0;

/// A NUL-terminated UTF-16 buffer plus the `UNICODE_STRING` view onto it. The
/// `UnicodeString` borrows the buffer, so both must stay alive together.
struct WideString {
    buffer: Vec<u16>,
}

impl WideString {
    fn new(text: &str) -> Self {
        Self {
            buffer: text.encode_utf16().chain(std::iter::once(0)).collect(),
        }
    }

    fn as_unicode_string(&mut self) -> UnicodeString {
        // `length` excludes the terminator and is measured in bytes.
        let units = self.buffer.len().saturating_sub(1);
        UnicodeString {
            length: (units * 2) as u16,
            maximum_length: (self.buffer.len() * 2) as u16,
            buffer: self.buffer.as_mut_ptr(),
        }
    }
}

/// Enable one token privilege, returning whether it ended up enabled.
///
/// `AdjustTokenPrivileges` reports success even when it granted nothing, so the
/// trailing `GetLastError` is what actually decides.
fn enable_privilege(name: &str) -> bool {
    let wide = WideString::new(name);
    let mut token: *mut c_void = std::ptr::null_mut();
    let mut state = TokenPrivileges {
        privilege_count: 1,
        privileges: [LuidAndAttributes::default()],
    };

    unsafe {
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return false;
        }
        let mut luid = Luid::default();
        if LookupPrivilegeValueW(std::ptr::null(), wide.buffer.as_ptr(), &mut luid) == 0 {
            CloseHandle(token);
            return false;
        }
        state.privileges[0].luid = luid;
        state.privileges[0].attributes = SE_PRIVILEGE_ENABLED;
        let adjusted = AdjustTokenPrivileges(
            token,
            0,
            &mut state,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let granted = adjusted != 0 && GetLastError() == ERROR_SUCCESS;
        CloseHandle(token);
        granted
    }
}

/// Privileges the creates below need. Both are held by SYSTEM and by
/// administrators, but disabled by default, so they are enabled explicitly.
fn enable_required_privileges() -> Vec<&'static str> {
    ["SeCreateSymbolicLinkPrivilege", "SeCreatePermanentPrivilege"]
        .into_iter()
        .filter(|name| !enable_privilege(name))
        .collect()
}

fn create_symlink(link: &str, target: &str) -> NtStatus {
    let mut link_wide = WideString::new(link);
    let mut target_wide = WideString::new(target);
    let mut link_name = link_wide.as_unicode_string();
    let mut target_name = target_wide.as_unicode_string();

    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut link_name,
        attributes: OBJ_PERMANENT,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };

    let mut handle: *mut c_void = std::ptr::null_mut();
    // SAFETY: both `UNICODE_STRING`s point at buffers that outlive the call,
    // and `attributes` is fully initialised with the documented layout.
    let status = unsafe {
        NtCreateSymbolicLinkObject(
            &mut handle,
            SYMBOLIC_LINK_ALL_ACCESS,
            &mut attributes,
            &mut target_name,
        )
    };
    if status >= 0 && !handle.is_null() {
        unsafe { NtClose(handle) };
    }
    status
}

fn remove_symlink(link: &str) -> NtStatus {
    let mut link_wide = WideString::new(link);
    let mut link_name = link_wide.as_unicode_string();
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut link_name,
        attributes: 0,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };

    let mut handle: *mut c_void = std::ptr::null_mut();
    // SAFETY: same invariants as `create_symlink`.
    let status = unsafe { NtOpenSymbolicLinkObject(&mut handle, DELETE, &mut attributes) };
    if status < 0 {
        return status;
    }
    // A permanent object is only destroyed once it is made temporary.
    let status = unsafe { NtMakeTemporaryObject(handle) };
    unsafe { NtClose(handle) };
    status
}

/// Counts from one pass over the device-class names.
#[derive(Debug, Default, Clone, Copy)]
pub struct Applied {
    /// Links newly created.
    pub created: usize,
    /// Links that already existed with the right name.
    pub existing: usize,
    /// Names that could not be linked.
    pub failed: usize,
    /// First failure status, when `failed` is non-zero.
    pub first_error: Option<NtStatus>,
}

impl Applied {
    /// Names this pass was responsible for.
    pub fn total(&self) -> usize {
        self.created + self.existing + self.failed
    }

    pub fn ok(&self) -> bool {
        self.failed == 0
    }
}

/// Outcome of one `apply` pass.
#[derive(Debug, Clone)]
pub struct Report {
    pub keyboard: Applied,
    pub pointer: Applied,
    /// A sample link was still present after its creating handle closed. A
    /// temporary object vanishes at `NtClose` while the create still reports
    /// success, so this is what separates a real pass from a no-op.
    pub persisted: bool,
    /// Privileges this token could not enable.
    pub missing_privileges: Vec<&'static str>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.keyboard.ok() && self.pointer.ok() && self.persisted
    }
}

/// Create `\Device\<class>{n} -> \Device\<class>{n % 10}` for `n` in
/// `10..count`, for both the keyboard and pointer classes.
pub fn apply(count: usize) -> Report {
    let missing_privileges = enable_required_privileges();
    let keyboard = apply_class(KEYBOARD_CLASS, count);
    let pointer = apply_class(POINTER_CLASS, count);
    let persisted =
        link_exists(KEYBOARD_CLASS, 10) && link_exists(KEYBOARD_CLASS, count.saturating_sub(1));
    Report {
        keyboard,
        pointer,
        persisted,
        missing_privileges,
    }
}

/// Whether `\Device\<class><number>` currently resolves to a symbolic link.
fn link_exists(class: &str, number: usize) -> bool {
    let mut link_wide = WideString::new(&format!(r"\Device\{class}{number}"));
    let mut link_name = link_wide.as_unicode_string();
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut link_name,
        attributes: 0,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };

    let mut handle: *mut c_void = std::ptr::null_mut();
    // SAFETY: same invariants as `create_symlink`.
    let status =
        unsafe { NtOpenSymbolicLinkObject(&mut handle, SYMBOLIC_LINK_QUERY, &mut attributes) };
    if status < 0 {
        return false;
    }
    unsafe { NtClose(handle) };
    true
}

fn apply_class(class: &str, count: usize) -> Applied {
    let mut result = Applied::default();
    for number in 10..count {
        let link = format!(r"\Device\{class}{number}");
        let target = format!(r"\Device\{class}{}", number % 10);
        match create_symlink(&link, &target) {
            STATUS_SUCCESS => result.created += 1,
            // Already occupied — by our own link from an earlier run, or by a
            // device that got there first. Both mean the name is taken, which
            // is the outcome this pass wants.
            STATUS_OBJECT_NAME_COLLISION | STATUS_OBJECT_TYPE_MISMATCH => result.existing += 1,
            status => {
                result.failed += 1;
                if result.first_error.is_none() {
                    result.first_error = Some(status);
                }
            }
        }
    }
    result
}

/// Delete the links created by [`apply`].
pub fn remove(count: usize) -> (Applied, Applied) {
    enable_required_privileges();
    (
        remove_class(KEYBOARD_CLASS, count),
        remove_class(POINTER_CLASS, count),
    )
}

fn remove_class(class: &str, count: usize) -> Applied {
    let mut result = Applied::default();
    for number in 10..count {
        let link = format!(r"\Device\{class}{number}");
        match remove_symlink(&link) {
            STATUS_SUCCESS => result.created += 1,
            STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_TYPE_MISMATCH => result.existing += 1,
            status => {
                result.failed += 1;
                if result.first_error.is_none() {
                    result.first_error = Some(status);
                }
            }
        }
    }
    result
}

/// Human-readable form of the failure statuses worth reporting.
pub fn describe(status: NtStatus) -> String {
    match status {
        STATUS_ACCESS_DENIED => {
            "权限不足：在 \\Device 下创建符号链接需要 SYSTEM 身份（请通过服务执行）".to_string()
        }
        STATUS_PRIVILEGE_NOT_HELD => "缺少创建符号链接的权限（需要管理员或 SYSTEM）".to_string(),
        STATUS_OBJECT_NAME_COLLISION => "名称已被占用".to_string(),
        STATUS_OBJECT_TYPE_MISMATCH => "同名对象不是符号链接".to_string(),
        STATUS_OBJECT_NAME_NOT_FOUND => "未找到".to_string(),
        other => format!("NTSTATUS 0x{:08X}", other as u32),
    }
}
