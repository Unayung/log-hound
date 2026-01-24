use crate::aws::multi_region::{MultiRegionClientPool, RegionalLogGroup};
use anyhow::{anyhow, Result};
use aws_sdk_cloudwatchlogs::Client;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub log_group: String,
    pub log_stream: Option<String>,
    pub region: Option<String>,
}

pub struct LogSearcher {
    client: Client,
}

impl LogSearcher {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// List log groups, optionally filtered by prefix
    pub async fn list_log_groups(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let mut log_groups = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut request = self.client.describe_log_groups();

            if let Some(p) = prefix {
                request = request.log_group_name_prefix(p);
            }

            if let Some(token) = next_token {
                request = request.next_token(token);
            }

            let response = request.send().await?;

            if let Some(groups) = response.log_groups {
                for group in groups {
                    if let Some(name) = group.log_group_name {
                        log_groups.push(name);
                    }
                }
            }

            next_token = response.next_token;
            if next_token.is_none() {
                break;
            }
        }

        Ok(log_groups)
    }

    /// Search a single log group with the given patterns (AND condition)
    pub async fn search_log_group(
        &self,
        log_group: &str,
        patterns: &[String],
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: i32,
    ) -> Result<Vec<LogEntry>> {
        let query = build_insights_query(patterns, limit);

        if std::env::var("LOG_HOUND_DEBUG").is_ok() {
            eprintln!("DEBUG: Log group: {}", log_group);
            eprintln!("DEBUG: Query:\n{}", query);
        }

        let start_epoch = start_time.timestamp();
        let end_epoch = end_time.timestamp();

        // Start the query
        let start_response = self
            .client
            .start_query()
            .log_group_name(log_group)
            .start_time(start_epoch)
            .end_time(end_epoch)
            .query_string(&query)
            .send()
            .await?;

        let query_id = start_response
            .query_id
            .ok_or_else(|| anyhow!("No query ID returned"))?;

        // Poll for results
        let results = self.poll_query_results(&query_id, log_group).await?;

        Ok(results)
    }

    async fn poll_query_results(
        &self,
        query_id: &str,
        log_group: &str,
    ) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();

        loop {
            let response = self
                .client
                .get_query_results()
                .query_id(query_id)
                .send()
                .await?;

            let status = response
                .status
                .map(|s| s.as_str().to_string())
                .unwrap_or_default();

            match status.as_str() {
                "Complete" => {
                    if let Some(results) = response.results {
                        for result in results {
                            let entry = parse_log_result(&result, log_group);
                            if let Some(e) = entry {
                                entries.push(e);
                            }
                        }
                    }
                    break;
                }
                "Failed" | "Cancelled" | "Timeout" => {
                    return Err(anyhow!("Query {}: {}", query_id, status));
                }
                _ => {
                    // Still running, wait and poll again
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        Ok(entries)
    }
}

/// Build CloudWatch Insights query for message filtering (multiple patterns = AND)
fn build_insights_query(patterns: &[String], limit: i32) -> String {
    let filter_conditions: Vec<String> = patterns
        .iter()
        .map(|p| {
            let escaped = p.replace('\'', "\\'");
            format!("@message like /{}/", escaped)
        })
        .collect();

    let filter_clause = filter_conditions.join(" and ");

    format!(
        r#"fields @timestamp, @message, @logStream
| filter {}
| sort @timestamp desc
| limit {}"#,
        filter_clause, limit
    )
}

/// Parse a Log Insights result row into a LogEntry
fn parse_log_result(
    result: &[aws_sdk_cloudwatchlogs::types::ResultField],
    log_group: &str,
) -> Option<LogEntry> {
    let mut timestamp: Option<DateTime<Utc>> = None;
    let mut message: Option<String> = None;
    let mut log_stream: Option<String> = None;

    for field in result {
        match field.field.as_deref() {
            Some("@timestamp") => {
                if let Some(val) = &field.value {
                    // CloudWatch Insights returns timestamps like "2026-01-23 05:36:05.200"
                    timestamp = parse_cloudwatch_timestamp(val);
                }
            }
            Some("@message") => {
                message = field.value.clone();
            }
            Some("@logStream") => {
                log_stream = field.value.clone();
            }
            _ => {}
        }
    }

    Some(LogEntry {
        timestamp: timestamp?,
        message: message?,
        log_group: log_group.to_string(),
        log_stream,
        region: None,
    })
}

/// Parse CloudWatch Insights timestamp format: "2026-01-23 05:36:05.200"
fn parse_cloudwatch_timestamp(val: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 first (in case format changes)
    if let Ok(dt) = DateTime::parse_from_rfc3339(val) {
        return Some(dt.with_timezone(&Utc));
    }

    // CloudWatch format: "2026-01-23 05:36:05.200"
    let formats = [
        "%Y-%m-%d %H:%M:%S%.3f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ];

    for fmt in formats {
        if let Ok(naive) = NaiveDateTime::parse_from_str(val, fmt) {
            return Some(naive.and_utc());
        }
    }

    None
}

/// Multi-region log searcher that can search across different AWS regions
pub struct MultiRegionSearcher {
    client_pool: MultiRegionClientPool,
}

impl MultiRegionSearcher {
    pub fn new(profile: Option<String>, default_region: Option<String>) -> Self {
        Self {
            client_pool: MultiRegionClientPool::new(profile, default_region),
        }
    }

    /// Search multiple log groups, potentially across different regions
    pub async fn search_log_groups(
        &self,
        log_groups: &[String],
        patterns: &[String],
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: i32,
    ) -> Vec<Result<Vec<LogEntry>>> {
        let regional_groups = RegionalLogGroup::parse_many(log_groups);

        let futures: Vec<_> = regional_groups
            .into_iter()
            .map(|rg| {
                self.search_single_log_group(
                    rg,
                    patterns.to_vec(),
                    start_time,
                    end_time,
                    limit,
                )
            })
            .collect();

        futures::future::join_all(futures).await
    }

    async fn search_single_log_group(
        &self,
        regional_group: RegionalLogGroup,
        patterns: Vec<String>,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: i32,
    ) -> Result<Vec<LogEntry>> {
        let client = self
            .client_pool
            .get_client(regional_group.region.as_deref())
            .await?;

        let query = build_insights_query(&patterns, limit);

        if std::env::var("LOG_HOUND_DEBUG").is_ok() {
            eprintln!(
                "DEBUG: Region: {:?}, Log group: {}",
                regional_group.region, regional_group.log_group
            );
            eprintln!("DEBUG: Query:\n{}", query);
        }

        let start_epoch = start_time.timestamp();
        let end_epoch = end_time.timestamp();

        // Start the query
        let start_response = client
            .start_query()
            .log_group_name(&regional_group.log_group)
            .start_time(start_epoch)
            .end_time(end_epoch)
            .query_string(&query)
            .send()
            .await?;

        let query_id = start_response
            .query_id
            .ok_or_else(|| anyhow!("No query ID returned"))?;

        // Poll for results
        let mut entries = Vec::new();

        loop {
            let response = client
                .get_query_results()
                .query_id(&query_id)
                .send()
                .await?;

            let status = response
                .status
                .map(|s| s.as_str().to_string())
                .unwrap_or_default();

            match status.as_str() {
                "Complete" => {
                    if let Some(results) = response.results {
                        for result in results {
                            if let Some(mut entry) =
                                parse_log_result(&result, &regional_group.log_group)
                            {
                                entry.region = regional_group.region.clone();
                                entries.push(entry);
                            }
                        }
                    }
                    break;
                }
                "Failed" | "Cancelled" | "Timeout" => {
                    return Err(anyhow!("Query {}: {}", query_id, status));
                }
                _ => {
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        Ok(entries)
    }

    /// List log groups from a specific region
    pub async fn list_log_groups(
        &self,
        region: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Vec<String>> {
        let client = self.client_pool.get_client(region).await?;

        let mut log_groups = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut request = client.describe_log_groups();

            if let Some(p) = prefix {
                request = request.log_group_name_prefix(p);
            }

            if let Some(token) = next_token {
                request = request.next_token(token);
            }

            let response = request.send().await?;

            if let Some(groups) = response.log_groups {
                for group in groups {
                    if let Some(name) = group.log_group_name {
                        log_groups.push(name);
                    }
                }
            }

            next_token = response.next_token;
            if next_token.is_none() {
                break;
            }
        }

        Ok(log_groups)
    }
}
