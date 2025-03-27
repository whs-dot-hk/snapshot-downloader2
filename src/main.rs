use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Skip downloading the snapshot (use existing snapshot file)
    #[arg(long)]
    skip_download_snapshot: bool,

    /// Skip extracting the snapshot
    #[arg(long)]
    skip_extract_snapshot: bool,
}

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
    // Parse command line arguments
    let args = Args::parse();

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

    // Handle snapshot
    let snapshot_path = if args.skip_download_snapshot {
        info!("Skipping snapshot download, using existing snapshot file");
        let snapshot_filename = config
            .snapshot_url
            .split('/')
            .next_back()
            .context("Failed to determine filename from snapshot URL")?;
        config.downloads_dir.join(snapshot_filename)
    } else {
        download::download_file(&config.snapshot_url, &config.downloads_dir, "snapshot")
            .await
            .context("Failed to download snapshot")?
    };

    // Extract snapshot and run post-snapshot command if configured
    if args.skip_extract_snapshot {
        info!("Skipping snapshot extraction");
    } else {
        extract::extract_snapshot(
            &snapshot_path,
            &config.home_dir,
            config.post_snapshot_command.as_deref(),
        )
        .context("Failed to extract snapshot")?;
    }

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
