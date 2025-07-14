use anyhow::{Context, Result};
use serde_yaml::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};
use toml::value::Table;
use toml::Value as TomlValue;
use tracing::info;

pub struct TomlModifier {
    home_dir: PathBuf,
}

impl TomlModifier {
    /// Create a new TomlModifier with the given workspace directory
    pub fn new<P: AsRef<Path>>(home_dir: P) -> Self {
        Self {
            home_dir: home_dir.as_ref().to_path_buf(),
        }
    }

    /// Apply configuration changes to app.toml and config.toml based on YAML configuration
    pub fn apply_config_changes(
        &self,
        app_yaml: Option<&YamlValue>,
        config_yaml: Option<&YamlValue>,
    ) -> Result<()> {
        if let Some(app_config) = app_yaml {
            self.modify_app_toml(app_config)
                .context("Failed to modify app.toml")?;
        }

        if let Some(config_toml) = config_yaml {
            self.modify_config_toml(config_toml)
                .context("Failed to modify config.toml")?;
        }

        Ok(())
    }

    /// Modify app.toml with the provided YAML configuration
    fn modify_app_toml(&self, app_yaml: &YamlValue) -> Result<()> {
        let app_toml_path = self.home_dir.join("config/app.toml");
        self.modify_toml(app_toml_path, app_yaml, "app.toml")
    }

    /// Modify config.toml with the provided YAML configuration
    fn modify_config_toml(&self, config_yaml: &YamlValue) -> Result<()> {
        let config_toml_path = self.home_dir.join("config/config.toml");
        self.modify_toml(config_toml_path, config_yaml, "config.toml")
    }

    /// Generic method to modify a TOML file with the provided YAML configuration
    fn modify_toml(
        &self,
        toml_path: PathBuf,
        yaml_config: &YamlValue,
        file_name: &str,
    ) -> Result<()> {
        info!("Modifying {} at {}", file_name, toml_path.display());

        // Read existing TOML file
        let toml_content = fs::read_to_string(&toml_path).context(format!(
            "Failed to read {} at {}",
            file_name,
            toml_path.display()
        ))?;

        // Parse existing TOML
        let mut toml_value: TomlValue = toml::from_str(&toml_content)
            .context(format!("Failed to parse {file_name} content"))?;

        // Convert YAML to TOML-compatible structure and merge
        let yaml_as_toml = Self::yaml_to_toml(yaml_config)?;
        Self::merge_toml_values(&mut toml_value, &yaml_as_toml);

        // Write back to file
        let modified_toml = toml::to_string_pretty(&toml_value)
            .context(format!("Failed to serialize modified {file_name}"))?;

        fs::write(&toml_path, modified_toml).context(format!(
            "Failed to write modified {} to {}",
            file_name,
            toml_path.display()
        ))?;

        info!("Successfully modified {}", file_name);
        Ok(())
    }

