use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::Config;

pub fn run_binary_init(binary_path: &Path, config: &Config) -> Result<()> {
    info!("Initializing binary...");

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
