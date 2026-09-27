//! Local speech-to-text via the whisper.cpp command-line tool.
//!
//! We intentionally shell out to a prebuilt `whisper-cli.exe` instead of
//! linking `whisper-rs`: no cmake / C++ toolchain is required at build time,
//! and the binary + model can be dropped in at runtime.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::SttConfig;
use crate::download::{emit_progress, stream_download};

/// Readiness of the STT backend, surfaced to the settings UI.
#[derive(Debug, Clone, Serialize)]
pub struct SttStatus {
    pub runtime: String,
    pub runtime_ready: bool,
    pub installed_runtimes: Vec<String>,
    pub binary_ready: bool,
    pub binary_path: Option<String>,
    pub model_ready: bool,
    pub model_path: String,
    pub warm: bool,
}

/// Inspect whether the whisper binary and the configured model are present.
pub fn status(config: &SttConfig) -> SttStatus {
    let binary = resolve_binary(&config.runtime, config.binary_path.as_deref()).ok();
    let model = resolve_model(&config.model, config.model_path.as_deref()).ok();
    let server_bin = binary.as_ref().and_then(|bin| resolve_server_binary(bin));
    let warm = match (server_bin.as_deref(), model.as_deref()) {
        (Some(server_bin), Some(model_file)) if model_file.is_file() => {
            server_running(server_bin, model_file, &config.language)
        }
        _ => false,
    };
    let installed_runtimes: Vec<String> = RUNTIMES
        .iter()
        .filter(|runtime| runtime_ready(runtime))
        .map(|runtime| runtime.to_string())
        .collect();
    SttStatus {
        runtime: config.runtime.clone(),
        runtime_ready: runtime_ready(&config.runtime),
        installed_runtimes,
        binary_ready: binary.is_some(),
        binary_path: binary.map(|p| p.to_string_lossy().into_owned()),
        model_ready: model.as_ref().map(|p| p.is_file()).unwrap_or(false),
        model_path: model
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        warm,
    }
}

/// Base directory for the whisper binary and models:
/// `%LOCALAPPDATA%/clay-mic/whisper`.
pub fn home_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("whisper")
}

/// Selectable whisper.cpp runtimes.
pub const RUNTIMES: [&str; 3] = ["cpu", "cuda12", "cuda11"];

/// Marker recording the runtime of a legacy single-directory install.
fn runtime_marker() -> PathBuf {
    home_dir().join(".runtime")
}

/// Runtime of a legacy single-directory install; pre-GPU installs are CPU.
fn legacy_runtime() -> String {
    std::fs::read_to_string(runtime_marker())
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cpu".to_string())
}

/// Install directory for one runtime, so several can coexist side by side.
fn runtime_dir(runtime: &str) -> PathBuf {
    home_dir().join(runtime)
}

/// Whether `runtime`'s binaries are installed. A legacy flat install counts
/// when it belongs to this runtime.
pub fn runtime_ready(runtime: &str) -> bool {
    runtime_dir(runtime).join("whisper-cli.exe").is_file()
        || (legacy_runtime() == runtime && home_dir().join("whisper-cli.exe").is_file())
}

/// Move a legacy flat install into its proper per-runtime directory.
pub fn migrate_flat_install() {
    let flat_exe = home_dir().join("whisper-cli.exe");
    if !flat_exe.is_file() {
        return;
    }
    let runtime = legacy_runtime();
    let dir = runtime_dir(&runtime);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(entries) = std::fs::read_dir(home_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = std::fs::rename(&path, dir.join(entry.file_name()));
            }
        }
    }
    let _ = std::fs::remove_file(runtime_marker());
    log::info!("migrated flat whisper install to {:?}", dir);
}

/// Directory that holds downloaded models.
pub fn model_dir() -> PathBuf {
    home_dir().join("models")
}

/// Resolve the whisper executable for `runtime`.
///
/// Order: explicit config path → `<home>/<runtime>/whisper-cli.exe` → a legacy
/// flat install belonging to this runtime → a matching binary on `PATH`.
fn resolve_binary(runtime: &str, explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(value) = explicit.map(str::trim).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("whisper 可执行文件不存在：{}", path.display()));
    }

    let mut dirs = vec![runtime_dir(runtime)];
    if legacy_runtime() == runtime {
        dirs.push(home_dir());
    }
    for dir in dirs {
        for name in ["whisper-cli.exe", "main.exe"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
            // The official zip unpacks everything under a `Release/` folder.
            let nested = dir.join("Release").join(name);
            if nested.is_file() {
                return Ok(nested);
            }
        }
    }
    for name in ["whisper-cli.exe", "whisper-cli", "main.exe"] {
        if let Some(found) = find_on_path(name) {
            return Ok(found);
        }
    }

    Err(format!(
        "未找到 whisper 可执行文件。请下载运行后端（{runtime}）或设置 STT 可执行文件路径。"
    ))
}

