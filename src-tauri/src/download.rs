//! Streamed file downloads with `download://progress` events.
//!
//! Shared by the STT runtime/model downloads and the Interception driver
//! download so the UI can show one progress bar regardless of the source.

use std::io::Write as _;
use std::path::Path;

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
    // Emit immediately so the UI shows the bar before the (possibly slow)
    // redirect + connection setup completes.
    emit_progress(source, "downloading", 0);

    let client = reqwest::Client::new();
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
