//! Static download profiles loaded from `profiles.yaml`.
//!
//! The profile file is resolved from (in order): an explicit path (e.g. the
//! `--profiles` CLI flag), `./profiles.yaml`, a `profiles.yaml` next to the
//! executable, or the copy embedded at build time.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_yaml::Value as YamlValue;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A resolved, ready-to-download snapshot description.
#[derive(Debug, Clone)]
pub struct ResolvedSnapshot {
    /// Final filename of the (possibly multi-part) snapshot.
    pub filename: String,
    /// Single-file download URL (empty when the snapshot is multi-part).
    pub download_url: String,
    /// Ordered part download URLs (empty when the snapshot is single-file).
    pub part_urls: Vec<String>,
    /// Chain binary version from the snapshot index (e.g. "v1.7.7").
    pub version: String,
}

impl ResolvedSnapshot {
    /// Whether the snapshot is split into multiple parts.
    pub fn is_multipart(&self) -> bool {
        !self.part_urls.is_empty()
    }
}

/// A static profile entry loaded from `profiles.yaml`.
#[derive(Debug, Clone)]
pub struct Profile {
    /// Full profile name as passed to `--profile`.
    pub name: String,
    /// Cosmos chain id.
    pub chain_id: String,
    /// Moniker used if the binary is later initialised.
    pub moniker: String,
    /// Curated `app.toml` overrides as a YAML literal (empty when none).
    pub app_yaml: String,
    /// Curated `config.toml` overrides as a YAML literal (empty when none).
    pub config_yaml: String,
    /// URL that returns a `snapshots.json`-compatible index.
    pub snapshot_index_url: String,
    /// Top-level chain key in the snapshot index (e.g. "cronos").
    pub chain_prefix: String,
    /// Environment key in the snapshot index (e.g. "mainnet-snapshot").
    pub env_key: String,
    /// Database key in the snapshot index (e.g. "rocksdb").
    pub db_type: String,
    /// Pruning key in the snapshot index (e.g. "pruned").
    pub pruning_type: String,
    /// Binary download URL template (supports `{version}`, `{version_no_v}`, `{os}`, `{arch}`).
    pub binary_url: String,
    /// Relative path to the binary inside the workspace after extraction.
    pub binary_relative_path: String,
}

const DEFAULT_MONIKER: &str = "snapshot-downloader";

/// Return all profile names loaded from the profiles file.
///
/// When `path` is `Some`, that file is read directly; otherwise the resolution
/// chain (cwd, executable dir, embedded copy) is used.
pub fn profile_names(path: Option<&Path>) -> Result<Vec<String>> {
    let mut names: Vec<String> = load_profiles_data(path)?
        .profiles
        .into_iter()
        .map(|p| p.name)
        .collect();
    names.sort();
    Ok(names)
}

/// Resolve a profile by its exact `name` from the profiles file.
///
/// When `path` is `Some`, that file is read directly; otherwise the resolution
/// chain (cwd, executable dir, embedded copy) is used.
pub fn builtin(name: &str, path: Option<&Path>) -> Result<Option<Profile>> {
    Ok(load_profiles_data(path)?
        .profiles
        .into_iter()
        .find(|p| p.name == name))
}

impl Profile {
    /// Parse the curated `app.toml` overrides, if any.
    pub fn app_yaml_value(&self) -> Result<Option<YamlValue>> {
        parse_yaml(&self.app_yaml)
    }

    /// Parse the curated `config.toml` overrides, if any.
    pub fn config_yaml_value(&self) -> Result<Option<YamlValue>> {
        parse_yaml(&self.config_yaml)
    }

    /// Resolve the latest snapshot for this profile using its configured index URL.
    pub async fn resolved_snapshot(&self) -> Result<ResolvedSnapshot> {
        if self.snapshot_index_url.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Profile '{}' must set snapshot_index_url",
                self.name
            ));
        }

        let client = Client::builder()
            .build()
            .context("Failed to create HTTP client")?;

        resolve_from_index_url(&client, &self.snapshot_index_url, self).await
    }

    /// Resolve the binary download URL from the profile template and snapshot version.
    pub fn resolved_binary_url(&self, snapshot: &ResolvedSnapshot) -> Result<String> {
        expand_url_template(&self.binary_url, &snapshot.version, &self.name)
    }

    /// Relative path to the extracted binary inside the workspace.
    pub fn resolved_binary_relative_path(&self) -> String {
        self.binary_relative_path.clone()
    }
}

fn parse_yaml(s: &str) -> Result<Option<YamlValue>> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    let value: YamlValue =
        serde_yaml::from_str(s).context("Failed to parse built-in profile YAML")?;
    Ok(Some(value))
}

#[derive(Debug, Clone)]
struct ProfilesData {
    profiles: Vec<Profile>,
}

#[derive(Debug, Deserialize)]
struct ProfilesYaml {
    profiles: Vec<ProfileYaml>,
}

