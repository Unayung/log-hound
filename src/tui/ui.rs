use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::collections::HashMap;

use super::app::{App, Focus, SearchState};

// Color palette for different log groups
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

/// Shorten region name: ap-northeast-1 -> AN1, ap-east-2 -> AE2, us-west-1 -> UW1, etc.
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

/// Shorten log group name: take first letter of each word segment
/// e.g., "my-lambda-function" → "MLF", "auth_service" → "AS"
fn shorten_group(group: &str) -> String {
    let name = group.rsplit('/').next().unwrap_or(group);

    // Split by common separators and take first char of each part
    let abbrev: String = name
        .split(|c| c == '-' || c == '_' || c == '.')
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().next().unwrap_or(' '))
        .collect::<String>()
        .to_uppercase();

    if abbrev.len() >= 2 {
        abbrev
    } else {
        // Fallback: take first 3 chars uppercase
        name.chars().take(3).collect::<String>().to_uppercase()
    }
}

/// Build color mapping for log groups
fn build_color_map(app: &App) -> HashMap<String, Color> {
    let mut map = HashMap::new();
    for (idx, group) in app.log_groups.iter().filter(|g| g.selected).enumerate() {
        let key = format!("{}:{}", group.region, group.name);
        map.insert(key, LOG_GROUP_COLORS[idx % LOG_GROUP_COLORS.len()]);
    }
    map
}

pub fn render(f: &mut Frame, app: &App) {
    // Determine section heights based on focus (collapsible)
    let selection_height = if app.focus == Focus::Regions || app.focus == Focus::LogGroups {
        10 // Expanded when selecting
    } else {
        3 // Collapsed to summary
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),                         // Patterns input
            Constraint::Length(selection_height as u16),   // Regions + Log groups (collapsible)
            Constraint::Length(3),                         // Time range + Status
            Constraint::Min(8),                            // Results
            Constraint::Length(2),                         // Help
        ])
        .split(f.area());

    render_patterns_input(f, app, chunks[0]);

    if app.focus == Focus::Regions || app.focus == Focus::LogGroups {
        render_selection_section(f, app, chunks[1]);
    } else {
        render_selection_collapsed(f, app, chunks[1]);
    }

    render_time_status_section(f, app, chunks[2]);
    render_results(f, app, chunks[3]);
    render_help(f, app, chunks[4]);
}

