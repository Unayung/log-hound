# 🐕 Log Hound

**The AI-first AWS CloudWatch log search tool.**

Built for seamless integration with AI coding assistants like Claude, Cursor, and Copilot. Search your production logs across multiple AWS regions with natural, readable output that AI can understand and act upon.

## Why AI-First?

Traditional log tools output noisy, hard-to-parse data. Log Hound is designed differently:

- **Clean, structured output** - Results AI assistants can easily parse and analyze
- **JSON output mode** - Perfect for programmatic access and AI analysis
- **Concise formatting** - No clutter, just the logs that matter
- **Natural language friendly** - Simple CLI that AI can invoke directly
- **Fast iteration** - Quick searches for rapid debugging with AI pair programming

## Features

- 🌍 **Cross-Region Search** - Query multiple AWS regions in a single command
- 🔍 **Fast Concurrent Search** - Query multiple log groups in parallel
- 🤖 **AI-Optimized Output** - Clean, parseable results for AI analysis (with JSON mode)
- 📊 **Multiple Output Modes** - Interleaved, grouped, streaming, or JSON
- 🖥️ **Interactive TUI** - Terminal UI for manual exploration
- ⏰ **Flexible Time Ranges** - Relative (`1h`, `30m`, `2d`) or absolute
- 🔗 **AND Conditions** - Multiple patterns combined with AND logic
- ❌ **Exclude Patterns** - Filter out noisy logs (health checks, etc.)
- 📋 **Presets & Config** - Save common searches for quick access

## Installation

```bash
git clone https://github.com/willyjie23/log-hound.git
cd log-hound
cargo build --release

# Binary at ./target/release/log-hound
```

**Prerequisites:** Rust 1.70+, AWS credentials configured

## Usage

### Basic Search

```bash
# Single log group
log-hound search "ERROR" -g my-app/production

# Multiple log groups
log-hound search "timeout" -g api/logs,web/logs --last 2h

# AND condition (multiple patterns)
log-hound search "ERROR" "user_id=123" -g app/logs

# With limit
log-hound search "exception" -g service/prod --last 4h --limit 50
```

### Exclude Patterns

Filter out noisy logs that you don't want to see:

```bash
# Exclude health checks
log-hound search "ERROR" -g app/prod --exclude health-check

# Exclude multiple patterns
log-hound search "ERROR" -g app/prod -x "health-check,ping,warmup"

# Combine with AND patterns
log-hound search "ERROR" "database" -g api/logs -x "retry,reconnect"
```

### Cross-Region Search

```bash
# Search across regions in one command
log-hound search "ERROR" -g us-east-1:app/prod,ap-northeast-1:app/prod --last 1h

# Compare logs between regions
log-hound search "timeout" -g us-west-2:api/logs,eu-west-1:api/logs --last 2h
```

### JSON Output (AI-Friendly)

```bash
# Get results in JSON format for programmatic access
log-hound search "ERROR" -g app/prod -o json --limit 50

# Perfect for piping to AI tools
log-hound search "ERROR" -g app/prod -o json | jq '.entries | length'
```

### Presets & Configuration

Save common searches for quick access:

```bash
# Initialize config file
log-hound config init

# View current configuration
log-hound config show

# List available presets
log-hound config presets

# Use a preset
log-hound search "ERROR" -p production
log-hound search "timeout" -p staging
```

**Config file example** (`~/.log-hound.toml`):

```toml
# Default settings
default_time_range = "1h"
default_limit = 100

# Presets for quick access
[presets.production]
description = "Production environment"
groups = ["app/production", "api/production"]
time_range = "1h"
limit = 200

[presets.staging]
description = "Staging environment"
groups = ["app/staging", "api/staging"]
exclude = ["health-check", "ping"]  # Auto-filter noise

[presets.all-regions]
description = "Search across all regions"
groups = [
    "us-east-1:app/prod",
    "ap-northeast-1:app/prod",
    "eu-west-1:app/prod"
]
```

### List Log Groups

```bash
log-hound groups
log-hound groups --prefix pluto/
```

### Interactive TUI

```bash
log-hound tui
```

Features:
- Visual region and log group selection
- Preset quick-apply
- Exclude pattern support
- Real-time search
- Keyboard navigation
- Help overlay (F1)

### AWS Profile

```bash
log-hound --profile production search "ERROR" -g app/logs
log-hound --region us-west-2 groups
```

## Output Modes

| Mode | Description |
|------|-------------|
| `interleaved` | Merged and sorted by timestamp (default) |
| `grouped` | Grouped by log group source |
| `streaming` | Displayed as results arrive |
| `json` | JSON format for AI/programmatic use |

## Time Formats

**Relative:** `30m`, `1h`, `2d`, `1w`

**Absolute:** `2024-01-20 10:00:00` or `2024-01-20`

## Example: AI Debugging Session

```bash
# AI asks: "Find recent errors across all regions"
log-hound search "ERROR" -g us-east-1:app/prod,ap-northeast-1:app/prod --last 1h

# AI asks: "Search for this specific user's activity"  
log-hound search "user_id=12345" -g api/prod,web/prod --last 24h

# AI asks: "Check for timeout issues, excluding health checks"
log-hound search "timeout" -g service/prod -x health-check --limit 200

# AI asks: "Get logs in JSON format for analysis"
log-hound search "ERROR" -g app/prod -o json --limit 100 | jq '.entries[] | .message'

# Use preset for quick access
log-hound search "ERROR" -p production
```

## License

MIT
