use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Configuration file structure for log-hound
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default AWS profile
    #[serde(default)]
    pub default_profile: Option<String>,

    /// Default AWS region
    #[serde(default)]
    pub default_region: Option<String>,

    /// Default log groups (used when no -g is specified)
    #[serde(default)]
    pub default_groups: Vec<String>,

    /// Default time range (e.g., "1h", "30m")
    #[serde(default)]
    pub default_time_range: Option<String>,

    /// Default result limit
    #[serde(default)]
    pub default_limit: Option<i32>,

    /// Saved presets for quick access
    #[serde(default)]
    pub presets: HashMap<String, Preset>,
}

/// A saved preset configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    /// Log groups to search
    pub groups: Vec<String>,

    /// Optional default patterns to include
    #[serde(default)]
    pub patterns: Vec<String>,

    /// Optional exclude patterns
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Time range override
    #[serde(default)]
    pub time_range: Option<String>,

    /// Result limit override
    #[serde(default)]
    pub limit: Option<i32>,

    /// Description for this preset
    #[serde(default)]
    pub description: Option<String>,
}

impl Config {
    /// Load configuration from the default location (~/.log-hound.toml)
    pub fn load() -> Result<Self> {
        let config_path = Self::default_path();

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;

        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {:?}", config_path))?;

        Ok(config)
    }

    /// Get the default configuration file path
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".log-hound.toml")
    }

    /// Get a preset by name
    pub fn get_preset(&self, name: &str) -> Option<&Preset> {
        self.presets.get(name)
    }

    /// List all available presets
    pub fn list_presets(&self) -> Vec<(&String, &Preset)> {
        self.presets.iter().collect()
    }

    /// Create a sample configuration file
    pub fn create_sample() -> String {
        r#"# Log Hound Configuration
# Place this file at ~/.log-hound.toml

# Default AWS profile (optional)
# default_profile = "production"

# Default AWS region (optional)
# default_region = "ap-northeast-1"

# Default log groups when no -g is specified
default_groups = []

# Default time range
default_time_range = "1h"

# Default result limit
default_limit = 100

# Presets for quick access
# Use with: log-hound search -p <preset_name> "ERROR"

[presets.prod]
description = "Production environment"
groups = ["app/production", "api/production"]
time_range = "1h"
limit = 200

[presets.staging]
description = "Staging environment"
groups = ["app/staging", "api/staging"]
exclude = ["health-check", "ping"]

[presets.all-regions]
description = "Search across all regions"
groups = [
    "us-east-1:app/prod",
    "ap-northeast-1:app/prod",
    "eu-west-1:app/prod"
]
"#
        .to_string()
    }
}
