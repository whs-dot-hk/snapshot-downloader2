use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
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
    skip_download_addrbook: bool,

    /// Skip execute the binary
    #[arg(long)]
    skip_execute_binary: bool
}

mod config;
mod download;
mod extract;
mod runner;
mod toml_modifier;
mod utils;

use config::Config;
use toml_modifier::TomlModifier;

/// Download snapshot (single file or multi-part)
async fn download_snapshot(config: &Config) -> Result<PathBuf> {
    let urls = config.get_snapshot_urls();
    if urls.is_empty() {
        return Err(anyhow::anyhow!("No snapshot URLs configured"));
    }

    if urls.len() == 1 {
        let url = &urls[0];
        if download::is_s3_url(url) {
            download::download_s3_file(
                url,
                &config.downloads_dir,
                "snapshot",
                &config.download_retry,
                config.s3.as_ref(),
            )
            .await
            .context("Failed to download snapshot from S3")
        } else {
            download::download_file(
                url,
                &config.downloads_dir,
                "snapshot",
                &config.download_retry,
            )
            .await
            .context("Failed to download snapshot")
        }
    } else {
        let filename = config.get_snapshot_filename()?;
        download::download_multipart_snapshot(
            &urls,
            &config.downloads_dir,
            &filename,
            &config.download_retry,
        )
        .await
        .context("Failed to download multi-part snapshot")
    }
}

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
        let binary_path = if download::is_s3_url(&config.binary_url) {
            download::download_s3_file(
                &config.binary_url,
                &config.downloads_dir,
                "binary",
                &config.download_retry,
                config.s3.as_ref(),
            )
            .await
            .context("Failed to download binary from S3")?
        } else {
            download::download_file(
                &config.binary_url,
                &config.downloads_dir,
                "binary",
                &config.download_retry,
            )
            .await
            .context("Failed to download binary")?
        };

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

    // Handle snapshot download
    let snapshot_path = if args.skip_download_snapshot {
        info!("Skipping snapshot download, using existing file");
        let filename = config.get_snapshot_filename()?;
        config.downloads_dir.join(filename)
    } else {
        let path = download_snapshot(&config).await?;

        // Execute post-snapshot-download command if configured
        if let Some(ref cmd) = config.post_snapshot_download_command {
            if let Err(e) = runner::execute_post_snapshot_download_command(cmd) {
                warn!(
                    "Post-snapshot-download command failed after snapshot download: {}",
                    e
                );
            }
        }

        path
    };

    // Extract snapshot and run post-snapshot command if configured
    if args.skip_extract_snapshot {
        info!("Skipping snapshot extraction");
    } else {
        extract::extract_snapshot(
            &snapshot_path,
            &config.home_dir,
            config.post_snapshot_extract_command.as_deref(),
        )
        .context("Failed to extract snapshot")?;
    }

    info!("Snapshot downloader completed successfully!");

    // Helper function to check if a YAML value is a non-empty mapping (valid for TOML modification)
    let is_valid_yaml_config = |yaml_opt: &Option<serde_yaml::Value>| -> bool {
        match yaml_opt {
            Some(serde_yaml::Value::Mapping(map)) => !map.is_empty(),
            _ => false,
        }
    };

    // Only apply TOML modifications if there are valid (non-empty mapping) configurations
    let should_modify_app = is_valid_yaml_config(&config.app_yaml);
    let should_modify_config = is_valid_yaml_config(&config.config_yaml);

    if should_modify_app || should_modify_config {
        info!("Applying configuration changes to TOML files");
        let toml_modifier = TomlModifier::new(&config.home_dir);
        toml_modifier
            .apply_config_changes(
                if should_modify_app {
                    config.app_yaml.as_ref()
                } else {
                    None
                },
                if should_modify_config {
                    config.config_yaml.as_ref()
                } else {
                    None
                },
            )
            .context("Failed to apply TOML configuration changes")?;
    }

    // Download addrbook if configured
    if let Some(addrbook_url) = &config.addrbook_url {
        if args.skip_download_addrbook {
            info!("Skipping address book download");
        } else {
            info!("Downloading addrbook from {}", addrbook_url);
            let downloaded_addrbook_path = if download::is_s3_url(addrbook_url) {
                download::download_s3_file(
                    addrbook_url,
                    &config.downloads_dir,
                    "addrbook",
                    &config.download_retry,
                    config.s3.as_ref(),
                )
                .await
                .context("Failed to download addrbook from S3")?
            } else {
                download::download_file(
                    addrbook_url,
                    &config.downloads_dir,
                    "addrbook",
                    &config.download_retry,
                )
                .await
                .context("Failed to download addrbook")?
            };

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

    if args.skip_execute_binary {
        info!("Skipping binary execution");
        return Ok(());
    }

    // Execute pre-start command if configured
    if let Some(ref cmd) = config.pre_start_command {
        if let Err(e) = runner::execute_pre_start_command(cmd) {
            warn!("Pre-start command failed before binary start: {}", e);
        }
    }

    // Start the binary and get the process handle
    let (binary_process, post_start_shutdown_rx) =
        runner::run_binary_start(&config).context("Failed to start binary")?;

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

    // Create a separate task that waits for the process to exit and holds the process handle
    let process_wait_task = tokio::task::spawn_blocking(move || {
        let mut binary_process = binary_process;
        let result = binary_process.wait();
        let _ = exit_tx.send(result);
        binary_process
    });

    // Block the main thread until we receive a shutdown signal, post start shutdown, OR the process exits on its own
    tokio::select! {
        _ = shutdown_rx => {
            info!("Shutdown signal received, terminating process {}", process_id);

            // Abort the waiting task to get the process handle back
            process_wait_task.abort();

            // Try to get the process handle back from the aborted task
            match process_wait_task.await {
                Ok(mut binary_process) => {
                    // Terminate the process directly using the process handle
                    info!("Attempting graceful termination of process {}", process_id);
                    match binary_process.kill() {
                        Ok(_) => {
                            info!("Successfully sent kill signal to process {}", process_id);
                            // Wait for the process to exit
                            match binary_process.wait() {
                                Ok(status) => {
                                    info!("Process exited with status: {:?}", status);
                                }
                                Err(e) => {
                                    warn!("Error waiting for process: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to kill process directly: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Could not get process handle back for termination: {}", e);
                }
            }
        }
        _ = async {
            if let Some(rx) = post_start_shutdown_rx {
                rx.await.ok();
            } else {
                // If no post start shutdown is configured, wait forever
                std::future::pending::<()>().await;
            }
        } => {
            info!("Post start command completed, terminating process {} and exiting program", process_id);
            std::process::exit(0);
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
