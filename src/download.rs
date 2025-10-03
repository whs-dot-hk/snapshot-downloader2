use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::{CONTENT_LENGTH, RANGE};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;
use tracing::{debug, error, info, trace, warn};

use crate::config::{DownloadRetryConfig, S3Config};

pub async fn download_file(
    url: &str,
    download_dir: &Path,
    file_type: &str,
    retry_config: &DownloadRetryConfig,
) -> Result<PathBuf> {
    for attempt in 0..=retry_config.max_retries {
        match download_file_attempt(url, download_dir, file_type, attempt).await {
            Ok(path) => return Ok(path),
            Err(e) if attempt == retry_config.max_retries => {
                error!("Final attempt failed for {} download: {}", file_type, e);
                return Err(e);
            }
            Err(e) => {
                let delay = retry_config.calculate_delay(attempt);
                warn!(
                    "Attempt {} failed for {} download: {}. Retrying in {:?}...",
                    attempt + 1,
                    file_type,
                    e,
                    delay
                );
                sleep(delay).await;
            }
        }
    }

    unreachable!("Loop should have returned or errored")
}

async fn download_file_attempt(
    url: &str,
    download_dir: &Path,
    file_type: &str,
    attempt: u32,
) -> Result<PathBuf> {
    let client = reqwest::Client::builder()
        .build()
        .context("Failed to create HTTP client")?;

    // Create filename from URL
    let file_name = url
        .split('/')
        .next_back()
        .context("Failed to determine filename from URL")?;

    let file_path = download_dir.join(file_name);

    if attempt == 0 {
        debug!("Download path set to: {:?}", file_path);
    } else {
        debug!("Retry attempt {} for: {:?}", attempt + 1, file_path);
    }

    // Check if file already exists (for resuming)
    let file_size = if file_path.exists() {
        let size = file_path.metadata()?.len();
        if attempt == 0 {
            debug!("Existing file found with size: {} bytes", size);
        }
        size
    } else {
        if attempt == 0 {
            debug!("No existing file found, starting fresh download");
        }
        0
    };

    // Get total file size by requesting just the first byte
    trace!(
        "Requesting file metadata from server (attempt {})",
        attempt + 1
    );
    let resp = client
        .get(url)
        .header(RANGE, "bytes=0-0")
        .send()
        .await
        .context("Failed to get file metadata")?;

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

    if attempt == 0 {
        debug!("Total file size: {} bytes", total_size);
    }

    // If file is already complete, return early
    if file_size == total_size && total_size > 0 {
        info!("{} is already downloaded completely", file_type);
        return Ok(file_path);
    }

    // Prepare request with range header for resuming
    let mut request = client.get(url);
    if file_size > 0 {
        if attempt == 0 {
            info!("Resuming {} download from {} bytes", file_type, file_size);
        }
        request = request.header(RANGE, format!("bytes={file_size}-"));
    } else if attempt == 0 {
        info!("Starting {} download", file_type);
    }

    let response = request
        .send()
        .await
        .context("Failed to start download request")?;

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

    // Convert HTTP response to AsyncRead and use unified download logic
    let reader = tokio_util::io::StreamReader::new(
        response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other)),
    );

    download_async_read_to_file(
        reader, &file_path, file_size, total_size, attempt, file_type,
    )
    .await?;

    Ok(file_path)
}

/// Download multiple snapshot parts and concatenate them into a single file
pub async fn download_multipart_snapshot(
    urls: &[String],
    download_dir: &Path,
    final_filename: &str,
    retry_config: &DownloadRetryConfig,
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
    let part_paths = download_all_parts(urls, download_dir, retry_config).await?;

    // Concatenate parts into final file
    info!("Concatenating parts into final snapshot");
    concatenate_files(&part_paths, &final_path).await?;

    // Clean up part files
    cleanup_part_files(&part_paths);

    info!("Multi-part snapshot ready: {}", final_path.display());
    Ok(final_path)
}

