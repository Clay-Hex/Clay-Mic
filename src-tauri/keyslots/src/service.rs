//! Minimal service-control-manager glue.
//!
//! Hand-rolled rather than pulling in a service crate: the surface is four
//! functions, and keeping the helper dependency-free keeps the exe small and
//! its supply chain empty.

use std::ffi::c_void;

/// Must match the name the service is registered under.
pub const SERVICE_NAME: &str = "clay-mic-keyslots";

const SERVICE_WIN32_OWN_PROCESS: u32 = 0x0000_0010;
const SERVICE_RUNNING: u32 = 0x0000_0004;
const SERVICE_STOPPED: u32 = 0x0000_0001;
const SERVICE_ACCEPT_STOP: u32 = 0x0000_0001;
const NO_ERROR: u32 = 0;

/// Returned by `StartServiceCtrlDispatcherW` when the process was not launched
/// by the SCM — i.e. someone ran the exe by hand.
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: u32 = 1063;

#[repr(C)]
struct ServiceStatus {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
}

#[repr(C)]
struct ServiceTableEntryW {
    service_name: *const u16,
    service_proc: Option<extern "system" fn(u32, *mut *mut u16)>,
}

#[link(name = "advapi32")]
extern "system" {
    fn StartServiceCtrlDispatcherW(table: *const ServiceTableEntryW) -> i32;
    fn RegisterServiceCtrlHandlerExW(
        service_name: *const u16,
        handler: Option<extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32>,
        context: *mut c_void,
    ) -> *mut c_void;
    fn SetServiceStatus(handle: *mut c_void, status: *mut ServiceStatus) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}

extern "system" fn handler(
    _control: u32,
    _event_type: u32,
    _event_data: *mut c_void,
    _context: *mut c_void,
) -> u32 {
    // One-shot service: there is nothing to interrupt, and the SCM only ever
    // sends a stop after the work has already finished.
    NO_ERROR
}

fn report(handle: *mut c_void, state: u32) {
    if handle.is_null() {
        return;
    }
    let mut status = ServiceStatus {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        controls_accepted: SERVICE_ACCEPT_STOP,
        win32_exit_code: 0,
        service_specific_exit_code: 0,
        checkpoint: 0,
        wait_hint: 0,
    };
    unsafe { SetServiceStatus(handle, &mut status) };
}

extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let name: Vec<u16> = SERVICE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle =
        unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), std::ptr::null_mut()) };
    if handle.is_null() {
        crate::status::log("service: RegisterServiceCtrlHandlerExW failed");
        return;
    }

    // Report RUNNING before doing anything slow, so the SCM's start timeout can
    // never fire while the links are being created.
    report(handle, SERVICE_RUNNING);
    let ok = crate::run_from_env();
    crate::status::log(&format!("service: work finished ok={ok}"));
    report(handle, SERVICE_STOPPED);
}

/// Run as a service. Returns the process exit code.
///
/// When the exe is launched from a console instead of by the SCM the dispatcher
/// cannot connect; rather than doing nothing, the work runs inline so the same
/// binary is testable by hand.
pub fn run() -> i32 {
    let name: Vec<u16> = SERVICE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let table = [ServiceTableEntryW {
        service_name: name.as_ptr(),
        service_proc: Some(service_main),
    }];

    let started = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
    if started != 0 {
        return 0;
    }

    let error = unsafe { GetLastError() };
    if error == ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
        crate::status::log("service: not launched by the SCM; running the work inline");
        return if crate::run_from_env() { 0 } else { 1 };
    }

    crate::status::log(&format!(
        "service: StartServiceCtrlDispatcherW failed (win32 error {error})"
    ));
    1
}
