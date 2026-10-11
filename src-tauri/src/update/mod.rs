//! Check-for-update, download, and apply for the about page.
//!
//! The version comes from the `releases/latest` redirect, so the core path
//! spends no GitHub API quota; only the notes use the API and degrade to empty.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Every link this module opens lives under this prefix; `open_url` re-checks
/// it so the command cannot be used as a generic launcher.
const REPO_PREFIX: &str = "https://github.com/Clay-Hex/Clay-Mic/";
/// The redirect target of this URL is the newest tag — no API quota spent.
const LATEST_URL: &str = "https://github.com/Clay-Hex/Clay-Mic/releases/latest";
const API_LATEST_URL: &str =
    "https://api.github.com/repos/Clay-Hex/Clay-Mic/releases/latest";

/// A cached check younger than this is served as-is; `force` bypasses it.
const CHECK_TTL_SECS: u64 = 6 * 60 * 60;
/// Release-note lines shown in the about page.
const NOTES_MAX_LINES: usize = 6;
/// The main executable must be this large before the package counts.
const MIN_EXECUTABLE_SIZE: u64 = 1024 * 1024;
/// Sanity ceiling for the uncompressed package contents.
const MAX_EXTRACTED_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Set by `cancel_download`, polled per chunk by the streaming download.
static CANCEL: AtomicBool = AtomicBool::new(false);
/// One download at a time: two writers on the same `.part` would corrupt it.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Newest release known to the about page, as the frontend sees it.
/// `current` describes the running build rather than the release, so every
/// read path re-fills it via `with_current` — the copy persisted in the
/// check cache is only a placeholder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    /// Newest tag without the `v` prefix.
    pub latest: String,
    /// git-cliff Markdown already reduced to a few plain lines.
    pub notes: String,
    /// ISO timestamp from the release API; empty when the notes fetch failed.
    pub published_at: String,
    pub asset_url: String,
    pub asset_size: u64,
    /// Hex digest of the `.sha256` asset; `None` = this release ships none.
    pub sha256: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCheck {
    checked_at: u64,
    info: UpdateInfo,
}

/// Releases the single-download slot on every exit path, including when the
/// future is dropped (window reload during development, app shutdown).
struct DownloadGuard;

impl DownloadGuard {
    fn acquire() -> Result<Self, String> {
        if DOWNLOADING.swap(true, Ordering::SeqCst) {
            return Err("已有下载正在进行".to_string());
        }
        CANCEL.store(false, Ordering::SeqCst);
        Ok(Self)
    }
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        DOWNLOADING.store(false, Ordering::SeqCst);
    }
}

/// Check the repository for a newer release. `force` skips the on-disk
/// cache; `Ok(None)` means the repository has no releases at all — "already
/// latest" is the frontend comparing `current` against `latest`.
#[tauri::command]
pub async fn check_update(force: bool) -> Result<Option<UpdateInfo>, String> {
    if !force {
        if let Some(cached) = read_cache() {
            let age = now_secs().saturating_sub(cached.checked_at);
            if age < CHECK_TTL_SECS {
                return Ok(Some(with_current(cached.info)));
            }
        }
    }
    let fetched = fetch_latest().await?;
    if let Some(info) = &fetched {
        write_cache(info);
    }
    Ok(fetched.map(with_current))
}

/// Download the release zip into the updates directory: `.part` first,
/// verify (SHA256 when the release ships a checksum, then the zip
/// structure), and only then rename it into place. Returns the final path.
#[tauri::command]
pub async fn download_update() -> Result<String, String> {
    let info = read_cache()
        .map(|cached| cached.info)
        .ok_or_else(|| "请先检查更新".to_string())?;
    let file_name = asset_file_name(&info)?;
    let dir = updates_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建更新目录失败：{}", error))?;
    let final_path = dir.join(&file_name);
    let part_path = dir.join(format!("{}.part", file_name));

    // A package left behind by an earlier run is trusted only after it has
    // been verified again.
    if final_path.exists() {
        verify_package(&final_path, &info)?;
        return Ok(final_path.to_string_lossy().into_owned());
    }

    let _guard = DownloadGuard::acquire()?;
    let outcome = crate::download::stream_download_with(
        &info.asset_url,
        &part_path,
        "update",
        Some(&CANCEL),
        Some(Duration::from_secs(600)),
    )
    .await;
    if let Err(error) = outcome {
        let _ = std::fs::remove_file(&part_path);
        return Err(error);
    }
    if let Err(error) = verify_package(&part_path, &info) {
        let _ = std::fs::remove_file(&part_path);
        return Err(error);
    }
    // The final path only ever holds a fully verified package.
    if final_path.exists() {
        let _ = std::fs::remove_file(&final_path);
    }
    std::fs::rename(&part_path, &final_path).map_err(|error| format!("保存更新包失败：{}", error))?;
    crate::download::emit_progress("update", "done", 100);
    Ok(final_path.to_string_lossy().into_owned())
}

