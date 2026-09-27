//! Loopback UDP event server and key dispatch.

use super::*;
use super::status::emit_status;


// ---------------------------------------------------------------------------
// Loopback UDP event server
// ---------------------------------------------------------------------------

/// Start the UDP recv thread if it is not already running.
pub(super) fn ensure_server() -> Result<(), String> {
    let _guard = SERVER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if SERVER_RUNNING.load(Ordering::SeqCst) == 1 {
        return Ok(());
    }
    SERVER_RUNNING.store(1, Ordering::SeqCst);
    let handle = std::thread::spawn(|| {
        log::info!("tap: UDP listening on {UDP_BIND}");
        udp_recv_loop();
        log::info!("tap: UDP server stopped");
    });
    *SERVER_THREAD.lock().unwrap() = Some(handle);
    Ok(())
}

/// App-launch entry: bring UDP up for a resident hook DLL (no inject).
pub fn ensure_server_public() {
    if let Err(error) = ensure_server() {
        log::warn!("tap: ensure_server on launch: {error}");
    } else {
        emit_status();
    }
}

/// Stop the UDP recv thread and wait for it to exit so the port is released.
pub(super) fn stop_server() {
    let _guard = SERVER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    SERVER_RUNNING.store(0, Ordering::SeqCst);
    *LAST_PACKET.lock().unwrap() = None;
    *DLL_VERSION.lock().unwrap() = None;
    WAS_CONNECTED.store(0, Ordering::SeqCst);
    if let Some(handle) = SERVER_THREAD.lock().unwrap().take() {
        let _ = handle.join();
    }
    log::info!("tap: UDP server stopped");
}

fn udp_recv_loop() {
    use std::net::UdpSocket;
    let Ok(sock) = UdpSocket::bind(UDP_BIND) else {
        log::warn!("tap: UDP bind failed on {UDP_BIND}");
        SERVER_RUNNING.store(0, Ordering::SeqCst);
        return;
    };
    let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(200)));
    let mut buf = [0u8; 256];
    while SERVER_RUNNING.load(Ordering::SeqCst) == 1 {
        match sock.recv_from(&mut buf) {
            Ok((n, _from)) => {
                if ACCEPTING.load(Ordering::SeqCst) != 1 {
                    continue;
                }
                let text = String::from_utf8_lossy(&buf[..n]);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let was_up = client_connected();
                *LAST_PACKET.lock().unwrap() = Some(std::time::Instant::now());
                WAS_CONNECTED.store(1, Ordering::SeqCst);
                if text.contains("\"hb\"") {
                    if let Ok(packet) = serde_json::from_str::<HbPacket>(text) {
                        *DLL_VERSION.lock().unwrap() = packet.v;
                    }
                    if !was_up {
                        log::info!("tap: client heartbeat (UDP)");
                        emit_status();
                    }
                    continue;
                }
                if !was_up {
                    log::info!("tap: client connected (UDP)");
                    emit_status();
                }
                let Ok(event) = serde_json::from_str::<TapEvent>(text) else {
                    continue;
                };
                on_tap_event(&event.button, event.down);
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {
                let now = client_connected() as u32;
                let prev = WAS_CONNECTED.swap(now, Ordering::SeqCst);
                if prev == 1 && now == 0 {
                    log::info!("tap: client heartbeat lost (UDP)");
                    emit_status();
                }
            }
            Err(e) => {
                log::warn!("tap: UDP recv error: {e}");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct HbPacket {
    /// Matched by the fast `contains` gate above; the value is unused.
    #[allow(dead_code)]
    hb: u32,
    /// Absent on hooks that predate the versioned heartbeat.
    #[serde(default)]
    v: Option<String>,
}

#[derive(serde::Deserialize)]
struct TapEvent {
    button: String,
    down: bool,
}

/// Dispatch one tap edge through the same keymap rules as Interception.
fn on_tap_event(button: &str, down: bool) {
    let suppress = crate::keymap::suppresses(button);
    let (action, key, command, terminal_exit) = crate::keymap::binding_for(button);
    if !suppress {
        // First-seen only: releases are frequent and would flood the log.
        if down {
            log::info!(
                "tap {button} ignored (suppress off or action={action} not swallowable)"
            );
        }
        return;
    }
    if action == "voice" {
        return;
    }
    // Actions fire on the press edge only (mirrors `process_target_stroke`).
    if !down {
        return;
    }
    log::info!("tap button={button} action={action} key={key:?} command={command:?}");
    if let Err(error) = crate::keymap::execute_action(
        &action,
        key.as_deref(),
        command.as_deref(),
        &terminal_exit,
    ) {
        log::warn!("tap button={button} action={action} failed: {error}");
    }
}