/// A profile entry as written in `profiles.yaml`: a name plus two sections,
/// `config` (how to configure/run the node) and `snapshot` (where to find the
/// snapshot in the JSON index). Flattened into the runtime `Profile`.
#[derive(Debug, Deserialize)]
struct ProfileYaml {
    name: String,
    config: ProfileConfigYaml,
    snapshot: ProfileSnapshotYaml,
}

#[derive(Debug, Deserialize)]
struct ProfileConfigYaml {
    chain_id: String,
    #[serde(default = "default_moniker")]
    moniker: String,
    binary_url: String,
    binary_relative_path: String,
    #[serde(default)]
    app_yaml: String,
    #[serde(default)]
    config_yaml: String,
}

#[derive(Debug, Deserialize)]
struct ProfileSnapshotYaml {
    index_url: String,
    chain_prefix: String,
    env_key: String,
    db_type: String,
    pruning_type: String,
}

fn load_profiles_data(path: Option<&Path>) -> Result<ProfilesData> {
    let content = load_profiles_content(path)?;
    let parsed: ProfilesYaml =
        serde_yaml::from_str(&content).context("Failed to parse profiles.yaml")?;
    if parsed.profiles.is_empty() {
        return Err(anyhow::anyhow!(
            "profiles.yaml contains no profiles; add at least one profile entry"
        ));
    }

    let mut profiles = Vec::with_capacity(parsed.profiles.len());
    for p in parsed.profiles {
        if p.snapshot.index_url.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Profile '{}' must set snapshot.index_url",
                p.name
            ));
        }
        if p.config.binary_url.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Profile '{}' must set config.binary_url",
                p.name
            ));
        }
        if p.config.binary_relative_path.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Profile '{}' must set config.binary_relative_path",
                p.name
            ));
        }

        // Effective node config lives in each profile's `config` section and the
        // snapshot index path in its `snapshot` section (composed via YAML
        // anchors in profiles.yaml); both are flattened into `Profile` here.
        profiles.push(Profile {
            name: p.name,
            chain_id: p.config.chain_id,
            moniker: p.config.moniker,
            app_yaml: p.config.app_yaml,
            config_yaml: p.config.config_yaml,
            snapshot_index_url: p.snapshot.index_url,
            chain_prefix: p.snapshot.chain_prefix,
            env_key: p.snapshot.env_key,
            db_type: p.snapshot.db_type,
            pruning_type: p.snapshot.pruning_type,
            binary_url: p.config.binary_url,
            binary_relative_path: p.config.binary_relative_path,
        });
    }

    Ok(ProfilesData { profiles })
}

/// The `profiles.yaml` shipped with the source tree, embedded at build time as
/// the always-available fallback.
const EMBEDDED_PROFILES: &str = include_str!("../profiles.yaml");

/// Read the profiles file content.
///
/// When `path` is `Some`, that file is read directly and a missing/unreadable
/// file is a hard error. When `None`, the resolution chain is tried in order:
/// `./profiles.yaml`, a `profiles.yaml` next to the executable, then the
/// embedded copy.
fn load_profiles_content(path: Option<&Path>) -> Result<String> {
    if let Some(path) = path {
        return fs::read_to_string(path)
            .with_context(|| format!("Failed to read profiles file: {}", path.display()));
    }

    for dir in profile_search_dirs() {
        let path = dir.join("profiles.yaml");
        if path.exists() {
            return fs::read_to_string(&path)
                .with_context(|| format!("Failed to read profiles file: {}", path.display()));
        }
    }

    Ok(EMBEDDED_PROFILES.to_string())
}

/// Directories searched for a `profiles.yaml` override: the current working
/// directory, then the directory containing the executable.
fn profile_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    dirs
}

fn default_moniker() -> String {
    DEFAULT_MONIKER.to_string()
}

#[derive(Debug, Deserialize, Clone)]
struct SnapshotsIndex {
    chains: HashMap<String, EnvIndex>,
}

type EnvIndex = HashMap<String, DbIndex>;
type DbIndex = HashMap<String, PruningIndex>;
type PruningIndex = HashMap<String, Vec<SnapshotEntry>>;

#[derive(Debug, Deserialize, Clone)]
struct SnapshotEntry {
    filename: String,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    last_modified: String,
    #[serde(default)]
    part_files: Vec<PartFile>,
}

#[derive(Debug, Deserialize, Clone)]
struct PartFile {
    filename: String,
    #[serde(default)]
    download_url: String,
}

