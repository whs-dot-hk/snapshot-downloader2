use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::{CONTENT_LENGTH, RANGE};
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, trace};

pub async fn download_file(url: &str, download_dir: &Path, file_type: &str) -> Result<PathBuf> {
    let client = reqwest::Client::new();

    // Create filename from URL
    let file_name = url
        .split('/')
        .last()
        .context("Failed to determine filename from URL")?;

    let file_path = download_dir.join(file_name);
    debug!("Download path set to: {:?}", file_path);

    // Check if file already exists (for resuming)
    let file_size = if file_path.exists() {
        let size = file_path.metadata()?.len();
        debug!("Existing file found with size: {} bytes", size);
        size
    } else {
        debug!("No existing file found, starting fresh download");
        0
    };

    // Get total file size by requesting just the first byte
    trace!("Requesting file metadata from server");
    let resp = client.get(url).header(RANGE, "bytes=0-0").send().await?;

    let total_size = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        // Server supports range requests if it returns 206 Partial Content
        resp.headers()
            .get("content-range")
            .and_then(|val| val.to_str().ok())
            .and_then(|val| {
                // Parse content-range header like "bytes 0-0/12345"
                val.split('/')
                    .last()
                    .and_then(|size| size.parse::<u64>().ok())
            })
            .unwrap_or(0)
    } else {
        // If server doesn't support range requests, try to get content length from response
        resp.headers()
            .get(CONTENT_LENGTH)
            .and_then(|ct_len| ct_len.to_str().ok())
            .and_then(|ct_len| ct_len.parse::<u64>().ok())
            .unwrap_or(0)
    };

    debug!("Total file size: {} bytes", total_size);

    // If file is already complete, return early
    if file_size == total_size && total_size > 0 {
        info!("{} is already downloaded completely", file_type);
        return Ok(file_path);
    }

    // Set up progress bar
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_position(file_size);

    // Prepare request with range header for resuming
    let mut request = client.get(url);
    if file_size > 0 {
        info!("Resuming {} download from {} bytes", file_type, file_size);
        request = request.header(RANGE, format!("bytes={}-", file_size));
    } else {
        info!("Starting {} download", file_type);
    }

    let response = request.send().await?;

    // Handle potential 416 Range Not Satisfiable error (file already complete)
    if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        info!("{} is already downloaded completely", file_type);
        pb.finish();
        return Ok(file_path);
    }

    // Ensure successful response
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download {}: HTTP status {}",
            file_type,
            response.status()
        ));
    }

    // Open file for writing, with append mode if resuming
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(file_size > 0)
        .open(&file_path)?;

    // If not resuming, ensure we start from the beginning
    if file_size == 0 {
        file.seek(SeekFrom::Start(0))?;
    }

    // Download the file
    let mut stream = response.bytes_stream();
    let mut downloaded = file_size;
    trace!("Beginning download stream");

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;

        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);

        // Log progress at reasonable intervals (every 10% or so)
        if downloaded % (total_size / 10).max(1) < (chunk.len() as u64) {
            trace!("Download progress: {}/{} bytes", downloaded, total_size);
        }
    }

    pb.finish_with_message(format!("{} download complete", file_type));
    info!(
        "{} download completed successfully: {}",
        file_type,
        file_path.display()
    );

    Ok(file_path)
}
