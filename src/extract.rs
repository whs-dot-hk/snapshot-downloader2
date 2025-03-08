use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use lz4::Decoder;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tar::Archive;
use tracing::{debug, info, trace, warn};

pub fn extract_archive(archive_path: &Path, target_dir: &Path) -> Result<PathBuf> {
    info!("Extracting archive: {:?}", archive_path);

    fs::create_dir_all(target_dir)?;

    if let Some(extension) = archive_path.extension() {
        match extension.to_str() {
            Some("gz") | Some("tgz") => {
                extract_tar_gz(archive_path, target_dir)?;
                Ok(target_dir.to_path_buf())
            }
            Some("lz4") => {
                extract_tar_lz4(archive_path, target_dir)?;
                Ok(target_dir.to_path_buf())
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

pub fn extract_binary(binary_path: &Path, workspace_dir: &Path) -> Result<PathBuf> {
    info!("Extracting binary...");
    debug!("Binary extraction target directory: {:?}", workspace_dir);
    extract_archive(binary_path, workspace_dir)
}

pub fn extract_snapshot(snapshot_path: &Path, home_dir: &Path) -> Result<()> {
    info!("Extracting snapshot...");
    debug!("Snapshot extraction target directory: {:?}", home_dir);
    extract_archive(snapshot_path, home_dir)?;
    Ok(())
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
