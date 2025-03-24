use anyhow::{Context, Result};
use tracing::info;

mod config;
mod download;
mod extract;
mod runner;
mod toml_modifier;
mod utils;

use config::Config;
use toml_modifier::TomlModifier;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = Config::from_file("config.yaml").context("Failed to load configuration")?;

    // Create required directories
    utils::create_directories(&config).context("Failed to create required directories")?;

    // Download binary
    let binary_path = download::download_file(&config.binary_url, &config.downloads_dir, "binary")
        .await
        .context("Failed to download binary")?;

    // Extract binary
    extract::extract_binary(
        &binary_path,
        &config.workspace_dir,
        &config.binary_relative_path,
    )
    .context("Failed to extract binary")?;

    // Run binary init
    runner::run_binary_init(&config).context("Failed to initialize binary")?;

    // Download snapshot
    let snapshot_path =
        download::download_file(&config.snapshot_url, &config.downloads_dir, "snapshot")
            .await
            .context("Failed to download snapshot")?;

    // Extract snapshot and run post-snapshot command if configured
    extract::extract_snapshot(
        &snapshot_path,
        &config.home_dir,
        config.post_snapshot_command.as_deref(),
    )
    .context("Failed to extract snapshot")?;

    info!("Snapshot downloader completed successfully!");

    if config.app_yaml.as_ref().is_some() || config.config_yaml.as_ref().is_some() {
        info!("Applying configuration changes to TOML files");
        let toml_modifier = TomlModifier::new(&config.workspace_dir);
        toml_modifier
            .apply_config_changes(config.app_yaml.as_ref(), config.config_yaml.as_ref())
            .context("Failed to apply TOML configuration changes")?;
    }

    runner::run_binary_start(&config).context("Failed to start binary")?;

    Ok(())
}
