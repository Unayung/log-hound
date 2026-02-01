use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::collections::HashMap;

use super::app::{App, Focus, SearchState, SourceMode};
use std::path::Path;

const LOG_GROUP_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::Blue,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightYellow,
    Color::LightGreen,
    Color::LightBlue,
];

fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn highlight_patterns(message: &str, patterns: &[String]) -> Vec<Span<'static>> {
    if patterns.is_empty() || message.is_empty() {
        return vec![Span::styled(message.to_string(), Style::default().fg(Color::Gray))];
    }

    let highlight_style = Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(Color::Gray);

    let mut matches: Vec<(usize, usize)> = Vec::new();
    let message_lower = message.to_lowercase();

    for pattern in patterns {
        if pattern.is_empty() {
            continue;
        }
        let pattern_lower = pattern.to_lowercase();
        let mut start = 0;
        while let Some(pos) = message_lower[start..].find(&pattern_lower) {
            let abs_pos = start + pos;
            matches.push((abs_pos, abs_pos + pattern.len()));
            start = abs_pos + 1;
        }
    }

    if matches.is_empty() {
        return vec![Span::styled(message.to_string(), normal_style)];
    }

    matches.sort_by_key(|m| m.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in matches {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut pos = 0;

    for (start, end) in merged {
        if pos < start {
            spans.push(Span::styled(message[pos..start].to_string(), normal_style));
        }
        spans.push(Span::styled(message[start..end].to_string(), highlight_style));
        pos = end;
    }

    if pos < message.len() {
        spans.push(Span::styled(message[pos..].to_string(), normal_style));
    }

    spans
}

fn shorten_region(region: &str) -> String {
    let parts: Vec<&str> = region.split('-').collect();
    if parts.len() >= 3 {
        let prefix = match (parts[0], parts[1]) {
            ("ap", "northeast") => "AN",
            ("ap", "southeast") => "AS",
            ("ap", "south") => "AO",
            ("ap", "east") => "AE",
            ("us", "east") => "UE",
            ("us", "west") => "UW",
            ("eu", "west") => "EW",
            ("eu", "central") => "EC",
            ("eu", "north") => "EN",
            ("ca", "central") => "CC",
            ("sa", "east") => "SE",
            _ => return region.chars().take(6).collect(),
        };
        format!("{}{}", prefix, parts.last().unwrap_or(&""))
    } else {
        region.chars().take(6).collect()
    }
}

fn shorten_group(group: &str) -> String {
    let name = group.rsplit('/').next().unwrap_or(group);
    let abbrev: String = name
        .split(|c| c == '-' || c == '_' || c == '.')
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().next().unwrap_or(' '))
        .collect::<String>()
        .to_uppercase();

    if abbrev.len() >= 2 {
        abbrev
    } else {
        name.chars().take(3).collect::<String>().to_uppercase()
    }
}

fn build_color_map(app: &App) -> HashMap<String, Color> {
    let mut map = HashMap::new();
    for (idx, group) in app.log_groups.iter().filter(|g| g.selected).enumerate() {
        let key = format!("{}:{}", group.region, group.name);
        map.insert(key, LOG_GROUP_COLORS[idx % LOG_GROUP_COLORS.len()]);
    }
    map
}

