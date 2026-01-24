mod aws;
mod cli;
mod output;
mod time;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, OutputMode};
use colored::Colorize;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Search {
            patterns,
            groups,
            last,
            start,
            end,
            output,
            limit,
        } => {
            let searcher = aws::MultiRegionSearcher::new(
                cli.profile.clone(),
                cli.region.clone(),
            );
            run_search(&searcher, patterns, groups, last, start, end, output, limit).await?;
        }
        Commands::Groups { prefix } => {
            let client = aws::create_client(cli.profile.as_deref(), cli.region.as_deref()).await?;
            let searcher = aws::LogSearcher::new(client);
            list_groups(&searcher, prefix).await?;
        }
        Commands::Tui => {
            let searcher = aws::MultiRegionSearcher::new(
                cli.profile.clone(),
                cli.region.clone(),
            );
            tui::run_tui(searcher).await?;
        }
    }

    Ok(())
}

async fn run_search(
    searcher: &aws::MultiRegionSearcher,
    patterns: Vec<String>,
    groups: Vec<String>,
    last: String,
    start: Option<String>,
    end: Option<String>,
    output_mode: OutputMode,
    limit: i32,
) -> Result<()> {
    // Determine time range
    let time_range = if let Some(start_str) = start {
        time::TimeRange::from_explicit(&start_str, end.as_deref())?
    } else {
        time::TimeRange::from_relative(&last)?
    };

    // Format patterns for display
    let pattern_display = if patterns.len() == 1 {
        format!("'{}'", patterns[0])
    } else {
        patterns
            .iter()
            .map(|p| format!("'{}'", p))
            .collect::<Vec<_>>()
            .join(" AND ")
    };

    println!(
        "{} {} from {} to {}",
        "Searching".cyan(),
        pattern_display.yellow(),
        time_range.start.format("%Y-%m-%d %H:%M:%S"),
        time_range.end.format("%Y-%m-%d %H:%M:%S"),
    );

    if groups.is_empty() {
        eprintln!(
            "{}",
            "No log groups specified. Use --groups or configure defaults.".red()
        );
        return Ok(());
    }

    println!("Log groups: {}\n", groups.join(", ").dimmed());

    // Search all log groups concurrently (supports cross-region with region:group syntax)
    let mut all_entries = Vec::new();

    match output_mode {
        OutputMode::Streaming => {
            // For streaming, search sequentially to show results as they come
            let results = searcher
                .search_log_groups(&groups, &patterns, time_range.start, time_range.end, limit)
                .await;

            for (group, result) in groups.iter().zip(results) {
                println!("{} {}...", "Querying".dimmed(), group.cyan());
                match result {
                    Ok(entries) => {
                        for entry in entries {
                            output::print_entry(&entry);
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}: {}", "Error".red(), group, e);
                    }
                }
            }
        }
        _ => {
            // For interleaved/grouped, collect all results first
            let results = searcher
                .search_log_groups(&groups, &patterns, time_range.start, time_range.end, limit)
                .await;

            for (group, result) in groups.iter().zip(results) {
                match result {
                    Ok(entries) => all_entries.extend(entries),
                    Err(e) => eprintln!("{} {}: {}", "Error".red(), group, e),
                }
            }

            output::display_results(all_entries, &output_mode);
        }
    }

    Ok(())
}

async fn list_groups(searcher: &aws::LogSearcher, prefix: Option<String>) -> Result<()> {
    println!("{}", "Fetching log groups...".dimmed());

    let groups = searcher.list_log_groups(prefix.as_deref()).await?;

    if groups.is_empty() {
        println!("{}", "No log groups found.".yellow());
    } else {
        println!("\n{} log groups:\n", groups.len().to_string().cyan());
        for group in groups {
            println!("  {}", group);
        }
    }

    Ok(())
}
