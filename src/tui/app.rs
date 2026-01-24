use crate::aws::{LogEntry, MultiRegionSearcher};
use crate::time::TimeRange;
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind, MouseButton},
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

// Common AWS regions - add more as needed
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

// Default enabled regions
const DEFAULT_ENABLED_REGIONS: &[&str] = &["ap-east-2", "ap-northeast-1"];

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Patterns,
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

pub struct App {
    pub patterns_input: String,
    pub time_range_index: usize,
    pub limit_index: usize,
    pub focus: Focus,
    pub results: Vec<LogEntry>,
    pub results_scroll: usize,
    pub search_state: SearchState,
    pub should_quit: bool,

    // Region selection
    pub regions: Vec<RegionItem>,
    pub regions_cursor: usize,

    // Log group selection
    pub log_groups: Vec<LogGroupItem>,
    pub log_groups_cursor: usize,

    // Track if we need to reload log groups
    pub regions_changed: bool,

    // Horizontal scroll for results
    pub horizontal_scroll: usize,

    // Selected result index (for mouse selection)
    pub selected_result: Option<usize>,
}

impl App {
    pub fn new() -> Self {
        // Initialize regions with defaults enabled
        let regions: Vec<RegionItem> = AWS_REGIONS
            .iter()
            .map(|&r| RegionItem {
                name: r.to_string(),
                selected: DEFAULT_ENABLED_REGIONS.contains(&r),
            })
            .collect();

        Self {
            patterns_input: String::new(),
            time_range_index: 3, // Default to 1h (index shifted due to 5m addition)
            limit_index: 2,      // Default to 1000
            focus: Focus::Patterns,
            results: Vec::new(),
            results_scroll: 0,
            search_state: SearchState::Idle,
            should_quit: false,
            regions,
            regions_cursor: 0,
            log_groups: Vec::new(),
            log_groups_cursor: 0,
            regions_changed: true, // Load on first focus
            horizontal_scroll: 0,
            selected_result: None,
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
            Focus::Patterns => Focus::Regions,
            Focus::Regions => Focus::LogGroups,
            Focus::LogGroups => Focus::TimeRange,
            Focus::TimeRange => Focus::Limit,
            Focus::Limit => Focus::Results,
            Focus::Results => Focus::Patterns,
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Patterns => Focus::Results,
            Focus::Regions => Focus::Patterns,
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

    /// Select a result by clicking on it (row is relative to results area content)
    pub fn select_result_at_row(&mut self, row: usize) {
        let index = self.results_scroll + row;
        if index < self.results.len() {
            self.selected_result = Some(index);
        }
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        self.selected_result = None;
    }

    // Log group navigation
    pub fn log_groups_down(&mut self) {
        if !self.log_groups.is_empty() && self.log_groups_cursor < self.log_groups.len() - 1 {
            self.log_groups_cursor += 1;
        }
    }

    pub fn log_groups_up(&mut self) {
        if self.log_groups_cursor > 0 {
            self.log_groups_cursor -= 1;
        }
    }

    pub fn toggle_log_group(&mut self) {
        if let Some(item) = self.log_groups.get_mut(self.log_groups_cursor) {
            item.selected = !item.selected;
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
}

pub async fn run_tui(searcher: MultiRegionSearcher) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    let result = run_app(&mut terminal, &mut app, &searcher).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
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
    // Initial load of log groups for default regions
    load_log_groups(app, searcher).await;

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        // Poll for events with timeout
        if event::poll(std::time::Duration::from_millis(100))? {
            let event = event::read()?;

            // Handle mouse events
            if let Event::Mouse(mouse) = event {
                handle_mouse_event(app, mouse, terminal.size()?);
            }

            if let Event::Key(key) = event {
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
                            // Force refresh log groups
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
                        // If leaving regions and they changed, reload log groups
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
                        match app.focus {
                            Focus::Regions => {
                                // Toggle region and reload
                                app.toggle_region();
                                load_log_groups(app, searcher).await;
                            }
                            Focus::LogGroups => {
                                app.toggle_log_group();
                            }
                            Focus::Results => {}
                            _ => {
                                // Trigger search
                                let patterns = app.get_patterns();
                                let groups = app.get_selected_log_groups();

                                if !patterns.is_empty() && !groups.is_empty() {
                                    app.search_state = SearchState::Searching;
                                    app.results.clear();

                                    let time_range =
                                        TimeRange::from_relative(app.time_range_value());
                                    match time_range {
                                        Ok(tr) => {
                                            let results = searcher
                                                .search_log_groups(
                                                    &groups,
                                                    &patterns,
                                                    tr.start,
                                                    tr.end,
                                                    app.limit_value(),
                                                )
                                                .await;

                                            let mut all_entries = Vec::new();
                                            let mut errors = Vec::new();

                                            for result in results {
                                                match result {
                                                    Ok(entries) => all_entries.extend(entries),
                                                    Err(e) => errors.push(e.to_string()),
                                                }
                                            }

                                            all_entries
                                                .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

                                            let count = all_entries.len();
                                            app.results = all_entries;
                                            app.results_scroll = 0;

                                            if errors.is_empty() {
                                                app.search_state = SearchState::Complete(count);
                                            } else if count > 0 {
                                                app.search_state = SearchState::Complete(count);
                                            } else {
                                                app.search_state =
                                                    SearchState::Error(errors.join("; "));
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
                    }
                    KeyCode::Char(' ') => {
                        match app.focus {
                            Focus::Regions => {
                                app.toggle_region();
                                // Don't reload immediately - wait for Tab or Enter
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
                        }
                    }
                    _ => {
                        // Focus-specific keybindings
                        match app.focus {
                            Focus::Patterns => {
                                handle_text_input(key.code, &mut app.patterns_input)
                            }
                            Focus::Regions => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => app.regions_up(),
                                KeyCode::Down | KeyCode::Char('j') => app.regions_down(),
                                _ => {}
                            },
                            Focus::LogGroups => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => app.log_groups_up(),
                                KeyCode::Down | KeyCode::Char('j') => app.log_groups_down(),
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

fn handle_mouse_event(app: &mut App, mouse: MouseEvent, size: ratatui::layout::Size) {
    // Calculate results area based on layout (same as render function)
    let selection_height: u16 = if app.focus == Focus::Regions || app.focus == Focus::LogGroups {
        10
    } else {
        3
    };

    // Layout: margin(1), then patterns(3), selection(variable), time_status(3), results(min 8), help(2)
    let results_top = 1 + 3 + selection_height + 3; // margin + patterns + selection + time_status
    let results_bottom = (size.height as u16).saturating_sub(1 + 2); // minus margin and help
    let results_left: u16 = 1; // margin
    let results_right = (size.width as u16).saturating_sub(1); // minus margin

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let x = mouse.column;
            let y = mouse.row;

            // Check if click is within results area (accounting for border)
            if y > results_top && y < results_bottom && x > results_left && x < results_right {
                let row_in_results = (y - results_top - 1) as usize; // -1 for top border
                app.select_result_at_row(row_in_results);
                app.focus = Focus::Results;
            }
        }
        MouseEventKind::ScrollUp => {
            if app.focus == Focus::Results {
                app.scroll_results_up();
            }
        }
        MouseEventKind::ScrollDown => {
            if app.focus == Focus::Results {
                app.scroll_results_down();
            }
        }
        _ => {}
    }
}