async fn resolve_from_index_url(
    client: &Client,
    index_url: &str,
    profile: &Profile,
) -> Result<ResolvedSnapshot> {
    let index: SnapshotsIndex = client
        .get(index_url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch snapshot index: {index_url}"))?
        .error_for_status()
        .with_context(|| format!("Snapshot index request failed: {index_url}"))?
        .json()
        .await
        .with_context(|| format!("Failed to parse snapshot index JSON: {index_url}"))?;

    let entries = index
        .chains
        .get(&profile.chain_prefix)
        .and_then(|env| env.get(&profile.env_key))
        .and_then(|db| db.get(&profile.db_type))
        .and_then(|pruning| pruning.get(&profile.pruning_type))
        .with_context(|| {
            format!(
                "Missing path {}/{}/{}/{} in {} for profile '{}'",
                profile.chain_prefix,
                profile.env_key,
                profile.db_type,
                profile.pruning_type,
                index_url,
                profile.name
            )
        })?;

    if entries.is_empty() {
        return Err(anyhow::anyhow!(
            "No entries for profile '{}' in {}",
            profile.name,
            index_url
        ));
    }

    // The index is published newest-first; sort defensively by last_modified
    // (ISO-8601 UTC, so lexical == chronological), then filename as a tiebreak.
    let mut sorted = entries.clone();
    sorted.sort_by(|a, b| {
        b.last_modified
            .cmp(&a.last_modified)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    let latest = &sorted[0];

    let part_urls: Vec<String> = {
        let mut parts = latest.part_files.clone();
        parts.sort_by(|a, b| a.filename.cmp(&b.filename));
        parts.into_iter().map(|p| p.download_url).collect()
    };

    Ok(ResolvedSnapshot {
        filename: latest.filename.clone(),
        download_url: if part_urls.is_empty() {
            latest.download_url.clone()
        } else {
            String::new()
        },
        part_urls,
        version: latest.version.clone(),
    })
}

fn target_os_label() -> Result<&'static str> {
    match std::env::consts::OS {
        "linux" => Ok("Linux"),
        "macos" => Ok("Darwin"),
        "windows" => Ok("Windows"),
        other => Err(anyhow::anyhow!(
            "Unsupported OS for binary download templates: {other}"
        )),
    }
}

fn target_arch_label() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("arm64"),
        other => Err(anyhow::anyhow!(
            "Unsupported CPU architecture for binary download templates: {other}"
        )),
    }
}

/// Expand `{version}`, `{version_no_v}`, `{os}`, and `{arch}` in a URL template.
fn expand_url_template(template: &str, version: &str, profile_name: &str) -> Result<String> {
    if version.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Profile '{profile_name}' needs a binary URL template but the snapshot index entry has no version"
        ));
    }

    let version_no_v = version.strip_prefix('v').unwrap_or(version);
    let expanded = template
        .replace("{version}", version)
        .replace("{version_no_v}", version_no_v)
        .replace("{os}", target_os_label()?)
        .replace("{arch}", target_arch_label()?);

    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_url_template_replaces_placeholders() {
        let url = expand_url_template(
            "https://example.com/{version}/widgetd_{version_no_v}_{os}_{arch}.tar.gz",
            "v1.7.7",
            "test-profile",
        )
        .unwrap();
        assert_eq!(
            url,
            format!(
                "https://example.com/v1.7.7/widgetd_1.7.7_{}_{}.tar.gz",
                target_os_label().unwrap(),
                target_arch_label().unwrap()
            )
        );
    }

    #[test]
    fn expand_url_template_requires_version() {
        let err = expand_url_template("https://example.com/{version}", "", "test-profile")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no version"));
    }

    #[test]
    fn embedded_profiles_parse_sectioned_shape() {
        // The shipped profiles.yaml uses the `config:` / `snapshot:` sections;
        // ensure they deserialize and flatten onto `Profile`.
        let data = load_profiles_data(None).expect("embedded profiles.yaml must parse");
        assert!(!data.profiles.is_empty());

        let p = builtin("cronos-mainnet-leveldb-archive", None)
            .expect("lookup must not error")
            .expect("profile must exist");
        // config section
        assert_eq!(p.chain_id, "cronosmainnet_25-1");
        assert_eq!(p.binary_relative_path, "bin/cronosd");
        assert!(p.binary_url.contains("{version}"));
        assert!(p.app_yaml.contains("app-db-backend"));
        assert!(p.app_yaml.contains("goleveldb"));
        assert!(p.config_yaml.contains("db_backend"));
        assert!(p.config_yaml.contains("goleveldb"));
        // snapshot section
        assert_eq!(
            p.snapshot_index_url,
            "https://snapshot.cronos.com/snapshots.json"
        );
        assert_eq!(p.chain_prefix, "cronos");
        assert_eq!(p.env_key, "mainnet-snapshot");
        assert_eq!(p.db_type, "leveldb");
        assert_eq!(p.pruning_type, "archive");

        let rocksdb = builtin("cronos-mainnet-rocksdb-pruned", None)
            .expect("lookup must not error")
            .expect("profile must exist");
        assert!(rocksdb.app_yaml.contains("app-db-backend: \"rocksdb\""));
        assert!(rocksdb.config_yaml.contains("db_backend: \"rocksdb\""));

        let versiondb = builtin("cronos-mainnet-versiondb-pruned", None)
            .expect("lookup must not error")
            .expect("profile must exist");
        assert!(versiondb.app_yaml.contains("versiondb:"));
        assert!(versiondb.app_yaml.contains("enable: true"));
    }
}
