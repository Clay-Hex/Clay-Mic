//! Loaded into WUDFHost.exe. Hooks `ntdll!NtDeviceIoControlFile`, reads the
//! 9-byte HID-over-GATT report for back / volume±, and sends JSON datagrams
//! to clay-mic on loopback UDP (plus a 1s heartbeat).
//!
//! Design constraints:
//! - One hook site only: never load two copies (unload the previous DLL first).
//! - DllMain does no work. The injector calls the exported `clay_tap_init`
//!   from a separate remote thread, so socket + hook setup never runs under
//!   the loader lock (which deadlocks LoadLibrary).
//! - DETACH stops the heartbeat (joining only that thread), unhooks, returns.

#[cfg(target_os = "windows")]
mod w {
    use min_hook_rs::{
        create_hook_api, disable_hook, enable_hook, initialize, remove_hook, uninitialize,
    };
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    const UDP_PORT: u16 = 49733;
    const IOCTL_READ_CHARACTERISTIC: u32 = 0x8001_8483;
    const REPORT_LENGTH: usize = 9;
    const USAGE_OFFSET: usize = 3;
    const KEY_USAGES: [u16; 3] = [0x00F1, 0x0080, 0x0081];
    const KEY_NAMES: [&str; 3] = ["back", "volume_up", "volume_down"];
    const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001u32 as i32;

