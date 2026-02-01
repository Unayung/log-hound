use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Represents a parsed Kamal deploy.yml configuration
#[derive(Debug, Clone)]
pub struct KamalConfig {
    pub service: String,
    pub servers: Vec<String>,
    pub ssh_user: String,
    /// Destination name extracted from deploy.{destination}.yml filename
    pub destination: Option<String>,
}

/// Raw YAML structure for Kamal deploy files
/// All fields optional to support split config (base + environment)
#[derive(Debug, Deserialize, Default)]
struct KamalYaml {
    service: Option<String>,
    servers: Option<ServersConfig>,
    #[serde(default)]
    ssh: Option<SshConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum ServersConfig {
    /// Simple list: servers: ["host1", "host2"]
    Simple(Vec<String>),
    /// Kamal 2.x role-based: servers: { web: ["host1"], job: ["host2"] }
    RoleBasedSimple(HashMap<String, Vec<String>>),
    /// Kamal 1.x role-based: servers: { web: { hosts: ["host1"], ... }, job: { hosts: ["host2"], ... } }
    RoleBasedWithOptions(HashMap<String, RoleConfig>),
}

/// Kamal 1.x role configuration with hosts and optional settings
#[derive(Debug, Deserialize, Clone)]
struct RoleConfig {
    hosts: Vec<String>,
    // Other fields like labels, cmd, options are ignored for our purposes
}

#[derive(Debug, Deserialize, Clone)]
struct SshConfig {
    user: Option<String>,
}

impl KamalConfig {
    /// Load a Kamal configuration from a deploy file
    /// Supports Kamal's convention: deploy.yml (base) + deploy.{env}.yml (environment)
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read Kamal config: {:?}", path))?;

        let env_config: KamalYaml = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse Kamal YAML: {:?}", path))?;

        // Check if we need to load base config
        // If the file is deploy.{env}.yml (not deploy.yml), try to merge with base
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let needs_base = filename != "deploy.yml"
            && filename.starts_with("deploy.")
            && filename.ends_with(".yml");

        // Extract destination from filename: deploy.rhinoshield.yml -> rhinoshield
        let destination = if needs_base {
            filename
                .strip_prefix("deploy.")
                .and_then(|s| s.strip_suffix(".yml"))
                .map(|s| s.to_string())
        } else {
            None
        };

        let merged = if needs_base {
            // Try to load base deploy.yml from same directory
            let base_path = path.parent()
                .map(|p| p.join("deploy.yml"))
                .unwrap_or_else(|| Path::new("config/deploy.yml").to_path_buf());

            if base_path.exists() {
                let base_contents = std::fs::read_to_string(&base_path)
                    .with_context(|| format!("Failed to read base config: {:?}", base_path))?;
                let base_config: KamalYaml = serde_yaml::from_str(&base_contents)
                    .with_context(|| format!("Failed to parse base YAML: {:?}", base_path))?;

                // Merge: environment overrides base
                KamalYaml {
                    service: env_config.service.or(base_config.service),
                    servers: env_config.servers.or(base_config.servers),
                    ssh: env_config.ssh.or(base_config.ssh),
                }
            } else {
                env_config
            }
        } else {
            env_config
        };

        Self::from_yaml(merged, destination)
    }

    /// Convert parsed YAML to KamalConfig
    fn from_yaml(raw: KamalYaml, destination: Option<String>) -> Result<Self> {
        let service = raw.service
            .ok_or_else(|| anyhow!("Missing 'service' in Kamal config"))?;

        let servers_config = raw.servers
            .ok_or_else(|| anyhow!("Missing 'servers' in Kamal config"))?;

        // Extract all servers from the config
        let servers = match servers_config {
            ServersConfig::Simple(hosts) => hosts,
            ServersConfig::RoleBasedSimple(roles) => {
                // Kamal 2.x: Flatten all roles into a single list, prioritizing 'web' role
                let mut all_servers = Vec::new();
                if let Some(web_servers) = roles.get("web") {
                    all_servers.extend(web_servers.clone());
                }
                for (role, hosts) in roles {
                    if role != "web" {
                        all_servers.extend(hosts);
                    }
                }
                all_servers
            }
            ServersConfig::RoleBasedWithOptions(roles) => {
                // Kamal 1.x: Extract hosts from role configs
                let mut all_servers = Vec::new();
                if let Some(web_config) = roles.get("web") {
                    all_servers.extend(web_config.hosts.clone());
                }
                for (role, config) in roles {
                    if role != "web" {
                        all_servers.extend(config.hosts.clone());
                    }
                }
                all_servers
            }
        };

        // Deduplicate servers (same host might appear in multiple roles)
        let mut seen = std::collections::HashSet::new();
        let servers: Vec<String> = servers.into_iter().filter(|s| seen.insert(s.clone())).collect();

        if servers.is_empty() {
            return Err(anyhow!("No servers found in Kamal config"));
        }

        // Get SSH user, defaulting to "root"
        let ssh_user = raw
            .ssh
            .and_then(|s| s.user)
            .unwrap_or_else(|| "root".to_string());

        Ok(KamalConfig {
            service,
            servers,
            ssh_user,
            destination,
        })
    }

    /// Parse Kamal configuration from YAML string (for testing)
    pub fn parse(yaml: &str) -> Result<Self> {
        let raw: KamalYaml = serde_yaml::from_str(yaml)
            .context("Failed to parse Kamal YAML")?;
        Self::from_yaml(raw, None)
    }

    /// Get the container name pattern for docker logs
    /// Kamal names containers as: {service}-{role}-{hash}
    pub fn container_pattern(&self) -> String {
        format!("{}*", self.service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_servers() {
        let yaml = r#"
service: my-app
servers:
  - host1.example.com
  - host2.example.com
"#;
        let config = KamalConfig::parse(yaml).unwrap();
        assert_eq!(config.service, "my-app");
        assert_eq!(config.servers, vec!["host1.example.com", "host2.example.com"]);
        assert_eq!(config.ssh_user, "root");
    }

    #[test]
    fn test_parse_role_based_servers_kamal2() {
        let yaml = r#"
service: my-app
servers:
  web:
    - web1.example.com
  job:
    - job1.example.com
ssh:
  user: deploy
"#;
        let config = KamalConfig::parse(yaml).unwrap();
        assert_eq!(config.service, "my-app");
        assert!(config.servers.contains(&"web1.example.com".to_string()));
        assert!(config.servers.contains(&"job1.example.com".to_string()));
        assert_eq!(config.ssh_user, "deploy");
    }

    #[test]
    fn test_parse_role_based_servers_kamal1() {
        // Kamal 1.x format with hosts under each role
        let yaml = r#"
service: foundation
servers:
  web:
    hosts:
      - 35.74.156.92
    labels:
      traefik.http.routers.foundation-web.rule: Host(`example.com`)
  job:
    hosts:
      - 35.74.156.92
    cmd: bundle exec sidekiq
ssh:
  user: apps
"#;
        let config = KamalConfig::parse(yaml).unwrap();
        assert_eq!(config.service, "foundation");
        assert!(config.servers.contains(&"35.74.156.92".to_string()));
        assert_eq!(config.ssh_user, "apps");
    }
}