/// Stop an in-flight download; `download_update` removes the partial file.
#[tauri::command]
pub fn cancel_download() -> Result<(), String> {
    CANCEL.store(true, Ordering::SeqCst);
    Ok(())
}

/// Open a link from the update flow in the default browser. The whitelist
/// keeps this command from becoming a generic launcher, and rejecting shell
/// metacharacters keeps `cmd /c start` from parsing anything past the URL.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with(REPO_PREFIX) {
        return Err("只允许打开项目仓库页面".to_string());
    }
    if url.chars().any(|c| {
        matches!(c, '"' | '\'' | '&' | '|' | '<' | '>' | '^' | '%' | '!' | '\r' | '\n')
    }) {
        return Err("链接包含不允许的字符".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        // `start` reads the first quoted argument as its window title, hence "".
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", "", &url]);
        crate::process::hide_console(&mut command);
        command.spawn().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Reveal the updates directory (downloaded packages live there).
#[tauri::command]
pub fn open_updates_dir() -> Result<(), String> {
    let dir = updates_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建更新目录失败：{}", error))?;
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("explorer");
        command.arg(&dir);
        crate::process::hide_console(&mut command);
        command.spawn().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// `Ok(None)` when the repository has no releases at all (HTTP 404).
async fn fetch_latest() -> Result<Option<UpdateInfo>, String> {
    // The tag arrives in the redirect target, so this one client must NOT
    // follow it — following would hand back the HTML release page.
    let tag_client = build_client(Duration::from_secs(30), false)?;
    let response = tag_client
        .get(LATEST_URL)
        .send()
        .await
        .map_err(|error| format!("检查更新失败：{}", error))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    // A 302 is not a success status: it is the expected answer here.
    if !response.status().is_redirection() {
        return Err(format!("检查更新失败：HTTP {}", response.status()));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    // Location: /Clay-Hex/Clay-Mic/releases/tag/v1.2.0
    let tag = location.rsplit('/').next().unwrap_or_default().to_string();
    let version = tag.trim_start_matches('v').to_string();
    if !tag.starts_with('v') || parse_version(&version).is_none() {
        return Err("检查更新失败：无法从跳转地址解析版本号".to_string());
    }

    // Everything below follows redirects normally (asset URLs 302 to the
    // release CDN), and needs no API quota except for the notes call.
    let client = build_client(Duration::from_secs(30), true)?;
    // The asset URL follows the release naming convention; only its size
    // needs a request (HEAD, no API quota).
    let asset_url =
        format!("{REPO_PREFIX}releases/download/{tag}/Clay-Mic-{version}-portable.zip");
    let asset_size = head_length(&client, &asset_url).await;
    // 404 here simply means the release ships no checksum (historical
    // releases predate the CI step that produces them).
    let sha256 = fetch_sha256(&client, &format!("{asset_url}.sha256")).await?;
    let (notes, published_at) = fetch_notes(&client)
        .await
        .unwrap_or_else(|| (String::new(), String::new()));

    Ok(Some(UpdateInfo {
        // `with_current` fills this on the way out.
        current: String::new(),
        latest: version,
        notes,
        published_at,
        asset_url,
        asset_size,
        sha256,
        html_url: format!("{REPO_PREFIX}releases/tag/{tag}"),
    }))
}

/// Content-Length of the asset; 0 when the CDN declines to say.
async fn head_length(client: &reqwest::Client, url: &str) -> u64 {
    client
        .head(url)
        .send()
        .await
        .ok()
        .filter(|response| response.status().is_success())
        .and_then(|response| response.content_length())
        .unwrap_or(0)
}

/// The `.sha256` asset next to the zip, as a lowercase hex digest.
async fn fetch_sha256(client: &reqwest::Client, url: &str) -> Result<Option<String>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("获取校验和失败：{}", error))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("获取校验和失败：HTTP {}", response.status()));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("获取校验和失败：{}", error))?;
    // sha256sum style: `<hex>  <file name>`.
    let hex = body
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("获取校验和失败：内容格式不正确".to_string());
    }
    Ok(Some(hex))
}