/// Download all snapshot parts
async fn download_all_parts(
    urls: &[String],
    download_dir: &Path,
    retry_config: &DownloadRetryConfig,
) -> Result<Vec<PathBuf>> {
    let mut part_paths = Vec::with_capacity(urls.len());

    for (i, url) in urls.iter().enumerate() {
        let part_num = i + 1;
        let part_path =
            download_file(url, download_dir, &format!("part {part_num}"), retry_config).await?;
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

    let pb = create_progress_bar(
        input_paths.len() as u64,
        "[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} parts",
    )?;

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

/// Create a progress bar with the given template
fn create_progress_bar(total: u64, template: &str) -> Result<ProgressBar> {
    let pb = ProgressBar::new(total);

    let style = ProgressStyle::default_bar()
        .template(template)?
        .progress_chars("#>-");

    pb.set_style(style);
    Ok(pb)
}

/// Create a progress bar for a specific attempt (handles retry formatting)
fn create_progress_bar_for_attempt(total: u64, attempt: u32) -> Result<ProgressBar> {
    if attempt == 0 {
        create_progress_bar(
            total,
            "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
    } else {
        create_progress_bar(
            total,
            &format!("[Retry {}] [{{elapsed_precise}}] [{{bar:40.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{eta}})", attempt + 1),
        )
    }
}

/// Helper to write data to file and update progress
async fn write_chunk_with_progress(
    file: &mut tokio::fs::File,
    chunk: &[u8],
    downloaded: &mut u64,
    total_size: u64,
    pb: &ProgressBar,
    attempt: u32,
) -> Result<()> {
    file.write_all(chunk)
        .await
        .context("Failed to write bytes to file")?;

    *downloaded += chunk.len() as u64;
    pb.set_position(*downloaded);

    // Log progress at reasonable intervals
    if total_size > 0 && *downloaded % (total_size / 10).max(1) < (chunk.len() as u64) {
        trace!(
            "Download progress: {}/{} bytes (attempt {})",
            *downloaded,
            total_size,
            attempt + 1
        );
    }
    Ok(())
}

/// Finish download and log completion
fn finish_download(pb: ProgressBar, file_type: &str, file_path: &Path) {
    pb.finish_with_message(format!("{} download complete", file_type));
    info!(
        "{} download completed successfully: {}",
        file_type,
        file_path.display()
    );
}

/// Unified download logic using AsyncRead trait - works for both HTTP and S3
async fn download_async_read_to_file<R>(
    mut reader: R,
    file_path: &Path,
    existing_size: u64,
    total_size: u64,
    attempt: u32,
    file_type: &str,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // Set up progress bar
    let pb = create_progress_bar_for_attempt(total_size, attempt)?;
    pb.set_position(existing_size);

    // Open file for writing
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(existing_size > 0)
        .open(file_path)
        .await
        .context("Failed to open file for writing")?;

    let mut downloaded = existing_size;
    let mut buffer = vec![0u8; 256 * 1024]; // 256KB buffer for better performance
    trace!("Beginning download (attempt {})", attempt + 1);

    loop {
        let bytes_read = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer)
            .await
            .context("Failed to read from stream")?;

        if bytes_read == 0 {
            break; // EOF
        }

        write_chunk_with_progress(
            &mut file,
            &buffer[..bytes_read],
            &mut downloaded,
            total_size,
            &pb,
            attempt,
        )
        .await?;
    }

    file.flush().await.context("Failed to flush file")?;
    drop(file);

    finish_download(pb, file_type, file_path);
    Ok(())
}

/// Check existing file size and log appropriately
fn check_existing_file(file_path: &Path, attempt: u32) -> Result<u64> {
    let existing_size = if file_path.exists() {
        let size = file_path.metadata()?.len();
        if attempt == 0 {
            debug!("Existing file found with size: {} bytes", size);
        }
        size
    } else {
        if attempt == 0 {
            debug!("No existing file found, starting fresh download");
        }
        0
    };
    Ok(existing_size)
}

/// Parse S3 URL into bucket and key
/// Supported formats: s3://bucket/key or s3://bucket/path/to/key
fn parse_s3_url(url: &str) -> Result<(String, String)> {
    if !url.starts_with("s3://") {
        return Err(anyhow::anyhow!("Invalid S3 URL format: {}", url));
    }

    let path = &url[5..]; // Remove "s3://" prefix
    let parts: Vec<&str> = path.splitn(2, '/').collect();

    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid S3 URL format. Expected s3://bucket/key, got: {}",
            url
        ));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Check if a URL is an S3 URL
pub fn is_s3_url(url: &str) -> bool {
    url.starts_with("s3://")
}

/// Create an S3 client from configuration
/// Uses AWS default credentials chain (environment variables, AWS config files, IAM roles, etc.)
async fn create_s3_client(s3_config: Option<&S3Config>) -> Result<S3Client> {
    let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

    if let Some(s3_cfg) = s3_config {
        // Set region if provided
        if let Some(region) = &s3_cfg.region {
            config_loader = config_loader.region(aws_config::Region::new(region.clone()));
        }
    }

    let config = config_loader.load().await;
    Ok(S3Client::new(&config))
}

/// Download a file from S3
pub async fn download_s3_file(
    url: &str,
    download_dir: &Path,
    file_type: &str,
    retry_config: &DownloadRetryConfig,
    s3_config: Option<&S3Config>,
) -> Result<PathBuf> {
    for attempt in 0..=retry_config.max_retries {
        match download_s3_file_attempt(url, download_dir, file_type, attempt, s3_config).await {
            Ok(path) => return Ok(path),
            Err(e) if attempt == retry_config.max_retries => {
                error!("Final attempt failed for {} S3 download: {}", file_type, e);
                return Err(e);
            }
            Err(e) => {
                let delay = retry_config.calculate_delay(attempt);
                warn!(
                    "Attempt {} failed for {} S3 download: {}. Retrying in {:?}...",
                    attempt + 1,
                    file_type,
                    e,
                    delay
                );
                sleep(delay).await;
            }
        }
    }

    unreachable!("Loop should have returned or errored")
}

async fn download_s3_file_attempt(
    url: &str,
    download_dir: &Path,
    file_type: &str,
    attempt: u32,
    s3_config: Option<&S3Config>,
) -> Result<PathBuf> {
    // Parse S3 URL
    let (bucket, key) = parse_s3_url(url)?;

    if attempt == 0 {
        info!(
            "Downloading {} from S3: bucket={}, key={}",
            file_type, bucket, key
        );
    } else {
        debug!("Retry attempt {} for S3 download: {}", attempt + 1, url);
    }

    // Create S3 client
    let client = create_s3_client(s3_config).await?;

    // Extract filename from key
    let file_name = key
        .split('/')
        .next_back()
        .context("Failed to determine filename from S3 key")?;

    let file_path = download_dir.join(file_name);

    if attempt == 0 {
        debug!("Download path set to: {:?}", file_path);
    }

    // Check if file already exists
    let existing_size = check_existing_file(&file_path, attempt)?;

    // Get object metadata to check size
    let head_output = client
        .head_object()
        .bucket(&bucket)
        .key(&key)
        .send()
        .await
        .context("Failed to get S3 object metadata")?;

    let total_size = head_output.content_length().unwrap_or(0) as u64;

    if attempt == 0 {
        debug!("Total file size: {} bytes", total_size);
    }

    // If file is already complete, return early
    if existing_size == total_size && total_size > 0 {
        info!("{} is already downloaded completely", file_type);
        return Ok(file_path);
    }

    // Download the object
    let get_output = if existing_size > 0 && existing_size < total_size {
        if attempt == 0 {
            info!(
                "Resuming {} download from {} bytes",
                file_type, existing_size
            );
        }
        // Resume download using range
        client
            .get_object()
            .bucket(&bucket)
            .key(&key)
            .range(format!("bytes={}-", existing_size))
            .send()
            .await
            .context("Failed to start S3 download with range")?
    } else {
        if attempt == 0 {
            info!("Starting {} download from S3", file_type);
        }
        client
            .get_object()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
            .context("Failed to start S3 download")?
    };

    // Convert S3 ByteStream to AsyncRead and use unified download logic
    let reader = get_output.body.into_async_read();

    download_async_read_to_file(
        reader,
        &file_path,
        existing_size,
        total_size,
        attempt,
        file_type,
    )
    .await?;

    Ok(file_path)
}
