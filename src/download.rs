use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::{CONTENT_LENGTH, RANGE};
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, trace, warn};

pub async fn download_file(url: &str, download_dir: &Path, file_type: &str) -> Result<PathBuf> {
    let client = reqwest::Client::new();

    // Create filename from URL
    let file_name = url
        .split('/')
        .next_back()
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

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        error!("File not found at URL: {}", url);
        return Err(anyhow::anyhow!(
            "404 Not Found: The requested file does not exist"
        ));
    }

    let total_size = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        // Server supports range requests if it returns 206 Partial Content
        resp.headers()
            .get("content-range")
            .and_then(|val| val.to_str().ok())
            .and_then(|val| {
                // Parse content-range header like "bytes 0-0/12345"
                val.split('/')
                    .next_back()
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

    // Prepare request with range header for resuming
    let mut request = client.get(url);
    if file_size > 0 {
        info!("Resuming {} download from {} bytes", file_type, file_size);
        request = request.header(RANGE, format!("bytes={file_size}-"));
    } else {
        info!("Starting {} download", file_type);
    }

    let response = request.send().await?;

    // Handle potential 416 Range Not Satisfiable error (file already complete)
    if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        info!("{} is already downloaded completely", file_type);
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

    // Set up progress bar now that we know we'll be downloading
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
            .progress_chars("#>-"),
    );

    pb.set_position(file_size);

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

    pb.finish_with_message(format!("{file_type} download complete"));
    info!(
        "{} download completed successfully: {}",
        file_type,
        file_path.display()
    );

    Ok(file_path)
}

/// Download multiple snapshot parts and concatenate them into a single file
pub async fn download_multipart_snapshot(
    urls: &[String],
    download_dir: &Path,
    final_filename: &str,
) -> Result<PathBuf> {
    let final_path = download_dir.join(final_filename);

    if final_path.exists() {
        info!(
            "Multi-part snapshot already exists: {}",
            final_path.display()
        );
        return Ok(final_path);
    }

    info!("Downloading {} snapshot parts", urls.len());

    // Download all parts
    let part_paths = download_all_parts(urls, download_dir).await?;

    // Concatenate parts into final file
    info!("Concatenating parts into final snapshot");
    concatenate_files(&part_paths, &final_path).await?;

    // Clean up part files
    cleanup_part_files(&part_paths);

    info!("Multi-part snapshot ready: {}", final_path.display());
    Ok(final_path)
}

/// Download all snapshot parts
async fn download_all_parts(urls: &[String], download_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut part_paths = Vec::with_capacity(urls.len());

    for (i, url) in urls.iter().enumerate() {
        let part_num = i + 1;
        let part_path = download_file(url, download_dir, &format!("part {part_num}")).await?;
        part_paths.push(part_path);
    }

    Ok(part_paths)
}

/// Clean up temporary part files
fn cleanup_part_files(part_paths: &[PathBuf]) {
    for path in part_paths {
        if let Err(e) = fs::remove_file(path) {
            warn!("Failed to remove part file {}: {}", path.display(), e);
        }
    }
}

/// Concatenate multiple files into a single output file
async fn concatenate_files(input_paths: &[PathBuf], output_path: &Path) -> Result<()> {
    let mut output_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_path)
        .with_context(|| format!("Failed to create output file: {}", output_path.display()))?;

    let pb = create_concatenation_progress_bar(input_paths.len());

    for (i, input_path) in input_paths.iter().enumerate() {
        debug!("Concatenating part {}: {}", i + 1, input_path.display());

        let mut input_file = fs::File::open(input_path)
            .with_context(|| format!("Failed to open part file: {}", input_path.display()))?;

        std::io::copy(&mut input_file, &mut output_file)
            .with_context(|| format!("Failed to copy part {} to output", i + 1))?;

        pb.set_position((i + 1) as u64);
    }

    pb.finish_with_message("Parts concatenated successfully");
    Ok(())
}

/// Create a progress bar for file concatenation
fn create_concatenation_progress_bar(total_parts: usize) -> ProgressBar {
    let pb = ProgressBar::new(total_parts as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} parts")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb
}
