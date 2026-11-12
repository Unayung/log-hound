mod aws;
mod cli;
mod config;
mod kamal;
mod output;
mod time;
mod tui;

use anyhow::{anyhow, Result};
use aws::SearchParams;
use clap::Parser;
use cli::{Cli, Commands, ConfigAction, LogSource, OutputMode};
use colored::Colorize;
use config::Config;
use kamal::KamalSearchParams;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load().unwrap_or_default();

    match cli.command {
        Commands::Search {
            patterns,
            groups,
            deploy_file,
            preset,
            exclude,
            last,
            start,
            end,
            output,
            limit,
            follow,
            athena,
            max_scan_gb,
        } => {
            // Resolve preset if specified
            let (resolved_groups, resolved_patterns, resolved_exclude, resolved_last, resolved_limit, resolved_deploy, resolved_source) =
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

                            // Kamal presets can specify deploy file
                            let final_deploy = deploy_file.or(p.deploy_file.clone());
                            let final_source = if p.source.as_deref() == Some("kamal") {
                                LogSource::Kamal
                            } else {
                                cli.source.clone()
                            };

                            (final_groups, final_patterns, final_exclude, final_last, final_limit, final_deploy, final_source)
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
                    (final_groups, patterns, exclude, last, limit, deploy_file, cli.source.clone())
                };

            match resolved_source {
                LogSource::Cloudwatch => {
                    let searcher = aws::MultiRegionSearcher::new(
                        cli.profile.clone().or(config.default_profile.clone()),
                        cli.region.clone().or(config.default_region.clone()),
                    );

                    run_cloudwatch_search(
                        &searcher,
                        resolved_patterns,
                        resolved_groups,
                        resolved_exclude,
                        resolved_last,
                        start,
                        end,
                        output,
                        resolved_limit,
                        &config.athena,
                        athena,
                        max_scan_gb.or(config.max_scan_gb).unwrap_or(30.0),
                        config.athena_above_gb.unwrap_or(5.0),
                    )
                    .await?;
                }
                LogSource::Kamal => {
                    let deploy_path = resolved_deploy.unwrap_or_else(|| "config/deploy.yml".to_string());

                    run_kamal_search(
                        &deploy_path,
                        resolved_patterns,
                        resolved_exclude,
                        resolved_last,
                        output,
                        resolved_limit as usize,
                        follow,
                    )
                    .await?;
                }
            }
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
                    let source_tag = match preset.source.as_deref() {
                        Some("kamal") => "[kamal]".magenta().to_string(),
                        _ => "[cloudwatch]".blue().to_string(),
                    };
                    println!("  {} {} {}", name.green().bold(), source_tag,
                        preset.description.as_deref().unwrap_or("").dimmed());

                    // Show source-specific info
                    if preset.source.as_deref() == Some("kamal") {
                        if let Some(ref deploy) = preset.deploy_file {
                            println!("    Deploy: {}", deploy.dimmed());
                        }
                    } else if !preset.groups.is_empty() {
                        println!("    Groups: {}", preset.groups.join(", ").dimmed());
                    }

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

async fn run_cloudwatch_search(
    searcher: &aws::MultiRegionSearcher,
    patterns: Vec<String>,
    groups: Vec<String>,
    exclude: Vec<String>,
    last: String,
    start: Option<String>,
    end: Option<String>,
    output_mode: OutputMode,
    limit: i32,
    athena_targets: &std::collections::HashMap<String, config::AthenaTarget>,
    force_athena: bool,
    max_scan_gb: f64,
    athena_above_gb: f64,
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

    let total = groups.len();
    let mut failed = 0;
    let now = chrono::Utc::now();
    let athena_for = |g: &str| athena_targets.get(&aws::RegionalLogGroup::parse(g).log_group).cloned();

    // Time older than a group's retention is gone from CloudWatch, and Insights just
    // returns nothing for it: send it to the archive, or say so instead of a misleading
    // "No matching logs found."
    let retentions =
        futures::future::join_all(groups.iter().map(|g| searcher.retention_days(g))).await;
    let mut via_athena = Vec::new();
    let mut via_insights = Vec::new(); // (group, start of what CloudWatch still has)
    for (group, retention) in groups.into_iter().zip(retentions) {
        // ponytail: lookup failure skips the check; the search reports access errors itself
        let cutoff = match retention {
            Ok(Some(days)) => time::retention_cutoff(time_range.start, now, days).map(|c| (c, days)),
            _ => None,
        };
        if let Some(target) = athena_for(&group) {
            if force_athena || cutoff.is_some() {
                via_athena.push((group, target));
                continue;
            }
        }
        let mut start = time_range.start;
        if let Some((cutoff, days)) = cutoff {
            let shown = cutoff.format("%Y-%m-%d %H:%M UTC");
            if time_range.end <= cutoff {
                failed += 1;
                eprintln!(
                    "{} {}: whole range is older than its {}-day retention (nothing before {}), \
                     and no [athena] archive is configured for it.",
                    "Error".red(), group, days, shown
                );
                continue;
            }
            eprintln!(
                "{} {}: {}-day retention, nothing before {} is searched; results are partial.",
                "Warning".yellow(), group, days, shown
            );
            start = cutoff;
        }
        via_insights.push((group, start));
    }

    // Cost gate. Insights bills every byte ingested in the window, whatever the filter
    // or --limit, so a month of a busy group costs the same for any pattern. Estimate
    // first; over budget, archived groups go to Athena, and the rest is refused.
    let estimates = futures::future::join_all(
        via_insights
            .iter()
            .map(|(g, start)| searcher.estimate_scan_bytes(g, *start, time_range.end)),
    )
    .await;
    let gb = |bytes: f64| bytes / 1e9;
    let mut planned: Vec<(String, f64)> = Vec::new();
    for ((group, _), estimate) in via_insights.into_iter().zip(estimates) {
        match estimate {
            Ok(bytes) => planned.push((group, bytes)),
            Err(e) => {
                // ponytail: fail open, so a missing cloudwatch:GetMetricStatistics permission doesn't block searching
                eprintln!("{} {}: can't estimate scan size ({}), not counted", "Warning".yellow(), group, e);
                planned.push((group, 0.0));
            }
        }
    }
    // Archived groups go to Athena once the scan is big enough to matter: it scans
    // ~1/6 of what Insights does for the same window, at up to ~5 minutes of lag
    let over_budget = gb(planned.iter().map(|(_, b)| b).sum()) > max_scan_gb;
    planned.retain(|(g, bytes)| match athena_for(g) {
        Some(target) if over_budget || gb(*bytes) > athena_above_gb => {
            via_athena.push((g.clone(), target));
            false
        }
        _ => true,
    });
    let scan_gb = gb(planned.iter().map(|(_, b)| b).sum());
    if scan_gb > max_scan_gb {
        let per_group = planned
            .iter()
            .map(|(g, b)| format!("{} {:.1} GB", g, gb(*b)))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "Refusing: Logs Insights would scan ~{:.1} GB ({}), over the {} GB limit. \
             Insights bills the whole window regardless of patterns or --limit. \
             Narrow --start/--end (pin the time from the DB or a known event), \
             or pass --max-scan-gb {} if this is intended.",
            scan_gb, per_group, max_scan_gb, scan_gb.ceil()
        ));
    }
    let insights_groups: Vec<String> = planned.into_iter().map(|(g, _)| g).collect();
    if !insights_groups.is_empty() {
        eprintln!("{}", format!("Logs Insights will scan ~{:.1} GB", scan_gb).dimmed());
    }

    // Search everything concurrently
    let (insights_results, athena_results) = tokio::join!(
        searcher.search_log_groups(&insights_groups, &params, time_range.start, time_range.end),
        futures::future::join_all(via_athena.iter().map(|(g, target)| {
            searcher.search_athena(g, target, &params, time_range.start, time_range.end)
        })),
    );
    let results = insights_groups
        .into_iter()
        .zip(insights_results)
        .chain(via_athena.into_iter().map(|(g, _)| g).zip(athena_results));

    let mut all_entries = Vec::new();
    for (group, result) in results {
        if output_mode == OutputMode::Streaming {
            println!("{} {}...", "Querying".dimmed(), group.cyan());
        }
        match result {
            Ok(entries) if output_mode == OutputMode::Streaming => {
                entries.iter().for_each(output::print_entry)
            }
            Ok(entries) => all_entries.extend(entries),
            Err(e) => {
                // stderr even for JSON output: stdout stays valid JSON
                failed += 1;
                eprintln!("{} {}: {}", "Error".red(), group, e);
            }
        }
    }

    // All groups failed: "No matching logs found." would be a lie
    if output_mode != OutputMode::Streaming && failed < total {
        output::display_results(all_entries, &output_mode);
    }

    if failed > 0 {
        return Err(anyhow!("{} of {} log group(s) failed", failed, total));
    }

    Ok(())
}

