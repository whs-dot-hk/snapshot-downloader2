use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub snapshot_url: String,
    #[serde(default)]
    pub snapshot_urls: Vec<String>,
    #[serde(default)]
    pub snapshot_filename: Option<String>,
    pub binary_url: String,
    pub binary_relative_path: String,
    pub chain_id: String,
    pub moniker: String,
    #[serde(default)]
    pub app_yaml: Option<YamlValue>,
    #[serde(default)]
    pub config_yaml: Option<YamlValue>,
    #[serde(default)]
    pub post_snapshot_command: Option<String>,
    #[serde(default)]
    pub post_start_command: Option<String>,
    #[serde(default)]
    pub post_start_pattern: Option<String>,
    #[serde(default)]
    pub chain_home_dir: Option<String>,
    #[serde(default)]
    pub addrbook_url: Option<String>,
    #[serde(skip)]
    pub base_dir: PathBuf,
    #[serde(skip)]
    pub downloads_dir: PathBuf,
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub home_dir: PathBuf,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;

        let mut config: Config =
            serde_yaml::from_str(&content).context("Failed to parse config YAML")?;

        // Validate configuration
        if !config.snapshot_urls.is_empty() && config.snapshot_filename.is_none() {
            return Err(anyhow::anyhow!(
                "snapshot_filename is required when using snapshot_urls (multipart snapshots)"
            ));
        }

        let user_home_dir = dirs::home_dir().context("Failed to determine user home directory")?;

        config.base_dir = user_home_dir.join(".snapshot-downloader");
        config.downloads_dir = config.base_dir.join("downloads");
        config.workspace_dir = config.base_dir.join("workspace");
        config.home_dir = match config.chain_home_dir.as_ref() {
            Some(custom_home) => PathBuf::from(custom_home),
            None => config.workspace_dir.join("home"),
        };

        Ok(config)
    }

    /// Get the list of snapshot URLs to download
    /// Returns the multi-part URLs if available, otherwise falls back to single URL
    pub fn get_snapshot_urls(&self) -> Vec<String> {
        if !self.snapshot_urls.is_empty() {
            self.snapshot_urls.clone()
        } else if !self.snapshot_url.is_empty() {
            vec![self.snapshot_url.clone()]
        } else {
            vec![]
        }
    }

    /// Get the final snapshot filename
    pub fn get_snapshot_filename(&self) -> Result<String> {
        let urls = self.get_snapshot_urls();
        if urls.is_empty() {
            return Err(anyhow::anyhow!("No snapshot URLs configured"));
        }

        if urls.len() == 1 {
            // Single file - use the original filename
            Ok(urls[0]
                .split('/')
                .next_back()
                .context("Failed to determine filename from snapshot URL")?
                .to_string())
        } else {
            // Multi-part - snapshot_filename should exist due to validation
            self.snapshot_filename.clone().context(
                "snapshot_filename is required when using snapshot_urls (multipart snapshots)",
            )
        }
    }
}
