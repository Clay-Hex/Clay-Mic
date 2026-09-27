use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(path: PathBuf) {
    let _ = DATA_DIR.set(path);
}

pub fn data_dir() -> PathBuf {
    DATA_DIR.get().cloned().unwrap_or_else(|| {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("clay-mic")
    })
}

pub fn status_path() -> PathBuf {
    data_dir().join("keyslots.json")
}

pub fn log_path() -> PathBuf {
    data_dir().join("keyslots.log")
}

fn ensure_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[link(name = "advapi32")]
extern "system" {
    fn GetUserNameW(buffer: *mut u16, size: *mut u32) -> i32;
}

/// Account this process runs as. Recorded with every run because creating the
/// links only works as `SYSTEM`, and an elevated administrator still fails.
pub fn current_account() -> String {
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer is `size` units long and `size` is passed by pointer.
    if unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return "unknown".to_string();
    }
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    String::from_utf16_lossy(&buffer[..end])
}

/// Append one timestamped line to `keyslots.log`. A service that fails to start
/// leaves no other trace, so this is the only place its reason survives.
pub fn log(message: &str) {
    if ensure_dir().is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(file, "[{}] {}", epoch_seconds(), message);
    }
}

/// Escape the few characters that would break the hand-written JSON below.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .map(|ch| match ch {
            '"' => '\'',
            '\\' => '/',
            other => other,
        })
        .collect()
}

/// Write `keyslots.json`. Always called, success or failure, so the app can
/// tell "not run yet" apart from "ran and failed".
pub fn write_status(count: usize, keyboard: usize, pointer: usize, ok: bool, error: Option<&str>) {
    if ensure_dir().is_err() {
        return;
    }
    let error = match error {
        Some(message) => format!("\"{}\"", sanitize(message)),
        None => "null".to_string(),
    };
    let json = format!(
        "{{\"applied_unix\":{},\"count\":{},\"keyboard\":{},\"pointer\":{},\"ok\":{},\"error\":{}}}\n",
        epoch_seconds(),
        count,
        keyboard,
        pointer,
        ok,
        error
    );
    let _ = std::fs::write(status_path(), json);
}

pub fn remove_status() {
    let _ = std::fs::remove_file(status_path());
}