async fn run_kamal_search(
    deploy_path: &str,
    patterns: Vec<String>,
    exclude: Vec<String>,
    last: String,
    output_mode: OutputMode,
    limit: usize,
    follow: bool,
) -> Result<()> {
    use kamal::KamalSearcher;

    // Load Kamal configuration
    let searcher = KamalSearcher::from_file(deploy_path)?;

    // Convert time range to Docker --since format
    let since = time::to_docker_since(&last)?;

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

        if follow {
            println!(
                "{} {}{} on {}",
                "Following".cyan().bold(),
                pattern_display.yellow(),
                exclude_display,
                searcher.servers().first().unwrap_or(&"unknown".to_string()).green(),
            );
            println!("Service: {} | Press Ctrl+C to stop\n", searcher.service().green());
        } else {
            println!(
                "{} {}{} (last {})",
                "Searching".cyan(),
                pattern_display.yellow(),
                exclude_display,
                last.cyan(),
            );
            println!(
                "Service: {} | Servers: {}\n",
                searcher.service().green(),
                searcher.servers().join(", ").dimmed()
            );
        }
    }

    // Create search params
    let params = KamalSearchParams {
        patterns,
        exclude,
        limit,
        since: Some(since),
        follow,
    };

    // Follow mode - stream logs in real-time
    if follow {
        return searcher.follow_logs(&params).await;
    }

    // Search all servers
    let mut all_entries = Vec::new();

    match output_mode {
        OutputMode::Streaming => {
            let results = searcher.search_logs(&params).await;
            for (server, result) in searcher.servers().iter().zip(results) {
                println!("{} {}...", "Querying".dimmed(), server.cyan());
                match result {
                    Ok(entries) => {
                        for entry in entries {
                            output::print_entry(&entry);
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}: {}", "Error".red(), server, e);
                    }
                }
            }
        }
        _ => {
            let results = searcher.search_logs(&params).await;
            for (server, result) in searcher.servers().iter().zip(results) {
                match result {
                    Ok(entries) => all_entries.extend(entries),
                    Err(e) => {
                        if output_mode != OutputMode::Json {
                            eprintln!("{} {}: {}", "Error".red(), server, e);
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
