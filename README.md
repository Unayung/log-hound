# 🐕 Log Hound

**The AI-first AWS CloudWatch log search tool.**

Built for seamless integration with AI coding assistants like Claude, Cursor, and Copilot. Search your production logs with natural, readable output that AI can understand and act upon.

## Why AI-First?

Traditional log tools output noisy, hard-to-parse data. Log Hound is designed differently:

- **Clean, structured output** - Results AI assistants can easily parse and analyze
- **Concise formatting** - No clutter, just the logs that matter
- **Natural language friendly** - Simple CLI that AI can invoke directly
- **Fast iteration** - Quick searches for rapid debugging with AI pair programming

## Features

- 🔍 **Fast Concurrent Search** - Query multiple log groups in parallel
- 🤖 **AI-Optimized Output** - Clean, parseable results for AI analysis
- 📊 **Multiple Output Modes** - Interleaved, grouped, or streaming
- 🖥️ **Interactive TUI** - Terminal UI for manual exploration
- ⏰ **Flexible Time Ranges** - Relative (`1h`, `30m`, `2d`) or absolute
- 🔗 **AND Conditions** - Multiple patterns combined with AND logic
- 🌍 **Multi-Region Support** - Search across AWS regions

## Installation

```bash
git clone https://github.com/willyjie23/log-hound.git
cd log-hound
cargo build --release

# Binary at ./target/release/log-hound
```

**Prerequisites:** Rust 1.70+, AWS credentials configured

## Usage

### Search Logs

```bash
# Basic search
log-hound search "ERROR" -g my-app/production

# Multiple log groups
log-hound search "timeout" -g api/logs,web/logs --last 2h

# AND condition
log-hound search "ERROR" "user_id=123" -g app/logs

# With limit
log-hound search "exception" -g service/prod --last 4h --limit 50
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

## Time Formats

**Relative:** `30m`, `1h`, `2d`, `1w`

**Absolute:** `2024-01-20 10:00:00` or `2024-01-20`

## Example: AI Debugging Session

```bash
# AI asks: "Find recent errors in production"
log-hound search "ERROR" -g pluto/production --last 1h

# AI asks: "Search for this specific user's activity"  
log-hound search "user_id=12345" -g api/prod,web/prod --last 24h

# AI asks: "Check for timeout issues"
log-hound search "timeout" "connection" -g service/prod --limit 200
```

## License

MIT
