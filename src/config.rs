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
    /// Window title substrings to exclude and blackout completely (e.g. `["rgc-cli", "Terminal"]`).
    #[serde(default)]
    pub exclude_windows: Vec<String>,
    /// Coordinate regions to automatically mask (e.g. `["0,0,200,50:blackout"]`).
    #[serde(default)]
    pub mask_regions: Vec<String>,
    /// Whether to allow capture if a configured mask window is not found.
    #[serde(default)]
    pub allow_missing_mask: Option<bool>,
    /// Enforce mandatory privacy mode: when true, requests cannot weaken or bypass masking rules.
    #[serde(default)]
    pub strict: Option<bool>,
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
    #[serde(default)]
    pub som: Option<bool>,
    #[serde(default)]
    pub accessibility: Option<bool>,
}

/// Unified privacy policy resolved from configuration and request-specific options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrivacyPolicy {
    pub mask_windows: Vec<crate::cli::MaskWindow>,
    pub exclude_windows: Vec<String>,
    pub mask_regions: Vec<crate::cli::MaskRegion>,
    pub allow_missing_mask: bool,
}

impl PrivacyPolicy {
    /// Create a PrivacyPolicy directly from a configuration.
    pub fn from_config(cfg: &PrivacyConfig) -> Self {
        Self::from_config_and_request(cfg, &[], &[], &[], None)
    }

    /// Validate that all configured mask rules and regions parse successfully.
    pub fn try_from_config(cfg: &PrivacyConfig) -> Result<Self, String> {
        let mut mask_windows = Vec::new();
        for w in &cfg.mask_windows {
            let mw = w
                .parse::<crate::cli::MaskWindow>()
                .map_err(|e| format!("Invalid mask_windows entry '{w}': {e}"))?;
            mask_windows.push(mw);
        }

        let mut mask_regions = Vec::new();
        for r in &cfg.mask_regions {
            let mr = r
                .parse::<crate::cli::MaskRegion>()
                .map_err(|e| format!("Invalid mask_regions entry '{r}': {e}"))?;
            mask_regions.push(mr);
        }

        let allow_missing_mask = cfg.allow_missing_mask.unwrap_or(false);

        Ok(Self {
            mask_windows,
            exclude_windows: cfg.exclude_windows.clone(),
            mask_regions,
            allow_missing_mask,
        })
    }

    /// Merge global configuration with request-level options.
    ///
    /// Rules:
    /// - Configured `mask_windows` are parsed and retained; request-level masks are appended (union).
    /// - Configured `exclude_windows` are retained; request-level excludes are appended (union without duplicates).
    /// - Configured `mask_regions` are parsed and retained; request-level regions are appended.
    /// - `allow_missing_mask`:
    ///   - If globally configured as `Some(false)` (strict enforcement), requests CANNOT weaken or disable it.
    ///   - If globally configured as `Some(true)` or `None`, requests can tighten it by requesting `Some(false)`.
    pub fn from_config_and_request(
        cfg: &PrivacyConfig,
        request_mask_windows: &[crate::cli::MaskWindow],
        request_exclude_windows: &[String],
        request_mask_regions: &[crate::cli::MaskRegion],
        request_allow_missing: Option<bool>,
    ) -> Self {
        let mut mask_windows = Vec::new();
        for w in &cfg.mask_windows {
            match w.parse::<crate::cli::MaskWindow>() {
                Ok(mw) => mask_windows.push(mw),
                Err(e) => log::warn!("Skipping invalid configured mask_windows '{w}': {e}"),
            }
        }
        for rm in request_mask_windows {
            if !mask_windows
                .iter()
                .any(|m| m.query == rm.query && m.effect == rm.effect)
            {
                mask_windows.push(rm.clone());
            }
        }

        let mut exclude_windows = cfg.exclude_windows.clone();
        for ew in request_exclude_windows {
            if !exclude_windows.iter().any(|e| e == ew) {
                exclude_windows.push(ew.clone());
            }
        }

        let mut mask_regions = Vec::new();
        for r in &cfg.mask_regions {
            match r.parse::<crate::cli::MaskRegion>() {
                Ok(mr) => mask_regions.push(mr),
                Err(e) => log::warn!("Skipping invalid configured mask_regions '{r}': {e}"),
            }
        }
        mask_regions.extend_from_slice(request_mask_regions);

        let allow_missing_mask = if cfg.strict == Some(true) {
            // Mandatory privacy enforcement: requests cannot weaken or bypass strict mask requirements.
            false
        } else {
            request_allow_missing
                .or(cfg.allow_missing_mask)
                .unwrap_or(false)
        };

        Self {
            mask_windows,
            exclude_windows,
            mask_regions,
            allow_missing_mask,
        }
    }
}

static ACTIVE_CONFIG: std::sync::RwLock<Option<RgcConfig>> = std::sync::RwLock::new(None);

/// Set the active global configuration.
pub fn set_active_config(config: RgcConfig) {
    if let Ok(mut lock) = ACTIVE_CONFIG.write() {
        *lock = Some(config);
    }
}

/// Retrieve the active global configuration or default.
pub fn get_active_config() -> RgcConfig {
    if let Ok(lock) = ACTIVE_CONFIG.read() {
        if let Some(ref cfg) = *lock {
            return cfg.clone();
        }
    }
    RgcConfig::load(None).unwrap_or_default()
}

