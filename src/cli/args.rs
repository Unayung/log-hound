use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "log-hound")]
#[command(about = "Search AWS CloudWatch Log Insights from your terminal")]
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
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search logs with a filter string
    #[command(after_help = "Examples:
  log-hound search \"ERROR\" -g my-app/production
  log-hound search \"user_id=123\" -g api/logs,web/logs --last 2h
  log-hound search \"timeout\" -g service/prod --limit 50 -o grouped
  log-hound search \"ERROR\" \"user_id=123\" -g app/logs  # AND condition")]
    Search {
        /// Search patterns to match in @message (multiple = AND condition)
        #[arg(required = true)]
        patterns: Vec<String>,

        /// Log groups to search (required, comma-separated for multiple)
        #[arg(short, long, value_delimiter = ',', required = true)]
        groups: Vec<String>,

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
}

#[derive(ValueEnum, Clone, Debug, Default)]
pub enum OutputMode {
    /// Results merged and sorted by timestamp
    #[default]
    Interleaved,
    /// Results grouped by log group source
    Grouped,
    /// Results displayed as they arrive
    Streaming,
}
