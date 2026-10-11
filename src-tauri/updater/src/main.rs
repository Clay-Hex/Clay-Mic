//! clay-update — applies a staged update after the app exits.
//!
//! Waits for the app to exit, swaps the staged files in (rename the old file
//! aside, copy the new one in), launches the new build, and rolls back if it
//! never reports in. Dependency-free; the app cleans up the saved originals.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const PARENT_TIMEOUT: Duration = Duration::from_secs(30);
const OK_TIMEOUT: Duration = Duration::from_secs(30);
const OK_FILE: &str = "update.ok";
const MAIN_EXE: &str = "Clay-Mic.exe";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|arg| arg == "--apply") {
        eprintln!("clay-update: expected --apply");
        std::process::exit(2);
    }

    let log_path = value(&args, "--log").map(PathBuf::from);
    let (staging, target, journal, version) = match (
        value(&args, "--staging"),
        value(&args, "--target"),
        value(&args, "--journal"),
        value(&args, "--version"),
    ) {
        (Some(staging), Some(target), Some(journal), Some(version)) => (
            PathBuf::from(staging),
            PathBuf::from(target),
            PathBuf::from(journal),
            version,
        ),
        _ => {
            log(&log_path, "missing --staging/--target/--journal/--version");
            std::process::exit(2);
        }
    };
    let parent = value(&args, "--parent").and_then(|value| value.parse::<u32>().ok());
    let main_exe = value(&args, "--main-exe").unwrap_or_else(|| MAIN_EXE.to_string());

    std::process::exit(run(
        &staging,
        &target,
        &journal,
        &version,
        &main_exe,
        parent,
        &log_path,
    ));
}

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn run(
    staging: &Path,
    target: &Path,
    journal: &Path,
    version: &str,
    main_exe: &str,
    parent: Option<u32>,
    log_path: &Option<PathBuf>,
) -> i32 {
    // Nothing is touched before the app has exited.
    if let Some(pid) = parent {
        if !wait_for_pid(pid, PARENT_TIMEOUT) {
            log(log_path, "parent did not exit in time; nothing changed");
            return 2;
        }
    }

    if !writable(target) {
        log(log_path, "target directory is not writable");
        return 3;
    }

    let files = match collect(staging) {
        Ok(files) => files,
        Err(error) => {
            log(log_path, &error);
            return 3;
        }
    };
    if files.is_empty() {
        log(log_path, "staging directory is empty");
        return 3;
    }

    let mut lines = vec!["clay-mic-update/1".to_string(), format!("version={version}")];
    write_journal(journal, &lines);

    // Swap each file in, journaling after each so a crash can still roll back.
    let mut applied: Vec<Entry> = Vec::new();
    for relative in &files {
        let source = staging.join(relative);
        let destination = target.join(relative);
        if let Some(parent) = destination.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                log(log_path, &format!("create {} failed: {error}", parent.display()));
                rollback(target, &applied);
                return 4;
            }
        }
        let entry = if destination.exists() {
            let old = old_path(&destination);
            let _ = fs::remove_file(&old);
            if let Err(error) = fs::rename(&destination, &old) {
                log(log_path, &format!("rename {} failed: {error}", destination.display()));
                rollback(target, &applied);
                return 4;
            }
            if let Err(error) = fs::copy(&source, &destination) {
                let _ = fs::rename(&old, &destination);
                log(log_path, &format!("copy {} failed: {error}", destination.display()));
                rollback(target, &applied);
                return 4;
            }
            Entry::Replaced(relative.clone())
        } else {
            if let Err(error) = fs::copy(&source, &destination) {
                log(log_path, &format!("copy {} failed: {error}", destination.display()));
                rollback(target, &applied);
                return 4;
            }
            Entry::Added(relative.clone())
        };
        lines.push(entry.journal_line());
        applied.push(entry);
        write_journal(journal, &lines);
    }
    lines.push("stage=applied".to_string());
    write_journal(journal, &lines);

    let exe = target.join(main_exe);
    let mut child = match spawn(&exe) {
        Ok(child) => child,
        Err(error) => {
            log(log_path, &format!("launch {} failed: {error}", exe.display()));
            rollback(target, &applied);
            relaunch(target, main_exe, log_path);
            return 5;
        }
    };

    // A healthy new build writes update.ok.
    let ok = target.join(OK_FILE);
    let deadline = Instant::now() + OK_TIMEOUT;
    loop {
        if ok.exists() {
            return 0;
        }
        if let Ok(Some(status)) = child.try_wait() {
            log(log_path, &format!("new build exited early ({status})"));
            rollback(target, &applied);
            relaunch(target, main_exe, log_path);
            return 6;
        }
        if Instant::now() >= deadline {
            log(log_path, "new build did not confirm in time");
            let _ = child.kill();
            rollback(target, &applied);
            relaunch(target, main_exe, log_path);
            return 6;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

enum Entry {
    Replaced(String),
    Added(String),
}

impl Entry {
    fn journal_line(&self) -> String {
        match self {
            Entry::Replaced(relative) => format!("R={relative}"),
            Entry::Added(relative) => format!("A={relative}"),
        }
    }
}

fn rollback(target: &Path, applied: &[Entry]) {
    for entry in applied.iter().rev() {
        match entry {
            Entry::Replaced(relative) => {
                let destination = target.join(relative);
                let old = old_path(&destination);
                let _ = fs::remove_file(&destination);
                let _ = fs::rename(&old, &destination);
            }
            Entry::Added(relative) => {
                let _ = fs::remove_file(target.join(relative));
            }
        }
    }
}

fn relaunch(target: &Path, main_exe: &str, log_path: &Option<PathBuf>) {
    let exe = target.join(main_exe);
    if let Err(error) = spawn(&exe) {
        log(log_path, &format!("relaunch {} failed: {error}", exe.display()));
    }
}

fn collect(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    Ok(files)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("read {} failed: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            walk(root, &path, out)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?;
            out.push(relative.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

fn old_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.old", path.display()))
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".clay-update-write-probe");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn write_journal(journal: &Path, lines: &[String]) {
    if let Some(parent) = journal.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(journal, format!("{}\n", lines.join("\n")));
}

fn spawn(exe: &Path) -> std::io::Result<std::process::Child> {
    let mut command = Command::new(exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}

fn log(path: &Option<PathBuf>, message: &str) {
    use std::io::Write as _;
    let line = format!("[{}] {message}\n", now_millis());
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
    let _ = std::io::stderr().write_all(line.as_bytes());
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

// A process that cannot be opened is already gone, which counts as exited.
#[cfg(target_os = "windows")]
fn wait_for_pid(pid: u32, timeout: Duration) -> bool {
    use win::{CloseHandle, OpenProcess, WaitForSingleObject, SYNCHRONIZE, WAIT_OBJECT_0};
    unsafe {
        let handle = OpenProcess(SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return true;
        }
        let result = WaitForSingleObject(handle, timeout.as_millis() as u32);
        let _ = CloseHandle(handle);
        result == WAIT_OBJECT_0
    }
}

#[cfg(not(target_os = "windows"))]
fn wait_for_pid(_pid: u32, _timeout: Duration) -> bool {
    true
}

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub const SYNCHRONIZE: u32 = 0x0010_0000;
    pub const WAIT_OBJECT_0: u32 = 0x0000_0000;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn OpenProcess(desired_access: u32, inherit: i32, process_id: u32) -> Handle;
        pub fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        pub fn CloseHandle(object: Handle) -> i32;
    }
}
