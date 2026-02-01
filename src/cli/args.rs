use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "log-hound")]
#[command(about = "Search logs from AWS CloudWatch or Kamal deployments")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// AWS profile to use (from ~/.aws/credentials)
    #[arg(long, global = true, env = "AWS_PROFILE")]
    pub profile: Option<String>,

    /// AWS region
    #[arg(long, global = true, env = "AWS_REGION")]
    pub region: Option<String>,

    /// Log source to use
    #[arg(long, global = true, default_value = "cloudwatch")]
    pub source: LogSource,
}

#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
pub enum LogSource {
    /// AWS CloudWatch Log Insights
    #[default]
    Cloudwatch,
    /// Kamal-deployed Docker containers via SSH
    Kamal,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search logs with a filter string
    #[command(after_help = "Examples:
  # CloudWatch (default):
  log-hound search \"ERROR\" -g my-app/production
  log-hound search \"user_id=123\" -g api/logs,web/logs --last 2h
  log-hound search \"ERROR\" -p production  # Use preset

  # Kamal deployments:
  log-hound search \"ERROR\" --source kamal -d config/deploy.yml
  log-hound search \"timeout\" --source kamal -d config/deploy.saiens.yml --last 30m
  log-hound search -p saiens \"ERROR\"  # Preset with kamal source
  log-hound search --source kamal -d config/deploy.yml -f  # Follow/tail logs live")]
    Search {
        /// Search patterns to match in @message (multiple = AND condition)
        #[arg(required_unless_present = "preset")]
        patterns: Vec<String>,

        /// Log groups to search (CloudWatch) - comma-separated for multiple
        #[arg(short, long, value_delimiter = ',')]
        groups: Vec<String>,

        /// Kamal deploy.yml file path (for --source kamal)
        #[arg(short = 'd', long = "deploy")]
        deploy_file: Option<String>,

        /// Use a saved preset from config
        #[arg(short, long)]
        preset: Option<String>,

        /// Exclude patterns (NOT condition, comma-separated)
        #[arg(short = 'x', long, value_delimiter = ',')]
        exclude: Vec<String>,

        /// Time range: e.g., "1h", "30m", "2d"
        #[arg(short, long, default_value = "1h")]
        last: String,

        /// Start time (alternative to --last)
        #[arg(long)]
        start: Option<String>,

        /// End time (used with --start)
        #[arg(long)]
        end: Option<String>,

        /// Output mode for results
        #[arg(short, long, default_value = "interleaved")]
        output: OutputMode,

        /// Maximum number of results per log group
        #[arg(long, default_value = "100")]
        limit: i32,

        /// Follow/tail logs in real-time (Kamal source only)
        #[arg(short = 'f', long)]
        follow: bool,
    },

    /// List available log groups
    Groups {
        /// Filter log groups by prefix
        #[arg(short, long)]
        prefix: Option<String>,
    },

    /// Launch interactive TUI mode
    #[command(alias = "ui")]
    Tui,

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Show config file path
    Path,
    /// Generate a sample configuration file
    Init,
    /// List available presets
    Presets,
}

#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
pub enum OutputMode {
    /// Results merged and sorted by timestamp
    #[default]
    Interleaved,
    /// Results grouped by log group source
    Grouped,
    /// Results displayed as they arrive
    Streaming,
    /// JSON output for AI/programmatic use
    Json,
}