    type NtFn = unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
    ) -> i32;

    static SEND: Mutex<Option<Socket>> = Mutex::new(None);
    static ORIGINAL: Mutex<Option<NtFn>> = Mutex::new(None);
    /// Trampoline/target address from min_hook_rs — a plain pointer, wrapped
    /// so Mutex<Target> is Sync for the static.
    static TARGET: Mutex<Option<Target>> = Mutex::new(None);
    static STOP: AtomicBool = AtomicBool::new(false);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static HEARTBEAT: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

    #[derive(Clone, Copy)]
    struct Socket(u64);
    unsafe impl Send for Socket {}

    #[derive(Clone, Copy)]
    struct Target(*mut c_void);
    unsafe impl Send for Target {}
    unsafe impl Sync for Target {}

    #[link(name = "ws2_32")]
    extern "system" {
        fn WSAStartup(version: u16, data: *mut c_void) -> i32;
        fn WSACleanup() -> i32;
        fn socket(af: i32, ty: i32, protocol: i32) -> u64;
        fn connect(s: u64, name: *const c_void, namelen: i32) -> i32;
        fn send(s: u64, buf: *const c_void, len: i32, flags: i32) -> i32;
        fn closesocket(s: u64) -> i32;
    }

    fn send_bytes(payload: &[u8]) {
        let guard = SEND.lock().unwrap();
        let Some(sock) = guard.as_ref() else { return };
        unsafe {
            send(sock.0, payload.as_ptr().cast(), payload.len() as i32, 0);
        }
    }

    fn send_json_button(name: &str, down: bool) {
        send_bytes(format!(r#"{{"button":"{name}","down":{down}}}"#).as_bytes());
    }

    fn send_heartbeat() {
        send_bytes(format!(r#"{{"hb":1,"v":"{}"}}"#, env!("CARGO_PKG_VERSION")).as_bytes());
    }

    /// Parse a 9-byte GATT report and emit edge-triggered button events.
    fn process_report(buf: &[u8]) {
        if buf.len() < USAGE_OFFSET + 6 {
            return;
        }
        static HELD: Mutex<[bool; 3]> = Mutex::new([false; 3]);
        let mut present = [false; 3];
        let mut i = USAGE_OFFSET;
        while i + 1 < buf.len() {
            let usage = u16::from_le_bytes([buf[i], buf[i + 1]]);
            if let Some(idx) = KEY_USAGES.iter().position(|&u| u == usage) {
                present[idx] = true;
            }
            i += 2;
        }
        let mut held = HELD.lock().unwrap();
        for idx in 0..3 {
            if present[idx] != held[idx] {
                held[idx] = present[idx];
                send_json_button(KEY_NAMES[idx], present[idx]);
            }
        }
    }

    unsafe extern "system" fn nt_detour(
        file_handle: *mut c_void,
        event: *mut c_void,
        apc_routine: *mut c_void,
        apc_context: *mut c_void,
        io_status_block: *mut c_void,
        io_control_code: u32,
        input_buffer: *mut c_void,
        input_buffer_length: u32,
        output_buffer: *mut c_void,
        output_buffer_length: u32,
    ) -> i32 {
        let orig = ORIGINAL.lock().unwrap();
        let Some(real) = *orig else {
            return STATUS_UNSUCCESSFUL;
        };
        drop(orig);
        let status = real(
            file_handle,
            event,
            apc_routine,
            apc_context,
            io_status_block,
            io_control_code,
            input_buffer,
            input_buffer_length,
            output_buffer,
            output_buffer_length,
        );
        if status >= 0
            && io_control_code == IOCTL_READ_CHARACTERISTIC
            && output_buffer_length as usize >= REPORT_LENGTH
            && !output_buffer.is_null()
        {
            let slice =
                std::slice::from_raw_parts(output_buffer.cast::<u8>(), REPORT_LENGTH);
            process_report(slice);
        }
        status
    }

    fn init() -> bool {
        if STOP.load(Ordering::SeqCst) {
            return false;
        }
        unsafe {
            let mut wsa = [0u8; 512];
            if WSAStartup(0x0202, wsa.as_mut_ptr() as *mut c_void) != 0 {
                return false;
            }
            let s = socket(2, 2, 17);
            if s == u64::MAX || s == 0 {
                WSACleanup();
                return false;
            }
            let mut addr = [0u8; 16];
            addr[0] = 2;
            addr[2] = ((UDP_PORT >> 8) & 0xff) as u8;
            addr[3] = (UDP_PORT & 0xff) as u8;
            addr[4] = 127;
            addr[7] = 1;
            if connect(s, addr.as_ptr() as *const c_void, 16) != 0 {
                closesocket(s);
                WSACleanup();
                return false;
            }
            *SEND.lock().unwrap() = Some(Socket(s));

            if !INSTALLED.swap(true, Ordering::SeqCst) {
                if initialize().is_err() {
                    INSTALLED.store(false, Ordering::SeqCst);
                    return false;
                }
                match create_hook_api(
                    "ntdll.dll",
                    "NtDeviceIoControlFile",
                    nt_detour as *mut c_void,
                ) {
                    Ok((trampoline, target)) => {
                        *ORIGINAL.lock().unwrap() =
                            Some(std::mem::transmute::<*mut c_void, NtFn>(trampoline));
                        *TARGET.lock().unwrap() = Some(Target(target));
                        if enable_hook(target).is_err() {
                            INSTALLED.store(false, Ordering::SeqCst);
                            *ORIGINAL.lock().unwrap() = None;
                            *TARGET.lock().unwrap() = None;
                            let _ = remove_hook(target);
                            let _ = uninitialize();
                        }
                    }
                    Err(_) => {
                        INSTALLED.store(false, Ordering::SeqCst);
                        let _ = uninitialize();
                    }
                }
            }

            if !STOP.load(Ordering::SeqCst) {
                send_heartbeat();
                let hb = std::thread::Builder::new()
                    .name("clay-tap-hb".into())
                    .spawn(|| {
                        while !STOP.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(200));
                            if STOP.load(Ordering::SeqCst) {
                                break;
                            }
                            send_heartbeat();
                        }
                    })
                    .ok();
                *HEARTBEAT.lock().unwrap() = hb;
            }
        }
        INSTALLED.load(Ordering::SeqCst)
    }

    /// Called by the injector from a separate remote thread after LoadLibrary
    /// returns, so no setup runs under the loader lock.
    #[no_mangle]
    pub extern "system" fn clay_tap_init(_: *mut c_void) -> u32 {
        if init() {
            0
        } else {
            1
        }
    }

    /// Full teardown: unhook + WSACleanup. Runs on a remote thread created
    /// by the injector (NOT under loader lock) so MinHook's thread-suspend
    /// does not deadlock.
    #[no_mangle]
    pub extern "system" fn clay_tap_cleanup(_: *mut c_void) -> u32 {
        STOP.store(true, Ordering::SeqCst);
        if let Some(hb) = HEARTBEAT.lock().unwrap().take() {
            let _ = hb.join();
        }
        if INSTALLED.swap(false, Ordering::SeqCst) {
            if let Some(Target(target)) = TARGET.lock().unwrap().take() {
                let _ = disable_hook(target);
                let _ = remove_hook(target);
            }
            let _ = uninitialize();
        }
        *ORIGINAL.lock().unwrap() = None;
        if let Some(sock) = SEND.lock().unwrap().take() {
            unsafe {
                closesocket(sock.0);
                WSACleanup();
            }
        }
        0
    }

    /// DETACH: best-effort cleanup only. In normal eject the injector calls
    /// `clay_tap_cleanup` first via a remote thread (no loader lock), so by
    /// the time FreeLibrary reaches here the hook is already removed and all
    /// we need is to close a possibly-still-open socket.
    fn shutdown() {
        STOP.store(true, Ordering::SeqCst);
        if let Some(hb) = HEARTBEAT.lock().unwrap().take() {
            let _ = hb.join();
        }
        if let Some(sock) = SEND.lock().unwrap().take() {
            unsafe { closesocket(sock.0); }
        }
    }

    #[no_mangle]
    pub extern "system" fn DllMain(h: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
        const DLL_PROCESS_ATTACH: u32 = 1;
        const DLL_PROCESS_DETACH: u32 = 0;
        match reason {
            DLL_PROCESS_ATTACH => {
                unsafe {
                    kernel32_disable_thread_library_calls(h);
                }
                1
            }
            DLL_PROCESS_DETACH => {
                shutdown();
                1
            }
            _ => 1,
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn DisableThreadLibraryCalls(h: *mut c_void) -> i32;
    }

    unsafe fn kernel32_disable_thread_library_calls(h: *mut c_void) {
        DisableThreadLibraryCalls(h);
    }
}

#[cfg(target_os = "windows")]
pub use w::{clay_tap_cleanup, clay_tap_init, DllMain};
