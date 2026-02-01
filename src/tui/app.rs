use crate::aws::{LogEntry, MultiRegionSearcher, SearchParams};
use crate::config::Config;
use crate::kamal::{KamalSearcher, KamalSearchParams};
use crate::time::TimeRange;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc;

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
    Source,
    DeployFile,  // Kamal only - list of detected deploy files
    Patterns,
    Exclude,
    Regions,     // CloudWatch only
    LogGroups,   // CloudWatch only
    TimeRange,
    Limit,
    Results,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SourceMode {
    #[default]
    CloudWatch,
    Kamal,
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

    // Source selection (CloudWatch vs Kamal)
    pub source_mode: SourceMode,

    // Deploy file selection (Kamal only) - detected from config/ folder
    pub deploy_files: Vec<String>,
    pub deploy_files_cursor: usize,
    pub deploy_files_filter: String,

    // Region selection (CloudWatch only)
    pub regions: Vec<RegionItem>,
    pub regions_cursor: usize,

    // Log group selection (CloudWatch only)
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

    // Follow mode - stream logs in real-time
    pub follow_mode: bool,
    pub is_following: bool,
    pub follow_receiver: Option<mpsc::Receiver<LogEntry>>,
    pub follow_stop_flag: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn new(_config: &Config) -> Self {
        let regions: Vec<RegionItem> = AWS_REGIONS
            .iter()
            .map(|&r| RegionItem {
                name: r.to_string(),
                selected: DEFAULT_ENABLED_REGIONS.contains(&r),
            })
            .collect();

        // Detect deploy files from config/ folder
        let deploy_files = Self::detect_deploy_files();

        Self {
            patterns_input: String::new(),
            exclude_input: String::new(),
            time_range_index: 3,
            limit_index: 2,
            focus: Focus::Source,
            results: Vec::new(),
            results_scroll: 0,
            search_state: SearchState::Idle,
            should_quit: false,
            source_mode: SourceMode::CloudWatch,
            deploy_files,
            deploy_files_cursor: 0,
            deploy_files_filter: String::new(),
            regions,
            regions_cursor: 0,
            log_groups: Vec::new(),
            log_groups_cursor: 0,
            log_groups_filter: String::new(),
            regions_changed: true,
            horizontal_scroll: 0,
            selected_result: None,
            show_help: false,
            follow_mode: false,
            is_following: false,
            follow_receiver: None,
            follow_stop_flag: None,
        }
    }

    pub fn toggle_follow_mode(&mut self) {
        // Don't toggle if currently following
        if !self.is_following {
            self.follow_mode = !self.follow_mode;
        }
    }

    pub fn stop_following(&mut self) {
        // Signal the background task to stop
        if let Some(stop_flag) = &self.follow_stop_flag {
            stop_flag.store(true, Ordering::Relaxed);
        }
        self.is_following = false;
        self.follow_receiver = None;
        self.follow_stop_flag = None;
        self.search_state = SearchState::Complete(self.results.len());
    }

    pub fn time_range_label(&self) -> &str {
        TIME_RANGES[self.time_range_index].1
    }

    pub fn time_range_value(&self) -> &str {
        TIME_RANGES[self.time_range_index].0
    }

    pub fn next_focus(&mut self) {
        self.focus = match (&self.focus, &self.source_mode) {
            (Focus::Source, SourceMode::CloudWatch) => Focus::Patterns,
            (Focus::Source, SourceMode::Kamal) => Focus::DeployFile,
            (Focus::DeployFile, _) => Focus::Patterns,
            (Focus::Patterns, _) => Focus::Exclude,
            (Focus::Exclude, SourceMode::CloudWatch) => Focus::Regions,
            (Focus::Exclude, SourceMode::Kamal) => Focus::TimeRange,
            (Focus::Regions, _) => Focus::LogGroups,
            (Focus::LogGroups, _) => Focus::TimeRange,
            (Focus::TimeRange, _) => Focus::Limit,
            (Focus::Limit, _) => Focus::Results,
            (Focus::Results, _) => Focus::Source,
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = match (&self.focus, &self.source_mode) {
            (Focus::Source, _) => Focus::Results,
            (Focus::DeployFile, _) => Focus::Source,
            (Focus::Patterns, SourceMode::CloudWatch) => Focus::Source,
            (Focus::Patterns, SourceMode::Kamal) => Focus::DeployFile,
            (Focus::Exclude, _) => Focus::Patterns,
            (Focus::Regions, _) => Focus::Exclude,
            (Focus::LogGroups, _) => Focus::Regions,
            (Focus::TimeRange, SourceMode::CloudWatch) => Focus::LogGroups,
            (Focus::TimeRange, SourceMode::Kamal) => Focus::Exclude,
            (Focus::Limit, _) => Focus::TimeRange,
            (Focus::Results, _) => Focus::Limit,
        };
    }

    pub fn toggle_source(&mut self) {
        self.source_mode = match self.source_mode {
            SourceMode::CloudWatch => {
                // Refresh deploy files when switching to Kamal
                self.deploy_files = Self::detect_deploy_files();
                self.deploy_files_cursor = 0;
                SourceMode::Kamal
            }
            SourceMode::Kamal => SourceMode::CloudWatch,
        };
    }

    /// Detect deploy*.yml files from config/ folder
    fn detect_deploy_files() -> Vec<String> {
        let config_path = Path::new("config");
        let mut files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(config_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("deploy") && name.ends_with(".yml") {
                        files.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }

        // Sort by simplified display name (alphabetically)
        files.sort_by(|a, b| {
            let name_a = Self::extract_deploy_name(a);
            let name_b = Self::extract_deploy_name(b);
            name_a.cmp(&name_b)
        });

        // If no files found, add a default
        if files.is_empty() {
            files.push("config/deploy.yml".to_string());
        }

        files
    }

    /// Extract simplified name from deploy file path: deploy.production.yml -> production
    fn extract_deploy_name(path: &str) -> String {
        let filename = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);

        let name = filename
            .strip_prefix("deploy.")
            .unwrap_or(filename)
            .trim_end_matches(".yml")
            .trim_end_matches(".yaml");

        if name.is_empty() || name == "yml" {
            "default".to_string()
        } else {
            name.to_string()
        }
    }

    /// Get the currently selected deploy file
    pub fn selected_deploy_file(&self) -> &str {
        let filtered = self.filtered_deploy_files_indices();
        if let Some(&idx) = filtered.iter().find(|&&i| i == self.deploy_files_cursor) {
            self.deploy_files.get(idx).map(|s| s.as_str()).unwrap_or("config/deploy.yml")
        } else if let Some(&first_idx) = filtered.first() {
            self.deploy_files.get(first_idx).map(|s| s.as_str()).unwrap_or("config/deploy.yml")
        } else {
            "config/deploy.yml"
        }
    }

    /// Get filtered deploy files indices based on current filter
    pub fn filtered_deploy_files_indices(&self) -> Vec<usize> {
        if self.deploy_files_filter.is_empty() {
            return (0..self.deploy_files.len()).collect();
        }
        let filter_lower = self.deploy_files_filter.to_lowercase();
        self.deploy_files
            .iter()
            .enumerate()
            .filter(|(_, path)| {
                let name = Self::extract_deploy_name(path).to_lowercase();
                name.contains(&filter_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Reset deploy files cursor to first filtered item
    pub fn reset_deploy_files_cursor(&mut self) {
        let filtered = self.filtered_deploy_files_indices();
        if let Some(&first) = filtered.first() {
            self.deploy_files_cursor = first;
        }
    }

    // Deploy file navigation (filter-aware)
    pub fn deploy_files_down(&mut self) {
        let filtered = self.filtered_deploy_files_indices();
        if let Some(pos) = filtered.iter().position(|&i| i == self.deploy_files_cursor) {
            if pos + 1 < filtered.len() {
                self.deploy_files_cursor = filtered[pos + 1];
            }
        } else if let Some(&first) = filtered.first() {
            self.deploy_files_cursor = first;
        }
    }

    pub fn deploy_files_up(&mut self) {
        let filtered = self.filtered_deploy_files_indices();
        if let Some(pos) = filtered.iter().position(|&i| i == self.deploy_files_cursor) {
            if pos > 0 {
                self.deploy_files_cursor = filtered[pos - 1];
            }
        } else if let Some(&first) = filtered.first() {
            self.deploy_files_cursor = first;
        }
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

    // For CloudWatch polling in follow mode
    let mut last_poll_time = std::time::Instant::now();
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

    loop {
        // Check for new entries from follow mode channel
        if let Some(ref mut receiver) = app.follow_receiver {
            while let Ok(entry) = receiver.try_recv() {
                // Insert at beginning (newest first) and maintain scroll position
                app.results.insert(0, entry);
                // Keep results from growing unbounded
                if app.results.len() > 10000 {
                    app.results.pop();
                }
                app.search_state = SearchState::Complete(app.results.len());
            }
        }

        // CloudWatch follow mode: periodic polling
        if app.is_following && app.source_mode == SourceMode::CloudWatch {
            if last_poll_time.elapsed() >= POLL_INTERVAL {
                last_poll_time = std::time::Instant::now();

                let patterns = app.get_patterns();
                let exclude = app.get_exclude();
                let groups = app.get_selected_log_groups();

                if !groups.is_empty() {
                    if let Ok(tr) = TimeRange::from_relative("1m") {
                        let params = SearchParams::new(patterns, exclude, 100);
                        let results = searcher.search_log_groups(&groups, &params, tr.start, tr.end).await;

                        for result in results {
                            if let Ok(entries) = result {
                                for entry in entries {
                                    // Avoid duplicates by checking timestamp
                                    if !app.results.iter().take(100).any(|e| e.timestamp == entry.timestamp && e.message == entry.message) {
                                        app.results.insert(0, entry);
                                    }
                                }
                            }
                        }
                        // Sort and limit
                        app.results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                        if app.results.len() > 10000 {
                            app.results.truncate(10000);
                        }
                        app.search_state = SearchState::Complete(app.results.len());
                    }
                }
            }
        }

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
                    if key.code == KeyCode::Char('c') || key.code == KeyCode::Char('q') {
                        app.should_quit = true;
                    }
                }

                if app.should_quit {
                    return Ok(());
                }

                match key.code {
                    KeyCode::Tab => {
                        if app.source_mode == SourceMode::CloudWatch && app.focus == Focus::Regions && app.regions_changed {
                            load_log_groups(app, searcher).await;
                        }
                        app.next_focus();
                    }
                    KeyCode::BackTab => {
                        if app.source_mode == SourceMode::CloudWatch && app.focus == Focus::LogGroups && app.regions_changed {
                            load_log_groups(app, searcher).await;
                        }
                        app.prev_focus();
                    }
                    KeyCode::Enter => {
                        if app.focus != Focus::Results {
                            // Search based on source mode
                            match app.source_mode {
                                SourceMode::CloudWatch => {
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

                                                // Enable follow mode polling if requested
                                                if app.follow_mode {
                                                    app.is_following = true;
                                                    last_poll_time = std::time::Instant::now();
                                                }

                                                app.focus = Focus::Results;
                                            }
                                            Err(e) => {
                                                app.search_state = SearchState::Error(e.to_string());
                                            }
                                        }
                                    }
                                }
                                SourceMode::Kamal => {
                                    // Search using KamalSearcher
                                    let patterns = app.get_patterns();
                                    let exclude = app.get_exclude();

                                    app.search_state = SearchState::Searching;
                                    app.results.clear();

                                    match KamalSearcher::from_file(app.selected_deploy_file()) {
                                        Ok(kamal_searcher) => {
                                            let since = crate::time::to_docker_since(app.time_range_value());
                                            match since {
                                                Ok(since_str) => {
                                                    let params = KamalSearchParams {
                                                        patterns,
                                                        exclude,
                                                        limit: app.limit_value() as usize,
                                                        since: Some(since_str),
                                                        follow: app.follow_mode,
                                                    };

                                                    if app.follow_mode {
                                                        // Start follow mode with channel
                                                        let (tx, rx) = mpsc::channel(1000);
                                                        let stop_flag = Arc::new(AtomicBool::new(false));

                                                        match kamal_searcher.follow_logs_channel(&params, tx, stop_flag.clone()).await {
                                                            Ok(()) => {
                                                                app.follow_receiver = Some(rx);
                                                                app.follow_stop_flag = Some(stop_flag);
                                                                app.is_following = true;
                                                                app.search_state = SearchState::Searching;
                                                                app.focus = Focus::Results;
                                                            }
                                                            Err(e) => {
                                                                app.search_state = SearchState::Error(format!("Follow failed: {}", e));
                                                            }
                                                        }
                                                    } else {
                                                        // Regular search
                                                        let results = kamal_searcher.search_logs(&params).await;

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
                                                }
                                                Err(e) => {
                                                    app.search_state = SearchState::Error(e.to_string());
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            app.search_state = SearchState::Error(format!("Failed to load deploy file: {}", e));
                                        }
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
                        if app.is_following {
                            // Stop follow mode
                            app.stop_following();
                        } else if app.focus == Focus::Results {
                            app.focus = Focus::Patterns;
                        } else if app.focus == Focus::DeployFile && !app.deploy_files_filter.is_empty() {
                            app.deploy_files_filter.clear();
                            app.reset_deploy_files_cursor();
                        } else if app.focus == Focus::LogGroups && !app.log_groups_filter.is_empty() {
                            app.log_groups_filter.clear();
                            app.reset_log_groups_cursor();
                        }
                    }
                    KeyCode::Char('f') if app.focus != Focus::Patterns && app.focus != Focus::Exclude && app.focus != Focus::LogGroups && app.focus != Focus::DeployFile => {
                        if app.is_following {
                            // Stop following
                            app.stop_following();
                        } else if app.focus == Focus::Results {
                            // Start following immediately from Results view
                            app.follow_mode = true;

                            match app.source_mode {
                                SourceMode::CloudWatch => {
                                    // For CloudWatch, just enable polling mode
                                    app.is_following = true;
                                    last_poll_time = std::time::Instant::now();
                                }
                                SourceMode::Kamal => {
                                    // Start Kamal follow mode
                                    let patterns = app.get_patterns();
                                    let exclude = app.get_exclude();

                                    if let Ok(kamal_searcher) = KamalSearcher::from_file(app.selected_deploy_file()) {
                                        if let Ok(since_str) = crate::time::to_docker_since("1m") {
                                            let params = KamalSearchParams {
                                                patterns,
                                                exclude,
                                                limit: app.limit_value() as usize,
                                                since: Some(since_str),
                                                follow: true,
                                            };

                                            let (tx, rx) = mpsc::channel(1000);
                                            let stop_flag = Arc::new(AtomicBool::new(false));

                                            if kamal_searcher.follow_logs_channel(&params, tx, stop_flag.clone()).await.is_ok() {
                                                app.follow_receiver = Some(rx);
                                                app.follow_stop_flag = Some(stop_flag);
                                                app.is_following = true;
                                                app.search_state = SearchState::Complete(app.results.len());
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            // Toggle follow mode for next search
                            app.toggle_follow_mode();
                        }
                    }
                    _ => {
                        match app.focus {
                            Focus::Source => match key.code {
                                KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Char(' ') => {
                                    app.toggle_source();
                                }
                                _ => {}
                            },
                            Focus::DeployFile => match key.code {
                                KeyCode::Left | KeyCode::Up => app.deploy_files_up(),
                                KeyCode::Right | KeyCode::Down => app.deploy_files_down(),
                                KeyCode::Char(c) => {
                                    // h/l for navigation, other chars for filtering
                                    if c == 'h' || c == 'k' {
                                        app.deploy_files_up();
                                    } else if c == 'l' || c == 'j' {
                                        app.deploy_files_down();
                                    } else {
                                        app.deploy_files_filter.push(c);
                                        app.reset_deploy_files_cursor();
                                    }
                                }
                                KeyCode::Backspace => {
                                    app.deploy_files_filter.pop();
                                    app.reset_deploy_files_cursor();
                                }
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
