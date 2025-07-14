use anyhow::{Context, Result};
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

pub fn run_binary_start(config: &Config) -> Result<std::process::Child> {
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
    let command_str = format!("{binary_abs_path_str} start --home {home_abs_path_str}");

    info!("To start the node later, run the following command:");
    info!("{}", command_str);

    // Run the binary start command
    info!("Running binary start command");
    let mut child = Command::new(&binary_abs_path)
        .arg("start")
        .arg("--home")
        .arg(&home_abs_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn binary process")?;

    info!("Binary process started, streaming logs...");

    if let Some(stdout) = child.stdout.take() {
        use std::io::{BufRead, BufReader};
        let stdout_reader = BufReader::new(stdout);

        std::thread::spawn(move || {
            for line in stdout_reader.lines().map_while(Result::ok) {
                println!("[STDOUT] {line}");
            }
        });
    }

    // Stream stderr
    if let Some(stderr) = child.stderr.take() {
        use std::io::{BufRead, BufReader};
        let stderr_reader = BufReader::new(stderr);

        std::thread::spawn(move || {
            for line in stderr_reader.lines().map_while(Result::ok) {
                eprintln!("[STDERR] {line}");
            }
        });
    }

    // Return the child process handle instead of waiting for it to complete
    Ok(child)
}

/// Gracefully terminates the provided child process
pub fn terminate_process(mut child: std::process::Child) -> Result<()> {
    info!("Gracefully shutting down binary process...");

    // Send termination signal to the process
    if let Err(e) = child.kill() {
        warn!("Failed to send termination signal: {}", e);
        return Err(anyhow::anyhow!("Failed to terminate process: {}", e));
    }
    info!("Termination signal sent to process {}", child.id());

    // Wait for the process to exit
    info!("Waiting for process to exit...");
    match child.wait() {
        Ok(status) => {
            info!("Process exited with status: {:?}", status);
            Ok(())
        }
        Err(e) => {
            warn!("Error waiting for process to exit: {}", e);
            Err(anyhow::anyhow!("Failed to wait for process: {}", e))
        }
    }
}
