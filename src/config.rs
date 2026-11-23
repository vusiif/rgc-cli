#![forbid(unsafe_code)]

//! Configuration file support and Privacy Profiles for rgc-cli.
//!
//! Searches for `rgc.toml` or `.rgc.toml` in the current working directory,
//! an explicitly provided `--config <PATH>`, or standard user configuration paths.
//! Prepopulates default sensitive window masks, security directory restrictions,
//! and common CLI/MCP capture parameters.

use std::fs;
use std::path::{Path, PathBuf};

use log::debug;
use serde::{Deserialize, Serialize};

/// Root configuration schema for `rgc.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RgcConfig {
    #[serde(default)]
    pub privacy: PrivacyConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

/// Privacy profile configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PrivacyConfig {
    /// Window title substrings to automatically mask (e.g. `["1Password", "Bitwarden"]`).
    #[serde(default)]
    pub mask_windows: Vec<String>,
    /// Coordinate regions to automatically mask (e.g. `["0,0,200,50:blackout"]`).
    #[serde(default)]
    pub mask_regions: Vec<String>,
    /// Whether to allow capture if a configured mask window is not found.
    #[serde(default)]
    pub allow_missing_mask: Option<bool>,
}

/// Security and directory constraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SecurityConfig {
    /// Allowed directories for saving output captures.
    #[serde(default)]
    pub allowed_output_dirs: Vec<String>,
    /// Upper bound on width or height for image operations.
    #[serde(default)]
    pub max_dimension: Option<u32>,
}

/// Default capture options when not overridden on CLI or MCP call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DefaultsConfig {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub cursor: Option<bool>,
    #[serde(default)]
    pub coords: Option<String>,
}

impl RgcConfig {
    /// Parse configuration from a TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Load configuration from an explicit path or search standard candidate locations.
    pub fn load(explicit_path: Option<&Path>) -> Option<Self> {
        if let Some(path) = explicit_path {
            if path.exists() {
                match fs::read_to_string(path) {
                    Ok(content) => match Self::from_toml(&content) {
                        Ok(cfg) => {
                            debug!("Loaded configuration from explicit path: {}", path.display());
                            return Some(cfg);
                        }
                        Err(e) => {
                            debug!("Failed to parse config at '{}': {e}", path.display());
                        }
                    },
                    Err(e) => {
                        debug!("Failed to read config file '{}': {e}", path.display());
                    }
                }
            }
            return None;
        }

        // Search candidate locations
        for candidate in get_candidate_config_paths() {
            if candidate.exists() && candidate.is_file() {
                if let Ok(content) = fs::read_to_string(&candidate) {
                    if let Ok(cfg) = Self::from_toml(&content) {
                        debug!("Loaded configuration from: {}", candidate.display());
                        return Some(cfg);
                    }
                }
            }
        }

        None
    }
}

/// Return candidate paths for configuration discovery.
pub fn get_candidate_config_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. Current working directory: rgc.toml, .rgc.toml
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("rgc.toml"));
        candidates.push(cwd.join(".rgc.toml"));
    }

    // 2. User home directory
    if let Some(home) = get_user_home_dir() {
        candidates.push(home.join(".config").join("rgc").join("config.toml"));
        candidates.push(home.join(".rgc.toml"));
    }

    candidates
}

/// Retrieve user home directory across Windows and Unix platforms.
fn get_user_home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| {
                let drive = std::env::var_os("HOMEDRIVE")?;
                let path = std::env::var_os("HOMEPATH")?;
                Some(PathBuf::from(drive).join(path))
            })
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_toml_config() {
        let toml_str = r#"
[privacy]
mask_windows = ["1Password", "Bitwarden"]
mask_regions = ["0,0,100,50:blackout"]
allow_missing_mask = false

[security]
allowed_output_dirs = ["./captures", "C:\\temp"]
max_dimension = 4096

[defaults]
format = "webp"
quality = 90
scale = 0.5
cursor = true
coords = "physical"
"#;

        let config = RgcConfig::from_toml(toml_str).expect("TOML should parse");
        assert_eq!(config.privacy.mask_windows, vec!["1Password", "Bitwarden"]);
        assert_eq!(config.privacy.mask_regions, vec!["0,0,100,50:blackout"]);
        assert_eq!(config.privacy.allow_missing_mask, Some(false));

        assert_eq!(
            config.security.allowed_output_dirs,
            vec!["./captures", "C:\\temp"]
        );
        assert_eq!(config.security.max_dimension, Some(4096));

        assert_eq!(config.defaults.format.as_deref(), Some("webp"));
        assert_eq!(config.defaults.quality, Some(90));
        assert_eq!(config.defaults.scale, Some(0.5));
        assert_eq!(config.defaults.cursor, Some(true));
        assert_eq!(config.defaults.coords.as_deref(), Some("physical"));
    }

    #[test]
    fn test_parse_empty_toml_defaults() {
        let config = RgcConfig::from_toml("").expect("Empty TOML should parse");
        assert_eq!(config, RgcConfig::default());
        assert!(config.privacy.mask_windows.is_empty());
        assert!(config.security.allowed_output_dirs.is_empty());
    }
}
