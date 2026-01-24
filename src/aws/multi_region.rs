use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_sdk_cloudwatchlogs::Client;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Parsed log group with optional region override
#[derive(Debug, Clone)]
pub struct RegionalLogGroup {
    pub region: Option<String>,
    pub log_group: String,
}

impl RegionalLogGroup {
    /// Parse a log group string, optionally prefixed with region
    /// Format: "region:log-group" or just "log-group"
    pub fn parse(input: &str) -> Self {
        let input = input.trim();

        // Check for region:log-group format
        if let Some(colon_pos) = input.find(':') {
            let potential_region = &input[..colon_pos];

            // Validate it looks like a region (e.g., ap-east-2, us-west-1)
            if is_valid_region_format(potential_region) {
                return Self {
                    region: Some(potential_region.to_string()),
                    log_group: input[colon_pos + 1..].to_string(),
                };
            }
        }

        // No region prefix
        Self {
            region: None,
            log_group: input.to_string(),
        }
    }

    /// Parse multiple log group strings
    pub fn parse_many(inputs: &[String]) -> Vec<Self> {
        inputs.iter().map(|s| Self::parse(s)).collect()
    }
}

/// Check if a string looks like an AWS region
fn is_valid_region_format(s: &str) -> bool {
    // AWS regions follow pattern: xx-xxxx-N (e.g., us-east-1, ap-southeast-2)
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 3 {
        return false;
    }

    // Last part should be a number
    if parts.last().map(|p| p.parse::<u32>().is_ok()) != Some(true) {
        return false;
    }

    // First part should be 2 letters (continent code)
    if parts[0].len() != 2 || !parts[0].chars().all(|c| c.is_ascii_lowercase()) {
        return false;
    }

    true
}

/// Manages CloudWatch clients for multiple regions
pub struct MultiRegionClientPool {
    profile: Option<String>,
    default_region: Option<String>,
    clients: RwLock<HashMap<String, Client>>,
}

impl MultiRegionClientPool {
    pub fn new(profile: Option<String>, default_region: Option<String>) -> Self {
        Self {
            profile,
            default_region,
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a client for the specified region
    pub async fn get_client(&self, region: Option<&str>) -> Result<Client> {
        let region_key = region
            .or(self.default_region.as_deref())
            .unwrap_or("default")
            .to_string();

        // Check if we already have this client
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(&region_key) {
                return Ok(client.clone());
            }
        }

        // Create new client
        let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

        if let Some(profile_name) = &self.profile {
            config_loader = config_loader.profile_name(profile_name);
        }

        let effective_region = region.or(self.default_region.as_deref());
        if let Some(region_name) = effective_region {
            config_loader = config_loader.region(aws_config::Region::new(region_name.to_string()));
        }

        let config = config_loader.load().await;
        let client = Client::new(&config);

        // Store for reuse
        {
            let mut clients = self.clients.write().await;
            clients.insert(region_key, client.clone());
        }

        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_region() {
        let parsed = RegionalLogGroup::parse("ap-east-2:/aws/app/rails");
        assert_eq!(parsed.region, Some("ap-east-2".to_string()));
        assert_eq!(parsed.log_group, "/aws/app/rails");
    }

    #[test]
    fn test_parse_without_region() {
        let parsed = RegionalLogGroup::parse("/aws/app/rails");
        assert_eq!(parsed.region, None);
        assert_eq!(parsed.log_group, "/aws/app/rails");
    }

    #[test]
    fn test_parse_with_colon_in_log_group() {
        // If something doesn't look like a region, treat whole thing as log group
        let parsed = RegionalLogGroup::parse("my-app:production");
        assert_eq!(parsed.region, None);
        assert_eq!(parsed.log_group, "my-app:production");
    }

    #[test]
    fn test_valid_region_format() {
        assert!(is_valid_region_format("us-east-1"));
        assert!(is_valid_region_format("ap-southeast-2"));
        assert!(is_valid_region_format("ap-northeast-1"));
        assert!(is_valid_region_format("eu-west-1"));

        assert!(!is_valid_region_format("production"));
        assert!(!is_valid_region_format("my-app"));
        assert!(!is_valid_region_format("us-east")); // missing number
    }
}
