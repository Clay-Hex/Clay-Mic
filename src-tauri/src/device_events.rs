//! Device arrival/removal notifications.
//!
//! Event-driven counterpart to polling the Interception slots: the HID capture
//! loop asks to be re-checked the moment Windows reports a HID interface
//! change, so a re-enumerated remote is picked up without a periodic scan.
//!
//! Registration goes through `CM_Register_Notification`, whose callbacks are
//! delivered on the registering thread — hence the message loop at the bottom.

/// Ask the HID capture loop to re-evaluate its binding on its next iteration.
pub use crate::hid::request_reconcile;

#[cfg(target_os = "windows")]
pub fn start() {
    use std::sync::OnceLock;

    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    if let Err(error) = std::thread::Builder::new()
        .name("clay-mic device events".into())
        .spawn(listen)
    {
        log::warn!("device_events: failed to start listener: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn start() {}

#[cfg(target_os = "windows")]
fn listen() {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0,
        CM_NOTIFY_FILTER_0_0, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Unregister_Notification,
        HCMNOTIFICATION,
    };
    use windows::Win32::Devices::HumanInterfaceDevice::GUID_DEVINTERFACE_HID;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, MSG,
    };

    unsafe extern "system" fn on_notify(
        _notify: HCMNOTIFICATION,
        _context: *const core::ffi::c_void,
        action: CM_NOTIFY_ACTION,
        _event: *const CM_NOTIFY_EVENT_DATA,
        _event_size: u32,
    ) -> u32 {
        use windows::Win32::Devices::DeviceAndDriverInstallation::{
            CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
        };
        if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL
            || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL
        {
            crate::hid::request_reconcile();
        }
        // ERROR_SUCCESS.
        0
    }

    let mut filter = CM_NOTIFY_FILTER::default();
    filter.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
    filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;
    filter.u = CM_NOTIFY_FILTER_0 {
        DeviceInterface: CM_NOTIFY_FILTER_0_0 {
            ClassGuid: GUID_DEVINTERFACE_HID,
        },
    };

    let mut notify = HCMNOTIFICATION(std::ptr::null_mut());
    let registered = unsafe {
        windows::Win32::Devices::DeviceAndDriverInstallation::CM_Register_Notification(
            &filter,
            None,
            Some(on_notify),
            &mut notify,
        )
    };
    if registered != windows::Win32::Devices::DeviceAndDriverInstallation::CR_SUCCESS {
        log::warn!(
            "device_events: CM_Register_Notification failed ({:#x}); \
             relying on the periodic fallback",
            registered.0
        );
        return;
    }
    log::info!("device_events: listening for HID interface changes");

    // One scan up front so a device already present binds without waiting for
    // the next interface event.
    crate::hid::request_reconcile();

    let mut message = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if got.0 <= 0 {
            break; // WM_QUIT (0) or error (-1)
        }
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }

    unsafe {
        let _ = CM_Unregister_Notification(notify);
    }
}