/// Release notes plus publish date from the API — best effort only. Any
/// failure returns `None`, which callers turn into an empty note: the check
/// itself never fails because of missing notes.
async fn fetch_notes(client: &reqwest::Client) -> Option<(String, String)> {
    let response = client
        .get(API_LATEST_URL)
        .header(reqwest::header::USER_AGENT, "clay-mic")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: serde_json::Value = response.json().await.ok()?;
    let raw = value.get("body").and_then(|body| body.as_str()).unwrap_or("");
    let published_at = value
        .get("published_at")
        .and_then(|field| field.as_str())
        .unwrap_or("");
    Some((notes_to_text(raw), published_at.to_string()))
}

/// Release notes are git-cliff Markdown; the about page shows plain lines.
/// Fenced blocks are dropped, heading/list markers stripped, links reduced
/// to their label, and the whole thing capped at `NOTES_MAX_LINES` lines.
fn notes_to_text(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut text = trimmed
            .trim_start_matches(|c| matches!(c, '#' | '-' | '*' | '+'))
            .trim()
            .to_string();
        // [label](url) → label
        if let Some(start) = text.find('[') {
            if let Some(middle) = text[start..].find("](") {
                let middle = start + middle;
                if let Some(end) = text[middle + 2..].find(')') {
                    let end = middle + 2 + end;
                    let label = text[start + 1..middle].to_string();
                    text.replace_range(start..=end, &label);
                }
            }
        }
        let text = text.replace('`', "").trim().to_string();
        if text.is_empty() {
            continue;
        }
        lines.push(text);
        if lines.len() >= NOTES_MAX_LINES {
            break;
        }
    }
    lines.join("\n")
}

/// SHA256 (when the release ships a checksum) plus a structural check of the
/// zip: the app executable must be present and no entry may escape the
/// extraction directory. Emits the "verifying" phase so the bar never looks
/// frozen between the download and completion.
fn verify_package(path: &Path, info: &UpdateInfo) -> Result<(), String> {
    crate::download::emit_progress("update", "verifying", 0);
    if let Some(expected) = &info.sha256 {
        let actual = sha256_hex(path).map_err(|error| format!("校验失败：{}", error))?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("校验失败：SHA256 不匹配，请重新下载".to_string());
        }
    }
    verify_zip(path)
}

/// Streaming digest so a large package never sits fully in memory.
fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    // LowerHex on the digest renders full-width lowercase hex (64 chars).
    Ok(format!("{:x}", hasher.finalize()))
}

/// Assert the package shape before it is ever trusted: real executable at
/// the root, no path-traversing entry names (zip slip), sane total size.
fn verify_zip(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("校验失败：{}", error))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "校验失败：文件不是有效的 zip".to_string())?;
    let mut has_executable = false;
    let mut total_size: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "校验失败：zip 结构损坏".to_string())?;
        let name = entry.name().to_string();
        if name.contains("..")
            || name.starts_with('/')
            || name.starts_with('\\')
            || name.contains(':')
        {
            return Err("校验失败：zip 内含异常路径".to_string());
        }
        if name == "Clay-Mic.exe" && entry.size() > MIN_EXECUTABLE_SIZE {
            has_executable = true;
        }
        total_size += entry.size();
    }
    if !has_executable {
        return Err("校验失败：压缩包内缺少 Clay-Mic.exe".to_string());
    }
    if total_size == 0 || total_size > MAX_EXTRACTED_SIZE {
        return Err("校验失败：压缩包内容大小异常".to_string());
    }
    Ok(())
}

/// Last path segment of the asset URL — the file name the release ships.
fn asset_file_name(info: &UpdateInfo) -> Result<String, String> {
    info.asset_url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && name.ends_with(".zip"))
        .map(str::to_string)
        .ok_or_else(|| "更新地址异常".to_string())
}

/// Every client gets a connect timeout: a stalled handshake must never hang
/// the UI. `follow_redirects` is off only for the tag lookup, which reads
/// the `Location` header instead of chasing it.
fn build_client(timeout: Duration, follow_redirects: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout);
    if !follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder
        .build()
        .map_err(|error| format!("创建请求失败：{}", error))
}

/// `1.2.3` / `v1.2.3-beta.1` → numeric triple; anything else is unusable.
fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let core = raw.trim().trim_start_matches('v').split('-').next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// `%LOCALAPPDATA%\clay-mic\updates` — downloads, check cache, and (P2)
/// the staging tree the updater applies.
fn updates_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("updates")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn cache_path() -> PathBuf {
    updates_dir().join("last-check.json")
}

