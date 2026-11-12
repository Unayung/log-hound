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

    /// Refuse Insights searches estimated to scan more than this (default 30 GB)
    #[serde(default)]
    pub max_scan_gb: Option<f64>,

    /// Archived groups go to Athena when Insights would scan more than this (default 5 GB)
    #[serde(default)]
    pub athena_above_gb: Option<f64>,

    /// Log group name (no region prefix) -> its S3 archive queryable with Athena
    #[serde(default)]
    pub athena: HashMap<String, AthenaTarget>,
}

/// Where a log group's archive lives in Athena
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AthenaTarget {
    pub region: String,
    pub workgroup: String,
    /// "database.table"
    pub table: String,
}

/// A saved preset configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    /// Log groups to search (CloudWatch only)
    #[serde(default)]
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

    /// Log source: "cloudwatch" or "kamal"
    #[serde(default)]
    pub source: Option<String>,

    /// Kamal deploy.yml file path (for source = "kamal")
    #[serde(default)]
    pub deploy_file: Option<String>,
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

# Refuse Insights searches estimated to scan more than this many GB.
# Insights bills every byte ingested in the time window, whatever the filter or limit.
# max_scan_gb = 30

# Log groups archived to S3: searched with Athena when the range is older than the
# group's CloudWatch retention, Insights would scan more than athena_above_gb
# (default 5), the total is over max_scan_gb, or with --athena
# athena_above_gb = 5
# [athena."app/production"]
# region = "ap-east-2"
# workgroup = "app-logs"
# table = "app_logs.app_production"

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

# Kamal deployment presets
# Use with: log-hound search -p <preset_name> "ERROR"

[presets.my-app]
description = "My Kamal-deployed app"
source = "kamal"
deploy_file = "config/deploy.yml"
time_range = "1h"
limit = 200
exclude = ["health-check", "ping"]
"#
        .to_string()
    }
}
