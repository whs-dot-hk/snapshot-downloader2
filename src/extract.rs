use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use lz4::Decoder;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use tar::Archive;
use tracing::{debug, info, warn};
use zstd::stream::read::Decoder as ZstdDecoder;

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
            Some("zst") => {
                extract_tar_zst(archive_path, target_dir)?;
                Ok(())
            }
            _ => {
                warn!("Unsupported archive format: {:?}", extension);
                Err(anyhow::anyhow!(
                    "Unsupported archive format. Only tar.gz, tar.lz4, and tar.zst are supported."
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

pub fn extract_binary(
    binary_path: &Path,
    workspace_dir: &Path,
    binary_relative_path: &str,
) -> Result<()> {
    info!("Processing binary...");
    debug!("Binary target directory: {:?}", workspace_dir);
    debug!("Binary relative path: {}", binary_relative_path);

    // Check if the file has an archive extension
    if let Some(extension) = binary_path.extension() {
        match extension.to_str() {
            Some("gz") | Some("tgz") | Some("lz4") | Some("zst") => {
                // This is an archive, extract it
                debug!("File appears to be an archive, extracting...");
                return extract_archive(binary_path, workspace_dir);
            }
            _ => {
                // Not a known archive type, treat as standalone binary
                debug!(
                    "File does not have a known archive extension, treating as standalone binary"
                );
            }
        }
    }

    // If we get here, treat the file as a standalone binary that just needs to be made executable
    info!("File appears to be a standalone binary, making it executable...");

    // Create the full destination path based on binary_relative_path
    let dest_path = workspace_dir.join(binary_relative_path);

    // Create the parent directory structure if it doesn't exist
    if let Some(parent) = dest_path.parent() {
        debug!("Creating directory structure: {:?}", parent);
        fs::create_dir_all(parent)?;
    }

    // Copy the binary to the destination
    debug!("Copying binary to {:?}", dest_path);
    fs::copy(binary_path, &dest_path)?;

    // Make the file executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest_path, perms)?;
        debug!("Made binary executable (chmod 755)");
    }

    Ok(())
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

fn extract_tar_zst(archive_path: &Path, target_dir: &Path) -> Result<()> {
    info!("Extracting tar.zst archive...");
    let file = File::open(archive_path)?;
    let decoder = ZstdDecoder::new(file)?;
    let mut archive = Archive::new(decoder);
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
