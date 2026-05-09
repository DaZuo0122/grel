//! Parallel download functionality.

use std::path::Path;

use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::NetworkError;

/// Download a single file
pub async fn download_file(
    client: &Client,
    url: &str,
    dest: &Path,
    progress_bar: Option<&ProgressBar>,
) -> Result<String, NetworkError> {
    // Create parent directory
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
        })?;
    }

    // Start download
    let response = client.get(url).send().await?;
    let total_size = response.content_length().unwrap_or(0);

    if let Some(pb) = progress_bar {
        if total_size > 0 {
            pb.set_length(total_size);
        }
    }

    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create file: {e}")))?;

    let mut hasher = Sha256::new();
    let mut downloaded = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
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

    let checksum = format!("{:x}", hasher.finalize());
    Ok(checksum)
}

/// Download multiple files in parallel
pub async fn download_parallel(
    client: &Client,
    downloads: &[DownloadTask],
    max_concurrent: usize,
) -> Result<Vec<DownloadResult>, NetworkError> {
    let total = downloads.len() as u64;
    let pb = ProgressBar::new(total);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap()
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
                let result = download_file_with_retry(&client, &task, max_concurrent).await;
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
    _max_retries: usize,
) -> DownloadResult {
    // Simple implementation without retry for now
    match download_file(client, &task.url, &task.dest_path, None).await {
        Ok(checksum) => {
            let success = task
                .expected_checksum
                .as_ref()
                .map_or(true, |expected| expected == &checksum);

            DownloadResult {
                filename: task.filename.clone(),
                dest_path: task.dest_path.clone(),
                checksum,
                success,
                error: None,
            }
        }
        Err(e) => DownloadResult {
            filename: task.filename.clone(),
            dest_path: task.dest_path.clone(),
            checksum: String::new(),
            success: false,
            error: Some(e.to_string()),
        },
    }
}
