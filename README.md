# 🐕 Log Hound

A fast CLI tool for searching AWS CloudWatch Log Insights from your terminal.

## Features

- 🔍 **Fast Search** - Query multiple log groups concurrently
- 📊 **Multiple Output Modes** - Interleaved, grouped, or streaming results
- 🖥️ **Interactive TUI** - Terminal UI for exploring logs
- ⏰ **Flexible Time Ranges** - Relative (`1h`, `30m`, `2d`) or absolute timestamps
- 🔗 **AND Conditions** - Multiple patterns are combined with AND logic
- 🌍 **Multi-Region Support** - Search across different AWS regions
- 🎨 **Colored Output** - Easy-to-read formatted results

## Installation

### From Source

```bash
git clone https://github.com/willyjie23/log-hound.git
cd log-hound
cargo build --release

# Binary will be at ./target/release/log-hound
```

### Prerequisites

- Rust toolchain (1.70+)
- AWS credentials configured (`~/.aws/credentials` or environment variables)

## Usage

### Search Logs

```bash
# Basic search
log-hound search "ERROR" -g my-app/production

# Search multiple log groups
log-hound search "timeout" -g api/logs,web/logs --last 2h

# AND condition (multiple patterns)
log-hound search "ERROR" "user_id=123" -g app/logs

# Custom time range
log-hound search "exception" -g service/prod --last 4h --limit 50

# Grouped output
log-hound search "ERROR" -g app/prod,app/staging -o grouped

# Absolute time range
log-hound search "crash" -g app/logs --start "2024-01-20 10:00:00" --end "2024-01-20 12:00:00"
```

### List Log Groups

```bash
# List all log groups
log-hound groups

# Filter by prefix
log-hound groups --prefix pluto/
```

### Interactive TUI

```bash
log-hound tui
```

Launch an interactive terminal UI for exploring and searching logs.

### AWS Configuration

```bash
# Use specific AWS profile
log-hound --profile production search "ERROR" -g app/logs

# Use specific region
log-hound --region us-west-2 search "ERROR" -g app/logs

# Or use environment variables
export AWS_PROFILE=production
export AWS_REGION=ap-northeast-1
log-hound search "ERROR" -g app/logs
```

## Output Modes

| Mode | Description |
|------|-------------|
| `interleaved` | Results merged and sorted by timestamp (default) |
| `grouped` | Results grouped by log group source |
| `streaming` | Results displayed as they arrive |

## Time Range Formats

### Relative (--last)

- `30m` - Last 30 minutes
- `1h` - Last 1 hour
- `2d` - Last 2 days
- `1w` - Last 1 week

### Absolute (--start / --end)

- `2024-01-20 10:00:00`
- `2024-01-20T10:00:00`
- `2024-01-20` (defaults to 00:00:00)

## Examples

```bash
# Find all errors in production in the last hour
log-hound search "ERROR" -g pluto/production --last 1h

# Find specific user activity across multiple services
log-hound search "user_id=12345" -g api/prod,web/prod,worker/prod --last 24h

# Search for timeout errors with grouped output
log-hound search "timeout" "connection" -g service/prod -o grouped --limit 200

# List all pluto-related log groups
log-hound groups --prefix pluto/
```

## License

MIT
