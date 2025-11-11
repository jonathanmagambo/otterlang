//! Application state management for the TUI REPL

use std::collections::VecDeque;

/// Application state for the REPL TUI
#[derive(Debug, Clone)]
pub struct AppState {
    /// Current input buffer (multi-line)
    pub input: String,
    /// Cursor position in the input buffer (line, column)
    pub cursor: (usize, usize),
    /// Output buffer with scrollable history
    pub output: VecDeque<OutputEntry>,
    /// Command history
    pub history: Vec<String>,
    /// Current history navigation index (None = not navigating)
    pub history_index: Option<usize>,
    /// Scroll position in output buffer
    pub output_scroll: usize,
    /// Scroll position in history sidebar
    pub history_scroll: usize,
    /// Current mode
    pub mode: Mode,
    /// Error count
    pub error_count: usize,
    /// Whether to show help overlay
    pub show_help: bool,
    /// Whether we're in multi-line continuation mode
    pub continuation_mode: bool,
}

/// Output entry with metadata
#[derive(Debug, Clone)]
pub struct OutputEntry {
    pub content: String,
    pub kind: OutputKind,
    pub timestamp: Option<String>,
}

/// Type of output entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    Input,
    Output,
    Error,
    Info,
}

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Input,
    History,
    Help,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: (0, 0),
            output: VecDeque::new(),
            history: Vec::new(),
            history_index: None,
            output_scroll: 0,
            history_scroll: 0,
            mode: Mode::Input,
            error_count: 0,
            show_help: false,
            continuation_mode: false,
        }
    }

    /// Add an entry to the output buffer
    pub fn add_output(&mut self, content: String, kind: OutputKind) {
        use chrono::Local;
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        
        self.output.push_back(OutputEntry {
            content,
            kind,
            timestamp: Some(timestamp),
        });
        
        // Limit output buffer size
        if self.output.len() > 1000 {
            self.output.pop_front();
        }
        
        // Auto-scroll to bottom
        self.output_scroll = 0;
    }

    /// Add input to history
    pub fn add_to_history(&mut self, input: String) {
        if !input.trim().is_empty() {
            // Avoid duplicate consecutive entries
            if self.history.last().map(|s| s.as_str()) != Some(&input) {
                self.history.push(input);
            }
            // Limit history size
            if self.history.len() > 1000 {
                self.history.remove(0);
            }
        }
        self.history_index = None;
    }

    /// Navigate history up
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        
        let index = self.history_index.unwrap_or(self.history.len());
        if index > 0 {
            self.history_index = Some(index - 1);
            self.input = self.history[index - 1].clone();
            self.update_cursor_from_input();
        }
    }

    /// Navigate history down
    pub fn history_down(&mut self) {
        if let Some(index) = self.history_index {
            if index + 1 < self.history.len() {
                self.history_index = Some(index + 1);
                self.input = self.history[index + 1].clone();
            } else {
                self.history_index = None;
                self.input.clear();
            }
            self.update_cursor_from_input();
        }
    }

    /// Clear current input
    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = (0, 0);
        self.history_index = None;
        self.continuation_mode = false;
    }

    /// Update cursor position based on input string
    fn update_cursor_from_input(&mut self) {
        let lines: Vec<&str> = self.input.lines().collect();
        if lines.is_empty() {
            self.cursor = (0, 0);
        } else {
            let line_idx = self.cursor.0.min(lines.len().saturating_sub(1));
            let col_idx = self.cursor.1.min(lines[line_idx].len());
            self.cursor = (line_idx, col_idx);
        }
    }

    /// Get current line number (0-indexed)
    pub fn current_line(&self) -> usize {
        self.cursor.0
    }

    /// Get lines of input
    pub fn input_lines(&self) -> Vec<String> {
        if self.input.is_empty() {
            vec!["".to_string()]
        } else {
            self.input.lines().map(|s| s.to_string()).collect()
        }
    }

    /// Scroll output up
    pub fn scroll_output_up(&mut self, amount: usize) {
        let max_scroll = self.output.len().saturating_sub(10).max(0);
        self.output_scroll = (self.output_scroll + amount).min(max_scroll);
    }

    /// Scroll output down
    pub fn scroll_output_down(&mut self, amount: usize) {
        self.output_scroll = self.output_scroll.saturating_sub(amount);
    }

    /// Scroll history up
    pub fn scroll_history_up(&mut self, amount: usize) {
        let max_scroll = self.history.len().saturating_sub(10).max(0);
        self.history_scroll = (self.history_scroll + amount).min(max_scroll);
    }

    /// Scroll history down
    pub fn scroll_history_down(&mut self, amount: usize) {
        self.history_scroll = self.history_scroll.saturating_sub(amount);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

