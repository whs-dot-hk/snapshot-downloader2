use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::Config;

pub fn genesis_exists(config: &Config) -> bool {
    let genesis_path = config.home_dir.join("config").join("genesis.json");
    debug!("Checking for genesis file at: {:?}", genesis_path);
    genesis_path.exists()
}

pub fn run_binary_init(config: &Config) -> Result<()> {
    if genesis_exists(config) {
        info!("Genesis file already exists, skipping initialization");
        return Ok(());
    }

    info!("Initializing binary...");

    let binary_path = config.workspace_dir.join(&config.binary_relative_path);

    // Get absolute paths
    let binary_abs_path = binary_path.canonicalize()?;
    let home_abs_path = config.home_dir.canonicalize()?;

    debug!("Binary path: {:?}", binary_abs_path);
    debug!("Home path: {:?}", home_abs_path);

    // Run the binary init command
    info!(
        "Running binary init command with chain-id: {} and moniker: {}",
        config.chain_id, config.moniker
    );
    let output = Command::new(&binary_abs_path)
        .arg("init")
        .arg(&config.moniker)
        .arg("--chain-id")
        .arg(&config.chain_id)
        .arg("--home")
        .arg(&home_abs_path)
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        warn!("Binary init failed: {}", error);
        return Err(anyhow::anyhow!("Binary init failed: {}", error));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    debug!("Binary init output: {}", stdout);
    info!("Binary initialized successfully");

    Ok(())
}

pub fn run_binary_start(config: &Config) -> Result<()> {
    info!("Starting binary...");

    let binary_path = config.workspace_dir.join(&config.binary_relative_path);

    // Get absolute paths
    let binary_abs_path = binary_path.canonicalize()?;
    let home_abs_path = config.home_dir.canonicalize()?;

    debug!("Binary path: {:?}", binary_abs_path);
    debug!("Home path: {:?}", home_abs_path);

    // Print the command for the user to run later
    let binary_abs_path_str = binary_abs_path.to_string_lossy();
    let home_abs_path_str = home_abs_path.to_string_lossy();
    let command_str = format!("{} start --home {}", binary_abs_path_str, home_abs_path_str);

    info!("To start the node later, run the following command:");
    info!("{}", command_str);

    // Run the binary start command
    info!("Running binary start command");
    let output = Command::new(&binary_abs_path)
        .arg("start")
        .arg("--home")
        .arg(&home_abs_path)
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        warn!("Binary start failed: {}", error);
        return Err(anyhow::anyhow!("Binary start failed: {}", error));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    debug!("Binary start output: {}", stdout);
    info!("Binary started successfully");

    Ok(())
}