pub fn render(f: &mut Frame, app: &App) {
    let show_cloudwatch_panels = app.source_mode == SourceMode::CloudWatch;
    let selection_height = if show_cloudwatch_panels && (app.focus == Focus::Regions || app.focus == Focus::LogGroups) {
        10
    } else if show_cloudwatch_panels {
        3
    } else {
        0  // Hidden in Kamal mode
    };

    // Build constraints dynamically to avoid empty slots
    let mut constraints: Vec<Constraint> = Vec::new();
    constraints.push(Constraint::Length(3));                // Source selector
    if app.source_mode == SourceMode::Kamal {
        constraints.push(Constraint::Length(3));            // Deploy file (Kamal only)
    }
    constraints.push(Constraint::Length(3));                // Patterns input
    constraints.push(Constraint::Length(3));                // Exclude input
    if show_cloudwatch_panels {
        constraints.push(Constraint::Length(selection_height as u16)); // Regions + Log groups
    }
    constraints.push(Constraint::Length(3));                // Time range + Limit + Status
    constraints.push(Constraint::Min(6));                   // Results
    constraints.push(Constraint::Length(2));                // Help

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(f.area());

    let mut chunk_idx = 0;

    render_source_selector(f, app, chunks[chunk_idx]);
    chunk_idx += 1;

    if app.source_mode == SourceMode::Kamal {
        render_deploy_file_input(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    render_patterns_input(f, app, chunks[chunk_idx]);
    chunk_idx += 1;

    render_exclude_input(f, app, chunks[chunk_idx]);
    chunk_idx += 1;

    if show_cloudwatch_panels {
        if app.focus == Focus::Regions || app.focus == Focus::LogGroups {
            render_selection_section(f, app, chunks[chunk_idx]);
        } else {
            render_selection_collapsed(f, app, chunks[chunk_idx]);
        }
        chunk_idx += 1;
    }

    render_time_status_section(f, app, chunks[chunk_idx]);
    chunk_idx += 1;

    render_results(f, app, chunks[chunk_idx]);
    chunk_idx += 1;

    render_help_bar(f, app, chunks[chunk_idx]);

    // Render help overlay if active
    if app.show_help {
        render_help_overlay(f);
    }
}

fn render_source_selector(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Source {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(" Source (h/l or ←/→) ")
        .borders(Borders::ALL)
        .border_style(style);

    let cloudwatch_style = if app.source_mode == SourceMode::CloudWatch {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let kamal_style = if app.source_mode == SourceMode::Kamal {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let content = Line::from(vec![
        Span::styled(
            if app.source_mode == SourceMode::CloudWatch { "[CloudWatch]" } else { " CloudWatch " },
            cloudwatch_style,
        ),
        Span::raw("  "),
        Span::styled(
            if app.source_mode == SourceMode::Kamal { "[Kamal]" } else { " Kamal " },
            kamal_style,
        ),
    ]);

    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}

fn render_deploy_file_input(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::DeployFile {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let filtered_indices = app.filtered_deploy_files_indices();
    let filter_info = if app.deploy_files_filter.is_empty() {
        String::new()
    } else {
        format!(" [filter: {}]", app.deploy_files_filter)
    };
    let title = format!(" Deploy File ({}/{}){} ", filtered_indices.len(), app.deploy_files.len(), filter_info);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    if app.deploy_files.is_empty() {
        let paragraph = Paragraph::new("No deploy*.yml found in config/")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, area);
    } else if filtered_indices.is_empty() {
        let msg = format!("No match: {}", app.deploy_files_filter);
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, area);
    } else {
        // Build display showing filtered files with selection indicator
        let mut spans: Vec<Span> = Vec::new();
        let mut first = true;

        for &idx in &filtered_indices {
            let file_path = &app.deploy_files[idx];

            // Extract simplified name: deploy.production.yml -> production
            let filename = Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);

            // Remove "deploy." prefix and ".yml"/".yaml" suffix
            let display_name = filename
                .strip_prefix("deploy.")
                .unwrap_or(filename)
                .trim_end_matches(".yml")
                .trim_end_matches(".yaml");

            // Handle edge case: deploy.yml becomes empty after stripping
            let display_name = if display_name.is_empty() || display_name == "yml" {
                "default"
            } else {
                display_name
            };

            let is_selected = idx == app.deploy_files_cursor;

            if !first {
                spans.push(Span::raw("  "));
            }
            first = false;

            if is_selected {
                spans.push(Span::styled(
                    format!("[{}]", display_name),
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {} ", display_name),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        let paragraph = Paragraph::new(Line::from(spans)).block(block);
        f.render_widget(paragraph, area);
    }
}

fn render_patterns_input(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Patterns {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(" Search Patterns (comma = AND) ")
        .borders(Borders::ALL)
        .border_style(style);

    let input = Paragraph::new(app.patterns_input.as_str())
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(input, area);

    if app.focus == Focus::Patterns {
        f.set_cursor_position((area.x + app.patterns_input.len() as u16 + 1, area.y + 1));
    }
}

fn render_exclude_input(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Exclude {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(" Exclude Patterns (NOT condition) ")
        .borders(Borders::ALL)
        .border_style(style);

    let display_text = if app.exclude_input.is_empty() && app.focus != Focus::Exclude {
        "none"
    } else {
        &app.exclude_input
    };

    let text_style = if app.exclude_input.is_empty() && app.focus != Focus::Exclude {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::LightRed)
    };

    let input = Paragraph::new(display_text)
        .block(block)
        .style(text_style);

    f.render_widget(input, area);

    if app.focus == Focus::Exclude {
        f.set_cursor_position((area.x + app.exclude_input.len() as u16 + 1, area.y + 1));
    }
}

fn render_selection_section(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    render_regions(f, app, chunks[0]);
    render_log_groups(f, app, chunks[1]);
}

fn render_selection_collapsed(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Selection (Tab to expand) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let selected_regions: Vec<String> = app
        .regions
        .iter()
        .filter(|r| r.selected)
        .map(|r| shorten_region(&r.name))
        .collect();

    let selected_groups: Vec<String> = app
        .log_groups
        .iter()
        .filter(|g| g.selected)
        .map(|g| shorten_group(&g.name))
        .collect();

    let regions_str = if selected_regions.is_empty() {
        "None".to_string()
    } else {
        selected_regions.join(", ")
    };

    let groups_str = if selected_groups.is_empty() {
        "None".to_string()
    } else if selected_groups.len() > 3 {
        format!("{}, +{} more", selected_groups[..3].join(", "), selected_groups.len() - 3)
    } else {
        selected_groups.join(", ")
    };

    let summary = Line::from(vec![
        Span::styled("Regions: ", Style::default().fg(Color::Cyan)),
        Span::styled(regions_str, Style::default().fg(Color::White)),
        Span::raw("  │  "),
        Span::styled("Groups: ", Style::default().fg(Color::Cyan)),
        Span::styled(groups_str, Style::default().fg(Color::White)),
    ]);

    let paragraph = Paragraph::new(summary).block(block);
    f.render_widget(paragraph, area);
}

fn render_regions(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Regions {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let selected_count = app.selected_regions_count();
    let title = format!(" Regions ({}) ", selected_count);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll_offset = if app.regions_cursor >= visible_height {
        app.regions_cursor - visible_height + 1
    } else {
        0
    };

    let items: Vec<ListItem> = app
        .regions
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(idx, item)| {
            let checkbox = if item.selected { "[x]" } else { "[ ]" };
            let is_cursor = idx == app.regions_cursor;

            let style = if is_cursor {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if item.selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Gray)
            };

            let line = Line::from(vec![
                Span::styled(format!("{} ", checkbox), style),
                Span::styled(&item.name, style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn render_log_groups(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::LogGroups {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let selected_count = app.selected_log_groups_count();
    let filtered_indices = app.filtered_log_groups_indices();
    let filter_info = if app.log_groups_filter.is_empty() {
        String::new()
    } else {
        format!(" [filter: {}]", app.log_groups_filter)
    };
    let title = format!(
        " Log Groups ({} selected, {} shown){} ",
        selected_count,
        filtered_indices.len(),
        filter_info
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    if app.log_groups.is_empty() {
        let msg = if app.selected_regions_count() == 0 {
            "Select regions to load log groups"
        } else {
            "No log groups found"
        };
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    } else if filtered_indices.is_empty() {
        let msg = format!("No groups match: {}", app.log_groups_filter);
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::Gray));
        f.render_widget(paragraph, area);
    } else {
        let visible_height = area.height.saturating_sub(2) as usize;
        let cursor_pos_in_filtered = filtered_indices
            .iter()
            .position(|&i| i == app.log_groups_cursor)
            .unwrap_or(0);

        let scroll_offset = if cursor_pos_in_filtered >= visible_height {
            cursor_pos_in_filtered - visible_height + 1
        } else {
            0
        };

        let items: Vec<ListItem> = filtered_indices
            .iter()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|&idx| {
                let item = &app.log_groups[idx];
                let checkbox = if item.selected { "[x]" } else { "[ ]" };
                let is_cursor = idx == app.log_groups_cursor;

                let style = if is_cursor {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if item.selected {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };

                let short_region: String = item.region.chars().take(12).collect();

                let line = Line::from(vec![
                    Span::styled(format!("{} ", checkbox), style),
                    Span::styled(
                        format!("[{}] ", short_region),
                        if is_cursor { style } else { Style::default().fg(Color::Cyan) },
                    ),
                    Span::styled(&item.name, style),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

fn render_time_status_section(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(45),
        ])
        .split(area);

    render_time_range(f, app, chunks[0]);
    render_limit(f, app, chunks[1]);
    render_status(f, app, chunks[2]);
}

fn render_time_range(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::TimeRange {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let time_text = format!("◄  {}  ►", app.time_range_label());
    let block = Block::default()
        .title(" Time Range ")
        .borders(Borders::ALL)
        .border_style(style);

    let widget = Paragraph::new(time_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan))
        .centered();

    f.render_widget(widget, area);
}

fn render_limit(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Limit {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let limit_text = format!("◄  {}  ►", app.limit_value());
    let block = Block::default()
        .title(" Limit ")
        .borders(Borders::ALL)
        .border_style(style);

    let widget = Paragraph::new(limit_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan))
        .centered();

    f.render_widget(widget, area);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let (status_text, status_color) = if app.is_following {
        (format!("● FOLLOWING ({}) - Esc to stop", app.results.len()), Color::LightRed)
    } else {
        match &app.search_state {
            SearchState::Idle => {
                let follow_indicator = if app.follow_mode { " [f]" } else { "" };
                let selected = app.selected_log_groups_count();
                if app.source_mode == super::app::SourceMode::Kamal {
                    (format!("Ready{}", follow_indicator), Color::Green)
                } else if selected == 0 {
                    ("Select log groups".to_string(), Color::Gray)
                } else {
                    (format!("Ready - {} groups{}", selected, follow_indicator), Color::Green)
                }
            }
            SearchState::LoadingGroups => ("Loading...".to_string(), Color::Yellow),
            SearchState::Searching => {
                if app.follow_mode {
                    ("Starting follow...".to_string(), Color::Yellow)
                } else {
                    ("Searching...".to_string(), Color::Yellow)
                }
            }
            SearchState::Complete(count) => {
                let follow_indicator = if app.follow_mode { " [f]" } else { "" };
                (format!("Found {} results{}", count, follow_indicator), Color::Green)
            }
            SearchState::Error(e) => {
                let truncated = if e.len() > 35 {
                    format!("{}...", &e[..32])
                } else {
                    e.clone()
                };
                (format!("Error: {}", truncated), Color::Red)
            }
        }
    };

    let block = Block::default().title(" Status ").borders(Borders::ALL);
    let widget = Paragraph::new(status_text)
        .block(block)
        .style(Style::default().fg(status_color))
        .centered();

    f.render_widget(widget, area);
}

fn render_results(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Results {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let scroll_indicator = if app.horizontal_scroll > 0 {
        format!(" ←{}", app.horizontal_scroll)
    } else {
        String::new()
    };

    let title = format!(
        " Results ({}/{}){}  F1:Help ",
        if app.results.is_empty() { 0 } else { app.results_scroll + 1 },
        app.results.len(),
        scroll_indicator
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    if app.results.is_empty() {
        let empty_msg = match &app.search_state {
            SearchState::Searching => "Searching...",
            SearchState::LoadingGroups => "Loading...",
            SearchState::Error(_) => "Search failed",
            _ => "Press Enter to search",
        };
        let paragraph = Paragraph::new(empty_msg)
            .block(block)
            .style(Style::default().fg(Color::Gray));
        f.render_widget(paragraph, area);
    } else {
        let color_map = build_color_map(app);
        let available_width = area.width.saturating_sub(2) as usize;
        let show_line_numbers = app.horizontal_scroll > 0;
        let line_num_width = if show_line_numbers { 3 } else { 0 };

        let items: Vec<ListItem> = app
            .results
            .iter()
            .enumerate()
            .skip(app.results_scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|(idx, entry)| {
                let timestamp = entry.timestamp.format("%H:%M:%S%.3f").to_string();
                let region_short = entry.region.as_ref().map(|r| shorten_region(r)).unwrap_or_default();
                let group_short = shorten_group(&entry.log_group);

                let group_key = entry.region.as_ref()
                    .map(|r| format!("{}:{}", r, entry.log_group))
                    .unwrap_or_else(|| entry.log_group.clone());
                let group_color = color_map.get(&group_key).copied().unwrap_or(Color::Blue);

                let clean_message = strip_ansi_codes(&entry.message);
                let patterns = app.get_patterns();

                let line = if app.horizontal_scroll == 0 {
                    let mut spans: Vec<Span<'static>> = vec![
                        Span::styled(format!("{} ", timestamp), Style::default().fg(Color::DarkGray)),
                        Span::styled(format!("[{}] ", region_short), Style::default().fg(group_color).add_modifier(Modifier::DIM)),
                        Span::styled(format!("[{}] ", group_short), Style::default().fg(group_color)),
                    ];
                    spans.extend(highlight_patterns(&clean_message, &patterns));
                    Line::from(spans)
                } else {
                    let line_num = format!("{:02} ", (idx % 100));
                    let content_width = available_width.saturating_sub(line_num_width);
                    let full_content = format!("{} [{}] [{}] {}", timestamp, region_short, group_short, clean_message);
                    let scrolled_content: String = full_content.chars().skip(app.horizontal_scroll).take(content_width).collect();

                    let mut spans: Vec<Span<'static>> = vec![
                        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    ];
                    spans.extend(highlight_patterns(&scrolled_content, &patterns));
                    Line::from(spans)
                };

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

fn render_help_bar(f: &mut Frame, app: &App, area: Rect) {
    let help_text = match app.focus {
        Focus::DeployFile => Line::from(vec![
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Select  "),
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Search"),
        ]),
        Focus::Regions | Focus::LogGroups => Line::from(vec![
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Toggle  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Nav  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Search  "),
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next"),
        ]),
        Focus::Results => {
            if app.is_following {
                Line::from(vec![
                    Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD).fg(Color::LightRed)),
                    Span::raw(" Stop  "),
                    Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Scroll  "),
                    Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Quit"),
                ])
            } else {
                Line::from(vec![
                    Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Scroll  "),
                    Span::styled("h/l", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Horiz  "),
                    Span::styled("f", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Follow  "),
                    Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" Quit"),
                ])
            }
        }
        _ => Line::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next  "),
            Span::styled("f", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Follow  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Search  "),
            Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Quit"),
        ]),
    };

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .centered();

    f.render_widget(help, area);
}

fn render_help_overlay(f: &mut Frame) {
    let area = f.area();
    let popup_width = 60;
    let popup_height = 15;
    let popup_area = Rect::new(
        (area.width.saturating_sub(popup_width)) / 2,
        (area.height.saturating_sub(popup_height)) / 2,
        popup_width.min(area.width),
        popup_height.min(area.height),
    );

    f.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled("Keyboard Shortcuts", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![Span::styled("Tab / Shift+Tab", Style::default().fg(Color::Cyan)), Span::raw("  Navigate sections")]),
        Line::from(vec![Span::styled("Enter", Style::default().fg(Color::Cyan)), Span::raw("            Execute search")]),
        Line::from(vec![Span::styled("f", Style::default().fg(Color::Cyan)), Span::raw("                Toggle follow mode")]),
        Line::from(vec![Span::styled("Space", Style::default().fg(Color::Cyan)), Span::raw("            Toggle selection")]),
        Line::from(vec![Span::styled("↑/↓ or j/k", Style::default().fg(Color::Cyan)), Span::raw("       Navigate lists")]),
        Line::from(vec![Span::styled("←/→ or h/l", Style::default().fg(Color::Cyan)), Span::raw("       Adjust values / scroll")]),
        Line::from(vec![Span::styled("Esc", Style::default().fg(Color::Cyan)), Span::raw("              Stop follow / back")]),
        Line::from(vec![Span::styled("Ctrl+C", Style::default().fg(Color::Cyan)), Span::raw("           Quit")]),
        Line::from(""),
        Line::from(Span::styled("Press any key to close", Style::default().fg(Color::DarkGray))),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, popup_area);
}