impl RgcConfig {
    /// Parse configuration from a TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Explicitly load and validate configuration from a given file path.
    /// Returns an error if the file does not exist, cannot be read, or contains invalid syntax/rules.
    pub fn load_explicit(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!(
                "Configuration file does not exist: {}",
                path.display()
            ));
        }
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {e}", path.display()))?;
        let cfg = Self::from_toml(&content).map_err(|e| {
            format!(
                "Failed to parse TOML in config file '{}': {e}",
                path.display()
            )
        })?;
        PrivacyPolicy::try_from_config(&cfg.privacy)
            .map_err(|e| format!("Invalid privacy configuration in '{}': {e}", path.display()))?;
        Ok(cfg)
    }

    /// Load configuration from an explicit path or search standard candidate locations.
    pub fn load(explicit_path: Option<&Path>) -> Option<Self> {
        if let Some(path) = explicit_path {
            match Self::load_explicit(path) {
                Ok(cfg) => {
                    debug!(
                        "Loaded configuration from explicit path: {}",
                        path.display()
                    );
                    return Some(cfg);
                }
                Err(e) => {
                    debug!("{e}");
                    return None;
                }
            }
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

    #[test]
    fn test_privacy_policy_merging_and_inheritance() {
        let cfg = PrivacyConfig {
            mask_windows: vec!["1Password:blackout".to_string(), "Bitwarden".to_string()],
            exclude_windows: vec!["Terminal".to_string()],
            mask_regions: vec!["0,0,100,50:blur".to_string()],
            allow_missing_mask: Some(false),
            strict: None,
        };

        let req_mask_win = vec![crate::cli::MaskWindow {
            query: "Slack".to_string(),
            effect: crate::cli::MaskEffect::Transparent,
        }];
        let req_exclude = vec!["rgc-cli".to_string(), "Terminal".to_string()];
        let req_region = vec![crate::cli::MaskRegion {
            region: crate::cli::Region {
                x: 10,
                y: 10,
                width: 50,
                height: 50,
            },
            effect: crate::cli::MaskEffect::Blackout,
        }];

        let policy = PrivacyPolicy::from_config_and_request(
            &cfg,
            &req_mask_win,
            &req_exclude,
            &req_region,
            None,
        );

        // Mask windows: 1Password, Bitwarden, Slack (union)
        assert_eq!(policy.mask_windows.len(), 3);
        assert_eq!(policy.mask_windows[0].query, "1Password");
        assert_eq!(
            policy.mask_windows[0].effect,
            crate::cli::MaskEffect::Blackout
        );
        assert_eq!(policy.mask_windows[1].query, "Bitwarden");
        assert_eq!(policy.mask_windows[2].query, "Slack");

        // Exclude windows: Terminal, rgc-cli (deduplicated union)
        assert_eq!(policy.exclude_windows, vec!["Terminal", "rgc-cli"]);

        // Mask regions: configured + request
        assert_eq!(policy.mask_regions.len(), 2);
        assert_eq!(policy.mask_regions[0].effect, crate::cli::MaskEffect::Blur);
        assert_eq!(
            policy.mask_regions[1].effect,
            crate::cli::MaskEffect::Blackout
        );

        // Inherited allow_missing_mask
        assert!(!policy.allow_missing_mask);

        // Request override of allow_missing_mask
        let policy_override =
            PrivacyPolicy::from_config_and_request(&cfg, &[], &[], &[], Some(true));
        assert!(policy_override.allow_missing_mask);
    }

    #[test]
    fn test_privacy_policy_strict_mode_cannot_be_downgraded() {
        let cfg = PrivacyConfig {
            mask_windows: vec!["1Password".to_string()],
            exclude_windows: vec![],
            mask_regions: vec![],
            allow_missing_mask: Some(true),
            strict: Some(true),
        };

        // When strict: true is configured, requests cannot bypass or allow missing masks
        let policy = PrivacyPolicy::from_config_and_request(&cfg, &[], &[], &[], Some(true));
        assert!(!policy.allow_missing_mask);
    }

    #[test]
    fn test_try_from_config_validation() {
        let valid_cfg = PrivacyConfig {
            mask_windows: vec!["Pass:blackout".to_string()],
            exclude_windows: vec!["Term".to_string()],
            mask_regions: vec!["0,0,10,10:blur".to_string()],
            allow_missing_mask: None,
            strict: None,
        };
        assert!(PrivacyPolicy::try_from_config(&valid_cfg).is_ok());

        let invalid_mask_cfg = PrivacyConfig {
            mask_windows: vec![":blur".to_string()],
            exclude_windows: vec![],
            mask_regions: vec![],
            allow_missing_mask: None,
            strict: None,
        };
        assert!(PrivacyPolicy::try_from_config(&invalid_mask_cfg).is_err());

        let invalid_region_cfg = PrivacyConfig {
            mask_windows: vec![],
            exclude_windows: vec![],
            mask_regions: vec!["invalid_coords".to_string()],
            allow_missing_mask: None,
            strict: None,
        };
        assert!(PrivacyPolicy::try_from_config(&invalid_region_cfg).is_err());
    }
}
