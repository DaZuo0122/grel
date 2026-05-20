//! Parallel download functionality.

use std::path::Path;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use crate::Client;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::NetworkError;

/// Download a single file
///
/// Supports resuming partial downloads when `resume = true` and the destination
/// file already exists. On 304 Not Modified (when ETag is provided), skips the
/// download and returns the cached checksum if available.
///
/// Returns `(checksum, etag)` where `etag` is the ETag from the response headers
/// (or `None` if the server did not send one).
pub async fn download_file(
    client: &Client,
    url: &str,
    dest: &Path,
    progress_bar: Option<&ProgressBar>,
    resume: bool,
    etag: Option<&str>,
) -> Result<(String, Option<String>), NetworkError> {
    // Create parent directory
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
        })?;
    }

    // Build request
    let mut request = client.get(url);

    // Add If-None-Match if we have a cached ETag
    if let Some(etag_value) = etag {
        request = request.header("If-None-Match", etag_value);
    }

    // Handle resume
    let existing_len = if resume {
        match tokio::fs::metadata(dest).await {
            Ok(meta) => meta.len(),
            Err(_) => 0,
        }
    } else {
        0
    };

    if resume && existing_len > 0 {
        request = request.header("Range", format!("bytes={}-", existing_len));
    }

    let response = request.send().await?;
    let status = response.status();

    // Extract ETag from response headers before consuming body
    let response_etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 304 Not Modified — nothing to download
    if status == reqwest::StatusCode::NOT_MODIFIED {
        // Re-read the existing file and compute its checksum
        let content = tokio::fs::read(dest)
            .await
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read cached file: {e}")))?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        return Ok((format!("{:x}", hasher.finalize()), response_etag));
    }

    // 416 Range Not Satisfiable — file is already complete
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let content = tokio::fs::read(dest)
            .await
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read file: {e}")))?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        return Ok((format!("{:x}", hasher.finalize()), response_etag));
    }

    // Check for other errors
    if !status.is_success() {
        return Err(NetworkError::OperationFailed(format!(
            "HTTP {status} for {url}"
        )));
    }

    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total_size = response.content_length().unwrap_or(0);

    if let Some(pb) = progress_bar {
        if total_size > 0 {
            pb.set_length(total_size + existing_len);
        }
    }

    let mut stream = response.bytes_stream();

    let mut file = if is_partial && existing_len > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .await
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to open file: {e}")))?
    } else {
        tokio::fs::File::create(dest)
            .await
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to create file: {e}")))?
    };

    let mut downloaded = existing_len;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)
            .await
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to write file: {e}")))?;

        downloaded += chunk.len() as u64;
        if let Some(pb) = progress_bar {
            pb.set_position(downloaded);
        }
    }

    file.flush()
        .await
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to flush file: {e}")))?;

    drop(file);

    // Re-read the full file to compute checksum (ensures resume correctness)
    let content = tokio::fs::read(dest)
        .await
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to read file for checksum: {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let checksum = format!("{:x}", hasher.finalize());

    Ok((checksum, response_etag))
}

/// Download multiple files in parallel
pub async fn download_parallel(
    client: &Client,
    downloads: &[DownloadTask],
    max_concurrent: usize,
    max_retries: u32,
) -> Result<Vec<DownloadResult>, NetworkError> {
    let total = downloads.len() as u64;
    let pb = ProgressBar::new(total);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
            .progress_chars("#>-"),
    );

    // Clone downloads to avoid lifetime issues
    let downloads_clone: Vec<DownloadTask> = downloads.to_vec();
    let pb_clone = pb.clone();

    let results = stream::iter(downloads_clone)
        .map(move |task| {
            let client = client.clone();
            let pb = pb_clone.clone();
            async move {
                let filename = task.filename.clone();
                let result = download_file_with_retry(&client, &task, max_retries).await;
                pb.inc(1);
                pb.set_message(filename.clone());
                result
            }
        })
        .buffer_unordered(max_concurrent)
        .collect::<Vec<_>>()
        .await;

    pb.finish_with_message("Download complete");
    Ok(results)
}

/// A download task
#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub filename: String,
    pub dest_path: std::path::PathBuf,
    pub expected_checksum: Option<String>,
}

/// Result of a download
#[derive(Debug)]
pub struct DownloadResult {
    pub filename: String,
    pub dest_path: std::path::PathBuf,
    pub checksum: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Download a file with retry logic
async fn download_file_with_retry(
    client: &Client,
    task: &DownloadTask,
    max_retries: u32,
) -> DownloadResult {
    for attempt in 0..=max_retries {
        match download_file(client, &task.url, &task.dest_path, None, true, None).await {
            Ok((checksum, _etag)) => {
                let success = task
                    .expected_checksum
                    .as_ref()
                    .map_or(true, |expected| expected == &checksum);

                return DownloadResult {
                    filename: task.filename.clone(),
                    dest_path: task.dest_path.clone(),
                    checksum,
                    success,
                    error: None,
                };
            }
            Err(e) => {
                if attempt < max_retries && is_transient_error(&e) {
                    let delay = backoff_delay(attempt);
                    tracing::warn!(
                        "Download attempt {}/{} failed for {}: {}. Retrying in {:?}...",
                        attempt + 1,
                        max_retries + 1,
                        task.filename,
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                } else {
                    return DownloadResult {
                        filename: task.filename.clone(),
                        dest_path: task.dest_path.clone(),
                        checksum: String::new(),
                        success: false,
                        error: Some(e.to_string()),
                    };
                }
            }
        }
    }

    // Unreachable, but satisfies compiler
    DownloadResult {
        filename: task.filename.clone(),
        dest_path: task.dest_path.clone(),
        checksum: String::new(),
        success: false,
        error: Some("Max retries exceeded".into()),
    }
}

/// Check if an error is transient (worth retrying)
fn is_transient_error(err: &NetworkError) -> bool {
    match err {
        NetworkError::HttpError(e) => {
            e.is_timeout()
                || e.is_connect()
                || e.status()
                    .map_or(false, |s| s.is_server_error() || s == reqwest::StatusCode::TOO_MANY_REQUESTS)
        }
        NetworkError::OperationFailed(msg) => {
            // Retry on broken stream / connection reset
            msg.contains("broken pipe")
                || msg.contains("connection reset")
                || msg.contains("connection refused")
        }
        _ => false,
    }
}

/// Compute exponential backoff delay with jitter
fn backoff_delay(attempt: u32) -> Duration {
    let base = 1000u64; // 1 second
    let exponential = base * 2_u64.pow(attempt);
    let capped = exponential.min(30_000); // cap at 30 seconds
    let jitter = fastrand::u64(0..=500); // 0-500ms jitter
    Duration::from_millis(capped + jitter)
}
