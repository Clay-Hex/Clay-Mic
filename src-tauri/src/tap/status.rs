//! Status snapshot and UI push.

use super::*;
use super::host::{
    configured_identity_display, configured_mac, find_host_pid_ex, pid_is_wudfhost,
    read_marker_pid, resolve_injected_pid,
};
use super::udp::ensure_server;

/// True when a built hook DLL is available (beside the exe or in target/).
pub(super) fn dll_ready() -> bool {
    find_tap_dll_source().is_some() || deployed_tap_dll().is_file()
}

/// Build the status payload for the UI.
pub fn status() -> TapStatus {
    let dll_ready = dll_ready();

    let (host_pid, lookup) = find_host_pid_ex().unwrap_or_else(|error| {
        (None, Some(error))
    });
    let host_alive = host_pid.is_some_and(pid_is_wudfhost);
    let injected_pid = resolve_injected_pid(host_pid);
    let listening = SERVER_RUNNING.load(Ordering::SeqCst) == 1;
    let client_connected = client_connected();
    let dll_version = if client_connected {
        DLL_VERSION.lock().unwrap().clone()
    } else {
        None
    };
    let dll_needs_update =
        client_connected && dll_version.as_deref() != Some(env!("CARGO_PKG_VERSION"));

    TapStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        dll_ready,
        host_pid,
        host_alive,
        injected: injected_pid.is_some(),
        listening,
        client_connected,
        last_error: LAST_ERROR.lock().unwrap().clone(),
        device_identity: configured_identity_display()
            .or_else(|| configured_mac().map(|mac| format!("MAC={mac}"))),
        lookup,
        dll_version,
        dll_needs_update,
    }
}

/// Last computed status so `get_tap_status` can return immediately.
static LAST_STATUS: Mutex<Option<TapStatus>> = Mutex::new(None);

/// Cached snapshot for a fast first paint; may be briefly stale.
pub fn status_cached() -> TapStatus {
    if let Some(cached) = LAST_STATUS.lock().unwrap().clone() {
        return cached;
    }
    let fresh = status();
    *LAST_STATUS.lock().unwrap() = Some(fresh.clone());
    fresh
}

pub(super) fn emit_status() {
    let fresh = status();
    *LAST_STATUS.lock().unwrap() = Some(fresh.clone());
    crate::runtime::emit("tap://status", fresh);
}

/// Recompute status off the command thread; emit only when it differs from
/// the cached snapshot (avoids a feedback loop with the UI listener).
pub fn refresh_status_async() {
    let fresh = status();
    // Marker from a previous session: bring UDP up so the resident hook's
    // heartbeat can mark the channel live without waiting for suppression.
    if fresh.injected && !fresh.listening && read_marker_pid().is_some() {
        let _ = ensure_server();
    }
    let fresh = status();
    let changed = LAST_STATUS.lock().unwrap().as_ref() != Some(&fresh);
    if changed {
        *LAST_STATUS.lock().unwrap() = Some(fresh.clone());
        crate::runtime::emit("tap://status", fresh);
    }
}
