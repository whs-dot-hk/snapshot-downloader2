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
            // Multi-part - derive base filename from first part
            let first_filename = urls[0]
                .split('/')
                .next_back()
                .context("Failed to determine filename from snapshot URL")?;

            // Remove common part patterns and extensions
            let base_name = Self::normalize_multipart_filename(first_filename)?;
            Ok(format!("{base_name}.tar.gz"))
        }
    }

    /// Normalize a multi-part filename by removing part indicators
    fn normalize_multipart_filename(filename: &str) -> Result<String> {
        use regex::Regex;

        // Remove common part patterns: .part001, .part1, .001, etc.
        let part_regex = Regex::new(r"\.part\d+|\.0+\d+")?;
        let without_parts = part_regex.replace_all(filename, "");

        // Remove extensions
        let result = without_parts
            .strip_suffix(".tar.gz")
            .or_else(|| without_parts.strip_suffix(".tar"))
            .unwrap_or(&without_parts)
            .to_string();

        Ok(result)
    }
}
