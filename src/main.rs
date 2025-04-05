use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::oneshot;
use tracing::{info, warn};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Skip downloading the snapshot (use existing snapshot file)
    #[arg(long)]
    skip_download_snapshot: bool,

    /// Skip extracting the snapshot
    #[arg(long)]
    skip_extract_snapshot: bool,

    /// Skip downloading and extracting the binary
    #[arg(long)]
    skip_binary_download: bool,

    /// Skip downloading the address book
    #[arg(long)]
    skip_addrbook_download: bool,
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

    // Handle binary download and extraction
    if !args.skip_binary_download {
        info!("Downloading and extracting binary...");
        // Download binary
        let binary_path =
            download::download_file(&config.binary_url, &config.downloads_dir, "binary")
                .await
                .context("Failed to download binary")?;

        // Extract binary
        extract::extract_binary(
            &binary_path,
            &config.workspace_dir,
            &config.binary_relative_path,
        )
        .context("Failed to extract binary")?;
        info!("Binary download and extraction complete.");
    } else {
        info!("Skipping binary download and extraction");
    }

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
        let toml_modifier = TomlModifier::new(&config.home_dir);
        toml_modifier
            .apply_config_changes(config.app_yaml.as_ref(), config.config_yaml.as_ref())
            .context("Failed to apply TOML configuration changes")?;
    }

    // Download addrbook if configured
    if let Some(addrbook_url) = &config.addrbook_url {
        if args.skip_addrbook_download {
            info!("Skipping address book download");
        } else {
            info!("Downloading addrbook from {}", addrbook_url);
            let downloaded_addrbook_path =
                download::download_file(addrbook_url, &config.downloads_dir, "addrbook")
                    .await
                    .context("Failed to download addrbook")?;

            let target_addrbook_dir = config.home_dir.join("config");
            let target_addrbook_path = target_addrbook_dir.join("addrbook.json"); // Assuming standard name

            // Ensure target directory exists
            tokio::fs::create_dir_all(&target_addrbook_dir)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create directory: {}",
                        target_addrbook_dir.display()
                    )
                })?;

            // Copy the downloaded file
            tokio::fs::copy(&downloaded_addrbook_path, &target_addrbook_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to copy addrbook from {} to {}",
                        downloaded_addrbook_path.display(),
                        target_addrbook_path.display()
                    )
                })?;

            // Remove the original downloaded file
            tokio::fs::remove_file(&downloaded_addrbook_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to remove original addrbook file {}",
                        downloaded_addrbook_path.display()
                    )
                })?;

            info!(
                "Addrbook downloaded and placed at {}", // Changed "moved to" -> "placed at" for clarity
                target_addrbook_path.display()
            );
        }
    }

    // Start the binary and get the process handle
    let mut binary_process = runner::run_binary_start(&config).context("Failed to start binary")?;

    // Store the process ID for later use
    let process_id = binary_process.id();

    // Set up channels to communicate between tasks
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();

    // Spawn a task to handle termination signals in a cross-platform way
    let signal_task = tokio::spawn(async move {
        // Wait for ctrl-c signal
        match tokio::signal::ctrl_c().await {
            Ok(_) => {
                info!("Received Ctrl+C, initiating graceful shutdown...");
            }
            Err(err) => {
                warn!("Unable to listen for shutdown signal: {}", err);
                return;
            }
        }

        // Signal the main task that we should shut down
        let _ = shutdown_tx.send(());
    });

    // Create a separate task that just waits for the process to exit
    // This avoids ownership issues with binary_process
    let process_wait_task = tokio::task::spawn_blocking(move || {
        let result = binary_process.wait();
        let _ = exit_tx.send(result); // Send the result back to the main task
        binary_process // Return ownership of the process back
    });

    // Block the main thread until we receive a shutdown signal OR the process exits on its own
    tokio::select! {
        _ = shutdown_rx => {
            info!("Shutdown signal received, terminating process {}", process_id);
            // Abort the waiting task to get the process handle back
            process_wait_task.abort();

            // Try to get the process handle back from the aborted task
            match process_wait_task.await {
                Ok(binary_process) => {
                    // Call our graceful termination function
                    if let Err(e) = runner::terminate_process(binary_process) {
                        warn!("Error during graceful shutdown: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Could not get binary process handle back for termination: {}", e);
                }
            }
        }
        exit_status = exit_rx => {
            match exit_status {
                Ok(Ok(status)) => {
                    info!("Binary process exited with status: {:?}", status);
                }
                Ok(Err(e)) => {
                    warn!("Error waiting for binary process: {}", e);
                }
                Err(_) => {
                    warn!("Failed to receive process exit status");
                }
            }
        }
    }

    // Clean up the signal task
    signal_task.abort();

    info!("Graceful shutdown complete");
    Ok(())
}
