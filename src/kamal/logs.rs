use crate::aws::LogEntry;
use crate::kamal::KamalConfig;
use crate::output;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use openssh::{KnownHosts, Session, SessionBuilder};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Search parameters for Kamal logs
#[derive(Debug, Clone)]
pub struct KamalSearchParams {
    pub patterns: Vec<String>,
    pub exclude: Vec<String>,
    pub limit: usize,
    pub since: Option<String>,
    pub follow: bool,
}

/// Searcher for Kamal-deployed Docker container logs
pub struct KamalSearcher {
    config: KamalConfig,
}

impl KamalSearcher {
    /// Create a new KamalSearcher from a deploy.yml path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = KamalConfig::load(path)?;
        Ok(Self { config })
    }

    /// Create a new KamalSearcher from an existing config
    pub fn new(config: KamalConfig) -> Self {
        Self { config }
    }

    /// Search logs from all configured servers
    pub async fn search_logs(
        &self,
        params: &KamalSearchParams,
    ) -> Vec<Result<Vec<LogEntry>>> {
        let futures: Vec<_> = self
            .config
            .servers
            .iter()
            .map(|server| self.search_server_logs(server, params))
            .collect();

        futures::future::join_all(futures).await
    }

    /// Follow logs from the primary server (first in list)
    /// Streams logs in real-time until interrupted
    pub async fn follow_logs(&self, params: &KamalSearchParams) -> Result<()> {
        use std::process::Stdio;
        use tokio::process::Command;

        let server = self.config.servers.first()
            .ok_or_else(|| anyhow!("No servers configured"))?;

        // First, get the container ID via SSH
        let session = self.connect_ssh(server).await?;
        let container_id = self.find_container(&session).await?;
        session.close().await?;

        // Build docker logs -f command
        let mut docker_cmd = format!("docker logs {} --timestamps -f", container_id);
        if let Some(since) = &params.since {
            docker_cmd.push_str(&format!(" --since {}", since));
        }

        if std::env::var("LOG_HOUND_DEBUG").is_ok() {
            eprintln!("DEBUG: SSH command: ssh {}@{} {}", self.config.ssh_user, server, docker_cmd);
        }

        // Use tokio::process::Command with ssh directly for streaming
        let destination = format!("{}@{}", self.config.ssh_user, server);
        let mut child = Command::new("ssh")
            .arg("-tt")  // Force pseudo-terminal allocation for proper streaming
            .arg(&destination)
            .arg(&docker_cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn ssh process")?;

        // With -tt, docker logs output goes to stdout via the pseudo-terminal
        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow!("Failed to capture stdout"))?;

        let mut reader = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(entry) = self.parse_log_line(&line, server) {
                // Apply filters
                let matches_include = params.patterns.is_empty()
                    || params.patterns.iter()
                        .all(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                let matches_exclude = params.exclude.iter()
                    .any(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                if matches_include && !matches_exclude {
                    output::print_entry(&entry);
                }
            }
        }

        Ok(())
    }

    /// Follow logs and send entries through a channel (for TUI integration)
    /// Returns immediately after spawning the background task
    /// Use the stop_flag to signal when to stop following
    pub async fn follow_logs_channel(
        &self,
        params: &KamalSearchParams,
        sender: mpsc::Sender<LogEntry>,
        stop_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        use std::process::Stdio;
        use tokio::process::Command;

        let server = self.config.servers.first()
            .ok_or_else(|| anyhow!("No servers configured"))?;

        // First, get the container ID via SSH
        let session = self.connect_ssh(server).await?;
        let container_id = self.find_container(&session).await?;
        session.close().await?;

        // Build docker logs -f command
        let mut docker_cmd = format!("docker logs {} --timestamps -f", container_id);
        if let Some(since) = &params.since {
            docker_cmd.push_str(&format!(" --since {}", since));
        }

        // Use tokio::process::Command with ssh directly for streaming
        let destination = format!("{}@{}", self.config.ssh_user, server);
        let mut child = Command::new("ssh")
            .arg("-tt")
            .arg(&destination)
            .arg(&docker_cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn ssh process")?;

        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow!("Failed to capture stdout"))?;

        let mut reader = BufReader::new(stdout).lines();
        let server = server.to_string();
        let patterns = params.patterns.clone();
        let exclude = params.exclude.clone();
        let service = self.config.service.clone();

        // Spawn task to read lines and send through channel
        tokio::spawn(async move {
            while !stop_flag.load(Ordering::Relaxed) {
                tokio::select! {
                    line_result = reader.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                if let Some(entry) = parse_log_line_static(&line, &server, &service) {
                                    // Apply filters
                                    let matches_include = patterns.is_empty()
                                        || patterns.iter()
                                            .all(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                                    let matches_exclude = exclude.iter()
                                        .any(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                                    if matches_include && !matches_exclude {
                                        if sender.send(entry).await.is_err() {
                                            break; // Receiver dropped
                                        }
                                    }
                                }
                            }
                            Ok(None) => break, // EOF
                            Err(_) => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Check stop flag periodically
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                }
            }
            // Kill the child process when done
            let _ = child.kill().await;
        });

        Ok(())
    }

    /// Search logs from a single server
    async fn search_server_logs(
        &self,
        server: &str,
        params: &KamalSearchParams,
    ) -> Result<Vec<LogEntry>> {
        // Connect via SSH
        let session = self.connect_ssh(server).await?;

        // Find the running container
        let container_id = self.find_container(&session).await?;

        // Fetch docker logs
        let raw_logs = self.fetch_docker_logs(&session, &container_id, params).await?;

        // Parse logs into LogEntry format
        let entries = self.parse_logs(&raw_logs, server, params)?;

        session.close().await?;

        Ok(entries)
    }

    /// Establish SSH connection to a server
    async fn connect_ssh(&self, server: &str) -> Result<Session> {
        let destination = format!("{}@{}", self.config.ssh_user, server);

        let session = SessionBuilder::default()
            .known_hosts_check(KnownHosts::Accept)
            .connect_timeout(Duration::from_secs(10))
            .connect(&destination)
            .await
            .with_context(|| format!("Failed to SSH to {}", destination))?;

        Ok(session)
    }

    /// Find the running container ID for the service
    async fn find_container(&self, session: &Session) -> Result<String> {
        // Find containers matching the service name pattern
        let cmd = format!(
            "docker ps --filter 'name={}' --format '{{{{.ID}}}}' | head -1",
            self.config.service
        );

        let output = session
            .command("bash")
            .arg("-c")
            .arg(&cmd)
            .output()
            .await
            .context("Failed to execute docker ps")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("docker ps failed: {}", stderr));
        }

        let container_id = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();

        if container_id.is_empty() {
            return Err(anyhow!(
                "No running container found for service: {}",
                self.config.service
            ));
        }

        Ok(container_id)
    }

    /// Fetch docker logs from a container
    async fn fetch_docker_logs(
        &self,
        session: &Session,
        container_id: &str,
        params: &KamalSearchParams,
    ) -> Result<String> {
        // Build docker logs command
        let mut cmd = format!("docker logs {} --timestamps", container_id);

        // Add --since if specified
        if let Some(since) = &params.since {
            cmd.push_str(&format!(" --since {}", since));
        }

        // Add tail limit (fetch more than needed for filtering)
        let fetch_limit = params.limit * 10; // Over-fetch to account for filtering
        cmd.push_str(&format!(" --tail {}", fetch_limit.max(1000)));

        // Execute and capture output
        let output = session
            .command("bash")
            .arg("-c")
            .arg(&cmd)
            .output()
            .await
            .context("Failed to execute docker logs")?;

        // Docker logs outputs to stderr for non-error logs too
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Combine stdout and stderr (docker logs can output to both)
        Ok(format!("{}{}", stdout, stderr))
    }

    /// Parse raw docker logs into LogEntry format with filtering
    fn parse_logs(
        &self,
        raw_logs: &str,
        server: &str,
        params: &KamalSearchParams,
    ) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();

        for line in raw_logs.lines() {
            // Skip empty lines
            if line.trim().is_empty() {
                continue;
            }

            // Parse the log line
            if let Some(entry) = self.parse_log_line(line, server) {
                // Apply include filters (AND condition)
                let matches_include = params.patterns.is_empty()
                    || params
                        .patterns
                        .iter()
                        .all(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                // Apply exclude filters
                let matches_exclude = params
                    .exclude
                    .iter()
                    .any(|p| entry.message.to_lowercase().contains(&p.to_lowercase()));

                if matches_include && !matches_exclude {
                    entries.push(entry);
                }
            }
        }

        // Sort by timestamp (newest first) and limit
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        entries.truncate(params.limit);

        Ok(entries)
    }

    /// Parse a single docker log line
    /// Format: 2026-01-31T12:34:56.789012345Z <message>
    fn parse_log_line(&self, line: &str, server: &str) -> Option<LogEntry> {
        // Docker timestamps are in RFC3339 format at the start
        // Example: 2026-01-31T12:34:56.789012345Z I, [2026-01-31...

        // Find the timestamp (first space separates timestamp from message)
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            // No timestamp, treat whole line as message
            return Some(LogEntry {
                timestamp: Utc::now(),
                message: line.to_string(),
                log_group: format!("kamal:{}", server),
                log_stream: Some(self.config.service.clone()),
                region: None,
            });
        }

        let timestamp_str = parts[0];
        let message = parts[1].to_string();

        // Parse Docker timestamp (RFC3339 with nanoseconds)
        let timestamp = parse_docker_timestamp(timestamp_str).unwrap_or_else(Utc::now);

        Some(LogEntry {
            timestamp,
            message,
            log_group: format!("kamal:{}", server),
            log_stream: Some(self.config.service.clone()),
            region: None,
        })
    }

    /// Get list of servers from config (for display)
    pub fn servers(&self) -> &[String] {
        &self.config.servers
    }

    /// Get service name
    pub fn service(&self) -> &str {
        &self.config.service
    }
}