/// Resolve the model file for a model name such as `base`.
///
/// Accepts either a configured absolute path or `<model_dir>/ggml-<name>.bin`.
fn resolve_model(model: &str, explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(value) = explicit.map(str::trim).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("语音模型不存在：{}", path.display()));
    }

    let name = model.trim();
    let file = if name.ends_with(".bin") {
        name.to_string()
    } else {
        format!("ggml-{}.bin", name)
    };
    Ok(model_dir().join(file))
}

/// Normalize the configured language into a whisper `-l` argument.
fn normalize_language(language: &str) -> String {
    match language.trim() {
        "" | "auto" => "auto".to_string(),
        other => other.to_string(),
    }
}

/// Transcribe a 16 kHz mono WAV file to text.
///
/// Prefers a long-lived `whisper-server` (the model stays loaded across
/// utterances) and falls back to a one-shot `whisper-cli` process when the
/// server binary is missing or fails.
pub fn transcribe_wav(
    wav: &Path,
    model: &str,
    language: &str,
    binary: Option<&str>,
    model_path: Option<&str>,
    runtime: &str,
    prompt: &str,
) -> Result<String, String> {
    let bin = resolve_binary(runtime, binary)?;
    let model_file = resolve_model(model, model_path)?;
    if !model_file.is_file() {
        return Err(format!(
            "未找到语音模型：{}（请下载 {} 或在其设置中指定模型路径）",
            model_file.display(),
            model_file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
    }

    match resolve_server_binary(&bin) {
        Some(server_bin) => {
            match transcribe_via_server(&server_bin, &model_file, wav, language, prompt) {
                Ok(text) => return Ok(text),
                Err(error) => log::warn!("whisper server 不可用，回退到 CLI：{error}"),
            }
        }
        None => log::info!("未找到 whisper-server，使用一次性 whisper-cli"),
    }

    transcribe_via_cli(&bin, &model_file, wav, language, prompt)
}

/// One-shot transcription through `whisper-cli` (cold model load each call).
fn transcribe_via_cli(
    bin: &Path,
    model_file: &Path,
    wav: &Path,
    language: &str,
    prompt: &str,
) -> Result<String, String> {
    log::info!("whisper: {:?} -m {:?} -f {:?}", bin, model_file, wav);

    let mut command = Command::new(bin);
    command
        .arg("-m")
        .arg(model_file)
        .arg("-f")
        .arg(wav)
        .arg("-nt") // no timestamps
        .arg("-np") // no non-essential prints
        .arg("-l")
        .arg(normalize_language(language));
    if !prompt.trim().is_empty() {
        command.arg("--prompt").arg(prompt);
    }
    crate::process::hide_console(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("启动 whisper 失败：{}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "whisper 退出码 {:?}：{}",
            output.status.code(),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(clean_transcript(&stdout))
}

// ---------------------------------------------------------------------------
// Persistent whisper-server
// ---------------------------------------------------------------------------

struct WhisperServer {
    child: Child,
    port: u16,
    server_bin: PathBuf,
    model_path: PathBuf,
    language: String,
    prompt: String,
}

fn server_slot() -> &'static Mutex<Option<WhisperServer>> {
    static SERVER: OnceLock<Mutex<Option<WhisperServer>>> = OnceLock::new();
    SERVER.get_or_init(|| Mutex::new(None))
}

/// Stop the background `whisper-server`, if any. Called on app exit so the
/// child process does not outlive the app.
pub fn shutdown_server() {
    if let Ok(mut slot) = server_slot().lock() {
        if let Some(mut server) = slot.take() {
            let _ = server.child.kill();
            let _ = server.child.wait();
        }
    }
}

/// Locate `whisper-server`, preferring the directory of the `whisper-cli` binary.
fn resolve_server_binary(cli: &Path) -> Option<PathBuf> {
    let names: [&str; 1] = if cfg!(target_os = "windows") {
        ["whisper-server.exe"]
    } else {
        ["whisper-server"]
    };

    if let Some(dir) = cli.parent() {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let home = home_dir();
    for name in names {
        let candidate = home.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let nested = home.join("Release").join(name);
        if nested.is_file() {
            return Some(nested);
        }
    }

    names.iter().find_map(|name| find_on_path(name))
}

/// Ensure a server for `model_file` + `language` is running, returning its port.
fn ensure_server(
    server_bin: &Path,
    model_file: &Path,
    language: &str,
    prompt: &str,
) -> Result<u16, String> {
    let mut slot = server_slot()
        .lock()
        .map_err(|_| "whisper server 状态锁被污染".to_string())?;

    if let Some(server) = slot.as_mut() {
        let alive = server
            .child
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false);
        if alive
            && server.server_bin == server_bin
            && server.model_path == model_file
            && server.language == language
            && server.prompt == prompt
        {
            return Ok(server.port);
        }
        let _ = server.child.kill();
        let _ = server.child.wait();
        *slot = None;
    }

    let port = free_port()?;
    log::info!(
        "starting whisper server: {:?} -m {:?} -l {} --port {}",
        server_bin,
        model_file,
        normalize_language(language),
        port
    );

    let mut command = Command::new(server_bin);
    command
        .arg("-m")
        .arg(model_file)
        .arg("-nt")
        .arg("-l")
        .arg(normalize_language(language))
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string());
    if !prompt.trim().is_empty() {
        command.arg("--prompt").arg(prompt);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::hide_console(&mut command);

    let child = command
        .spawn()
        .map_err(|e| format!("启动 whisper server 失败：{e}"))?;

    if !wait_until_ready(port, Duration::from_secs(120)) {
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        return Err("whisper server 启动超时".into());
    }

    *slot = Some(WhisperServer {
        child,
        port,
        server_bin: server_bin.to_path_buf(),
        model_path: model_file.to_path_buf(),
        language: language.to_string(),
        prompt: prompt.to_string(),
    });
    Ok(port)
}

/// Whether a live server already holds `model_file` + `language`. Uses a
/// non-blocking lock so status polling never waits on an in-progress start.
fn server_running(server_bin: &Path, model_file: &Path, language: &str) -> bool {
    let Ok(mut slot) = server_slot().try_lock() else {
        return false;
    };
    let Some(server) = slot.as_mut() else {
        return false;
    };
    let alive = server
        .child
        .try_wait()
        .map(|status| status.is_none())
        .unwrap_or(false);
    alive
        && server.server_bin == server_bin
        && server.model_path == model_file
        && server.language == language
}

/// Load the configured model into the persistent server now, so the next
/// utterance skips the cold start.
pub fn warm_up(config: &SttConfig) -> Result<(), String> {
    let bin = resolve_binary(&config.runtime, config.binary_path.as_deref())?;
    let model_file = resolve_model(&config.model, config.model_path.as_deref())?;
    if !model_file.is_file() {
        return Err(format!("未找到语音模型：{}", model_file.display()));
    }
    let server_bin = resolve_server_binary(&bin).ok_or_else(|| {
        "未找到 whisper-server，无法预热（会回退到一次性 whisper-cli）".to_string()
    })?;
    ensure_server(&server_bin, &model_file, &config.language, &config.prompt)?;
    Ok(())
}

fn transcribe_via_server(
    server_bin: &Path,
    model_file: &Path,
    wav: &Path,
    language: &str,
    prompt: &str,
) -> Result<String, String> {
    let port = ensure_server(server_bin, model_file, language, prompt)?;

    let audio = std::fs::read(wav).map_err(|e| format!("读取音频失败：{e}"))?;
    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(audio)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .map_err(|e| format!("构建上传请求失败：{e}"))?,
        )
        .text("response_format", "text")
        .text("language", normalize_language(language));

    let url = format!("http://127.0.0.1:{port}/inference");
    let text = tauri::async_runtime::block_on(async move {
        let response = stt_client()
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("whisper server 请求失败：{e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("whisper server 返回 {status}：{}", body.trim()));
        }
        response
            .text()
            .await
            .map_err(|e| format!("读取 whisper server 响应失败：{e}"))
    })?;

    Ok(clean_transcript(&text))
}

fn stt_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default()
    })
}