fn read_cache() -> Option<CachedCheck> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Best effort: the check itself already succeeded, so a failed cache write
/// only costs a repeat lookup later.
fn write_cache(info: &UpdateInfo) {
    let cached = CachedCheck {
        checked_at: now_secs(),
        info: info.clone(),
    };
    let _ = std::fs::create_dir_all(updates_dir());
    if let Ok(text) = serde_json::to_string_pretty(&cached) {
        let _ = std::fs::write(cache_path(), text);
    }
}

/// The cache keeps a placeholder (or stale) `current`; every path out of
/// this module re-fills it with the running build's version.
fn with_current(mut info: UpdateInfo) -> UpdateInfo {
    info.current = env!("CARGO_PKG_VERSION").to_string();
    info
}

const UPDATER_EXE: &str = "clay-update.exe";
const MAIN_EXE: &str = "Clay-Mic.exe";
const JOURNAL_FILE: &str = "update.pending";
const OK_FILE: &str = "update.ok";

fn extract_to_staging(info: &UpdateInfo) -> Result<PathBuf, String> {
    let version = info.latest.trim();
    if parse_version(version).is_none() {
        return Err("更新版本号异常".to_string());
    }
    let zip_path = updates_dir().join(asset_file_name(info)?);
    if !zip_path.is_file() {
        return Err("更新包不存在，请重新下载".to_string());
    }

    let destination = staging_dir(version);
    let _ = std::fs::remove_dir_all(&destination);
    std::fs::create_dir_all(&destination).map_err(|e| format!("创建暂存目录失败：{e}"))?;

    let file = std::fs::File::open(&zip_path).map_err(|e| format!("打开更新包失败：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "更新包不是有效的 zip".to_string())?;
    let total = archive.len().max(1);
    crate::download::emit_progress("update", "extracting", 0);
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "更新包结构损坏".to_string())?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| "更新包内含异常路径".to_string())?;
        let out = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| format!("解压失败：{e}"))?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("解压失败：{e}"))?;
            }
            let mut writer = std::fs::File::create(&out).map_err(|e| format!("解压失败：{e}"))?;
            std::io::copy(&mut entry, &mut writer).map_err(|e| format!("解压失败：{e}"))?;
        }
        let percent = ((index + 1) * 100 / total) as u32;
        crate::download::emit_progress("update", "extracting", percent);
    }
    Ok(destination)
}