/// Parse Docker's RFC3339 timestamp with nanoseconds
fn parse_docker_timestamp(s: &str) -> Option<DateTime<Utc>> {
    // Docker format: 2026-01-31T12:34:56.789012345Z
    // chrono can't handle 9-digit nanoseconds directly, so we truncate to 6

    // Try standard RFC3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try truncating nanoseconds if too long
    if let Some(dot_pos) = s.rfind('.') {
        if let Some(z_pos) = s.rfind('Z') {
            if z_pos > dot_pos + 7 {
                // Truncate to 6 decimal places
                let truncated = format!("{}{}Z", &s[..dot_pos + 7], "");
                if let Ok(dt) = DateTime::parse_from_rfc3339(&truncated) {
                    return Some(dt.with_timezone(&Utc));
                }
            }
        }
    }

    // Try without fractional seconds
    let formats = [
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%d %H:%M:%S",
    ];

    for fmt in formats {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), fmt.trim_end_matches('Z')) {
            return Some(naive.and_utc());
        }
    }

    None
}

/// Static version of parse_log_line for use in spawned tasks
fn parse_log_line_static(line: &str, server: &str, service: &str) -> Option<LogEntry> {
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return Some(LogEntry {
            timestamp: Utc::now(),
            message: line.to_string(),
            log_group: format!("kamal:{}", server),
            log_stream: Some(service.to_string()),
            region: None,
        });
    }

    let timestamp_str = parts[0];
    let message = parts[1].to_string();
    let timestamp = parse_docker_timestamp(timestamp_str).unwrap_or_else(Utc::now);

    Some(LogEntry {
        timestamp,
        message,
        log_group: format!("kamal:{}", server),
        log_stream: Some(service.to_string()),
        region: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_docker_timestamp() {
        // Standard Docker format
        let ts = parse_docker_timestamp("2026-01-31T12:34:56.789012345Z");
        assert!(ts.is_some());

        // Short format
        let ts = parse_docker_timestamp("2026-01-31T12:34:56Z");
        assert!(ts.is_some());
    }
}
