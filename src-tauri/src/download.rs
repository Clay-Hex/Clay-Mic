//! Streamed file downloads with `download://progress` events.
//!
//! Shared by the STT runtime/model downloads, the Interception driver
//! download and the in-app updater so the UI can show one progress bar
//! regardless of the source.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub source: &'static str,
    pub phase: &'static str,
    pub percent: u32,
}

pub fn emit_progress(source: &'static str, phase: &'static str, percent: u32) {
    crate::runtime::emit(
        "download://progress",
        DownloadProgress {
            source,
            phase,
            percent,
        },
    );
}

/// Stream-download a URL to a file, emitting progress as it goes.
pub async fn stream_download(url: &str, dest: &Path, source: &'static str) -> Result<(), String> {
    stream_download_with(url, dest, source, None, None).await
}

/// Same as [`stream_download`], plus a cancellation flag polled per chunk
/// and an optional total request timeout. The updater passes both: cancel
/// lets the about page abort, and the timeout stops a stalled connection
/// from hanging forever — while the long model downloads keep no total
/// timeout, so a slow-but-healthy transfer is never cut off midway.
///
/// Every caller gets a connect timeout either way: a stalled handshake must
/// never hang the UI.
pub async fn stream_download_with(
    url: &str,
    dest: &Path,
    source: &'static str,
    cancel: Option<&AtomicBool>,
    timeout: Option<Duration>,
) -> Result<(), String> {
    // Emit immediately so the UI shows the bar before the (possibly slow)
    // redirect + connection setup completes.
    emit_progress(source, "downloading", 0);

    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    let client = builder.build().map_err(|e| format!("下载失败：{}", e))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败：{}", e))?;
    if !response.status().is_success() {
        return Err(format!("下载失败：HTTP {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建文件失败：{}", e))?;
    let mut downloaded: u64 = 0;
    let mut last_reported: u32 = 0;

    let mut stream = response;
    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| format!("读取数据失败：{}", e))?
    {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("已取消".to_string());
        }
        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败：{}", e))?;
        downloaded += chunk.len() as u64;
        let pct = if total > 0 {
            (downloaded * 100 / total).min(100) as u32
        } else {
            // No Content-Length: report coarse progress by MiB received.
            (downloaded / 1024 / 1024).min(99) as u32
        };
        if pct > last_reported {
            last_reported = pct;
            emit_progress(source, "downloading", pct);
        }
    }
    emit_progress(source, "downloading", 100);
    Ok(())
}
