use crate::aws::{LogEntry, MultiRegionSearcher, SearchParams};
use crate::config::{Config, Preset};
use crate::time::TimeRange;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use super::ui;

const TIME_RANGES: &[(&str, &str)] = &[
    ("5m", "5 minutes"),
    ("15m", "15 minutes"),
    ("30m", "30 minutes"),
    ("1h", "1 hour"),
    ("2h", "2 hours"),
    ("6h", "6 hours"),
    ("12h", "12 hours"),
    ("1d", "1 day"),
    ("3d", "3 days"),
    ("1w", "1 week"),
];

const LIMIT_OPTIONS: &[i32] = &[100, 500, 1000, 5000, 10000];

// Common AWS regions
const AWS_REGIONS: &[&str] = &[
    "ap-east-1",
    "ap-east-2",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-south-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ca-central-1",
    "eu-central-1",
    "eu-north-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "sa-east-1",
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
];

const DEFAULT_ENABLED_REGIONS: &[&str] = &["ap-east-2", "ap-northeast-1"];

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Presets,
    Patterns,
    Exclude,
    Regions,
    LogGroups,
    TimeRange,
    Limit,
    Results,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchState {
    Idle,
    LoadingGroups,
    Searching,
    Complete(usize),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct RegionItem {
    pub name: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct LogGroupItem {
    pub name: String,
    pub region: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct PresetItem {
    pub name: String,
    pub preset: Preset,
}

pub struct App {
    pub patterns_input: String,
    pub exclude_input: String,
    pub time_range_index: usize,
    pub limit_index: usize,
    pub focus: Focus,
    pub results: Vec<LogEntry>,
    pub results_scroll: usize,
    pub search_state: SearchState,
    pub should_quit: bool,

    // Preset selection
    pub presets: Vec<PresetItem>,
    pub presets_cursor: usize,

    // Region selection
    pub regions: Vec<RegionItem>,
    pub regions_cursor: usize,

    // Log group selection
    pub log_groups: Vec<LogGroupItem>,
    pub log_groups_cursor: usize,
    pub log_groups_filter: String,

    // Track if we need to reload log groups
    pub regions_changed: bool,

    // Horizontal scroll for results
    pub horizontal_scroll: usize,

    // Selected result index
    pub selected_result: Option<usize>,

    // Show help overlay
    pub show_help: bool,
}

impl App {
    pub fn new(config: &Config) -> Self {
        let regions: Vec<RegionItem> = AWS_REGIONS
            .iter()
            .map(|&r| RegionItem {
                name: r.to_string(),
                selected: DEFAULT_ENABLED_REGIONS.contains(&r),
            })
            .collect();

        let presets: Vec<PresetItem> = config
            .presets
            .iter()
            .map(|(name, preset)| PresetItem {
                name: name.clone(),
                preset: preset.clone(),
            })
            .collect();

        Self {
            patterns_input: String::new(),
            exclude_input: String::new(),
            time_range_index: 3,
            limit_index: 2,
            focus: if presets.is_empty() { Focus::Patterns } else { Focus::Presets },
            results: Vec::new(),
            results_scroll: 0,
            search_state: SearchState::Idle,
            should_quit: false,
            presets,
            presets_cursor: 0,
            regions,
            regions_cursor: 0,
            log_groups: Vec::new(),
            log_groups_cursor: 0,
            log_groups_filter: String::new(),
            regions_changed: true,
            horizontal_scroll: 0,
            selected_result: None,
            show_help: false,
        }
    }

    pub fn time_range_label(&self) -> &str {
        TIME_RANGES[self.time_range_index].1
    }

    pub fn time_range_value(&self) -> &str {
        TIME_RANGES[self.time_range_index].0
    }

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Presets => Focus::Patterns,
            Focus::Patterns => Focus::Exclude,
            Focus::Exclude => Focus::Regions,
            Focus::Regions => Focus::LogGroups,
            Focus::LogGroups => Focus::TimeRange,
            Focus::TimeRange => Focus::Limit,
            Focus::Limit => Focus::Results,
            Focus::Results => if self.presets.is_empty() { Focus::Patterns } else { Focus::Presets },
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Presets => Focus::Results,
            Focus::Patterns => if self.presets.is_empty() { Focus::Results } else { Focus::Presets },
            Focus::Exclude => Focus::Patterns,
            Focus::Regions => Focus::Exclude,
            Focus::LogGroups => Focus::Regions,
            Focus::TimeRange => Focus::LogGroups,
            Focus::Limit => Focus::TimeRange,
            Focus::Results => Focus::Limit,
        };
    }

    pub fn next_time_range(&mut self) {
        if self.time_range_index < TIME_RANGES.len() - 1 {
            self.time_range_index += 1;
        }
    }

    pub fn prev_time_range(&mut self) {
        if self.time_range_index > 0 {
            self.time_range_index -= 1;
        }
    }

    pub fn limit_value(&self) -> i32 {
        LIMIT_OPTIONS[self.limit_index]
    }

    pub fn next_limit(&mut self) {
        if self.limit_index < LIMIT_OPTIONS.len() - 1 {
            self.limit_index += 1;
        }
    }

    pub fn prev_limit(&mut self) {
        if self.limit_index > 0 {
            self.limit_index -= 1;
        }
    }

    // Preset navigation
    pub fn presets_down(&mut self) {
        if !self.presets.is_empty() && self.presets_cursor < self.presets.len() - 1 {
            self.presets_cursor += 1;
        }
    }

    pub fn presets_up(&mut self) {
        if self.presets_cursor > 0 {
            self.presets_cursor -= 1;
        }
    }

    pub fn apply_preset(&mut self) {
        if let Some(item) = self.presets.get(self.presets_cursor) {
            let preset = &item.preset;
            
            // Apply preset patterns
            if !preset.patterns.is_empty() {
                self.patterns_input = preset.patterns.join(", ");
            }
            
            // Apply preset exclude
            if !preset.exclude.is_empty() {
                self.exclude_input = preset.exclude.join(", ");
            }
            
            // Apply time range if specified
            if let Some(ref tr) = preset.time_range {
                if let Some(idx) = TIME_RANGES.iter().position(|(v, _)| *v == tr.as_str()) {
                    self.time_range_index = idx;
                }
            }
            
            // Apply limit if specified
            if let Some(limit) = preset.limit {
                if let Some(idx) = LIMIT_OPTIONS.iter().position(|&l| l == limit) {
                    self.limit_index = idx;
                }
            }
            
            // Clear existing log group selections and apply preset groups
            for lg in &mut self.log_groups {
                lg.selected = false;
            }
            
            // Select log groups from preset
            for preset_group in &preset.groups {
                // Handle region:group format
                let (region, group_name) = if preset_group.contains(':') {
                    let parts: Vec<&str> = preset_group.splitn(2, ':').collect();
                    (Some(parts[0]), parts[1])
                } else {
                    (None, preset_group.as_str())
                };
                
                for lg in &mut self.log_groups {
                    if lg.name == group_name {
                        if let Some(r) = region {
                            if lg.region == r {
                                lg.selected = true;
                            }
                        } else {
                            lg.selected = true;
                        }
                    }
                }
            }
        }
    }

    // Region navigation
    pub fn regions_down(&mut self) {
        if !self.regions.is_empty() && self.regions_cursor < self.regions.len() - 1 {
            self.regions_cursor += 1;
        }
    }

    pub fn regions_up(&mut self) {
        if self.regions_cursor > 0 {
            self.regions_cursor -= 1;
        }
    }

    pub fn toggle_region(&mut self) {
        if let Some(item) = self.regions.get_mut(self.regions_cursor) {
            item.selected = !item.selected;
            self.regions_changed = true;
        }
    }

    // Results navigation
    pub fn scroll_results_down(&mut self) {
        if self.results_scroll < self.results.len().saturating_sub(1) {
            self.results_scroll += 1;
        }
    }

    pub fn scroll_results_up(&mut self) {
        if self.results_scroll > 0 {
            self.results_scroll -= 1;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        self.results_scroll = (self.results_scroll + page_size)
            .min(self.results.len().saturating_sub(1));
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.results_scroll = self.results_scroll.saturating_sub(page_size);
    }

    pub fn scroll_left(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_sub(10);
    }

    pub fn scroll_right(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_add(10);
    }

    pub fn select_result_at_row(&mut self, row: usize) {
        let index = self.results_scroll + row;
        if index < self.results.len() {
            self.selected_result = Some(index);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected_result = None;
    }

    pub fn filtered_log_groups_indices(&self) -> Vec<usize> {
        if self.log_groups_filter.is_empty() {
            return (0..self.log_groups.len()).collect();
        }
        let filter_lower = self.log_groups_filter.to_lowercase();
        self.log_groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.name.to_lowercase().contains(&filter_lower))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn log_groups_down(&mut self) {
        let filtered = self.filtered_log_groups_indices();
        if filtered.is_empty() {
            return;
        }
        if let Some(pos) = filtered.iter().position(|&i| i == self.log_groups_cursor) {
            if pos < filtered.len() - 1 {
                self.log_groups_cursor = filtered[pos + 1];
            }
        } else {
            self.log_groups_cursor = filtered[0];
        }
    }

    pub fn log_groups_up(&mut self) {
        let filtered = self.filtered_log_groups_indices();
        if filtered.is_empty() {
            return;
        }
        if let Some(pos) = filtered.iter().position(|&i| i == self.log_groups_cursor) {
            if pos > 0 {
                self.log_groups_cursor = filtered[pos - 1];
            }
        } else {
            self.log_groups_cursor = filtered[0];
        }
    }

    pub fn toggle_log_group(&mut self) {
        if let Some(item) = self.log_groups.get_mut(self.log_groups_cursor) {
            item.selected = !item.selected;
        }
    }

    pub fn reset_log_groups_cursor(&mut self) {
        let filtered = self.filtered_log_groups_indices();
        if !filtered.is_empty() {
            self.log_groups_cursor = filtered[0];
        }
    }

    pub fn select_all_log_groups(&mut self) {
        for item in &mut self.log_groups {
            item.selected = true;
        }
    }

    pub fn deselect_all_log_groups(&mut self) {
        for item in &mut self.log_groups {
            item.selected = false;
        }
    }

    pub fn get_patterns(&self) -> Vec<String> {
        self.patterns_input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn get_exclude(&self) -> Vec<String> {
        self.exclude_input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn get_selected_regions(&self) -> Vec<String> {
        self.regions
            .iter()
            .filter(|r| r.selected)
            .map(|r| r.name.clone())
            .collect()
    }

    pub fn selected_regions_count(&self) -> usize {
        self.regions.iter().filter(|r| r.selected).count()
    }

    pub fn get_selected_log_groups(&self) -> Vec<String> {
        self.log_groups
            .iter()
            .filter(|g| g.selected)
            .map(|g| format!("{}:{}", g.region, g.name))
            .collect()
    }

    pub fn selected_log_groups_count(&self) -> usize {
        self.log_groups.iter().filter(|g| g.selected).count()
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }
}

pub async fn run_tui(searcher: MultiRegionSearcher, config: Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(&config);

    let result = run_app(&mut terminal, &mut app, &searcher).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn load_log_groups(app: &mut App, searcher: &MultiRegionSearcher) {
    let selected_regions = app.get_selected_regions();
    if selected_regions.is_empty() {
        app.log_groups.clear();
        app.regions_changed = false;
        return;
    }

    app.search_state = SearchState::LoadingGroups;
    app.log_groups.clear();

    for region in &selected_regions {
        match searcher.list_log_groups(Some(region), None).await {
            Ok(groups) => {
                for name in groups {
                    app.log_groups.push(LogGroupItem {
                        name,
                        region: region.clone(),
                        selected: false,
                    });
                }
            }
            Err(e) => {
                app.search_state = SearchState::Error(format!("{}: {}", region, e));
                return;
            }
        }
    }

    app.search_state = SearchState::Idle;
    app.log_groups_cursor = 0;
    app.regions_changed = false;
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    searcher: &MultiRegionSearcher,
) -> Result<()> {
    load_log_groups(app, searcher).await;

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            let event = event::read()?;

            if let Event::Key(key) = event {
                // Help toggle
                if key.code == KeyCode::F(1) || (key.code == KeyCode::Char('?') && app.focus == Focus::Results) {
                    app.toggle_help();
                    continue;
                }

                // Close help with any key
                if app.show_help {
                    app.show_help = false;
                    continue;
                }

                // Global keybindings
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match key.code {
                        KeyCode::Char('c') | KeyCode::Char('q') => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('a') if app.focus == Focus::LogGroups => {
                            app.select_all_log_groups();
                        }
                        KeyCode::Char('d') if app.focus == Focus::LogGroups => {
                            app.deselect_all_log_groups();
                        }
                        KeyCode::Char('r') => {
                            load_log_groups(app, searcher).await;
                        }
                        _ => {}
                    }
                }

                if app.should_quit {
                    return Ok(());
                }

                match key.code {
                    KeyCode::Tab => {
                        if app.focus == Focus::Regions && app.regions_changed {
                            load_log_groups(app, searcher).await;
                        }
                        app.next_focus();
                    }
                    KeyCode::BackTab => {
                        if app.focus == Focus::LogGroups && app.regions_changed {
                            load_log_groups(app, searcher).await;
                        }
                        app.prev_focus();
                    }
                    KeyCode::Enter => {
                        // Apply preset on Enter in Presets section
                        if app.focus == Focus::Presets {
                            app.apply_preset();
                            app.next_focus();
                        } else if app.focus != Focus::Results {
                            // Search
                            let patterns = app.get_patterns();
                            let exclude = app.get_exclude();
                            let groups = app.get_selected_log_groups();

                            if !groups.is_empty() {
                                app.search_state = SearchState::Searching;
                                app.results.clear();

                                let time_range = TimeRange::from_relative(app.time_range_value());
                                match time_range {
                                    Ok(tr) => {
                                        let params = SearchParams::new(
                                            patterns,
                                            exclude,
                                            app.limit_value(),
                                        );

                                        let results = searcher
                                            .search_log_groups(&groups, &params, tr.start, tr.end)
                                            .await;

                                        let mut all_entries = Vec::new();
                                        let mut errors = Vec::new();

                                        for result in results {
                                            match result {
                                                Ok(entries) => all_entries.extend(entries),
                                                Err(e) => errors.push(e.to_string()),
                                            }
                                        }

                                        all_entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

                                        let count = all_entries.len();
                                        app.results = all_entries;
                                        app.results_scroll = 0;

                                        if errors.is_empty() {
                                            app.search_state = SearchState::Complete(count);
                                        } else if count > 0 {
                                            app.search_state = SearchState::Complete(count);
                                        } else {
                                            app.search_state = SearchState::Error(errors.join("; "));
                                        }

                                        app.focus = Focus::Results;
                                    }
                                    Err(e) => {
                                        app.search_state = SearchState::Error(e.to_string());
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char(' ') => {
                        match app.focus {
                            Focus::Regions => {
                                app.toggle_region();
                            }
                            Focus::LogGroups => {
                                app.toggle_log_group();
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Esc => {
                        if app.focus == Focus::Results {
                            app.focus = Focus::Patterns;
                        } else if app.focus == Focus::LogGroups && !app.log_groups_filter.is_empty() {
                            app.log_groups_filter.clear();
                            app.reset_log_groups_cursor();
                        }
                    }
                    _ => {
                        match app.focus {
                            Focus::Presets => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => app.presets_up(),
                                KeyCode::Down | KeyCode::Char('j') => app.presets_down(),
                                _ => {}
                            },
                            Focus::Patterns => {
                                handle_text_input(key.code, &mut app.patterns_input)
                            }
                            Focus::Exclude => {
                                handle_text_input(key.code, &mut app.exclude_input)
                            }
                            Focus::Regions => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => app.regions_up(),
                                KeyCode::Down | KeyCode::Char('j') => app.regions_down(),
                                _ => {}
                            },
                            Focus::LogGroups => match key.code {
                                KeyCode::Up => app.log_groups_up(),
                                KeyCode::Down => app.log_groups_down(),
                                KeyCode::Char(c) => {
                                    app.log_groups_filter.push(c);
                                    app.reset_log_groups_cursor();
                                }
                                KeyCode::Backspace => {
                                    app.log_groups_filter.pop();
                                    app.reset_log_groups_cursor();
                                }
                                _ => {}
                            },
                            Focus::TimeRange => match key.code {
                                KeyCode::Left | KeyCode::Char('h') => app.prev_time_range(),
                                KeyCode::Right | KeyCode::Char('l') => app.next_time_range(),
                                _ => {}
                            },
                            Focus::Limit => match key.code {
                                KeyCode::Left | KeyCode::Char('h') => app.prev_limit(),
                                KeyCode::Right | KeyCode::Char('l') => app.next_limit(),
                                _ => {}
                            },
                            Focus::Results => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => app.scroll_results_up(),
                                KeyCode::Down | KeyCode::Char('j') => app.scroll_results_down(),
                                KeyCode::Left | KeyCode::Char('h') => app.scroll_left(),
                                KeyCode::Right | KeyCode::Char('l') => app.scroll_right(),
                                KeyCode::PageUp => app.page_up(10),
                                KeyCode::PageDown => app.page_down(10),
                                KeyCode::Home | KeyCode::Char('g') => {
                                    app.results_scroll = 0;
                                    app.horizontal_scroll = 0;
                                }
                                KeyCode::End | KeyCode::Char('G') => {
                                    app.results_scroll = app.results.len().saturating_sub(1);
                                }
                                _ => {}
                            },
                        }
                    }
                }
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn handle_text_input(key: KeyCode, input: &mut String) {
    match key {
        KeyCode::Char(c) => input.push(c),
        KeyCode::Backspace => {
            input.pop();
        }
        _ => {}
    }
}