/// Reserve an ephemeral localhost port, then release it for the server to bind.
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("分配本地端口失败：{e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地端口失败：{e}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn wait_until_ready(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Strip timestamp prefixes and stray blank lines from whisper's stdout.
fn clean_transcript(raw: &str) -> String {
    let mut text = String::new();
    for line in raw.lines() {
        let line = strip_timestamp(line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(trimmed);
    }
    text.trim().to_string()
}

/// Remove a leading `[00:00:00.000 --> 00:00:02.000]` segment if present.
fn strip_timestamp(line: &str) -> &str {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('[') {
        return line;
    }
    match trimmed.find(']') {
        Some(end) => trimmed[end + 1..].trim_start(),
        None => line,
    }
}

/// Scan `PATH` for an executable.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Download and extract a prebuilt whisper.cpp Windows runtime.
///
/// Each runtime installs into its own subdirectory so several can coexist; the
/// configured one is what actually runs. The CUDA zips bundle their runtime
/// DLLs and use the GPU automatically.
#[cfg(target_os = "windows")]
pub async fn download_binary(runtime: &str) -> Result<PathBuf, String> {
    const BASE: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/";
    let asset = match runtime {
        "cuda12" => "whisper-cublas-12.4.0-bin-x64.zip",
        "cuda11" => "whisper-cublas-11.8.0-bin-x64.zip",
        _ => "whisper-bin-x64.zip",
    };
    let url = format!("{BASE}{asset}");

    let dir = runtime_dir(runtime);
    let target = dir.join("whisper-cli.exe");
    if target.is_file() {
        return Ok(target);
    }
    // A legacy flat install of this runtime already satisfies the request.
    let legacy = home_dir().join("whisper-cli.exe");
    if legacy_runtime() == runtime && legacy.is_file() {
        return Ok(legacy);
    }

    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{}", e))?;

    // A running server locks the DLLs the archive is about to overwrite; kill
    // the tracked server and any orphan left by a previously crashed run.
    shutdown_server();
    kill_whisper_processes();

    // A previous run may have unpacked into `Release/` but failed the check.
    if dir.join("Release").join("whisper-cli.exe").is_file() {
        flatten_release_dir(&dir)?;
        if target.is_file() {
            return Ok(target);
        }
    }

    log::info!("downloading whisper runtime from {url}");
    let zip_path = std::env::temp_dir().join(format!("clay-mic-{asset}"));
    stream_download(&url, &zip_path, "binary").await?;

    emit_progress("binary", "extracting", 0);

    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip_path.display(),
        dir.display()
    );
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    crate::process::hide_console(&mut command);
    let status = command
        .status()
        .map_err(|e| format!("调用 PowerShell 解压失败：{}", e))?;
    let _ = std::fs::remove_file(&zip_path);

    if !status.success() {
        return Err("解压 whisper 压缩包失败".into());
    }

    // The official zip nests everything under `Release/`; flatten it so the
    // exe sits next to its DLLs in `dir`.
    flatten_release_dir(&dir)?;

    if !target.is_file() {
        return Err(format!(
            "解压后未找到 whisper-cli.exe（目录：{}）",
            dir.display()
        ));
    }
    log::info!("whisper runtime {runtime} extracted to {:?}", dir);
    emit_progress("binary", "done", 100);
    Ok(target)
}

#[cfg(not(target_os = "windows"))]
pub async fn download_binary(_runtime: &str) -> Result<PathBuf, String> {
    Err("自动下载仅支持 Windows；请手动安装 whisper-cli".into())
}

/// The whisper.cpp release zip nests every file under `Release/`. Move them up
/// into `dir` so the executables find their DLLs.
#[cfg(target_os = "windows")]
fn flatten_release_dir(dir: &Path) -> Result<(), String> {
    let nested = dir.join("Release");
    if !nested.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&nested).map_err(|e| format!("读取 Release 目录失败：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let destination = dir.join(entry.file_name());
        rename_with_retry(&entry.path(), &destination)
            .map_err(|e| format!("移动 {} 失败：{e}", entry.file_name().to_string_lossy()))?;
    }
    let _ = std::fs::remove_dir(&nested);
    Ok(())
}

/// Kill any whisper process still holding the runtime files.
#[cfg(target_os = "windows")]
fn kill_whisper_processes() {
    for image in ["whisper-server.exe", "whisper-cli.exe"] {
        let mut command = Command::new("taskkill");
        command
            .args(["/F", "/IM", image])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::process::hide_console(&mut command);
        let _ = command.status();
    }
    std::thread::sleep(Duration::from_millis(300));
}

/// A release file can still be mapped for a moment after its process died, so
/// retry the rename briefly instead of failing the whole install.
#[cfg(target_os = "windows")]
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = std::io::Error::from(std::io::ErrorKind::Other);
    for attempt in 0..5u64 {
        let _ = std::fs::remove_file(to);
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = error;
                std::thread::sleep(Duration::from_millis(200 * (attempt + 1)));
            }
        }
    }
    Err(last)
}

/// Download a GGML model into [`model_dir`] if it is missing.
pub async fn download_model(model: &str) -> Result<PathBuf, String> {
    let target = resolve_model(model, None)?;
    if target.is_file() {
        return Ok(target);
    }
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("无效的模型名")?;

    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        name
    );
    log::info!("downloading whisper model from {url}");

    std::fs::create_dir_all(model_dir()).map_err(|e| e.to_string())?;
    stream_download(&url, &target, "model").await?;
    log::info!("model saved to {:?}", target);
    emit_progress("model", "done", 100);
    Ok(target)
}
