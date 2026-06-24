use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::download::ensure_secure_url;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadRetryConfig {
    /// Maximum number of retry attempts (default: 5)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Initial delay between retries in seconds (default: 1)
    #[serde(default = "default_initial_delay")]
    pub initial_delay_secs: u64,
    /// Maximum delay between retries in seconds (default: 300 = 5 minutes)
    #[serde(default = "default_max_delay")]
    pub max_delay_secs: u64,
    /// Exponential backoff multiplier (default: 2.0)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_retries() -> u32 {
    5
}
fn default_initial_delay() -> u64 {
    1
}
fn default_max_delay() -> u64 {
    300
}
fn default_backoff_multiplier() -> f64 {
    2.0
}

impl Default for DownloadRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay_secs: default_initial_delay(),
            max_delay_secs: default_max_delay(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

impl DownloadRetryConfig {
    /// Calculate delay for a given retry attempt
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let delay_secs =
            (self.initial_delay_secs as f64 * self.backoff_multiplier.powi(attempt as i32)) as u64;
        Duration::from_secs(delay_secs.min(self.max_delay_secs))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct S3Config {
    /// AWS region (e.g., "us-east-1")
    pub region: Option<String>,
}

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
    pub post_snapshot_download_command: Option<String>,
    #[serde(default)]
    pub post_snapshot_extract_command: Option<String>,
    #[serde(default)]
    pub pre_start_command: Option<String>,
    #[serde(default)]
    pub post_start_command: Option<String>,
    #[serde(default)]
    pub post_start_pattern: Option<String>,
    #[serde(default)]
    pub stop_after_post_start: bool,
    #[serde(default)]
    pub chain_home_dir: Option<String>,
    #[serde(default)]
    pub addrbook_url: Option<String>,
    #[serde(default)]
    pub snapshot_sha256: Option<String>,
    #[serde(default)]
    pub snapshot_part_sha256s: Vec<Option<String>>,
    #[serde(default)]
    pub binary_sha256: Option<String>,
    #[serde(default)]
    pub download_retry: DownloadRetryConfig,
    #[serde(default)]
    pub s3: Option<S3Config>,
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

        config.finalize_paths()?;

        // Set default retry configuration if not provided
        if config.download_retry.max_retries == 0 {
            config.download_retry = DownloadRetryConfig::default();
        }

        config.validate_urls()?;

        Ok(config)
    }

    /// Build a Config from a resolved profile snapshot.
    pub fn from_profile(
        profile: &crate::profile::Profile,
        snapshot: &crate::profile::ResolvedSnapshot,
    ) -> Result<Self> {
        let (snapshot_url, snapshot_urls, snapshot_filename) = if snapshot.is_multipart() {
            (
                String::new(),
                snapshot.part_urls.clone(),
                Some(snapshot.filename.clone()),
            )
        } else {
            (snapshot.download_url.clone(), Vec::new(), None)
        };

        let binary_url = profile.resolved_binary_url(snapshot)?;
        let binary_relative_path = profile.resolved_binary_relative_path();

        let mut config = Config {
            snapshot_url,
            snapshot_urls,
            snapshot_filename,
            binary_url,
            binary_relative_path,
            chain_id: profile.chain_id.to_string(),
            moniker: profile.moniker.to_string(),
            app_yaml: profile.app_yaml_value()?,
            config_yaml: profile.config_yaml_value()?,
            post_snapshot_download_command: None,
            post_snapshot_extract_command: None,
            pre_start_command: None,
            post_start_command: None,
            post_start_pattern: None,
            stop_after_post_start: false,
            chain_home_dir: None,
            addrbook_url: None,
            snapshot_sha256: snapshot.sha256.clone(),
            snapshot_part_sha256s: snapshot.part_sha256s.clone(),
            binary_sha256: profile.binary_sha256.clone(),
            download_retry: DownloadRetryConfig::default(),
            s3: profile.s3.clone(),
            base_dir: PathBuf::new(),
            downloads_dir: PathBuf::new(),
            workspace_dir: PathBuf::new(),
            home_dir: PathBuf::new(),
        };

        config.finalize_paths()?;
        config.validate_urls()?;
        Ok(config)
    }

    /// Reject insecure URL schemes on configured download targets.
    fn validate_urls(&self) -> Result<()> {
        if !self.snapshot_url.is_empty() {
            ensure_secure_url(&self.snapshot_url, "snapshot_url")?;
        }
        for (i, url) in self.snapshot_urls.iter().enumerate() {
            ensure_secure_url(url, &format!("snapshot_urls[{i}]"))?;
        }
        ensure_secure_url(&self.binary_url, "binary_url")?;
        if let Some(url) = &self.addrbook_url {
            ensure_secure_url(url, "addrbook_url")?;
        }
        Ok(())
    }

    /// Resolve the workspace directory layout from the user's home directory.
    fn finalize_paths(&mut self) -> Result<()> {
        let user_home_dir = dirs::home_dir().context("Failed to determine user home directory")?;

        self.base_dir = user_home_dir.join(".snapshot-downloader");
        self.downloads_dir = self.base_dir.join("downloads");
        self.workspace_dir = self.base_dir.join("workspace");
        self.home_dir = match self.chain_home_dir.as_ref() {
            Some(custom_home) => PathBuf::from(custom_home),
            None => self.workspace_dir.join("home"),
        };

        Ok(())
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