fn render_patterns_input(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.focus == Focus::Patterns {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(" Search Patterns (comma-separated = AND) ")
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

    // Build summary of selected regions
    let selected_regions: Vec<String> = app
        .regions
        .iter()
        .filter(|r| r.selected)
        .map(|r| shorten_region(&r.name))
        .collect();

    // Build summary of selected log groups
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

    // Calculate scroll offset
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
    let title = format!(" Log Groups ({} selected) ", selected_count);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    if app.log_groups.is_empty() {
        let msg = if app.selected_regions_count() == 0 {
            "Select regions to load log groups"
        } else {
            "No log groups found in selected regions"
        };
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    } else {
        let visible_height = area.height.saturating_sub(2) as usize;

        let scroll_offset = if app.log_groups_cursor >= visible_height {
            app.log_groups_cursor - visible_height + 1
        } else {
            0
        };

        let items: Vec<ListItem> = app
            .log_groups
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|(idx, item)| {
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

                // Shorten region for display
                let short_region: String = item.region.chars().take(12).collect();

                let line = Line::from(vec![
                    Span::styled(format!("{} ", checkbox), style),
                    Span::styled(
                        format!("[{}] ", short_region),
                        if is_cursor {
                            style
                        } else {
                            Style::default().fg(Color::Cyan)
                        },
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
    let (status_text, status_color) = match &app.search_state {
        SearchState::Idle => {
            let patterns = app.get_patterns();
            let selected = app.selected_log_groups_count();
            if patterns.is_empty() {
                ("Enter search patterns".to_string(), Color::Gray)
            } else if selected == 0 {
                ("Select log groups".to_string(), Color::Gray)
            } else {
                (format!("Ready - {} groups selected", selected), Color::Green)
            }
        }
        SearchState::LoadingGroups => ("Loading log groups...".to_string(), Color::Yellow),
        SearchState::Searching => ("Searching...".to_string(), Color::Yellow),
        SearchState::Complete(count) => (format!("Found {} results", count), Color::Green),
        SearchState::Error(e) => {
            let truncated = if e.len() > 40 {
                format!("{}...", &e[..37])
            } else {
                e.clone()
            };
            (format!("Error: {}", truncated), Color::Red)
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
        " Results ({}/{}){}  h/l:scroll ",
        if app.results.is_empty() {
            0
        } else {
            app.results_scroll + 1
        },
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
            SearchState::LoadingGroups => "Loading log groups...",
            SearchState::Error(_) => "Search failed - check status above",
            _ => "Enter patterns, select log groups, press Enter to search",
        };
        let paragraph = Paragraph::new(empty_msg)
            .block(block)
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    } else {
        // Build color map for log groups
        let color_map = build_color_map(app);
        let available_width = area.width.saturating_sub(2) as usize;
        let show_line_numbers = app.horizontal_scroll > 0;
        let line_num_width = if show_line_numbers { 3 } else { 0 }; // "99 " = 3 chars

        let items: Vec<ListItem> = app
            .results
            .iter()
            .enumerate()
            .skip(app.results_scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|(idx, entry)| {
                let is_selected = app.selected_result == Some(idx);
                let timestamp = entry.timestamp.format("%H:%M:%S%.3f").to_string();

                let region_short = entry
                    .region
                    .as_ref()
                    .map(|r| shorten_region(r))
                    .unwrap_or_default();

                let group_short = shorten_group(&entry.log_group);

                // Get color for this log group
                let group_key = entry
                    .region
                    .as_ref()
                    .map(|r| format!("{}:{}", r, entry.log_group))
                    .unwrap_or_else(|| entry.log_group.clone());
                let group_color = color_map.get(&group_key).copied().unwrap_or(Color::Blue);

                // Selection style
                let base_style = if is_selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let line = if app.horizontal_scroll == 0 {
                    // No horizontal scroll - show full formatted line
                    Line::from(vec![
                        Span::styled(
                            format!("{} ", timestamp),
                            base_style.fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("[{}] ", region_short),
                            base_style.fg(group_color).add_modifier(Modifier::DIM),
                        ),
                        Span::styled(
                            format!("[{}] ", group_short),
                            base_style.fg(group_color),
                        ),
                        Span::styled(&entry.message, base_style),
                    ])
                } else {
                    // Horizontal scroll - show line number + scrolled content with colors
                    let line_num = format!("{:02} ", (idx % 100));
                    let content_width = available_width.saturating_sub(line_num_width);

                    // Build the scrolled content
                    let full_content = format!(
                        "[{}] [{}] {}",
                        region_short, group_short, entry.message
                    );

                    let scrolled_content: String = if app.horizontal_scroll >= full_content.len() {
                        String::new()
                    } else {
                        full_content
                            .chars()
                            .skip(app.horizontal_scroll)
                            .take(content_width)
                            .collect()
                    };

                    Line::from(vec![
                        Span::styled(line_num, base_style.fg(Color::DarkGray)),
                        Span::styled(scrolled_content, base_style.fg(group_color)),
                    ])
                };

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

fn render_help(f: &mut Frame, app: &App, area: Rect) {
    let help_text = match app.focus {
        Focus::Regions => Line::from(vec![
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Toggle  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Navigate  "),
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next (loads groups)  "),
            Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Quit"),
        ]),
        Focus::LogGroups => Line::from(vec![
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Toggle  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Navigate  "),
            Span::styled("Ctrl+A", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" All  "),
            Span::styled("Ctrl+D", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" None  "),
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next"),
        ]),
        Focus::Results => Line::from(vec![
            Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Scroll  "),
            Span::styled("h/l", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Horizontal  "),
            Span::styled("g/G", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Top/Bottom  "),
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next  "),
            Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Quit"),
        ]),
        _ => Line::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Next  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Search  "),
            Span::styled("←/→", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Adjust  "),
            Span::styled("Ctrl+R", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Refresh  "),
            Span::styled("Ctrl+C", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" Quit"),
        ]),
    };

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .centered();

    f.render_widget(help, area);
}