#[tauri::command]
pub async fn apply_update(app: tauri::AppHandle) -> Result<(), String> {
    let launcher = app.clone();
    let result = tauri::async_runtime::spawn_blocking(prepare_and_launch)
        .await
        .map_err(|error| error.to_string())?;
    match result {
        Ok(()) => {
            launcher.exit(0);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Pre-flight everything before the app quits, then spawn the helper.
fn prepare_and_launch() -> Result<(), String> {
    let info = read_cache()
        .map(|cached| cached.info)
        .ok_or_else(|| "请先检查更新".to_string())?;
    let version = info.latest.trim().to_string();
    let current = env!("CARGO_PKG_VERSION");
    // Refuse a downgrade.
    match (parse_version(&version), parse_version(current)) {
        (Some(latest), Some(running)) if latest > running => {}
        (Some(_), Some(_)) => return Err("已是最新版本，无需更新".to_string()),
        _ => return Err("版本号异常，无法更新".to_string()),
    }

    let exe = std::env::current_exe().map_err(|e| format!("定位程序失败：{e}"))?;
    let target = exe
        .parent()
        .ok_or_else(|| "无法定位程序目录".to_string())?
        .to_path_buf();
    let main_exe = exe
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "无法确定程序文件名".to_string())?;

    let staging = extract_to_staging(&info)?;
    if !staging.join(MAIN_EXE).is_file() {
        return Err("更新包解压不完整（缺少 Clay-Mic.exe）".to_string());
    }

    if !target_writable(&target) {
        return Err("安装目录不可写：请「打开所在文件夹」后手动覆盖".to_string());
    }
    if let Some(free) = free_space(&target) {
        let needed = dir_size(&staging).saturating_add(64 * 1024 * 1024);
        if free < needed {
            return Err(format!("磁盘空间不足（可用约 {} MB）", free / 1024 / 1024));
        }
    }

    let updater = find_updater().ok_or_else(|| "未找到更新组件 clay-update.exe".to_string())?;
    let mut command = std::process::Command::new(&updater);
    command
        .arg("--apply")
        .arg("--staging")
        .arg(&staging)
        .arg("--target")
        .arg(&target)
        .arg("--parent")
        .arg(std::process::id().to_string())
        .arg("--journal")
        .arg(target.join(JOURNAL_FILE))
        .arg("--log")
        .arg(apply_log_path())
        .arg("--version")
        .arg(&version)
        .arg("--main-exe")
        .arg(&main_exe);
    crate::process::hide_console(&mut command);
    command
        .spawn()
        .map_err(|e| format!("启动更新程序失败：{e}"))?;

    log::info!("update: handed off to {} for {version}", updater.display());
    Ok(())
}

pub fn on_startup() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(target) = exe.parent() else {
        return;
    };
    let journal = target.join(JOURNAL_FILE);
    let ok = target.join(OK_FILE);
    let current = env!("CARGO_PKG_VERSION");

    if !journal.is_file() {
        if ok.is_file() {
            let _ = std::fs::remove_file(&ok);
        }
        return;
    }

    let text = std::fs::read_to_string(&journal).unwrap_or_default();
    let (version, stage_applied, replaced, added) = parse_journal(&text);

    if version.as_deref() == Some(current) {
        // Signal success, then drop the saved originals. The helper's own
        // image stays locked while it runs, so keep the journal until all are gone.
        let _ = std::fs::write(&ok, format!("{current}\n"));
        for relative in &replaced {
            let _ = std::fs::remove_file(old_path(&target.join(relative)));
        }
        let still_saved = replaced
            .iter()
            .filter(|relative| old_path(&target.join(relative)).exists())
            .count();
        if still_saved == 0 {
            let _ = std::fs::remove_file(&journal);
            if let Some(version) = version.as_deref() {
                let _ = std::fs::remove_dir_all(staging_dir(version));
                let _ = std::fs::remove_file(
                    updates_dir().join(format!("Clay-Mic-{version}-portable.zip")),
                );
            }
        }
        log::info!("update: {current} started ({still_saved} file(s) still saved)");
    } else if stage_applied {
        // Rolled back already; just clear the markers.
        let _ = std::fs::remove_file(&journal);
        let _ = std::fs::remove_file(&ok);
        if let Some(version) = version.as_deref() {
            let _ = std::fs::remove_dir_all(staging_dir(version));
        }
        log::info!("update: rolled back (running {current})");
    } else {
        // Interrupted mid-swap: restore the saved originals.
        restore_from_journal(&target, &replaced, &added);
        let _ = std::fs::remove_file(&journal);
        let _ = std::fs::remove_file(&ok);
        if let Some(version) = version.as_deref() {
            let _ = std::fs::remove_dir_all(staging_dir(version));
        }
        log::warn!("update: recovered an interrupted swap (running {current})");
    }
}

/// Undo a partial swap: put each renamed original back, newest first.
fn restore_from_journal(target: &Path, replaced: &[String], added: &[String]) {
    for relative in replaced.iter().rev() {
        let original = target.join(relative);
        let old = old_path(&original);
        let _ = std::fs::remove_file(&original);
        let _ = std::fs::rename(&old, &original);
    }
    for relative in added {
        let _ = std::fs::remove_file(target.join(relative));
    }
}

fn parse_journal(text: &str) -> (Option<String>, bool, Vec<String>, Vec<String>) {
    let mut version = None;
    let mut stage_applied = false;
    let mut replaced = Vec::new();
    let mut added = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("version=") {
            version = Some(value.trim().to_string());
        } else if line.trim() == "stage=applied" {
            stage_applied = true;
        } else if let Some(value) = line.strip_prefix("R=") {
            replaced.push(value.to_string());
        } else if let Some(value) = line.strip_prefix("A=") {
            added.push(value.to_string());
        }
    }
    (version, stage_applied, replaced, added)
}

fn old_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.old", path.display()))
}

fn target_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".clay-write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn dir_size(dir: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

/// Free bytes on the volume holding `dir`.
#[cfg(target_os = "windows")]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None).ok()?;
    }
    Some(free)
}

#[cfg(not(target_os = "windows"))]
fn free_space(_dir: &Path) -> Option<u64> {
    None
}

fn staging_dir(version: &str) -> PathBuf {
    updates_dir().join("staging").join(version)
}

fn apply_log_path() -> PathBuf {
    updates_dir().join("apply.log")
}

/// Locate the bundled `clay-update.exe`.
fn find_updater() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [
                dir.join(UPDATER_EXE),
                dir.join("resources").join(UPDATER_EXE),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(UPDATER_EXE);
    target.is_file().then_some(target)
}
