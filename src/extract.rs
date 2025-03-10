use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use lz4::Decoder;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use tar::Archive;
use tracing::{debug, info, warn};

pub fn extract_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
    info!("Extracting archive: {:?}", archive_path);

    fs::create_dir_all(target_dir)?;

    if let Some(extension) = archive_path.extension() {
        match extension.to_str() {
            Some("gz") | Some("tgz") => {
                extract_tar_gz(archive_path, target_dir)?;
                Ok(())
            }
            Some("lz4") => {
                extract_tar_lz4(archive_path, target_dir)?;
                Ok(())
            }
            _ => {
                warn!("Unsupported archive format: {:?}", extension);
                Err(anyhow::anyhow!(
                    "Unsupported archive format. Only tar.gz and tar.lz4 are supported."
                ))
            }
        }
    } else {
        warn!("Archive file has no extension: {:?}", archive_path);
        Err(anyhow::anyhow!(
            "Archive file has no extension, cannot determine format"
        ))
    }
}

pub fn extract_binary(binary_path: &Path, workspace_dir: &Path) -> Result<()> {
    info!("Extracting binary...");
    debug!("Binary extraction target directory: {:?}", workspace_dir);
    extract_archive(binary_path, workspace_dir)
}

pub fn extract_snapshot(
    snapshot_path: &Path,
    home_dir: &Path,
    post_command: Option<&str>,
) -> Result<()> {
    info!("Extracting snapshot...");
    debug!("Snapshot extraction target directory: {:?}", home_dir);
    extract_archive(snapshot_path, home_dir)?;

    if let Some(cmd) = post_command {
        execute_post_snapshot_command(cmd)?;
    }

    Ok(())
}

fn execute_post_snapshot_command(command: &str) -> Result<()> {
    info!("Executing post-snapshot command: {}", command);

    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute post-snapshot command")?;

    // Stream stdout in real-time
    if let Some(stdout) = child.stdout.take() {
        let stdout_reader = BufReader::new(stdout);
        std::thread::spawn(move || {
            for line in stdout_reader.lines().map_while(Result::ok) {
                info!("[Command stdout] {}", line);
            }
        });
    }

    // Stream stderr in real-time
    if let Some(stderr) = child.stderr.take() {
        let stderr_reader = BufReader::new(stderr);
        std::thread::spawn(move || {
            for line in stderr_reader.lines().map_while(Result::ok) {
                warn!("[Command stderr] {}", line);
            }
        });
    }

    // Wait for the process to complete
    let status = child
        .wait()
        .context("Failed to wait for post-snapshot command")?;

    if status.success() {
        info!("Post-snapshot command executed successfully");
        Ok(())
    } else {
        let exit_code = status.code().unwrap_or(-1);
        warn!("Post-snapshot command failed with exit code: {}", exit_code);
        Err(anyhow::anyhow!(
            "Post-snapshot command failed with exit code: {}",
            exit_code
        ))
    }
}

fn extract_tar_gz(archive_path: &Path, target_dir: &Path) -> Result<()> {
    info!("Extracting tar.gz archive...");
    let file = File::open(archive_path)?;
    let tar = GzDecoder::new(file);
    let mut archive = Archive::new(tar);
    archive.unpack(target_dir)?;
    Ok(())
}

fn extract_tar_lz4(archive_path: &Path, target_dir: &Path) -> Result<()> {
    info!("Extracting tar.lz4 archive...");
    let file = File::open(archive_path)?;
    let decoder = Decoder::new(file)?;
    let mut archive = Archive::new(decoder);
    archive.unpack(target_dir)?;
    Ok(())
}
