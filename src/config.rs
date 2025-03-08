use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub snapshot_url: String,
    pub binary_url: String,
    pub binary_relative_path: String,
    pub chain_id: String,
    pub moniker: String,

    #[serde(skip)]
    pub base_dir: PathBuf,
    #[serde(skip)]
    pub downloads_dir: PathBuf,
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub home_dir: PathBuf,
}

pub fn load_config() -> Result<Config> {
    info!("Loading configuration");

    // Find config file
    let config_path = Path::new("config.yaml");
    debug!("Looking for config file at: {:?}", config_path);

    // Read and parse config file
    trace!("Reading config file contents");
    let config_content =
        fs::read_to_string(config_path).context("Failed to read config.yaml file")?;

    trace!("Parsing config file as YAML");
    let mut config: Config =
        serde_yaml::from_str(&config_content).context("Failed to parse config.yaml")?;

    // Set up derived paths
    trace!("Setting up derived paths");
    let home_dir = dirs::home_dir().context("Failed to determine home directory")?;
    debug!("Home directory: {:?}", home_dir);

    config.base_dir = home_dir.join(".snapshot-downloader");
    config.downloads_dir = config.base_dir.join("downloads");
    config.workspace_dir = config.base_dir.join("workspace");
    config.home_dir = config.workspace_dir.join("home");

    debug!("Base directory: {:?}", config.base_dir);
    debug!("Downloads directory: {:?}", config.downloads_dir);
    debug!("Workspace directory: {:?}", config.workspace_dir);
    debug!("Home directory: {:?}", config.home_dir);

    info!("Configuration loaded successfully");
    Ok(config)
}
