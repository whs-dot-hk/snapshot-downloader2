use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process;
use tracing::{error, info};

mod config;
mod download;
mod extract;
mod runner;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = config::load_config().context("Failed to load configuration")?;

    // Create required directories
    utils::create_directories(&config).context("Failed to create required directories")?;

    // Download binary
    let binary_path = download::download_file(&config.binary_url, &config.downloads_dir, "binary")
        .await
        .context("Failed to download binary")?;

    // Extract binary
    let binary_extract_path = extract::extract_binary(&binary_path, &config.workspace_dir)
        .context("Failed to extract binary")?;

    // Run binary init
    runner::run_binary_init(&config).context("Failed to initialize binary")?;

    // Download snapshot
    let snapshot_path =
        download::download_file(&config.snapshot_url, &config.downloads_dir, "snapshot")
            .await
            .context("Failed to download snapshot")?;

    // Extract snapshot
    extract::extract_snapshot(&snapshot_path, &config.home_dir)
        .context("Failed to extract snapshot")?;

    info!("Snapshot downloader completed successfully!");

    // Start the binary
    runner::run_binary_start(&config).context("Failed to start binary")?;

    Ok(())
}
