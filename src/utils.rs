use anyhow::Result;
use std::fs;

use crate::config::Config;

pub fn create_directories(config: &Config) -> Result<()> {
    // Create base directory
    fs::create_dir_all(&config.base_dir)?;

    // Create downloads directory
    fs::create_dir_all(&config.downloads_dir)?;

    // Create workspace directory
    fs::create_dir_all(&config.workspace_dir)?;

    // Create home directory
    fs::create_dir_all(&config.home_dir)?;

    Ok(())
}
