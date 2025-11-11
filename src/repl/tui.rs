use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::time::Duration;

use crate::repl::engine::ReplEngine;
use crate::repl::events::{AppEvent, EventHandler, is_ctrl, matches_key};
use crate::repl::state::{AppState, Mode, OutputKind};
use crate::repl::ui::draw_ui;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    event_handler: EventHandler,
    engine: ReplEngine,
    state: AppState,
}

impl Tui {
    pub fn new(engine: ReplEngine) -> Result<Self> {
        enable_raw_mode().context("Failed to enable raw mode")?;
        
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("Failed to create terminal")?;

        let event_handler = EventHandler::new(Duration::from_millis(250));
        let state = AppState::new();

        Ok(Self {
            terminal,
            event_handler,
            engine,
            state,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        self.load_history();

        loop {
            match self.terminal.draw(|f| draw_ui(f, &mut self.state)) {
                Ok(_) => {}
                Err(e) => {
                    let _ = disable_raw_mode();
                    let _ = execute!(io::stdout(), LeaveAlternateScreen);
                    return Err(anyhow::anyhow!("Failed to draw UI: {}", e));
                }
            }

            match self.event_handler.next() {
                Ok(event) => {
                    match self.handle_event(event) {
                        Ok(true) => break,
                        Ok(false) => continue,
                        Err(e) => {
                            let _ = disable_raw_mode();
                            let _ = execute!(io::stdout(), LeaveAlternateScreen);
                            return Err(e).context("Event handling error");
                        }
                    }
                }
                Err(e) => {
                    let _ = disable_raw_mode();
                    let _ = execute!(io::stdout(), LeaveAlternateScreen);
                    return Err(anyhow::anyhow!("Event handler error: {}", e));
                }
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) -> Result<bool> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize(_width, _height) => Ok(false),
            AppEvent::Tick => Ok(false),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if matches_key(&key, KeyCode::Char('c'), KeyModifiers::CONTROL) {
            if self.state.mode == Mode::Input {
                self.state.clear_input();
            }
            return Ok(false);
        }

        if matches_key(&key, KeyCode::Char('d'), KeyModifiers::CONTROL) {
            return Ok(true);
        }

        if matches_key(&key, KeyCode::Char('l'), KeyModifiers::CONTROL) {
            self.state.output.clear();
            return Ok(false);
        }

        if key.code == KeyCode::Esc {
            if self.state.show_help {
                self.state.show_help = false;
                self.state.mode = Mode::Input;
            } else if self.state.mode == Mode::History {
                self.state.mode = Mode::Input;
            } else {
                self.state.clear_input();
            }
            return Ok(false);
        }

        match self.state.mode {
            Mode::Input => self.handle_input_mode(key)?,
            Mode::History => self.handle_history_mode(key)?,
            Mode::Help => {
                if key.code == KeyCode::Enter || key.code == KeyCode::Esc {
                    self.state.show_help = false;
                    self.state.mode = Mode::Input;
                }
            }
        }

        Ok(false)
    }

    fn handle_input_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                if is_ctrl(&key) {
                    self.execute_input();
                } else {
                    if self.needs_continuation() {
                        self.insert_newline_with_indent();
                    } else {
                        self.execute_input();
                    }
                }
            }
            KeyCode::Up => {
                if is_ctrl(&key) {
                    self.state.scroll_output_up(1);
                } else {
                    self.state.history_up();
                }
            }
            KeyCode::Down => {
                if is_ctrl(&key) {
                    self.state.scroll_output_down(1);
                } else {
                    self.state.history_down();
                }
            }
            KeyCode::Left => {
                if self.state.cursor.1 > 0 {
                    self.state.cursor.1 -= 1;
                } else if self.state.cursor.0 > 0 {
                    let lines: Vec<&str> = self.state.input.lines().collect();
                    self.state.cursor.0 -= 1;
                    self.state.cursor.1 = lines
                        .get(self.state.cursor.0)
                        .map(|l| l.len())
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                let lines: Vec<&str> = self.state.input.lines().collect();
                if let Some(current_line) = lines.get(self.state.cursor.0) {
                    if self.state.cursor.1 < current_line.len() {
                        self.state.cursor.1 += 1;
                    } else if self.state.cursor.0 + 1 < lines.len() {
                        self.state.cursor.0 += 1;
                        self.state.cursor.1 = 0;
                    }
                }
            }
            KeyCode::Backspace => {
                let lines: Vec<String> = self.state.input_lines();
                if let Some(current_line) = lines.get(self.state.cursor.0) {
                    if self.state.cursor.1 > 0 {
                        let mut new_line = current_line.clone();
                        new_line.remove(self.state.cursor.1 - 1);
                        self.state.input = lines
                            .iter()
                            .enumerate()
                            .map(|(i, l)| {
                                if i == self.state.cursor.0 {
                                    new_line.clone()
                                } else {
                                    l.clone()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.state.cursor.1 -= 1;
                    } else if self.state.cursor.0 > 0 {
                        let prev_line = lines[self.state.cursor.0 - 1].clone();
                        let current_line = current_line.clone();
                        let mut new_lines = lines.clone();
                        new_lines.remove(self.state.cursor.0);
                        new_lines[self.state.cursor.0 - 1] = format!("{}{}", prev_line, current_line);
                        self.state.input = new_lines.join("\n");
                        self.state.cursor.0 -= 1;
                        self.state.cursor.1 = prev_line.len();
                    }
                }
            }
            KeyCode::Delete => {
                let lines: Vec<String> = self.state.input_lines();
                if let Some(current_line) = lines.get(self.state.cursor.0) {
                    if self.state.cursor.1 < current_line.len() {
                        let mut new_line = current_line.clone();
                        new_line.remove(self.state.cursor.1);
                        self.state.input = lines
                            .iter()
                            .enumerate()
                            .map(|(i, l)| {
                                if i == self.state.cursor.0 {
                                    new_line.clone()
                                } else {
                                    l.clone()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                    } else if self.state.cursor.0 + 1 < lines.len() {
                        let current_line = current_line.clone();
                        let next_line = lines[self.state.cursor.0 + 1].clone();
                        let mut new_lines = lines.clone();
                        new_lines.remove(self.state.cursor.0 + 1);
                        new_lines[self.state.cursor.0] = format!("{}{}", current_line, next_line);
                        self.state.input = new_lines.join("\n");
                    }
                }
            }
            KeyCode::Tab => {
                self.insert_text("    ");
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
            }
            KeyCode::F(1) => {
                self.state.show_help = !self.state.show_help;
                if self.state.show_help {
                    self.state.mode = Mode::Help;
                } else {
                    self.state.mode = Mode::Input;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_history_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                self.state.scroll_history_up(1);
            }
            KeyCode::Down => {
                self.state.scroll_history_down(1);
            }
            KeyCode::Enter => {
                if let Some(selected) = self.state.history.get(
                    self.state.history_scroll.min(self.state.history.len().saturating_sub(1)),
                ) {
                    self.state.input = selected.clone();
                    self.state.mode = Mode::Input;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn insert_newline_with_indent(&mut self) {
        let lines: Vec<String> = self.state.input_lines();
        if let Some(current_line) = lines.get(self.state.cursor.0) {
            let current_indent: String = current_line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            
            let trimmed = current_line.trim();
            let should_increase_indent = trimmed.ends_with(':');
            
            let new_indent = if should_increase_indent {
                format!("{}    ", current_indent)
            } else {
                current_indent.clone()
            };
            
            let cursor_pos = self.state.cursor.1.min(current_line.len());
            let before_cursor = &current_line[..cursor_pos];
            let after_cursor = &current_line[cursor_pos..];
            
            let mut new_lines = lines.clone();
            new_lines[self.state.cursor.0] = format!("{}\n{}{}", before_cursor, new_indent, after_cursor);
            self.state.input = new_lines.join("\n");
            
            self.state.cursor.0 += 1;
            self.state.cursor.1 = new_indent.len();
            self.state.continuation_mode = true;
        } else {
            self.state.input.push('\n');
            self.state.cursor.0 += 1;
            self.state.cursor.1 = 0;
            self.state.continuation_mode = true;
        }
    }

    fn insert_char(&mut self, c: char) {
        let mut lines: Vec<String> = self.state.input_lines();
        if let Some(current_line) = lines.get_mut(self.state.cursor.0) {
            let col = self.state.cursor.1.min(current_line.len());
            current_line.insert(col, c);
            self.state.input = lines.join("\n");
            self.state.cursor.1 += 1;
        }
    }

    fn insert_text(&mut self, text: &str) {
        for c in text.chars() {
            self.insert_char(c);
        }
    }

    fn needs_continuation(&self) -> bool {
        let trimmed = self.state.input.trim();
        if trimmed.is_empty() {
            return false;
        }

        trimmed.ends_with(':') ||
        trimmed.ends_with('\\') ||
        (trimmed.matches('(').count() > trimmed.matches(')').count()) ||
        (trimmed.matches('[').count() > trimmed.matches(']').count()) ||
        (trimmed.matches('{').count() > trimmed.matches('}').count())
    }

    fn execute_input(&mut self) {
        let input = self.state.input.trim().to_string();
        if input.is_empty() {
            return;
        }

        self.state.add_output(format!("otter> {}", input), OutputKind::Input);
        self.state.add_to_history(input.clone());
        match self.engine.evaluate(&input) {
            Ok(result) => {
                match result.kind {
                    crate::repl::engine::EvaluationKind::Info => {
                        if let Some(output) = result.output {
                            self.state.add_output(output, OutputKind::Info);
                        }
                    }
                    crate::repl::engine::EvaluationKind::Success => {
                        if let Some(output) = result.output {
                            self.state.add_output(output, OutputKind::Output);
                        }
                    }
                    crate::repl::engine::EvaluationKind::Error => {
                        if let Some(output) = result.output {
                            self.state.error_count += 1;
                            self.state.add_output(output, OutputKind::Error);
                        }
                    }
                }
            }
            Err(e) => {
                self.state.error_count += 1;
                self.state.add_output(format!("error: {}", e), OutputKind::Error);
            }
        }

        self.state.clear_input();
        self.state.continuation_mode = false;
    }

    fn load_history(&mut self) {
        use directories::ProjectDirs;
        use std::fs;
        use std::io::Read;

        if let Some(proj_dirs) = ProjectDirs::from("com", "otterlang", "otterlang") {
            let history_dir = proj_dirs.config_dir();
            let history_file = history_dir.join("repl_history");

            if let Ok(mut file) = fs::File::open(&history_file) {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    for line in contents.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            self.state.history.push(trimmed.to_string());
                        }
                    }
                    if self.state.history.len() > 1000 {
                        self.state.history.drain(..self.state.history.len() - 1000);
                    }
                }
            }
        }
    }

    fn save_history(&self) {
        use directories::ProjectDirs;
        use std::fs;
        use std::io::Write;

        if let Some(proj_dirs) = ProjectDirs::from("com", "otterlang", "otterlang") {
            let history_dir = proj_dirs.config_dir();
            
            if let Err(_) = fs::create_dir_all(history_dir) {
                return;
            }

            let history_file = history_dir.join("repl_history");
            
            if let Ok(mut file) = fs::File::create(&history_file) {
                let start = self.state.history.len().saturating_sub(1000);
                for line in &self.state.history[start..] {
                    if let Err(_) = writeln!(file, "{}", line) {
                        return;
                    }
                }
            }
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        self.save_history();
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

