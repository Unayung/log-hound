mod aws;
mod cli;
mod config;
mod output;
mod time;
mod tui;

use anyhow::Result;
use aws::SearchParams;
use clap::Parser;
use cli::{Cli, Commands, ConfigAction, OutputMode};
use colored::Colorize;
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load().unwrap_or_default();

    match cli.command {
        Commands::Search {
            patterns,
            groups,
            preset,
            exclude,
            last,
            start,
            end,
            output,
            limit,
        } => {
            // Resolve preset if specified
            let (resolved_groups, resolved_patterns, resolved_exclude, resolved_last, resolved_limit) =
                if let Some(preset_name) = &preset {
                    match config.get_preset(preset_name) {
                        Some(p) => {
                            let mut final_patterns = p.patterns.clone();
                            final_patterns.extend(patterns);
                            
                            let mut final_exclude = p.exclude.clone();
                            final_exclude.extend(exclude);
                            
                            let final_groups = if groups.is_empty() {
                                p.groups.clone()
                            } else {
                                groups
                            };
                            
                            let final_last = p.time_range.clone().unwrap_or(last);
                            let final_limit = p.limit.unwrap_or(limit);
                            
                            (final_groups, final_patterns, final_exclude, final_last, final_limit)
                        }
                        None => {
                            eprintln!("{} Preset '{}' not found", "Error:".red(), preset_name);
                            eprintln!("Available presets: {:?}", config.presets.keys().collect::<Vec<_>>());
                            return Ok(());
                        }
                    }
                } else {
                    // Use config defaults if no groups specified
                    let final_groups = if groups.is_empty() {
                        config.default_groups.clone()
                    } else {
                        groups
                    };
                    (final_groups, patterns, exclude, last, limit)
                };

            let searcher = aws::MultiRegionSearcher::new(
                cli.profile.clone().or(config.default_profile.clone()),
                cli.region.clone().or(config.default_region.clone()),
            );

            run_search(
                &searcher,
                resolved_patterns,
                resolved_groups,
                resolved_exclude,
                resolved_last,
                start,
                end,
                output,
                resolved_limit,
            )
            .await?;
        }
        Commands::Groups { prefix } => {
            let client = aws::create_client(
                cli.profile.as_deref().or(config.default_profile.as_deref()),
                cli.region.as_deref().or(config.default_region.as_deref()),
            )
            .await?;
            let searcher = aws::LogSearcher::new(client);
            list_groups(&searcher, prefix).await?;
        }
        Commands::Tui => {
            let searcher = aws::MultiRegionSearcher::new(
                cli.profile.clone().or(config.default_profile.clone()),
                cli.region.clone().or(config.default_region.clone()),
            );
            tui::run_tui(searcher, config).await?;
        }
        Commands::Config { action } => {
            handle_config_command(action, &config)?;
        }
    }

    Ok(())
}

fn handle_config_command(action: ConfigAction, config: &Config) -> Result<()> {
    match action {
        ConfigAction::Show => {
            let path = Config::default_path();
            if path.exists() {
                let contents = std::fs::read_to_string(&path)?;
                println!("{}", contents);
            } else {
                println!("{}", "No config file found.".yellow());
                println!("Run 'log-hound config init' to create one.");
            }
        }
        ConfigAction::Path => {
            println!("{}", Config::default_path().display());
        }
        ConfigAction::Init => {
            let path = Config::default_path();
            if path.exists() {
                eprintln!("{} Config file already exists at {:?}", "Warning:".yellow(), path);
                eprintln!("Remove it first if you want to regenerate.");
            } else {
                std::fs::write(&path, Config::create_sample())?;
                println!("{} Created config file at {:?}", "Success:".green(), path);
            }
        }
        ConfigAction::Presets => {
            let presets = config.list_presets();
            if presets.is_empty() {
                println!("{}", "No presets configured.".yellow());
                println!("Add presets to your config file (~/.log-hound.toml)");
            } else {
                println!("{}\n", "Available presets:".cyan().bold());
                for (name, preset) in presets {
                    println!("  {} {}", name.green().bold(), 
                        preset.description.as_deref().unwrap_or("").dimmed());
                    println!("    Groups: {}", preset.groups.join(", ").dimmed());
                    if !preset.exclude.is_empty() {
                        println!("    Exclude: {}", preset.exclude.join(", ").dimmed());
                    }
                    println!();
                }
            }
        }
    }
    Ok(())
}

async fn run_search(
    searcher: &aws::MultiRegionSearcher,
    patterns: Vec<String>,
    groups: Vec<String>,
    exclude: Vec<String>,
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

    // Create search params
    let params = SearchParams::new(patterns.clone(), exclude.clone(), limit);

    // Format patterns for display (skip for JSON output)
    if output_mode != OutputMode::Json {
        let pattern_display = if patterns.is_empty() {
            "*".to_string()
        } else if patterns.len() == 1 {
            format!("'{}'", patterns[0])
        } else {
            patterns
                .iter()
                .map(|p| format!("'{}'", p))
                .collect::<Vec<_>>()
                .join(" AND ")
        };

        let exclude_display = if exclude.is_empty() {
            String::new()
        } else {
            format!(
                " {} {}",
                "NOT".red(),
                exclude
                    .iter()
                    .map(|p| format!("'{}'", p))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        println!(
            "{} {}{}  from {} to {}",
            "Searching".cyan(),
            pattern_display.yellow(),
            exclude_display,
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
    }

    if groups.is_empty() {
        if output_mode == OutputMode::Json {
            println!("{{\"error\": \"No log groups specified\"}}");
        }
        return Ok(());
    }

    // Search all log groups concurrently
    let mut all_entries = Vec::new();

    match output_mode {
        OutputMode::Streaming => {
            // For streaming, search sequentially to show results as they come
            let results = searcher
                .search_log_groups(&groups, &params, time_range.start, time_range.end)
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
            // For interleaved/grouped/json, collect all results first
            let results = searcher
                .search_log_groups(&groups, &params, time_range.start, time_range.end)
                .await;

            for (group, result) in groups.iter().zip(results) {
                match result {
                    Ok(entries) => all_entries.extend(entries),
                    Err(e) => {
                        if output_mode != OutputMode::Json {
                            eprintln!("{} {}: {}", "Error".red(), group, e);
                        }
                    }
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
