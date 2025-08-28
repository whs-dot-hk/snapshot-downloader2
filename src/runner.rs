use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::process::Stdio;
use tokio::sync::oneshot;
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

pub fn run_binary_start(
    config: &Config,
) -> Result<(std::process::Child, Option<oneshot::Receiver<()>>)> {
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

    // Get the post start command and pattern from config
    let post_start_command = config.post_start_command.clone();
    let post_start_pattern = config
        .post_start_pattern
        .clone()
        .unwrap_or_else(|| "committed state".to_string());
    let stop_after_post_start = config.stop_after_post_start;

    // Channel to signal when post start pattern is detected and we should stop
    let (shutdown_tx, shutdown_rx) = if stop_after_post_start {
        let (tx, rx) = oneshot::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    if let Some(stdout) = child.stdout.take() {
        let stdout_reader = BufReader::new(stdout);
        let post_start_cmd = post_start_command.clone();
        let pattern = post_start_pattern.clone();
        let mut shutdown_sender = shutdown_tx;
        let mut pattern_detected = false;

        std::thread::spawn(move || {
            for line in stdout_reader.lines().map_while(Result::ok) {
                println!("[STDOUT] {line}");

                // Check for post-start pattern detection (only once)
                if !pattern_detected && line.contains(&pattern) {
                    pattern_detected = true;
                    info!("Detected pattern '{}' in stdout output", pattern);

                    // Execute post start command if configured
                    let command_success = if let Some(ref cmd) = post_start_cmd {
                        execute_post_start_command(cmd).is_ok()
                    } else {
                        info!("No post start command configured, proceeding to shutdown");
                        true
                    };

                    // Always shutdown - whether command succeeded or failed
                    if command_success {
                        info!("Post-start command succeeded. Shutting down binary process.");
                    } else {
                        warn!("Post-start command failed. Shutting down binary process.");
                    }

                    if let Some(tx) = shutdown_sender.take() {
                        let _ = tx.send(());
                    }
                }
            }
        });
    }

    // Stream stderr (no pattern detection, just logging)
    if let Some(stderr) = child.stderr.take() {
        let stderr_reader = BufReader::new(stderr);

        std::thread::spawn(move || {
            for line in stderr_reader.lines().map_while(Result::ok) {
                eprintln!("[STDERR] {line}");
            }
        });
    }

    // Return the child process handle and optional shutdown receiver
    Ok((child, shutdown_rx))
}

/// Execute the post start command
pub fn execute_post_start_command(command: &str) -> Result<()> {
    info!("Executing post-start command: {}", command);

    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute post-start command")?;

    let mut handles = Vec::new();

    // Stream stdout in real-time
    if let Some(stdout) = child.stdout.take() {
        let stdout_reader = BufReader::new(stdout);
        let handle = std::thread::spawn(move || {
            for line in stdout_reader.lines().map_while(Result::ok) {
                info!("[Post-start stdout] {}", line);
            }
        });
        handles.push(handle);
    }

    // Stream stderr in real-time
    if let Some(stderr) = child.stderr.take() {
        let stderr_reader = BufReader::new(stderr);
        let handle = std::thread::spawn(move || {
            for line in stderr_reader.lines().map_while(Result::ok) {
                warn!("[Post-start stderr] {}", line);
            }
        });
        handles.push(handle);
    }

    let status = child
        .wait()
        .context("Failed to wait for post-start command")?;

    for handle in handles {
        let _ = handle.join();
    }

    if status.success() {
        info!("Post-start command executed successfully");
        Ok(())
    } else {
        let exit_code = status.code().unwrap_or(-1);
        warn!("Post-start command failed with exit code: {}", exit_code);
        Err(anyhow::anyhow!(
            "Post-start command failed with exit code: {}",
            exit_code
        ))
    }
}