    /// Convert YAML value to TOML value
    fn yaml_to_toml(yaml_value: &YamlValue) -> Result<TomlValue> {
        match yaml_value {
            YamlValue::Null => Ok(TomlValue::String("".to_string())),
            YamlValue::Bool(b) => Ok(TomlValue::Boolean(*b)),
            YamlValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(TomlValue::Integer(i))
                } else if let Some(f) = n.as_f64() {
                    Ok(TomlValue::Float(f))
                } else {
                    anyhow::bail!("Unsupported YAML number type")
                }
            }
            YamlValue::String(s) => Ok(TomlValue::String(s.to_string())),
            YamlValue::Sequence(seq) => {
                let mut toml_array = Vec::new();
                for item in seq {
                    toml_array.push(Self::yaml_to_toml(item)?);
                }
                Ok(TomlValue::Array(toml_array))
            }
            YamlValue::Mapping(map) => {
                let mut toml_table = Table::new();
                for (key, value) in map {
                    if let YamlValue::String(key_str) = key {
                        toml_table.insert(key_str.to_string(), Self::yaml_to_toml(value)?);
                    } else {
                        anyhow::bail!("YAML mapping key must be a string");
                    }
                }
                Ok(TomlValue::Table(toml_table))
            }
            YamlValue::Tagged(tagged) => {
                // For tagged values, we just use the value and ignore the tag
                Self::yaml_to_toml(&tagged.value)
            }
        }
    }

    /// Recursively merge TOML values, preserving existing structure
    fn merge_toml_values(target: &mut TomlValue, source: &TomlValue) {
        match (target, source) {
            (TomlValue::Table(target_table), TomlValue::Table(source_table)) => {
                for (key, source_value) in source_table {
                    match target_table.get_mut(key.as_str()) {
                        Some(target_value) => {
                            // Recursively merge if both are tables
                            Self::merge_toml_values(target_value, source_value);
                        }
                        None => {
                            // Insert new key-value pair
                            target_table.insert(key.to_string(), source_value.clone());
                        }
                    }
                }
            }
            (target, source) => {
                // For non-table values, replace the target with the source
                *target = source.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_merge_toml_values() {
        let mut target = TomlValue::Table({
            let mut t = Table::new();
            t.insert(
                "existing".to_string(),
                TomlValue::String("value".to_string()),
            );
            t.insert(
                "section".to_string(),
                TomlValue::Table({
                    let mut st = Table::new();
                    st.insert("key1".to_string(), TomlValue::Integer(1));
                    st
                }),
            );
            t
        });

        let source = TomlValue::Table({
            let mut t = Table::new();
            t.insert("new".to_string(), TomlValue::String("value".to_string()));
            t.insert(
                "section".to_string(),
                TomlValue::Table({
                    let mut st = Table::new();
                    st.insert("key2".to_string(), TomlValue::Integer(2));
                    st
                }),
            );
            t
        });

        let modifier = TomlModifier::new("/tmp");
        modifier.merge_toml_values(&mut target, source);

        if let TomlValue::Table(table) = target {
            assert_eq!(table.get("existing").unwrap().as_str().unwrap(), "value");
            assert_eq!(table.get("new").unwrap().as_str().unwrap(), "value");

            if let TomlValue::Table(section) = table.get("section").unwrap() {
                assert_eq!(section.get("key1").unwrap().as_integer().unwrap(), 1);
                assert_eq!(section.get("key2").unwrap().as_integer().unwrap(), 2);
            } else {
                panic!("Expected section to be a table");
            }
        } else {
            panic!("Expected target to be a table");
        }
    }

    #[test]
    fn test_modify_toml_files() -> Result<()> {
        // Create a temporary directory to simulate workspace
        let temp_dir = tempdir()?;
        let config_dir = temp_dir.path().join("home/config");
        fs::create_dir_all(&config_dir)?;

        // Create sample app.toml
        let app_toml_content = r#"
[api]
enable = false
swagger = false

[grpc]
enable = false

[state-sync]
snapshot-interval = 1000
"#;
        let app_toml_path = config_dir.join("app.toml");
        let mut file = File::create(&app_toml_path)?;
        file.write_all(app_toml_content.as_bytes())?;

        // Create sample config.toml
        let config_toml_content = r#"
[rpc]
laddr = "tcp://127.0.0.1:26657"

[p2p]
seeds = ""
"#;
        let config_toml_path = config_dir.join("config.toml");
        let mut file = File::create(&config_toml_path)?;
        file.write_all(config_toml_content.as_bytes())?;

        // Create YAML values
        let app_yaml: YamlValue = serde_yaml::from_str(
            r#"
api:
  enable: true
  swagger: true
grpc:
  enable: true
"#,
        )?;

        let config_yaml: YamlValue = serde_yaml::from_str(
            r#"
rpc:
  laddr: "tcp://0.0.0.0:26657"
p2p:
  seeds: "seed1.example.com:26656,seed2.example.com:26656"
"#,
        )?;

        // Apply modifications
        let modifier = TomlModifier::new(temp_dir.path());
        modifier.apply_config_changes(Some(app_yaml), Some(config_yaml))?;

        // Verify app.toml changes
        let modified_app_toml = fs::read_to_string(&app_toml_path)?;
        let app_value: TomlValue = toml::from_str(&modified_app_toml)?;

        if let TomlValue::Table(table) = app_value {
            if let TomlValue::Table(api) = table.get("api").unwrap() {
                assert_eq!(api.get("enable").unwrap().as_bool().unwrap(), true);
                assert_eq!(api.get("swagger").unwrap().as_bool().unwrap(), true);
            }
            if let TomlValue::Table(grpc) = table.get("grpc").unwrap() {
                assert_eq!(grpc.get("enable").unwrap().as_bool().unwrap(), true);
            }
            if let TomlValue::Table(state_sync) = table.get("state-sync").unwrap() {
                assert_eq!(
                    state_sync
                        .get("snapshot-interval")
                        .unwrap()
                        .as_integer()
                        .unwrap(),
                    1000
                );
            }
        }

        // Verify config.toml changes
        let modified_config_toml = fs::read_to_string(&config_toml_path)?;
        let config_value: TomlValue = toml::from_str(&modified_config_toml)?;

        if let TomlValue::Table(table) = config_value {
            if let TomlValue::Table(rpc) = table.get("rpc").unwrap() {
                assert_eq!(
                    rpc.get("laddr").unwrap().as_str().unwrap(),
                    "tcp://0.0.0.0:26657"
                );
            }
            if let TomlValue::Table(p2p) = table.get("p2p").unwrap() {
                assert_eq!(
                    p2p.get("seeds").unwrap().as_str().unwrap(),
                    "seed1.example.com:26656,seed2.example.com:26656"
                );
            }
        }

        Ok(())
    }
}
