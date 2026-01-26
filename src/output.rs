use crate::aws::LogEntry;
use crate::cli::OutputMode;
use colored::Colorize;
use serde::Serialize;

/// JSON output wrapper for structured results
#[derive(Serialize)]
struct JsonOutput {
    total: usize,
    results: Vec<LogEntry>,
}

/// Format and display log entries based on the selected output mode
pub fn display_results(entries: Vec<LogEntry>, mode: &OutputMode) {
    if entries.is_empty() {
        if *mode == OutputMode::Json {
            println!("{}", serde_json::to_string_pretty(&JsonOutput {
                total: 0,
                results: vec![],
            }).unwrap_or_default());
        } else {
            println!("{}", "No matching logs found.".yellow());
        }
        return;
    }

    match mode {
        OutputMode::Interleaved => display_interleaved(entries),
        OutputMode::Grouped => display_grouped(entries),
        OutputMode::Json => display_json(entries),
        OutputMode::Streaming => {
            // Streaming mode displays as results arrive (handled differently)
            // When called here, just display interleaved as fallback
            display_interleaved(entries)
        }
    }
}

fn display_interleaved(mut entries: Vec<LogEntry>) {
    // Sort all entries by timestamp
    entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    println!(
        "{} {} results:\n",
        "Found".green(),
        entries.len().to_string().cyan()
    );

    for entry in entries {
        print_entry(&entry);
    }
}

fn display_grouped(entries: Vec<LogEntry>) {
    use std::collections::HashMap;

    let mut by_group: HashMap<String, Vec<LogEntry>> = HashMap::new();

    for entry in entries {
        by_group
            .entry(entry.log_group.clone())
            .or_default()
            .push(entry);
    }

    for (group_name, mut group_entries) in by_group {
        println!(
            "\n{} {} ({} results)",
            "━━━".blue(),
            group_name.cyan().bold(),
            group_entries.len()
        );
        println!();

        group_entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        for entry in group_entries {
            print_entry(&entry);
        }
    }
}

fn display_json(mut entries: Vec<LogEntry>) {
    // Sort by timestamp for consistent output
    entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    let output = JsonOutput {
        total: entries.len(),
        results: entries,
    };

    match serde_json::to_string_pretty(&output) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("Error serializing to JSON: {}", e),
    }
}

/// Print a single log entry with formatting
pub fn print_entry(entry: &LogEntry) {
    let timestamp = entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f");

    // Truncate log group to last segment for cleaner output
    let short_group = entry
        .log_group
        .rsplit('/')
        .next()
        .unwrap_or(&entry.log_group);

    // Include region if available
    let group_display = if let Some(ref region) = entry.region {
        format!("[{}:{}]", region, short_group)
    } else {
        format!("[{}]", short_group)
    };

    println!(
        "{} {} {}",
        timestamp.to_string().dimmed(),
        group_display.blue(),
        entry.message
    );
}

/// Print a single entry in JSON format (for streaming mode)
pub fn print_entry_json(entry: &LogEntry) {
    if let Ok(json) = serde_json::to_string(entry) {
        println!("{}", json);
    }
}
