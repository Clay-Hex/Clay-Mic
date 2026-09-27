//! clay-mic-keyslots
//!
//! Folds Windows' keyboard/mouse device numbering back into `0`..`9` so the
//! Interception driver's single-character slot parsing can never overflow.
//! See `symlink.rs` for why that matters.
//!
//! Runs either as a one-shot CLI (`--apply`, `--remove`) or as a service the
//! SCM starts at boot (no arguments, or `--service`).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
mod service;
#[cfg(target_os = "windows")]
mod status;
#[cfg(target_os = "windows")]
mod symlink;

#[cfg(target_os = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(dir) = data_dir_from(&args) {
        status::set_data_dir(dir);
    }

    if args.iter().any(|arg| arg == "--remove") {
        std::process::exit(if run_remove(count_from(&args)) { 0 } else { 1 });
    }

    if args.iter().any(|arg| arg == "--apply") {
        std::process::exit(if run_apply(count_from(&args)) { 0 } else { 1 });
    }

    // Default: the SCM starts us at boot.
    std::process::exit(service::run());
}

#[cfg(target_os = "windows")]
fn data_dir_from(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .position(|arg| arg == "--data-dir")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
}

#[cfg(target_os = "windows")]
fn count_from(args: &[String]) -> usize {
    args.iter()
        .position(|arg| arg == "--count")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 10)
        .unwrap_or(symlink::DEFAULT_COUNT)
}

#[cfg(target_os = "windows")]
pub(crate) fn run_from_env() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(dir) = data_dir_from(&args) {
        status::set_data_dir(dir);
    }
    if args.iter().any(|arg| arg == "--remove") {
        return run_remove(count_from(&args));
    }
    run_apply(count_from(&args))
}

#[cfg(target_os = "windows")]
fn run_apply(count: usize) -> bool {
    let report = symlink::apply(count);
    let keyboard = report.keyboard;
    let pointer = report.pointer;
    let ok = report.ok();
    let error = if report.persisted {
        keyboard
            .first_error
            .or(pointer.first_error)
            .map(symlink::describe)
    } else {
        Some("符号链接创建后未能保留：OBJ_PERMANENT 未生效".to_string())
    };

    status::log(&format!(
        "apply count={count} account={} keyboard={}+{}/{} pointer={}+{}/{} persisted={} missing_privileges={:?} ok={ok}",
        status::current_account(),
        keyboard.created,
        keyboard.existing,
        keyboard.total(),
        pointer.created,
        pointer.existing,
        pointer.total(),
        report.persisted,
        report.missing_privileges,
    ));
    if let Some(message) = error.as_deref() {
        status::log(&format!("apply failed: {message}"));
    }
    status::write_status(
        count,
        keyboard.created + keyboard.existing,
        pointer.created + pointer.existing,
        ok,
        error.as_deref(),
    );
    ok
}

#[cfg(target_os = "windows")]
fn run_remove(count: usize) -> bool {
    let (keyboard, pointer) = symlink::remove(count);
    let ok = keyboard.ok() && pointer.ok();
    status::log(&format!(
        "remove count={count} keyboard={} pointer={} ok={ok}",
        keyboard.created + keyboard.existing,
        pointer.created + pointer.existing,
    ));
    status::remove_status();
    ok
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("clay-mic-keyslots only runs on Windows");
    std::process::exit(1);
}
